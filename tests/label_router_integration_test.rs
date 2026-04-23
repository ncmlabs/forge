// Integration test for T8.3 (#358): label_router agent end-to-end.
//
// Stands up the same topology `forge serve` uses (TaskExecutor + shared
// EventBus + SystemRuntime + live Tracer → broadcast) and verifies that
// emitting a `GithubIssueLabeled` event causes `label_router_agent` to
// publish a `TaskRouted` SSE frame with the right specialist for the
// matching label, plus a `RoutingConflict` when two namespace labels
// collide on one issue.
//
// The FORGE source is inlined rather than loading the production
// stage2/*.forge files so the test doesn't depend on a filesystem
// FORGE_CLONEDEV_CONFIG fixture — routes live in agent memory.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use forge::compose;
use forge::runtime::confidence::{ConfidentValue, Value};
use forge::runtime::event_bus::EventBus;
use forge::runtime::executor::TaskExecutor;
use forge::runtime::instance_registry::InstanceRegistry;

const LABEL_ROUTER_SRC: &str = r##"#! boundary: server

type LabelRoute
  label: Text
  specialist: Text

type RoutingDecision
  specialist: Text
  matched_label: Text
  route_reason: Text

event GithubIssueLabeled
  repo: Text
  issue_number: Number
  title: Text
  body: Text
  labels: Text[]

event TaskRouted
  task_id: Text
  specialist: Text
  issue_id: Text
  repo: Text
  labels: Text[]
  matched_label: Text
  route_reason: Text

event RoutingConflict
  issue_id: Text
  repo: Text
  labels: Text[]
  note: Text

pure route_if_matches
  needs r: LabelRoute, l_lower: Text
  gives LabelRoute[]
  do
    route_label = r.label
    route_lower = route_label.lower()
    if route_lower == l_lower
      give [r]
    give []

pure routes_matching_label
  needs l: Text, routes: LabelRoute[]
  gives LabelRoute[]
  do
    l_lower = l.lower()
    result = []
    for r in routes
      hit = route_if_matches(r, l_lower)
      result = result + hit
    give result

pure match_routes
  needs issue_labels: Text[], routes: LabelRoute[]
  gives LabelRoute[]
  do
    result = []
    for l in issue_labels
      hits = routes_matching_label(l, routes)
      result = result + hits
    give result

pure label_router
  needs issue_labels: Text[], routes: LabelRoute[], fallback: Text
  gives RoutingDecision
  do
    matches = match_routes(issue_labels, routes)
    if matches.length == 0
      give RoutingDecision(specialist: fallback, matched_label: "", route_reason: "no-matching-label")
    if matches.length > 1
      give RoutingDecision(specialist: fallback, matched_label: "", route_reason: "multi-label-conflict")
    give RoutingDecision(specialist: matches[0].specialist, matched_label: matches[0].label, route_reason: "unique-label-match")

agent label_router_agent
  memory persistent
    routes: LabelRoute[]
    fallback: Text
  subscribe GithubIssueLabeled

  on start
    memory.routes = [LabelRoute(label: "clone-dev:plan", specialist: "plan_specialist"), LabelRoute(label: "clone-dev:impl", specialist: "impl_specialist"), LabelRoute(label: "clone-dev:test", specialist: "test_specialist"), LabelRoute(label: "clone-dev:review", specialist: "review_specialist"), LabelRoute(label: "clone-dev:merge", specialist: "merge_specialist"), LabelRoute(label: "clone-dev:ops", specialist: "ops_specialist")]
    memory.fallback = "triage_specialist"

  on GithubIssueLabeled(repo: Text, issue_number: Number, title: Text, body: Text, labels: Text[])
    task_id = "T-{repo}-{issue_number}"
    decision = label_router(labels, memory.routes, memory.fallback)
    emit TaskRouted(task_id: task_id, specialist: decision.specialist, issue_id: "{issue_number}", repo: repo, labels: labels, matched_label: decision.matched_label, route_reason: decision.route_reason)
    say "ROUTE task_id={task_id} specialist={decision.specialist} matched_label={decision.matched_label} route_reason={decision.route_reason}"
    if decision.route_reason == "multi-label-conflict"
      emit RoutingConflict(issue_id: "{issue_number}", repo: repo, labels: labels, note: "multiple namespace labels on one issue")
      say "CONFLICT repo={repo} issue_id={issue_number}"

warden w
  manages [label_router_agent]
  on stuck: nudge, self
  on timeout: restart, self
  on crash: restart, self
  on hallucination: nudge, self
  on contradiction: nudge, self
  on budget: nudge, self

system router_system
  use
    lr: label_router_agent

endpoint emit_issue_labeled(repo: Text, issue_number: Number, labels: Text[]) -> Text
  emit GithubIssueLabeled(repo: repo, issue_number: issue_number, title: "", body: "", labels: labels)
  give "queued"
"##;

fn mock_registry() -> Arc<forge::llm::registry::ProviderRegistry> {
    let config = forge::config::ForgeConfig::default_mock_config();
    Arc::new(
        forge::llm::registry::ProviderRegistry::from_config(config)
            .expect("mock registry should build"),
    )
}

async fn spin_up_router() -> (TaskExecutor, tokio::sync::broadcast::Receiver<String>) {
    let program = forge::parser::parse(LABEL_ROUTER_SRC).expect("parse label-router");
    let sources = [compose::SourceFile {
        path: "label_router.forge".to_string(),
        source: LABEL_ROUTER_SRC.to_string(),
        program: program.clone(),
    }];
    let composed = compose::merge_programs(&sources).expect("merge");
    let diagnostics = forge::checker::check_all(&composed.program, "label_router.forge");
    let errors: Vec<_> = diagnostics
        .iter()
        .filter(|d| d.kind == forge::diagnostic::DiagnosticKind::Error)
        .collect();
    assert!(errors.is_empty(), "checker errors: {:?}", errors);

    let (events_tx, events_rx) = tokio::sync::broadcast::channel::<String>(256);
    let tracer = forge::tracer::Tracer::with_live(events_tx.clone());

    let executor = TaskExecutor::new(composed.program, mock_registry(), Some(tracer.clone()))
        .with_config(forge::config::ForgeConfig::default_mock_config());

    let event_bus = EventBus::new_shared(executor.tracer().cloned());
    let instance_registry = Arc::new(tokio::sync::RwLock::new(InstanceRegistry::new()));

    let system_runtime = executor
        .build_system_runtime()
        .expect("build system runtime")
        .expect("router_system should produce a runtime")
        .with_shared_infrastructure(event_bus.clone(), instance_registry.clone());

    let executor = executor.with_event_bus(event_bus.clone());

    tokio::spawn(async move {
        let _ = system_runtime.start().await;
    });

    // Let the agent register its subscription and run `on start`.
    tokio::time::sleep(Duration::from_millis(300)).await;

    (executor, events_rx)
}

async fn drain_events(
    rx: &mut tokio::sync::broadcast::Receiver<String>,
    window: Duration,
) -> Vec<serde_json::Value> {
    let deadline = std::time::Instant::now() + window;
    let mut frames = Vec::<String>::new();
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
        .into_iter()
        .filter_map(|f| serde_json::from_str::<serde_json::Value>(&f).ok())
        .collect()
}

fn find_emit<'a>(
    parsed: &'a [serde_json::Value],
    source: &str,
    name: &str,
) -> Option<&'a serde_json::Value> {
    parsed
        .iter()
        .find(|v| v["event"] == name && v["source_agent"] == source)
}

// The tracer's `event_emit` frame records only {event, source_agent,
// subscribers} — not the payload. To assert on RoutingDecision fields,
// the agent `say`s a structured summary after each emit; the tracer's
// `say` frames carry the text, which we parse here.
fn find_say<'a>(parsed: &'a [serde_json::Value], needle: &str) -> Option<&'a serde_json::Value> {
    parsed
        .iter()
        .find(|v| v["event"] == "say" && v["text"].as_str().is_some_and(|t| t.contains(needle)))
}

async fn emit_issue(executor: &TaskExecutor, repo: &str, issue_number: f64, labels: Vec<&str>) {
    let mut args = HashMap::new();
    args.insert(
        "repo".to_string(),
        ConfidentValue::deterministic(Value::Text(repo.to_string())),
    );
    args.insert(
        "issue_number".to_string(),
        ConfidentValue::deterministic(Value::Number(issue_number)),
    );
    let labels_val = Value::Array(
        labels
            .into_iter()
            .map(|l| ConfidentValue::deterministic(Value::Text(l.to_string())))
            .collect(),
    );
    args.insert(
        "labels".to_string(),
        ConfidentValue::deterministic(labels_val),
    );
    executor
        .exec_endpoint("emit_issue_labeled", args, None)
        .await
        .expect("endpoint dispatch");
}

// ── Tests ───────────────────────────────────────────────────────────

#[tokio::test]
async fn unique_namespace_label_routes_to_matching_specialist() {
    let (executor, mut events_rx) = spin_up_router().await;

    emit_issue(&executor, "acme/alpha", 42.0, vec!["bug", "clone-dev:impl"]).await;

    let parsed = drain_events(&mut events_rx, Duration::from_millis(1500)).await;

    // The emit itself: observable by presence on the SSE broadcast.
    assert!(
        find_emit(&parsed, "label_router_agent", "TaskRouted").is_some(),
        "TaskRouted emit missing; frames = {:#?}",
        parsed
    );

    // Decision payload: observable via the agent's structured say line.
    let route_say = find_say(&parsed, "ROUTE task_id=T-acme/alpha-42")
        .unwrap_or_else(|| panic!("ROUTE say line missing; frames = {:#?}", parsed));
    let text = route_say["text"].as_str().unwrap();
    assert!(text.contains("specialist=impl_specialist"), "text={text}");
    assert!(text.contains("matched_label=clone-dev:impl"), "text={text}");
    assert!(
        text.contains("route_reason=unique-label-match"),
        "text={text}"
    );

    // No RoutingConflict on a clean single-match route.
    assert!(
        find_emit(&parsed, "label_router_agent", "RoutingConflict").is_none(),
        "no conflict expected, got frames = {:#?}",
        parsed
    );
}

#[tokio::test]
async fn multi_namespace_labels_fall_back_to_triage_and_emit_conflict() {
    let (executor, mut events_rx) = spin_up_router().await;

    emit_issue(
        &executor,
        "acme/beta",
        7.0,
        vec!["clone-dev:plan", "clone-dev:impl"],
    )
    .await;

    let parsed = drain_events(&mut events_rx, Duration::from_millis(1500)).await;
    assert!(
        find_emit(&parsed, "label_router_agent", "TaskRouted").is_some(),
        "TaskRouted emit missing; frames = {:#?}",
        parsed
    );
    assert!(
        find_emit(&parsed, "label_router_agent", "RoutingConflict").is_some(),
        "RoutingConflict emit missing; frames = {:#?}",
        parsed
    );

    let route_say = find_say(&parsed, "ROUTE task_id=T-acme/beta-7")
        .unwrap_or_else(|| panic!("ROUTE say line missing; frames = {:#?}", parsed));
    let text = route_say["text"].as_str().unwrap();
    assert!(text.contains("specialist=triage_specialist"), "text={text}");
    assert!(
        text.contains("route_reason=multi-label-conflict"),
        "text={text}"
    );
    assert!(text.contains("matched_label="), "text={text}");

    let conflict_say = find_say(&parsed, "CONFLICT repo=acme/beta")
        .unwrap_or_else(|| panic!("CONFLICT say line missing; frames = {:#?}", parsed));
    assert!(
        conflict_say["text"]
            .as_str()
            .unwrap()
            .contains("issue_id=7"),
        "conflict text missing issue_id"
    );
}

#[tokio::test]
async fn unlabeled_issue_falls_back_to_triage_without_conflict() {
    let (executor, mut events_rx) = spin_up_router().await;

    emit_issue(&executor, "acme/gamma", 3.0, vec![]).await;

    let parsed = drain_events(&mut events_rx, Duration::from_millis(1500)).await;
    assert!(
        find_emit(&parsed, "label_router_agent", "TaskRouted").is_some(),
        "TaskRouted emit missing; frames = {:#?}",
        parsed
    );

    let route_say = find_say(&parsed, "ROUTE task_id=T-acme/gamma-3")
        .unwrap_or_else(|| panic!("ROUTE say line missing; frames = {:#?}", parsed));
    let text = route_say["text"].as_str().unwrap();
    assert!(text.contains("specialist=triage_specialist"), "text={text}");
    assert!(
        text.contains("route_reason=no-matching-label"),
        "text={text}"
    );

    assert!(
        find_emit(&parsed, "label_router_agent", "RoutingConflict").is_none(),
        "unlabeled issue should not emit RoutingConflict"
    );
}
