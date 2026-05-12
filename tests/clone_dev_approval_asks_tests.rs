// Integration tests for T11.3 (#372) — release_manager's approval_asks
// counter in workflows/dev-cycle/agents.forge.
//
// The proof criterion for T11.3 is "approval-ask count for issue #10 <
// approval-ask count for issue #1". For that to mean anything, the
// counter has to be correct. This test pins down the wiring:
//
//   1. release_manager subscribes to IssueAssigned and resets
//      memory.approval_asks_count = 0 (so a re-run of the same issue
//      doesn't carry stale asks).
//   2. release_manager subscribes to ApprovalAsked (filtered to its
//      own issue_id) and increments the counter on each emit.
//   3. On PRMerged, release_manager emits TaskCompleted carrying
//      approval_asks = memory.approval_asks_count.
//
// The Stage-2 gates (gate_two + reviewer/gate_three) emit
// ApprovalAsked alongside their PostApproval. Gate-1 (Stage-1 issue
// triage) does NOT emit ApprovalAsked — it predates task_id
// allocation and is a separate Stage-1 learning signal.
//
// Env-lock pattern mirrors clone_dev_gate_two_tests.rs verbatim.
// ──────────────────────────────────────────────────────────────────────

#![allow(clippy::await_holding_lock)]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use forge::compose;
use forge::runtime::confidence::{ConfidentValue, Value};
use forge::runtime::event_bus::{EventBus, EventPayload};
use forge::runtime::executor::TaskExecutor;
use forge::runtime::instance_registry::InstanceRegistry;
use forge::runtime::skill_executor::SkillExecutor;
use forge::runtime::skill_registry::SkillRegistry;

const TYPES_PATH: &str = "workflows/clone-dev/shared/types.forge";
const DEV_CYCLE_AGENTS_PATH: &str = "workflows/dev-cycle/agents.forge";

// release_manager's `on TaskCompleted` runs `reason ... for review`
// against the mock provider, which resolves promptly but still needs a
// handler-dispatch window.
const HANDLER_WINDOW: Duration = Duration::from_millis(2500);

static ENV_LOCK: Mutex<()> = Mutex::new(());

const TEST_HARNESS: &str = r#"#! boundary: server

warden test_ward
  manages [release_manager, asks_probe]
  on stuck: escalate, self
  on crash: restart, self
  on timeout: restart, self

agent asks_probe
  memory
    last_task: Text
    last_approval_asks: Number
    task_completed_count: Number
  subscribe TaskCompleted

  on start
    memory.last_task = ""
    memory.last_approval_asks = -1
    memory.task_completed_count = 0

  on TaskCompleted(task_id: Text, repo: Text, outcome: Text, ci_passed_first_try: Bool, review_rounds: Number, time_to_merge: Number, reverted_within_7d: Bool, approval_asks: Number)
    memory.last_task = task_id
    memory.last_approval_asks = approval_asks
    memory.task_completed_count = memory.task_completed_count + 1
    say "[probe] task_completed task={task_id} approval_asks={approval_asks} count={memory.task_completed_count}"

system test_system
  use
    rm: release_manager
    probe: asks_probe
"#;

fn read_to_program(path: &str) -> (String, forge::ast::Program) {
    let src =
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("could not read {path}: {e}"));
    let prog = forge::parser::parse(&src).unwrap_or_else(|e| panic!("parse {path}: {e:?}"));
    (src, prog)
}

fn build_program() -> forge::ast::Program {
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
            path: "test_harness.forge".to_string(),
            source: TEST_HARNESS.to_string(),
            program: harness_prog,
        },
    ];
    let composed = compose::merge_programs(&files).expect("merge");
    composed.program
}

fn mock_registry() -> Arc<forge::llm::registry::ProviderRegistry> {
    let config = forge::config::ForgeConfig::default_mock_config();
    Arc::new(
        forge::llm::registry::ProviderRegistry::from_config(config)
            .expect("mock registry should build"),
    )
}

fn write_fixture_config(test_name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "forge-approval-asks-{}-{}",
        std::process::id(),
        test_name
    ));
    std::fs::create_dir_all(&dir).expect("create fixture dir");
    let path = dir.join("clone-dev.toml");
    let toml = r#"
[org]
name = "test-org"

[slack]
default_channel = "C-default"

[github]
default_repo = "test-org/test-repo"

[gates]
create_issue                      = true
create_issue_timeout_mins         = 30
start_implementation              = true
start_implementation_timeout_mins = 30
merge_pr                          = true
merge_pr_timeout_mins             = 30

[llm.routing]
classify = "mock"
plan     = "mock"
review   = "mock"
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

async fn boot() -> Harness {
    let program = build_program();

    let registry = SkillRegistry::new();
    let shared_registry = Arc::new(Mutex::new(registry));

    let (events_tx, events_rx) = tokio::sync::broadcast::channel::<String>(1024);
    let tracer = forge::tracer::Tracer::with_live(events_tx.clone());

    let providers = mock_registry();
    let skill_executor = Arc::new(
        SkillExecutor::new(providers.clone(), shared_registry)
            .with_tracer(Arc::new(tracer.clone())),
    );

    let executor = TaskExecutor::new(program, providers, Some(tracer.clone()))
        .with_config(forge::config::ForgeConfig::default_mock_config())
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

    tokio::time::sleep(Duration::from_millis(600)).await;

    Harness {
        event_bus,
        rx: events_rx,
        _tracer: tracer,
    }
}

fn cv_text(s: &str) -> ConfidentValue {
    ConfidentValue::deterministic(Value::Text(s.into()))
}

fn cv_bool(b: bool) -> ConfidentValue {
    ConfidentValue::deterministic(Value::Bool(b))
}

fn cv_num(n: f64) -> ConfidentValue {
    ConfidentValue::deterministic(Value::Number(n))
}

async fn fire_issue_assigned(h: &Harness, issue_id: &str, repo: &str) {
    let mut fields = HashMap::new();
    fields.insert("issue_id".to_string(), cv_text(issue_id));
    fields.insert("repo".to_string(), cv_text(repo));
    fields.insert("title".to_string(), cv_text("approval-asks counter test"));
    fields.insert(
        "body".to_string(),
        cv_text("Acceptance: counter increments on each Stage-2 ask."),
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

async fn fire_approval_asked(h: &Harness, task_id: &str, gate: &str) {
    let mut fields = HashMap::new();
    fields.insert("task_id".to_string(), cv_text(task_id));
    fields.insert("gate".to_string(), cv_text(gate));

    let payload = EventPayload {
        event_name: "ApprovalAsked".to_string(),
        args: vec![],
        source_agent: "test_driver".to_string(),
        fields,
    };
    let bus = h.event_bus.read().await;
    bus.publish(&payload);
}

async fn fire_pr_merged(h: &Harness, issue_id: &str, repo: &str, review_rounds: f64) {
    let mut fields = HashMap::new();
    fields.insert("issue_id".to_string(), cv_text(issue_id));
    fields.insert("repo".to_string(), cv_text(repo));
    fields.insert("branch".to_string(), cv_text("clone-dev/test"));
    fields.insert("ci_passed_first_try".to_string(), cv_bool(true));
    fields.insert("review_rounds".to_string(), cv_num(review_rounds));

    let payload = EventPayload {
        event_name: "PRMerged".to_string(),
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

fn count_emits(frames: &[serde_json::Value], event: &str, source: &str) -> usize {
    frames
        .iter()
        .filter(|v| v["event"] == event && v["source_agent"] == source)
        .count()
}

fn find_task_completed_approval_asks(frames: &[serde_json::Value]) -> Option<u64> {
    let probe_line = frames
        .iter()
        .filter(|v| v["event"] == "say")
        .filter_map(|v| v["text"].as_str())
        .find(|s| s.starts_with("[probe] task_completed"))?;
    let approval_asks_part = probe_line
        .split_whitespace()
        .find(|t| t.starts_with("approval_asks="))?;
    let value = approval_asks_part.trim_start_matches("approval_asks=");
    value.parse::<u64>().ok()
}

// ── Tests ─────────────────────────────────────────────────────────────

// Happy path: IssueAssigned → 2 ApprovalAsked (gate_two + gate_three)
// → PRMerged → TaskCompleted carries approval_asks=2.
#[tokio::test]
async fn counter_accumulates_gate_two_and_gate_three_asks() {
    let _guard = ENV_LOCK.lock().unwrap();
    let _fixture = write_fixture_config("happy");

    let mut h = boot().await;

    fire_issue_assigned(&h, "ISSUE-1", "test-org/test-repo").await;
    let _ = drain_frames(&mut h.rx, HANDLER_WINDOW).await;

    fire_approval_asked(&h, "ISSUE-1", "gate_two").await;
    fire_approval_asked(&h, "ISSUE-1", "gate_three").await;
    let _ = drain_frames(&mut h.rx, HANDLER_WINDOW).await;

    fire_pr_merged(&h, "ISSUE-1", "test-org/test-repo", 1.0).await;
    let frames = drain_frames(&mut h.rx, HANDLER_WINDOW).await;
    let says = say_lines(&frames);

    assert_eq!(
        count_emits(&frames, "TaskCompleted", "release_manager"),
        1,
        "exactly one TaskCompleted expected on PRMerged; says={says:?}"
    );
    let count = find_task_completed_approval_asks(&frames)
        .unwrap_or_else(|| panic!("probe should observe TaskCompleted; says={says:?}"));
    assert_eq!(
        count, 2,
        "approval_asks must equal 2 (gate_two + gate_three); says={says:?}"
    );
}

// Resets on new IssueAssigned: a second issue starts fresh at 0.
#[tokio::test]
async fn counter_resets_on_new_issue() {
    let _guard = ENV_LOCK.lock().unwrap();
    let _fixture = write_fixture_config("reset");

    let mut h = boot().await;

    // First issue: 3 asks → merged → approval_asks=3
    fire_issue_assigned(&h, "ISSUE-A", "test-org/test-repo").await;
    let _ = drain_frames(&mut h.rx, HANDLER_WINDOW).await;
    fire_approval_asked(&h, "ISSUE-A", "gate_two").await;
    fire_approval_asked(&h, "ISSUE-A", "gate_two").await;
    fire_approval_asked(&h, "ISSUE-A", "gate_three").await;
    let _ = drain_frames(&mut h.rx, HANDLER_WINDOW).await;
    fire_pr_merged(&h, "ISSUE-A", "test-org/test-repo", 2.0).await;
    let frames_a = drain_frames(&mut h.rx, HANDLER_WINDOW).await;
    let count_a = find_task_completed_approval_asks(&frames_a).unwrap_or_else(|| {
        panic!(
            "issue A: probe missed TaskCompleted; says={:?}",
            say_lines(&frames_a)
        )
    });
    assert_eq!(count_a, 3, "issue A should record 3 approval asks");

    // Second issue: 1 ask only → merged → approval_asks=1 (fresh start)
    fire_issue_assigned(&h, "ISSUE-B", "test-org/test-repo").await;
    let _ = drain_frames(&mut h.rx, HANDLER_WINDOW).await;
    fire_approval_asked(&h, "ISSUE-B", "gate_three").await;
    let _ = drain_frames(&mut h.rx, HANDLER_WINDOW).await;
    fire_pr_merged(&h, "ISSUE-B", "test-org/test-repo", 1.0).await;
    let frames_b = drain_frames(&mut h.rx, HANDLER_WINDOW).await;
    let count_b = find_task_completed_approval_asks(&frames_b).unwrap_or_else(|| {
        panic!(
            "issue B: probe missed TaskCompleted; says={:?}",
            say_lines(&frames_b)
        )
    });
    assert_eq!(
        count_b, 1,
        "issue B should restart counter at 0 and reach 1, not 4 (carry-over from A)"
    );
}

// Filter scope: ApprovalAsked for a different task_id is NOT counted.
// release_manager subscribes with `where task_id == memory.issue_id`.
#[tokio::test]
async fn counter_ignores_asks_for_other_issues() {
    let _guard = ENV_LOCK.lock().unwrap();
    let _fixture = write_fixture_config("scope");

    let mut h = boot().await;

    fire_issue_assigned(&h, "ISSUE-mine", "test-org/test-repo").await;
    let _ = drain_frames(&mut h.rx, HANDLER_WINDOW).await;

    // Asks for the wrong task — must be ignored by the filter.
    fire_approval_asked(&h, "ISSUE-someone-else", "gate_two").await;
    fire_approval_asked(&h, "ISSUE-someone-else", "gate_three").await;
    // One ask for the right task.
    fire_approval_asked(&h, "ISSUE-mine", "gate_two").await;
    let _ = drain_frames(&mut h.rx, HANDLER_WINDOW).await;

    fire_pr_merged(&h, "ISSUE-mine", "test-org/test-repo", 1.0).await;
    let frames = drain_frames(&mut h.rx, HANDLER_WINDOW).await;
    let count = find_task_completed_approval_asks(&frames)
        .unwrap_or_else(|| panic!("probe missed TaskCompleted; says={:?}", say_lines(&frames)));
    assert_eq!(
        count, 1,
        "only the matching-task_id ask should count; cross-issue noise must be filtered"
    );
}

// Zero-asks path: a task that flows straight through (no human gates)
// still produces TaskCompleted with approval_asks=0.
#[tokio::test]
async fn counter_is_zero_when_no_asks_fired() {
    let _guard = ENV_LOCK.lock().unwrap();
    let _fixture = write_fixture_config("zero");

    let mut h = boot().await;

    fire_issue_assigned(&h, "ISSUE-clean", "test-org/test-repo").await;
    let _ = drain_frames(&mut h.rx, HANDLER_WINDOW).await;

    fire_pr_merged(&h, "ISSUE-clean", "test-org/test-repo", 1.0).await;
    let frames = drain_frames(&mut h.rx, HANDLER_WINDOW).await;
    let count = find_task_completed_approval_asks(&frames)
        .unwrap_or_else(|| panic!("probe missed TaskCompleted; says={:?}", say_lines(&frames)));
    assert_eq!(count, 0, "approval_asks must be 0 when no gates fired");
}
