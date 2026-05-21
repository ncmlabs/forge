// Real-LLM live tests for T10.2 (#368) — gate_two revision loop.
//
// Mock harness (clone_dev_gate_two_tests.rs) proves the wiring; this
// test proves the *quality* of the revise-on-reject loop against a
// real Anthropic model. The mock LLM returns a deterministic string,
// so it cannot show that the planner actually consumed the rejection
// feedback — only the live model can.
//
// Gated on FORGE_LLM_LIVE=1 + ANTHROPIC_API_KEY (#288). A bare
// `cargo test` must never make paid API calls; the gate matches the
// existing pattern in tests/sensei_live_tests.rs:30-65.
//
// Run:
//   FORGE_LLM_LIVE=1 ANTHROPIC_API_KEY=sk-ant-... \
//     cargo test --test clone_dev_gate_two_live_tests -- --nocapture
//
// The test boots planner + gate_two + a probe agent under a fixture
// config that bypasses the Slack approval gate
// (gates_start_implementation = false → ImplementationApproved is
// auto-emitted, but we don't assert on that path; we drive the
// revise loop directly via ImplementationRejected). It captures the
// initial plan, injects ImplementationRejected with concrete
// feedback, and asserts that the revised plan (a) differs from the
// original and (b) contains a literal substring tied to the feedback.
// ──────────────────────────────────────────────────────────────────────

#![allow(clippy::await_holding_lock)]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use forge::compose;
use forge::config::ForgeConfig;
use forge::llm::registry::ProviderRegistry;
use forge::runtime::confidence::{ConfidentValue, Value};
use forge::runtime::event_bus::{EventBus, EventPayload};
use forge::runtime::executor::TaskExecutor;
use forge::runtime::instance_registry::InstanceRegistry;
use forge::runtime::skill_executor::SkillExecutor;
use forge::runtime::skill_registry::SkillRegistry;

const GATE_TWO_PATH: &str = "workflows/dev-cycle/gate_two.forge";
const TYPES_PATH: &str = "workflows/clone-dev/shared/types.forge";
const DEV_CYCLE_AGENTS_PATH: &str = "workflows/dev-cycle/agents.forge";

// LLM round-trip + handler dispatch headroom. Two real Anthropic
// calls run sequentially in the revise case, so this needs to be
// generous.
const HANDLER_WINDOW: Duration = Duration::from_secs(30);

static ENV_LOCK: Mutex<()> = Mutex::new(());

const TEST_HARNESS: &str = r#"#! boundary: server

warden test_ward
  manages [planner, gate_two, gate_probe]
  on stuck: escalate, self
  on crash: restart, self
  on timeout: restart, self

agent gate_probe
  memory
    plan_count: Number
    last_plan_a: Text
    last_plan_b: Text
  subscribe PlanReady
  subscribe ImplementationApproved
  subscribe ImplementationRejected
  subscribe PostMessage

  on start
    memory.plan_count = 0
    memory.last_plan_a = ""
    memory.last_plan_b = ""

  on PlanReady(issue_id: Text, repo: Text, title: Text, plan: Text, criteria: Text, branch: Text, channel: Text, callback_url: Text, test_cmd: Text)
    memory.plan_count = memory.plan_count + 1
    if memory.plan_count == 1
      memory.last_plan_a = plan
    if memory.plan_count == 2
      memory.last_plan_b = plan
    say "[probe] plan_ready#{memory.plan_count} issue={issue_id} plan={plan}"

  on ImplementationApproved(issue_id: Text, repo: Text, title: Text, plan: Text, criteria: Text, branch: Text, channel: Text, callback_url: Text, test_cmd: Text, decision_by: Text)
    say "[probe] approved issue={issue_id} decision_by={decision_by}"

  on ImplementationRejected(issue_id: Text, comment: Text, decision_by: Text)
    say "[probe] rejected issue={issue_id} by={decision_by} comment={comment}"

  on PostMessage(channel: Text, text: Text, thread_ts: Text)
    say "[probe] post_message channel={channel} text={text}"

system test_system
  use
    plan: planner
    gate2: gate_two
    probe: gate_probe
"#;

fn read_to_program(path: &str) -> (String, forge::ast::Program) {
    let src =
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("could not read {path}: {e}"));
    let prog = forge::parser::parse(&src).unwrap_or_else(|e| panic!("parse {path}: {e:?}"));
    (src, prog)
}

fn build_program() -> forge::ast::Program {
    let (gate_src, gate_prog) = read_to_program(GATE_TWO_PATH);
    let (types_src, types_prog) = read_to_program(TYPES_PATH);
    let (agents_src, agents_prog) = read_to_program(DEV_CYCLE_AGENTS_PATH);
    let harness_prog = forge::parser::parse(TEST_HARNESS).expect("parse harness");

    let files = vec![
        compose::SourceFile {
            path: TYPES_PATH.to_string(),
            source: types_src,
            program: types_prog,
        },
        compose::SourceFile {
            path: DEV_CYCLE_AGENTS_PATH.to_string(),
            source: agents_src,
            program: agents_prog,
        },
        compose::SourceFile {
            path: GATE_TWO_PATH.to_string(),
            source: gate_src,
            program: gate_prog,
        },
        compose::SourceFile {
            path: "test_harness.forge".to_string(),
            source: TEST_HARNESS.to_string(),
            program: harness_prog,
        },
    ];
    let composed = compose::merge_programs(&files).expect("merge");
    composed.program
}

/// Build a real Anthropic provider registry routed to claude-haiku.
/// Returns None when FORGE_LLM_LIVE!=1 or ANTHROPIC_API_KEY is unset
/// — the test gates on this and skips with a printed note rather
/// than failing.
fn haiku_registry() -> Option<Arc<ProviderRegistry>> {
    if std::env::var("FORGE_LLM_LIVE").ok().as_deref() != Some("1") {
        return None;
    }
    let api_key = std::env::var("ANTHROPIC_API_KEY").ok()?;
    if api_key.is_empty() {
        return None;
    }

    let mut config = ForgeConfig::default_mock_config();
    config.llm.default = "haiku".to_string();
    config.providers.clear();
    config.providers.insert(
        "haiku".to_string(),
        forge::config::ProviderConfig {
            type_: "anthropic".to_string(),
            model: Some("claude-haiku-4-5-20251001".to_string()),
            api_key: Some(api_key),
            base_url: None,
            fallback: None,
            capabilities: Some(forge::config::CapabilityOverride {
                max_context_tokens: None,
                quality_tier: Some(forge::llm::QualityTier::Balanced),
                local: None,
                cost_per_1k_input: None,
                cost_per_1k_output: None,
            }),
            headers: None,
            timeout_secs: None,
        },
    );

    ProviderRegistry::from_config(config).ok().map(Arc::new)
}

fn write_fixture_config(test_name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "forge-gate-two-live-{}-{}",
        std::process::id(),
        test_name
    ));
    std::fs::create_dir_all(&dir).expect("create fixture dir");
    let path = dir.join("clone-dev.toml");
    let toml = r#"
[org]
name = "test-org"

[slack]
default_channel  = "C-default"
approval_channel = ""

[github]
default_repo = "test-org/test-repo"

[gates]
create_issue                      = true
create_issue_timeout_mins         = 30
# Bypass the Slack gate so the real-LLM test focuses on the
# planner's revise loop. ImplementationApproved is auto-emitted
# but we drive ImplementationRejected directly.
start_implementation              = false
start_implementation_timeout_mins = 30

[defaults]
max_plan_revisions = 3

[llm.routing]
plan = "haiku"
"#;
    std::fs::write(&path, toml).expect("write fixture toml");
    std::env::set_var("FORGE_CLONEDEV_CONFIG", &path);
    path
}

struct Harness {
    event_bus: forge::runtime::event_bus::SharedEventBus,
    rx: tokio::sync::broadcast::Receiver<String>,
    _tracer: forge::tracer::Tracer,
}

async fn boot(providers: Arc<ProviderRegistry>) -> Harness {
    let program = build_program();

    let registry = SkillRegistry::new();
    let shared_registry = Arc::new(Mutex::new(registry));

    let (events_tx, events_rx) = tokio::sync::broadcast::channel::<String>(2048);
    let tracer = forge::tracer::Tracer::with_live(events_tx.clone());

    let skill_executor = Arc::new(
        SkillExecutor::new(providers.clone(), shared_registry)
            .with_tracer(Arc::new(tracer.clone())),
    );

    let executor = TaskExecutor::new(program, providers, Some(tracer.clone()))
        .with_config(ForgeConfig::default_mock_config())
        .with_skill_executor(skill_executor);

    let event_bus = EventBus::new_shared(executor.tracer().cloned());
    let instance_registry = Arc::new(tokio::sync::RwLock::new(InstanceRegistry::new()));

    let system_runtime = executor
        .build_system_runtime()
        .expect("build system runtime")
        .expect("test_system should produce a runtime")
        .with_shared_infrastructure(event_bus.clone(), instance_registry.clone());

    tokio::spawn(async move {
        let _ = system_runtime.start().await;
    });

    tokio::time::sleep(Duration::from_millis(800)).await;

    Harness {
        event_bus,
        rx: events_rx,
        _tracer: tracer,
    }
}

fn cv_text(s: &str) -> ConfidentValue {
    ConfidentValue::deterministic(Value::Text(s.into()))
}

async fn fire_issue_assigned(h: &Harness, issue_id: &str) {
    let mut fields = HashMap::new();
    fields.insert("issue_id".to_string(), cv_text(issue_id));
    fields.insert("repo".to_string(), cv_text("test-org/test-repo"));
    fields.insert("title".to_string(), cv_text("Add a hello-world endpoint"));
    fields.insert(
        "body".to_string(),
        cv_text(
            "Add a GET /hello endpoint that returns 'world' as plain text. \
             The endpoint must be unit-tested.",
        ),
    );
    fields.insert("channel".to_string(), cv_text("C-issue"));
    fields.insert(
        "callback_url".to_string(),
        cv_text("http://localhost:3300/webhook/approval"),
    );
    fields.insert("test_cmd".to_string(), cv_text("cargo test --quiet"));

    let payload = EventPayload {
        event_name: "IssueAssigned".to_string(),
        args: vec![],
        source_agent: "test_driver".to_string(),
        fields,
    };
    let bus = h.event_bus.read().await;
    bus.publish(&payload);
}

async fn fire_implementation_rejected(h: &Harness, issue_id: &str, comment: &str) {
    let mut fields = HashMap::new();
    fields.insert("issue_id".to_string(), cv_text(issue_id));
    fields.insert("comment".to_string(), cv_text(comment));
    fields.insert("decision_by".to_string(), cv_text("live-test-reviewer"));

    let payload = EventPayload {
        event_name: "ImplementationRejected".to_string(),
        args: vec![],
        source_agent: "test_driver".to_string(),
        fields,
    };
    let bus = h.event_bus.read().await;
    bus.publish(&payload);
}

async fn drain_frames(
    rx: &mut tokio::sync::broadcast::Receiver<String>,
    window: Duration,
) -> Vec<serde_json::Value> {
    let mut frames = Vec::<String>::new();
    let deadline = std::time::Instant::now() + window;
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        match tokio::time::timeout(remaining, rx.recv()).await {
            Ok(Ok(frame)) => frames.push(frame),
            Ok(Err(_)) | Err(_) => break,
        }
    }
    frames
        .iter()
        .filter_map(|f| serde_json::from_str(f).ok())
        .collect()
}

fn say_lines(frames: &[serde_json::Value]) -> Vec<String> {
    frames
        .iter()
        .filter(|v| v["event"] == "say")
        .filter_map(|v| v["text"].as_str())
        .map(String::from)
        .collect()
}

fn extract_plan(says: &[String], plan_index: usize) -> Option<String> {
    let prefix = format!("[probe] plan_ready#{plan_index} ");
    for s in says {
        if s.starts_with(&prefix) {
            // Format: "[probe] plan_ready#N issue={id} plan={...}"
            if let Some(idx) = s.find("plan=") {
                return Some(s[idx + "plan=".len()..].to_string());
            }
        }
    }
    None
}

// ── Live tests ────────────────────────────────────────────────────────

// Real-LLM revise loop: planner produces an initial plan, we reject
// with concrete feedback ("you forgot to add tests"), and the
// planner's `on ImplementationRejected` handler must call `reason
// "..." for plan` again with the feedback in the prompt and emit a
// fresh PlanReady. We assert (a) two PlanReady frames arrive and
// (b) the second plan differs from the first AND mentions the
// feedback signal.
#[tokio::test]
async fn replan_with_real_llm_produces_revised_plan() {
    let _guard = ENV_LOCK.lock().unwrap();

    let providers = match haiku_registry() {
        Some(r) => r,
        None => {
            eprintln!(
                "skipping replan_with_real_llm_produces_revised_plan: \
                 set FORGE_LLM_LIVE=1 + ANTHROPIC_API_KEY to run"
            );
            return;
        }
    };

    let _fixture = write_fixture_config("replan");
    let mut h = boot(providers).await;

    fire_issue_assigned(&h, "ISSUE-live").await;
    let frames_init = drain_frames(&mut h.rx, HANDLER_WINDOW).await;
    let says_init = say_lines(&frames_init);
    let plan_a = extract_plan(&says_init, 1).unwrap_or_else(|| {
        panic!(
            "expected first PlanReady with plan body; says={:?}",
            says_init
        )
    });
    assert!(
        !plan_a.trim().is_empty(),
        "first plan body must not be empty"
    );

    // Inject a rejection with very specific feedback. The revised
    // plan should incorporate the feedback signal.
    fire_implementation_rejected(
        &h,
        "ISSUE-live",
        "The plan doesn't mention writing automated tests. \
         Add an explicit step that uses 'cargo test' to verify the endpoint.",
    )
    .await;

    let frames_rev = drain_frames(&mut h.rx, HANDLER_WINDOW).await;
    let says_rev = say_lines(&frames_rev);
    let plan_b = extract_plan(&says_rev, 2).unwrap_or_else(|| {
        panic!(
            "expected second PlanReady after rejection; says={:?}",
            says_rev
        )
    });

    eprintln!("=== Plan A ===\n{plan_a}\n=== Plan B ===\n{plan_b}");

    assert_ne!(
        plan_a.trim(),
        plan_b.trim(),
        "revised plan must differ from original; planner did not consume feedback"
    );

    let plan_b_lower = plan_b.to_lowercase();
    let mentions_feedback = plan_b_lower.contains("test")
        || plan_b_lower.contains("cargo")
        || plan_b_lower.contains("verify");
    assert!(
        mentions_feedback,
        "revised plan should reference the rejection feedback (test/cargo/verify); got: {plan_b}"
    );
}
