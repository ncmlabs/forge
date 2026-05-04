// Integration test for T9.1 (#362) — mastermind_intake conversational
// classifier for Stage-1 fuzzy intake.
//
// Verifies the full Stage-1 loop on the event bus:
//   POST /devops_request → emit DevOpsRequest →
//     mastermind_intake (correlate on thread_ts → wake) →
//     classify_request (`reason for classify`) →
//     emit InvestigationRequested OR PostMessage →
//     echo_investigator → emit Finding (correlate match wakes intake) →
//     emit PostMessage(channel, text, thread_ts).
//
// Mirrors tests/label_router_integration_tests.rs for harness wiring
// (compose source files, build_program, EventBus + InstanceRegistry +
// system runtime). The MockProvider's pattern-matching API
// (src/llm/providers/mock.rs:43) lets us return deterministic
// classifications based on prompt substrings.
//
// Skips:
//   - HTTP /wake/... HMAC layer (covered by tests/webhook_integration_tests.rs)
//   - Real Slack delivery (PostMessage emission is verified; no skill.slack.* call)
//   - Cross-process redb rehydration (covered for the underlying primitive
//     by examples/agents/wake-rehydration-smoke and #334 tests; this test
//     verifies in-process correlation reuses the same agent instance)
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

const INTAKE_PATH: &str = "workflows/clone-dev/stage1/mastermind_intake.forge";

// Test harness: declares the four events mastermind_intake references
// (DevOpsRequest, InvestigationRequested, Finding, PostMessage), plus a
// minimal echo_investigator that closes the InvestigationRequested →
// Finding loop without needing the real T9.2 (#363) investigators.
//
// PostMessage is redeclared here (rather than sourcing slack-adapter)
// because slack_adapter calls `skill.slack.*` which won't work in the
// test sandbox — and the test only needs to observe the *emit*, not the
// downstream Slack API call.
const TEST_HARNESS_SRC: &str = r#"#! boundary: server

event DevOpsRequest
  channel: Text
  user: Text
  text: Text
  thread_ts: Text
  message_ts: Text

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

event PostMessage
  channel: Text
  text: Text
  thread_ts: Text

agent echo_investigator
  memory
    last_thread: Text
    last_domain: Text
  subscribe InvestigationRequested where domain == "ops"

  on start
    memory.last_thread = ""
    memory.last_domain = ""
    say "[echo_investigator] ready"

  on InvestigationRequested(thread_ts: Text, domain: Text, context: Text, channel: Text)
    memory.last_thread = thread_ts
    memory.last_domain = domain
    say "ECHO|thread_ts={thread_ts}|domain={domain}|context={context}"
    emit Finding(thread_ts: thread_ts, domain: "ops", summary: "echo: {context}", evidence: [], confidence: 0.9, suggested_action: "answer")

warden test_warden
  manages [mastermind_intake, echo_investigator]
  on stuck: nudge, self
  on timeout: restart, self
  on crash: restart, self
  on hallucination: nudge, self
  on contradiction: nudge, self
  on budget: nudge, self

system test_system
  use
    intake: mastermind_intake
    investigator: echo_investigator

endpoint fire_devops(channel: Text, user: Text, text: Text, thread_ts: Text, message_ts: Text) -> Text
  emit DevOpsRequest(channel: channel, user: user, text: text, thread_ts: thread_ts, message_ts: message_ts)
  give "queued"
"#;

/// Build a mock provider that returns a specific classification for the
/// primary classify, a specific domain for the domain classify, and a
/// canned clarification text for the ask_clarification fall-through.
/// Patterns are matched against the prompt text via substring search
/// (see src/llm/providers/mock.rs:90–94).
fn build_intake_registry(classification: &str, domain: &str) -> Arc<ProviderRegistry> {
    let mock = MockProvider::new("mock-classify")
        .with_response("Classify this DevOps request", classification)
        .with_response("Pick one domain", domain)
        .with_response(
            "Ask one focused clarifying",
            "Could you share a screenshot or log line?",
        )
        .with_default("mock fallback");

    let mut registry = ProviderRegistry::new("mock-classify");
    registry.register("mock-classify", Arc::new(mock));
    // mastermind_intake's three `reason ... for classify` calls (and the
    // un-phased ask_clarification reason) all resolve through this chain
    // because the default provider also points at mock-classify.
    registry.set_phase_chain("classify", vec!["mock-classify".into()]);
    Arc::new(registry)
}

fn build_program() -> forge::ast::Program {
    let intake_src = std::fs::read_to_string(INTAKE_PATH)
        .unwrap_or_else(|e| panic!("could not read {INTAKE_PATH}: {e}"));
    let intake_prog = forge::parser::parse(&intake_src).expect("parse mastermind_intake.forge");
    let harness_prog = forge::parser::parse(TEST_HARNESS_SRC).expect("parse harness");
    let files = vec![
        compose::SourceFile {
            path: INTAKE_PATH.to_string(),
            source: intake_src,
            program: intake_prog,
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

/// Boot the system, fire one DevOpsRequest, and drain SSE frames for a
/// fixed window. Returns the parsed JSON frames in arrival order.
async fn fire_and_drain_one(
    classification: &str,
    domain: &str,
    thread_ts: &str,
    text: &str,
) -> Vec<serde_json::Value> {
    let program = build_program();
    let (events_tx, mut events_rx) = tokio::sync::broadcast::channel::<String>(512);
    let tracer = forge::tracer::Tracer::with_live(events_tx.clone());

    let executor = TaskExecutor::new(
        program,
        build_intake_registry(classification, domain),
        Some(tracer.clone()),
    )
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

    // Let the system runtime register subscriptions + run `on start`.
    tokio::time::sleep(Duration::from_millis(300)).await;

    let mut args = HashMap::new();
    args.insert(
        "channel".to_string(),
        ConfidentValue::deterministic(Value::Text("C123".into())),
    );
    args.insert(
        "user".to_string(),
        ConfidentValue::deterministic(Value::Text("U456".into())),
    );
    args.insert(
        "text".to_string(),
        ConfidentValue::deterministic(Value::Text(text.into())),
    );
    args.insert(
        "thread_ts".to_string(),
        ConfidentValue::deterministic(Value::Text(thread_ts.into())),
    );
    args.insert(
        "message_ts".to_string(),
        ConfidentValue::deterministic(Value::Text(format!("{thread_ts}-m1"))),
    );

    let _ = executor
        .exec_endpoint("fire_devops", args, None)
        .await
        .expect("endpoint dispatch");

    drain_frames(&mut events_rx, Duration::from_millis(2000)).await
}

/// Boot the system, fire two DevOpsRequest events on the same thread,
/// and drain SSE frames. Used by the multi-turn correlation test.
async fn fire_two_turns_same_thread(
    classification: &str,
    domain: &str,
    thread_ts: &str,
) -> Vec<serde_json::Value> {
    let program = build_program();
    let (events_tx, mut events_rx) = tokio::sync::broadcast::channel::<String>(1024);
    let tracer = forge::tracer::Tracer::with_live(events_tx.clone());

    let executor = TaskExecutor::new(
        program,
        build_intake_registry(classification, domain),
        Some(tracer.clone()),
    )
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
            "channel".to_string(),
            ConfidentValue::deterministic(Value::Text("C123".into())),
        );
        args.insert(
            "user".to_string(),
            ConfidentValue::deterministic(Value::Text("U456".into())),
        );
        args.insert(
            "text".to_string(),
            ConfidentValue::deterministic(Value::Text(format!("turn {turn} message"))),
        );
        args.insert(
            "thread_ts".to_string(),
            ConfidentValue::deterministic(Value::Text(thread_ts.into())),
        );
        args.insert(
            "message_ts".to_string(),
            ConfidentValue::deterministic(Value::Text(format!("{thread_ts}-m{turn}"))),
        );

        let _ = executor
            .exec_endpoint("fire_devops", args, None)
            .await
            .expect("endpoint dispatch");

        // Pause between turns so the first one fully drains through the
        // bus before the second arrives. Without this the correlation
        // index for the first turn might not be persisted in time for
        // the second turn's lookup.
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
// SSE event_emit frames carry only `source_agent` + `event` name +
// `subscribers` count (src/tracer.rs:258), so we lean on `say` frames
// emitted by the agents to verify field-level outcomes — same approach
// label_router_integration_tests.rs uses.

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
async fn investigation_path_routes_to_ops_and_stitches_finding_back_into_thread() {
    // DoD: "Integration test: a two-turn conversation classifies
    // correctly, routes, and the investigator's reply lands in the
    // same thread." This single-turn variant proves the full classify
    // → investigate → finding → stitch-back loop in one shot.
    let frames = fire_and_drain_one(
        "investigation",
        "ops",
        "T1",
        "logs are spiking on api-gateway",
    )
    .await;

    // 1. mastermind_intake emitted InvestigationRequested for the ops domain.
    assert!(
        emits_event_from(&frames, "InvestigationRequested", "mastermind_intake"),
        "intake should emit InvestigationRequested; frames={frames:#?}"
    );

    // 2. echo_investigator received it (filter where domain == "ops" hit)
    //    and emitted Finding back.
    let says = say_lines(&frames);
    assert!(
        says.iter().any(|s| s.starts_with("ECHO|")
            && s.contains("thread_ts=T1")
            && s.contains("domain=ops")),
        "echo_investigator should print ECHO line; says={says:#?}"
    );
    assert!(
        emits_event_from(&frames, "Finding", "echo_investigator"),
        "echo_investigator should emit Finding; frames={frames:#?}"
    );

    // 3. mastermind_intake's Finding handler emitted PostMessage with
    //    the originating thread_ts (the stitch-back).
    assert!(
        emits_event_from(&frames, "PostMessage", "mastermind_intake"),
        "intake should re-emit PostMessage on Finding; frames={frames:#?}"
    );
}

#[tokio::test]
async fn dev_task_classification_emits_stub_postmessage_for_proposal_phase() {
    // dev_task is the route that T9.3 (#364) solution_proposer will own.
    // Until then mastermind_intake emits a stub PostMessage so the
    // conversation in the Slack thread doesn't dead-end.
    let frames = fire_and_drain_one(
        "dev_task",
        "ops", // unused on this path
        "T2",
        "we should add a /metrics endpoint",
    )
    .await;

    assert!(
        !emits_event_from(&frames, "InvestigationRequested", "mastermind_intake"),
        "dev_task path must not emit InvestigationRequested; frames={frames:#?}"
    );
    assert!(
        emits_event_from(&frames, "PostMessage", "mastermind_intake"),
        "dev_task path should emit a PostMessage stub; frames={frames:#?}"
    );

    let says = say_lines(&frames);
    assert!(
        says.iter().any(|s| s.contains("classification=dev_task")),
        "intake should log dev_task classification; says={says:#?}"
    );
}

#[tokio::test]
async fn clarification_classification_asks_a_question_in_thread() {
    // The DoD says: "Falls back to `reason \"ask for clarification\"` when
    // classification confidence is low." The MockProvider always returns
    // .sure, so we exercise the same code path by directly classifying as
    // "clarification_needed" — the agent then calls ask_clarification and
    // emits PostMessage(thread_ts) with the resulting question.
    let frames = fire_and_drain_one(
        "clarification_needed",
        "ops",
        "T3",
        "something is off but I'm not sure what",
    )
    .await;

    assert!(
        !emits_event_from(&frames, "InvestigationRequested", "mastermind_intake"),
        "clarification path must not emit InvestigationRequested"
    );
    assert!(
        emits_event_from(&frames, "PostMessage", "mastermind_intake"),
        "clarification path should emit PostMessage with the follow-up question"
    );
    let says = say_lines(&frames);
    assert!(
        says.iter()
            .any(|s| s.contains("classification=clarification_needed")),
        "intake should log clarification classification; says={says:#?}"
    );
}

#[tokio::test]
async fn two_turns_same_thread_share_persistent_memory_via_correlation() {
    // DoD: "Memory keyed by `thread_ts`; survives restart via `mode: wake`
    // rehydration (#333)." This test proves the in-process half — the
    // same agent instance handles both turns via the CorrelationDriver
    // index (the underlying redb persistence + cross-process restart is
    // proven by the #334 e2e smoke at examples/basics/correlate_e2e.forge
    // and tests/correlate_*_tests.rs).
    let frames = fire_two_turns_same_thread("investigation", "ops", "T4").await;

    let says = say_lines(&frames);
    let intake_lines: Vec<&String> = says
        .iter()
        .filter(|s| s.contains("[mastermind_intake] thread=T4"))
        .collect();

    assert!(
        intake_lines.iter().any(|s| s.contains("turn=1")),
        "first turn should log turn=1; intake_lines={intake_lines:#?}"
    );
    assert!(
        intake_lines.iter().any(|s| s.contains("turn=2")),
        "second turn should log turn=2 (same instance via correlation); intake_lines={intake_lines:#?}"
    );
}
