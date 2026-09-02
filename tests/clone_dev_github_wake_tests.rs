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
use forge::runtime::http_server::ForgeServer;
use forge::runtime::instance_registry::InstanceRegistry;
use forge::runtime::storage::ForgeStorage;
use tokio::net::TcpListener;
use tokio::sync::mpsc;

const TYPES_PATH: &str = "workflows/clone-dev/shared/types.forge";
const EVENTS_PATH: &str = "workflows/clone-dev/shared/events.forge";
const MASTERMIND_PATH: &str = "workflows/clone-dev/shared/mastermind.forge";
const ROUTER_PATH: &str = "workflows/clone-dev/stage2/label_router.forge";
const MAIN_PATH: &str = "workflows/clone-dev/main.forge";
const SLACK_MONITOR_PATH: &str = "workflows/clone-dev/stage1/slack_devops_monitor.forge";
const MASTERMIND_INTAKE_PATH: &str = "workflows/clone-dev/stage1/mastermind_intake.forge";
const CODE_INVESTIGATOR_PATH: &str =
    "workflows/clone-dev/stage1/investigators/code_investigator.forge";
const OPS_INVESTIGATOR_PATH: &str =
    "workflows/clone-dev/stage1/investigators/ops_investigator.forge";
const SECURITY_INVESTIGATOR_PATH: &str =
    "workflows/clone-dev/stage1/investigators/security_investigator.forge";
const SOLUTION_PROPOSER_PATH: &str = "workflows/clone-dev/stage1/solution_proposer.forge";
const GATE_ONE_PATH: &str = "workflows/clone-dev/stage1/gate_one.forge";
const ISSUE_CREATOR_PATH: &str = "workflows/clone-dev/stage1/issue_creator.forge";
const TRIAGE_SPECIALIST_PATH: &str = "workflows/clone-dev/stage2/triage_specialist.forge";
const DEV_CYCLE_AGENTS_PATH: &str = "workflows/dev-cycle/agents.forge";
const GATE_TWO_PATH: &str = "workflows/dev-cycle/gate_two.forge";
const SLACK_ADAPTER_PATH: &str = "examples/agents/slack-adapter/agents.forge";

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

fn build_full_clone_dev_program() -> forge::ast::Program {
    let files = vec![
        read_source(TYPES_PATH),
        read_source(EVENTS_PATH),
        read_source(MASTERMIND_PATH),
        read_source(SLACK_MONITOR_PATH),
        read_source(MASTERMIND_INTAKE_PATH),
        read_source(CODE_INVESTIGATOR_PATH),
        read_source(OPS_INVESTIGATOR_PATH),
        read_source(SECURITY_INVESTIGATOR_PATH),
        read_source(SOLUTION_PROPOSER_PATH),
        read_source(GATE_ONE_PATH),
        read_source(ISSUE_CREATOR_PATH),
        read_source(ROUTER_PATH),
        read_source(TRIAGE_SPECIALIST_PATH),
        read_source(DEV_CYCLE_AGENTS_PATH),
        read_source(GATE_TWO_PATH),
        read_source(SLACK_ADAPTER_PATH),
        read_source(MAIN_PATH),
    ];
    compose::merge_programs(&files)
        .expect("merge full clone-dev")
        .program
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

fn github_issues_opened_json_body() -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "action": "opened",
        "repository": {
            "full_name": "ncmlabs/forge-playground"
        },
        "issue": {
            "number": 25,
            "title": "TypeScript proof queue issue",
            "body": "Exercise the TypeScript queue through signed GitHub wake.",
            "labels": [
                { "name": "clone-dev:plan" }
            ]
        }
    }))
    .expect("serialize github payload")
}

fn hmac_hex(secret: &str, body: &[u8]) -> String {
    use hmac::{Hmac, KeyInit, Mac};
    use sha2::Sha256;
    let mut mac: Hmac<Sha256> = Hmac::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(body);
    hex::encode(mac.finalize().into_bytes())
}

async fn next_payload(rx: &mut mpsc::Receiver<EventPayload>, event_name: &str) -> EventPayload {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    while tokio::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        match tokio::time::timeout(remaining, rx.recv()).await {
            Ok(Some(payload)) if payload.event_name == event_name => return payload,
            Ok(Some(_)) => continue,
            Ok(None) | Err(_) => break,
        }
    }
    panic!("timed out waiting for event {event_name}");
}

fn field_text(payload: &EventPayload, name: &str) -> String {
    match payload.fields.get(name).map(|cv| &cv.value) {
        Some(Value::Text(v)) => v.clone(),
        other => panic!("expected text field {name}, got {other:?}"),
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

#[tokio::test]
async fn signed_wake_http_reaches_issue_assigned_with_typescript_test_cmd() {
    let _guard = ENV_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().expect("tempdir");
    let storage_root = dir.path().join("storage");
    std::env::set_var("FORGE_STORAGE_ROOT", &storage_root);

    let config_path = dir.path().join("clone-dev.toml");
    std::fs::write(
        &config_path,
        format!(
            r#"
[org]
name = "ncmlabs"

[slack]
default_channel = "C_default"
devops_channel = "C_devops"
approval_channel = ""

[labels]
namespace = "clone-dev"
triage_target = "triage_specialist"

[labels.routing]
plan = "planner"

[gates]
create_issue = true
start_implementation = true
merge_pr = true

[defaults]
test_cmd = "cargo test --quiet"
workdir_root = "/tmp/forge-test-workdir"
branch_prefix = "clone-dev/"
commit_template = "test commit"
fix_commit_template = "test fix commit"
max_iterations = 1
max_plan_revisions = 1

[repos."ncmlabs/forge-playground"]
test_cmd = "{TYPESCRIPT_TEST_CMD}"
"#
        ),
    )
    .expect("write fixture config");
    std::env::set_var("FORGE_CLONEDEV_CONFIG", &config_path);

    let hmac_key = ["github", "wake", "fixture"].join("-");
    let wake_storage = Arc::new(
        ForgeStorage::open_wake_from_config(None, None).expect("open canonical wake storage"),
    );
    wake_storage
        .upsert_wake_secret("mastermind", "github_issue_opened", &hmac_key)
        .expect("register wake secret");

    let runtime_storage = Arc::new(
        ForgeStorage::open_from_config(None, None, "server.redb").expect("open runtime storage"),
    );
    assert_eq!(
        runtime_storage
            .lookup_wake_secret("mastermind", "github_issue_opened")
            .unwrap(),
        None,
        "test must reproduce the CLI/server database split"
    );

    let program = build_full_clone_dev_program();
    let (events_tx, _events_rx) = tokio::sync::broadcast::channel::<String>(256);
    let tracer = forge::tracer::Tracer::with_live(events_tx.clone());
    let executor = TaskExecutor::new(program, mock_registry(), Some(tracer.clone()))
        .with_config(forge::config::ForgeConfig::default_mock_config())
        .with_storage(runtime_storage.clone());

    let event_bus = EventBus::new_shared(executor.tracer().cloned());
    let mut github_webhook_rx = {
        let mut guard = event_bus.write().await;
        guard.subscribe("GithubIssuesWebhook", "test_probe", None)
    };
    let mut github_opened_rx = {
        let mut guard = event_bus.write().await;
        guard.subscribe("GithubIssueOpened", "test_probe", None)
    };
    let mut clone_inbound_rx = {
        let mut guard = event_bus.write().await;
        guard.subscribe("CloneDevInbound", "test_probe", None)
    };
    let mut issue_assigned_rx = {
        let mut guard = event_bus.write().await;
        guard.subscribe("IssueAssigned", "test_probe", None)
    };

    let instance_registry = Arc::new(tokio::sync::RwLock::new(InstanceRegistry::new()));
    let mut system_runtime = executor
        .build_system_runtime()
        .expect("build system runtime")
        .expect("clone_dev_system should exist")
        .with_shared_infrastructure(event_bus.clone(), instance_registry.clone())
        .with_shared_storage(runtime_storage.clone());
    let webhook_driver = Arc::new(
        system_runtime
            .build_webhook_driver()
            .expect("clone-dev declares webhooks"),
    );
    assert!(
        webhook_driver
            .match_webhook("mastermind", "github_issue_opened")
            .is_some(),
        "full clone-dev composition must register mastermind/github_issue_opened"
    );
    let lifecycle = system_runtime.ensure_lifecycle();

    let server = ForgeServer::new(executor, None)
        .with_event_bus(event_bus)
        .with_events_tx(events_tx)
        .with_instance_registry(instance_registry)
        .with_webhook_driver(webhook_driver)
        .with_wake_storage(wake_storage)
        .with_agent_lifecycle(lifecycle);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let base = format!("http://127.0.0.1:{port}");
    let router = server.build_router();
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    tokio::spawn(async move {
        let _ = system_runtime.start().await;
    });

    tokio::time::sleep(Duration::from_millis(500)).await;

    let body = github_issues_opened_json_body();
    let sig = format!("sha256={}", hmac_hex(&hmac_key, &body));
    let res = reqwest::Client::new()
        .post(format!("{base}/wake/mastermind/github_issue_opened"))
        .header("Content-Type", "application/json")
        .header("X-Hub-Signature-256", sig)
        .body(body)
        .send()
        .await
        .expect("post signed wake");
    assert_eq!(res.status().as_u16(), 202);

    let webhook = next_payload(&mut github_webhook_rx, "GithubIssuesWebhook").await;
    assert_eq!(field_text(&webhook, "action"), "opened");

    let opened = next_payload(&mut github_opened_rx, "GithubIssueOpened").await;
    assert_eq!(field_text(&opened, "repo"), "ncmlabs/forge-playground");

    let inbound = next_payload(&mut clone_inbound_rx, "CloneDevInbound").await;
    assert_eq!(field_text(&inbound, "kind"), "github_issue");
    assert_eq!(field_text(&inbound, "test_cmd"), TYPESCRIPT_TEST_CMD);

    let assigned = next_payload(&mut issue_assigned_rx, "IssueAssigned").await;
    assert_eq!(field_text(&assigned, "issue_id"), "25");
    assert_eq!(field_text(&assigned, "repo"), "ncmlabs/forge-playground");
    assert_eq!(field_text(&assigned, "test_cmd"), TYPESCRIPT_TEST_CMD);

    std::env::remove_var("FORGE_CLONEDEV_CONFIG");
    std::env::remove_var("FORGE_STORAGE_ROOT");
}
