// Integration test for T8.3 (#358) — label_router + TaskRouted SSE.
//
// Verifies the routing pipeline end-to-end on the bus:
//   endpoint emit GithubIssueOpened → mastermind on-handler →
//   label_router(labels, routing) → emit TaskRouted → SSE broadcast.
//
// Modeled after tests/sse_agent_emit_live_test.rs, which is the live
// regression for #325 (agent-originated emits reaching SSE). We skip
// the HTTP /wake/... HMAC layer here — that's covered comprehensively
// in tests/webhook_integration_tests.rs — and instead exercise the
// label-routing decision against the same event-bus + tracer wiring
// that `forge serve` builds in production.
//
// The test harness includes a slim mastermind-shaped agent that owns
// LabelRouting in memory directly (rather than loading config.toml),
// keeping the test self-contained while still exercising the pure
// label_router task through the full agent → bus → tracer path.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use forge::compose;
use forge::runtime::confidence::{ConfidentValue, Value};
use forge::runtime::event_bus::EventBus;
use forge::runtime::executor::TaskExecutor;
use forge::runtime::instance_registry::InstanceRegistry;

const TEST_HARNESS_SRC: &str = r#"#! boundary: server

event GithubIssueOpened
  repo: Text
  issue_number: Number
  title: Text
  body: Text
  labels: Text[]

event TaskRouted
  task_id: Text
  specialist: Text
  repo: Text
  issue_id: Text
  outcome: Text
  matched_suffix: Text
  diagnostic: Text

agent test_mastermind
  memory
    counter: Number
    routing: LabelRouting
  subscribe GithubIssueOpened

  on start
    memory.counter = 0
    memory.routing = LabelRouting(namespace: "clone-dev", suffixes: ["impl", "merge", "ops", "plan", "review", "test"], targets: ["implementer", "release_manager", "release_manager", "planner", "reviewer", "tester"], triage_target: "triage_specialist")

  on GithubIssueOpened(repo: Text, issue_number: Number, title: Text, body: Text, labels: Text[])
    memory.counter = memory.counter + 1
    task_id = "T{memory.counter}"
    decision = label_router(labels, memory.routing)
    emit TaskRouted(task_id: task_id, specialist: decision.specialist, repo: repo, issue_id: "{issue_number}", outcome: decision.outcome, matched_suffix: decision.matched_suffix, diagnostic: decision.diagnostic)
    say "ROUTED|specialist={decision.specialist}|outcome={decision.outcome}|matched_suffix={decision.matched_suffix}|diagnostic={decision.diagnostic}|repo={repo}"

warden test_warden
  manages [test_mastermind]
  on stuck: nudge, self
  on timeout: restart, self
  on crash: restart, self
  on hallucination: nudge, self
  on contradiction: nudge, self
  on budget: nudge, self

system test_system
  use
    mm: test_mastermind

endpoint fire_issue_opened(repo: Text, issue_number: Number, title: Text, body: Text, labels: Text[]) -> Text
  emit GithubIssueOpened(repo: repo, issue_number: issue_number, title: title, body: body, labels: labels)
  give "queued"
"#;

const ROUTER_PATH: &str = "workflows/clone-dev/stage2/label_router.forge";

fn mock_registry() -> Arc<forge::llm::registry::ProviderRegistry> {
    let config = forge::config::ForgeConfig::default_mock_config();
    Arc::new(
        forge::llm::registry::ProviderRegistry::from_config(config)
            .expect("mock registry should build"),
    )
}

fn build_program() -> forge::ast::Program {
    let router_src = std::fs::read_to_string(ROUTER_PATH)
        .unwrap_or_else(|e| panic!("could not read {ROUTER_PATH}: {e}"));
    let router_prog = forge::parser::parse(&router_src).expect("parse label_router.forge");
    let harness_prog = forge::parser::parse(TEST_HARNESS_SRC).expect("parse harness");
    let files = vec![
        compose::SourceFile {
            path: ROUTER_PATH.to_string(),
            source: router_src,
            program: router_prog,
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

async fn fire_and_drain(
    repo: &str,
    issue_number: f64,
    labels: Vec<&str>,
) -> Vec<serde_json::Value> {
    let program = build_program();
    let (events_tx, mut events_rx) = tokio::sync::broadcast::channel::<String>(256);
    let tracer = forge::tracer::Tracer::with_live(events_tx.clone());

    let executor = TaskExecutor::new(program, mock_registry(), Some(tracer.clone()))
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

    let labels_value = Value::Array(
        labels
            .into_iter()
            .map(|s| ConfidentValue::deterministic(Value::Text(s.to_string())))
            .collect(),
    );

    let mut args = HashMap::new();
    args.insert(
        "repo".to_string(),
        ConfidentValue::deterministic(Value::Text(repo.to_string())),
    );
    args.insert(
        "issue_number".to_string(),
        ConfidentValue::deterministic(Value::Number(issue_number)),
    );
    args.insert(
        "title".to_string(),
        ConfidentValue::deterministic(Value::Text("test issue".into())),
    );
    args.insert(
        "body".to_string(),
        ConfidentValue::deterministic(Value::Text("body".into())),
    );
    args.insert(
        "labels".to_string(),
        ConfidentValue::deterministic(labels_value),
    );

    let _ = executor
        .exec_endpoint("fire_issue_opened", args, None)
        .await
        .expect("endpoint dispatch");

    // Drain SSE frames for ~1.5s — long enough for: GithubIssueOpened ->
    // bus -> agent handler -> label_router -> emit TaskRouted -> trace.
    let mut frames = Vec::<String>::new();
    let deadline = std::time::Instant::now() + Duration::from_millis(1500);
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        match tokio::time::timeout(remaining, events_rx.recv()).await {
            Ok(Ok(frame)) => frames.push(frame),
            Ok(Err(_)) | Err(_) => break,
        }
    }

    frames
        .iter()
        .filter_map(|f| serde_json::from_str(f).ok())
        .collect()
}

// SSE event_emit frames carry only `source_agent`, `event` name, and
// `subscribers` count — not the field values (see src/tracer.rs:258).
// To verify the *routing decision* via Observer SSE we have the test
// agent additionally `say` the decoded fields; tracer.say emits a
// separate SSE frame whose `text` field carries them.
fn task_routed_present(parsed: &[serde_json::Value]) -> bool {
    parsed
        .iter()
        .any(|v| v["event"] == "TaskRouted" && v["source_agent"] == "test_mastermind")
}

fn find_routed_say(parsed: &[serde_json::Value]) -> String {
    parsed
        .iter()
        .filter(|v| v["event"] == "say")
        .filter_map(|v| v["text"].as_str())
        .find(|s| s.starts_with("ROUTED|"))
        .map(|s| s.to_string())
        .unwrap_or_else(|| panic!("no ROUTED say frame; frames={parsed:#?}"))
}

#[tokio::test]
async fn webhook_emit_routes_clonedev_impl_to_implementer() {
    let parsed = fire_and_drain("acme/x", 42.0, vec!["clone-dev:impl"]).await;
    assert!(
        task_routed_present(&parsed),
        "TaskRouted should appear on bus"
    );
    let routed = find_routed_say(&parsed);
    assert!(routed.contains("specialist=implementer"), "got: {routed}");
    assert!(routed.contains("outcome=routed"), "got: {routed}");
    assert!(routed.contains("matched_suffix=impl"), "got: {routed}");
    assert!(routed.contains("diagnostic=ok"), "got: {routed}");
    assert!(routed.contains("repo=acme/x"), "got: {routed}");
}

#[tokio::test]
async fn webhook_emit_routes_unlabeled_issue_to_triage_specialist() {
    let parsed = fire_and_drain("acme/x", 7.0, vec![]).await;
    assert!(
        task_routed_present(&parsed),
        "TaskRouted should appear on bus"
    );
    let routed = find_routed_say(&parsed);
    assert!(
        routed.contains("specialist=triage_specialist"),
        "got: {routed}"
    );
    assert!(routed.contains("outcome=triage"), "got: {routed}");
    assert!(
        routed.contains("diagnostic=no_clonedev_label"),
        "got: {routed}"
    );
}

#[tokio::test]
async fn webhook_emit_routes_double_label_to_triage_with_conflict_diagnostic() {
    let parsed = fire_and_drain("acme/x", 9.0, vec!["clone-dev:plan", "clone-dev:impl"]).await;
    assert!(
        task_routed_present(&parsed),
        "TaskRouted should appear on bus"
    );
    let routed = find_routed_say(&parsed);
    assert!(
        routed.contains("specialist=triage_specialist"),
        "got: {routed}"
    );
    assert!(routed.contains("outcome=triage"), "got: {routed}");
    assert!(
        routed.contains("diagnostic=multi_clonedev_labels"),
        "got: {routed}"
    );
}
