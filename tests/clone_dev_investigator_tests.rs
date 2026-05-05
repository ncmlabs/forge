// Integration test for T9.2 (#363) — domain investigator agents
// (code / ops / security) under workflows/clone-dev/stage1/investigators/.
//
// Verifies the routing slice of Stage 1:
//   InvestigationRequested(domain) →
//     <domain>_investigator (subscribe filter where domain == "<self>") →
//       reason for <domain>_investigate (mocked) →
//       emit Finding(thread_ts, domain, summary, evidence, confidence,
//                    suggested_action)
//
// Mirrors tests/clone_dev_intake_tests.rs for harness wiring (compose
// source files, MockProvider per phase, EventBus + InstanceRegistry +
// system runtime, fire-and-drain SSE frames).
//
// Skips:
//   - The code_investigator's `skill.github.list_issues` call. End-to-end
//     evidence content is verified by the validation manifest case
//     ("clone-dev top-level assembly") plus a manual `forge serve` smoke
//     per the plan; mocking the agentic github skill in-process would
//     dwarf this slice's scope.
//   - HTTP layer (covered by webhook/e2e tests).
// ──────────────────────────────────────────────────────────────────────

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use forge::compose;
use forge::llm::providers::mock::MockProvider;
use forge::llm::registry::ProviderRegistry;
use forge::runtime::confidence::{ConfidentValue, Value};
use forge::runtime::event_bus::EventBus;
use forge::runtime::executor::TaskExecutor;
use forge::runtime::instance_registry::InstanceRegistry;

const CODE_INV_PATH: &str = "workflows/clone-dev/stage1/investigators/code_investigator.forge";
const OPS_INV_PATH: &str = "workflows/clone-dev/stage1/investigators/ops_investigator.forge";
const SEC_INV_PATH: &str = "workflows/clone-dev/stage1/investigators/security_investigator.forge";

// Test harness: declares the three events the investigators reference
// (InvestigationRequested, Finding, WardenEscalation) and a single
// fire_investigation endpoint that emits InvestigationRequested directly,
// standing in for mastermind_intake's classify-and-emit path.
//
// WardenEscalation is redeclared here (rather than sourcing slack-adapter)
// because slack_adapter calls `skill.slack.*` which is not wired in the
// test sandbox. The investigators emit the event from their `if stuck for
// 3 turns` block; this test doesn't drive that path, but the declaration
// is required for the agents to compose.
//
// `investigators_ward` is redeclared at minimal-policy form — the real
// definition lives in workflows/clone-dev/main.forge but we don't source
// main.forge in this test (it would pull in eleven other agents).
const TEST_HARNESS_SRC: &str = r#"#! boundary: server

event InvestigationRequested
  thread_ts: Text
  domain: Text
  context: Text
  channel: Text

event Finding
  thread_ts: Text
  domain: Text
  summary: Text
  evidence: Text[]
  confidence: Number
  suggested_action: Text

event WardenEscalation
  agent_id: Text
  cause: Text
  detail: Text
  channel: Text

warden investigators_ward
  manages [code_investigator, ops_investigator, security_investigator]
  on stuck: escalate, self
  on crash: restart, self
  on timeout: restart, self

system test_system
  use
    code_inv: code_investigator
    ops_inv: ops_investigator
    sec_inv: security_investigator

endpoint fire_investigation(thread_ts: Text, domain: Text, context: Text, channel: Text) -> Text
  emit InvestigationRequested(thread_ts: thread_ts, domain: domain, context: context, channel: channel)
  give "queued"
"#;

/// Mock provider with canned responses keyed by prompt substrings. Each
/// investigator's `reason for <domain>_investigate` resolves through one
/// of the three phase chains, all backed by this single mock.
fn build_investigator_registry() -> Arc<ProviderRegistry> {
    let mock = MockProvider::new("mock-investigate")
        .with_response(
            "Investigate the ops issue",
            "ops summary: api-gateway latency spike correlates with deploy 12 min ago",
        )
        .with_response(
            "Investigate the security issue",
            "security summary: token grant scope wider than required; review IAM policy",
        )
        .with_response(
            "Plan a code investigation",
            "Inspect handler.rs, look at retry logic in retry.rs",
        )
        .with_response(
            "Summarize the code investigation",
            "code summary: retry budget exhausted in handler.rs",
        )
        .with_default("mock fallback");

    let mut registry = ProviderRegistry::new("mock-investigate");
    registry.register("mock-investigate", Arc::new(mock));
    registry.set_phase_chain("ops_investigate", vec!["mock-investigate".into()]);
    registry.set_phase_chain("code_investigate", vec!["mock-investigate".into()]);
    registry.set_phase_chain("security_investigate", vec!["mock-investigate".into()]);
    Arc::new(registry)
}

fn read_to_program(path: &str) -> (String, forge::ast::Program) {
    let src =
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("could not read {path}: {e}"));
    let prog = forge::parser::parse(&src).unwrap_or_else(|e| panic!("parse {path}: {e:?}"));
    (src, prog)
}

fn build_program() -> forge::ast::Program {
    let (code_src, code_prog) = read_to_program(CODE_INV_PATH);
    let (ops_src, ops_prog) = read_to_program(OPS_INV_PATH);
    let (sec_src, sec_prog) = read_to_program(SEC_INV_PATH);
    let harness_prog = forge::parser::parse(TEST_HARNESS_SRC).expect("parse harness");

    let files = vec![
        compose::SourceFile {
            path: CODE_INV_PATH.to_string(),
            source: code_src,
            program: code_prog,
        },
        compose::SourceFile {
            path: OPS_INV_PATH.to_string(),
            source: ops_src,
            program: ops_prog,
        },
        compose::SourceFile {
            path: SEC_INV_PATH.to_string(),
            source: sec_src,
            program: sec_prog,
        },
        compose::SourceFile {
            path: "test_harness.forge".to_string(),
            source: TEST_HARNESS_SRC.to_string(),
            program: harness_prog,
        },
    ];
    let composed = compose::merge_programs(&files).expect("merge");
    let diagnostics = forge::checker::check_all(&composed.program, "test_harness.forge");
    let errors: Vec<_> = diagnostics
        .iter()
        .filter(|d| d.kind == forge::diagnostic::DiagnosticKind::Error)
        .collect();
    assert!(errors.is_empty(), "checker errors: {errors:#?}");
    composed.program
}

/// Boot the runtime, fire one InvestigationRequested, and drain SSE
/// frames for a fixed window. Returns the parsed JSON frames in arrival
/// order.
async fn fire_one(domain: &str, thread_ts: &str, context: &str) -> Vec<serde_json::Value> {
    let program = build_program();
    let (events_tx, mut events_rx) = tokio::sync::broadcast::channel::<String>(512);
    let tracer = forge::tracer::Tracer::with_live(events_tx.clone());

    let executor = TaskExecutor::new(program, build_investigator_registry(), Some(tracer.clone()))
        .with_config(forge::config::ForgeConfig::default_mock_config());

    let event_bus = EventBus::new_shared(executor.tracer().cloned());
    let instance_registry = Arc::new(tokio::sync::RwLock::new(InstanceRegistry::new()));

    let system_runtime = executor
        .build_system_runtime()
        .expect("build system runtime")
        .expect("test_system should produce a runtime")
        .with_shared_infrastructure(event_bus.clone(), instance_registry.clone());

    let executor = executor.with_event_bus(event_bus.clone());

    tokio::spawn(async move {
        let _ = system_runtime.start().await;
    });

    // Let subscriptions register and `on start` run for all three agents.
    tokio::time::sleep(Duration::from_millis(300)).await;

    let mut args = HashMap::new();
    args.insert(
        "thread_ts".to_string(),
        ConfidentValue::deterministic(Value::Text(thread_ts.into())),
    );
    args.insert(
        "domain".to_string(),
        ConfidentValue::deterministic(Value::Text(domain.into())),
    );
    args.insert(
        "context".to_string(),
        ConfidentValue::deterministic(Value::Text(context.into())),
    );
    args.insert(
        "channel".to_string(),
        ConfidentValue::deterministic(Value::Text("C-test".into())),
    );

    let _ = executor
        .exec_endpoint("fire_investigation", args, None)
        .await
        .expect("endpoint dispatch");

    drain_frames(&mut events_rx, Duration::from_millis(2000)).await
}

async fn fire_two_turns(domain: &str, thread_ts: &str) -> Vec<serde_json::Value> {
    let program = build_program();
    let (events_tx, mut events_rx) = tokio::sync::broadcast::channel::<String>(1024);
    let tracer = forge::tracer::Tracer::with_live(events_tx.clone());

    let executor = TaskExecutor::new(program, build_investigator_registry(), Some(tracer.clone()))
        .with_config(forge::config::ForgeConfig::default_mock_config());

    let event_bus = EventBus::new_shared(executor.tracer().cloned());
    let instance_registry = Arc::new(tokio::sync::RwLock::new(InstanceRegistry::new()));

    let system_runtime = executor
        .build_system_runtime()
        .expect("build system runtime")
        .expect("test_system should produce a runtime")
        .with_shared_infrastructure(event_bus.clone(), instance_registry.clone());

    let executor = executor.with_event_bus(event_bus.clone());

    tokio::spawn(async move {
        let _ = system_runtime.start().await;
    });

    tokio::time::sleep(Duration::from_millis(300)).await;

    for turn in 1..=2 {
        let mut args = HashMap::new();
        args.insert(
            "thread_ts".to_string(),
            ConfidentValue::deterministic(Value::Text(thread_ts.into())),
        );
        args.insert(
            "domain".to_string(),
            ConfidentValue::deterministic(Value::Text(domain.into())),
        );
        args.insert(
            "context".to_string(),
            ConfidentValue::deterministic(Value::Text(format!("turn {turn}"))),
        );
        args.insert(
            "channel".to_string(),
            ConfidentValue::deterministic(Value::Text("C-test".into())),
        );
        let _ = executor
            .exec_endpoint("fire_investigation", args, None)
            .await
            .expect("endpoint dispatch");
        // Same rationale as clone_dev_intake_tests:fire_two_turns_same_thread —
        // give the correlation index time to persist before the next turn.
        tokio::time::sleep(Duration::from_millis(400)).await;
    }

    drain_frames(&mut events_rx, Duration::from_millis(1500)).await
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

// ── Frame helpers ─────────────────────────────────────────────────────

fn emits_event_from(frames: &[serde_json::Value], event: &str, source_agent: &str) -> bool {
    frames
        .iter()
        .any(|v| v["event"] == event && v["source_agent"] == source_agent)
}

fn say_lines(frames: &[serde_json::Value]) -> Vec<String> {
    frames
        .iter()
        .filter(|v| v["event"] == "say")
        .filter_map(|v| v["text"].as_str())
        .map(String::from)
        .collect()
}

// ── Tests ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn ops_investigator_handles_ops_domain_and_emits_finding() {
    let frames = fire_one("ops", "T-ops", "logs are spiking on api-gateway").await;

    let says = say_lines(&frames);
    assert!(
        says.iter()
            .any(|s| s.contains("[ops_investigator] thread=T-ops")),
        "ops investigator should log the inbound event; says={says:#?}"
    );
    assert!(
        emits_event_from(&frames, "Finding", "ops_investigator"),
        "ops_investigator should emit Finding; frames={frames:#?}"
    );
}

#[tokio::test]
async fn security_investigator_handles_security_domain_and_emits_finding() {
    let frames = fire_one("security", "T-sec", "saw an unauthorized 200 in audit logs").await;

    let says = say_lines(&frames);
    assert!(
        says.iter()
            .any(|s| s.contains("[security_investigator] thread=T-sec")),
        "security investigator should log the inbound event; says={says:#?}"
    );
    assert!(
        emits_event_from(&frames, "Finding", "security_investigator"),
        "security_investigator should emit Finding; frames={frames:#?}"
    );
}

#[tokio::test]
async fn domain_filter_is_exclusive_for_ops_inbound() {
    // Per DoD: "InvestigationRequested(domain: \"ops\") lands in
    // ops_investigator only". Verify code/security investigators do
    // NOT emit a Finding when an ops-tagged event fires.
    let frames = fire_one("ops", "T-excl", "ops only").await;

    assert!(
        emits_event_from(&frames, "Finding", "ops_investigator"),
        "ops finding expected; frames={frames:#?}"
    );
    assert!(
        !emits_event_from(&frames, "Finding", "code_investigator"),
        "code_investigator must not emit on ops-tagged event; frames={frames:#?}"
    );
    assert!(
        !emits_event_from(&frames, "Finding", "security_investigator"),
        "security_investigator must not emit on ops-tagged event; frames={frames:#?}"
    );
}

#[tokio::test]
async fn domain_filter_is_exclusive_for_security_inbound() {
    let frames = fire_one("security", "T-secexcl", "security only").await;

    assert!(
        emits_event_from(&frames, "Finding", "security_investigator"),
        "security finding expected; frames={frames:#?}"
    );
    assert!(
        !emits_event_from(&frames, "Finding", "code_investigator"),
        "code_investigator must not emit on security-tagged event; frames={frames:#?}"
    );
    assert!(
        !emits_event_from(&frames, "Finding", "ops_investigator"),
        "ops_investigator must not emit on security-tagged event; frames={frames:#?}"
    );
}

#[tokio::test]
async fn two_ops_investigations_each_produce_a_finding() {
    // Without per-domain correlation (omitted intentionally — see the
    // comment in ops_investigator.forge), each InvestigationRequested
    // spawns a fresh handler invocation. Both turns must still produce
    // a Finding and log against the matching thread_ts.
    let frames = fire_two_turns("ops", "T-corr").await;

    let findings: Vec<&serde_json::Value> = frames
        .iter()
        .filter(|v| v["event"] == "Finding" && v["source_agent"] == "ops_investigator")
        .collect();
    assert!(
        findings.len() >= 2,
        "two fires should produce at least two Findings; got {} (frames={frames:#?})",
        findings.len()
    );

    let says = say_lines(&frames);
    let thread_lines: Vec<&String> = says
        .iter()
        .filter(|s| s.contains("[ops_investigator] thread=T-corr"))
        .collect();
    assert!(
        thread_lines.len() >= 2,
        "expected two log lines for the same thread; got {}",
        thread_lines.len()
    );
}
