// Regression for #412: GitHub's live `issues.opened` webhook payload is
// nested, while clone-dev's routing pipeline consumes a normalized flat event.
// This test drives the real agent/event-bus path and verifies the configured
// TypeScript test command reaches IssueAssigned.

#![allow(clippy::await_holding_lock)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use forge::compose;
use forge::runtime::confidence::{ConfidentValue, Value};
use forge::runtime::event_bus::{EventBus, EventPayload};
use forge::runtime::executor::TaskExecutor;
use forge::runtime::instance_registry::InstanceRegistry;

const TYPES_PATH: &str = "workflows/clone-dev/shared/types.forge";
const EVENTS_PATH: &str = "workflows/clone-dev/shared/events.forge";
const MASTERMIND_PATH: &str = "workflows/clone-dev/shared/mastermind.forge";
const ROUTER_PATH: &str = "workflows/clone-dev/stage2/label_router.forge";

const TYPESCRIPT_TEST_CMD: &str = "npm ci && npm run typecheck && npm test && npm run build";

static ENV_LOCK: Mutex<()> = Mutex::new(());

const HARNESS_SRC: &str = r#"#! boundary: server

event IssueAssigned
  issue_id: Text
  repo: Text
  title: Text
  body: Text
  channel: Text
  callback_url: Text
  test_cmd: Text

event TaskCompleted
  task_id: Text
  repo: Text
  outcome: Text
  ci_passed_first_try: Bool
  review_rounds: Number
  time_to_merge: Number
  reverted_within_7d: Bool

pure slugify_repo
  needs repo: Text
  gives Text
  do
    parts = repo.split("/")
    if parts.length == 2
      give parts[0] + "-" + parts[1]
    give repo

agent issue_probe
  subscribe IssueAssigned

  on IssueAssigned(issue_id: Text, repo: Text, title: Text, body: Text, channel: Text, callback_url: Text, test_cmd: Text)
    say "ISSUE_ASSIGNED|issue_id={issue_id}|repo={repo}|title={title}|test_cmd={test_cmd}"

system github_wake_test
  use
    mm: mastermind
    probe: issue_probe
"#;

fn mock_registry() -> Arc<forge::llm::registry::ProviderRegistry> {
    let config = forge::config::ForgeConfig::default_mock_config();
    Arc::new(
        forge::llm::registry::ProviderRegistry::from_config(config)
            .expect("mock registry should build"),
    )
}

fn read_source(path: &str) -> compose::SourceFile {
    let source =
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("could not read {path}: {e}"));
    let program = forge::parser::parse(&source).unwrap_or_else(|e| panic!("parse {path}: {e}"));
    compose::SourceFile {
        path: path.to_string(),
        source,
        program,
    }
}

fn build_program() -> forge::ast::Program {
    let mut files = vec![
        read_source(TYPES_PATH),
        read_source(EVENTS_PATH),
        read_source(MASTERMIND_PATH),
        read_source(ROUTER_PATH),
    ];
    files.push(compose::SourceFile {
        path: "github_wake_harness.forge".to_string(),
        source: HARNESS_SRC.to_string(),
        program: forge::parser::parse(HARNESS_SRC).expect("parse harness"),
    });
    compose::merge_programs(&files).expect("merge").program
}

fn cv_text(value: &str) -> ConfidentValue {
    ConfidentValue::deterministic(Value::Text(value.to_string()))
}

fn cv_number(value: f64) -> ConfidentValue {
    ConfidentValue::deterministic(Value::Number(value))
}

fn cv_record(fields: HashMap<String, ConfidentValue>) -> ConfidentValue {
    ConfidentValue::deterministic(Value::Record(fields))
}

fn github_issues_opened_payload() -> EventPayload {
    let mut repo = HashMap::new();
    repo.insert("full_name".to_string(), cv_text("ncmlabs/forge-playground"));

    let mut label = HashMap::new();
    label.insert("name".to_string(), cv_text("clone-dev:plan"));

    let mut issue = HashMap::new();
    issue.insert("number".to_string(), cv_number(17.0));
    issue.insert("title".to_string(), cv_text("TypeScript proof queue issue"));
    issue.insert(
        "body".to_string(),
        cv_text("Exercise the TypeScript queue."),
    );
    issue.insert(
        "labels".to_string(),
        ConfidentValue::deterministic(Value::Array(vec![cv_record(label)])),
    );

    let mut fields = HashMap::new();
    fields.insert("action".to_string(), cv_text("opened"));
    fields.insert("repository".to_string(), cv_record(repo));
    fields.insert("issue".to_string(), cv_record(issue));

    EventPayload {
        event_name: "GithubIssuesWebhook".to_string(),
        args: Vec::new(),
        source_agent: "webhook:github_issue_opened".to_string(),
        fields,
    }
}

async fn next_say_containing(
    rx: &mut tokio::sync::broadcast::Receiver<String>,
    needle: &str,
) -> String {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        match tokio::time::timeout(remaining, rx.recv()).await {
            Ok(Ok(frame)) => {
                let Ok(json) = serde_json::from_str::<serde_json::Value>(&frame) else {
                    continue;
                };
                if json["event"] == "say" {
                    if let Some(text) = json["text"].as_str() {
                        if text.contains(needle) {
                            return text.to_string();
                        }
                    }
                }
            }
            Ok(Err(_)) | Err(_) => break,
        }
    }
    panic!("timed out waiting for say frame containing {needle}");
}

#[tokio::test]
async fn github_issues_opened_reaches_issue_assigned_with_typescript_test_cmd() {
    let _guard = ENV_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().expect("tempdir");
    let config_path = dir.path().join("clone-dev.toml");
    std::fs::write(
        &config_path,
        format!(
            r#"
[org]
name = "ncmlabs"

[slack]
default_channel = "C_default"

[labels]
namespace = "clone-dev"
triage_target = "triage_specialist"

[labels.routing]
plan = "planner"

[defaults]
test_cmd = "cargo test --quiet"

[repos."ncmlabs/forge-playground"]
test_cmd = "{TYPESCRIPT_TEST_CMD}"
"#
        ),
    )
    .expect("write fixture config");
    std::env::set_var("FORGE_CLONEDEV_CONFIG", &config_path);

    let program = build_program();
    let (events_tx, mut events_rx) = tokio::sync::broadcast::channel::<String>(256);
    let tracer = forge::tracer::Tracer::with_live(events_tx);
    let executor = TaskExecutor::new(program, mock_registry(), Some(tracer.clone()))
        .with_config(forge::config::ForgeConfig::default_mock_config());

    let event_bus = EventBus::new_shared(executor.tracer().cloned());
    let instance_registry = Arc::new(tokio::sync::RwLock::new(InstanceRegistry::new()));
    let system_runtime = executor
        .build_system_runtime()
        .expect("build system runtime")
        .expect("github_wake_test system should exist")
        .with_shared_infrastructure(event_bus.clone(), instance_registry);

    tokio::spawn(async move {
        let _ = system_runtime.start().await;
    });

    tokio::time::sleep(Duration::from_millis(500)).await;
    {
        let guard = event_bus.read().await;
        guard.publish(&github_issues_opened_payload());
    }

    let assigned = next_say_containing(&mut events_rx, "ISSUE_ASSIGNED|").await;
    std::env::remove_var("FORGE_CLONEDEV_CONFIG");

    assert!(assigned.contains("issue_id=17"), "got: {assigned}");
    assert!(
        assigned.contains("repo=ncmlabs/forge-playground"),
        "got: {assigned}"
    );
    assert!(
        assigned.contains(&format!("test_cmd={TYPESCRIPT_TEST_CMD}")),
        "got: {assigned}"
    );
}
