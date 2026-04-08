// FORGE failure injection API integration tests — issue #143

use std::collections::HashMap;
use std::sync::Arc;

use forge::config::ForgeConfig;
use forge::llm::registry::ProviderRegistry;
use forge::runtime::agent::AgentSignal;
use forge::runtime::executor::TaskExecutor;
use forge::runtime::http_server::ForgeServer;
use forge::runtime::system::SharedSignalSenders;

// ── Helpers ──────────────────────────────────────────────────────

fn mock_registry() -> Arc<ProviderRegistry> {
    let config = ForgeConfig::default_mock_config();
    Arc::new(ProviderRegistry::from_config(config).expect("mock registry should build"))
}

fn parse_and_build(source: &str) -> TaskExecutor {
    let program = forge::parser::parse(source).expect("parse failed");
    TaskExecutor::new(program, mock_registry(), None)
}

const MINIMAL_SOURCE: &str = r#"#! boundary: server

task greet
  needs name: Text
  gives Text
  do
    give "Hello, {name}!"

endpoint home(name: Text) -> Text
  result = greet(name)
  give result
"#;

/// Create a SharedSignalSenders with a real mpsc channel for the given agent names.
/// Returns (shared_senders, receivers) so tests can verify signals were sent.
fn make_signal_senders(
    agent_names: &[&str],
) -> (
    SharedSignalSenders,
    HashMap<String, tokio::sync::mpsc::Receiver<AgentSignal>>,
) {
    let mut map = HashMap::new();
    let mut receivers = HashMap::new();
    for name in agent_names {
        let (tx, rx) = tokio::sync::mpsc::channel::<AgentSignal>(64);
        map.insert(name.to_string(), tx);
        receivers.insert(name.to_string(), rx);
    }
    (Arc::new(tokio::sync::RwLock::new(map)), receivers)
}

/// Spawn a server with optional signal senders wired up.
async fn spawn_inject_server(signal_senders: Option<SharedSignalSenders>) -> String {
    let executor = parse_and_build(MINIMAL_SOURCE);
    let mut server = ForgeServer::new(executor, None);

    if let Some(senders) = signal_senders {
        server = server.with_signal_senders(senders);
    }

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind failed");
    let port = listener.local_addr().unwrap().port();

    tokio::spawn(async move {
        server
            .run_on_listener(listener)
            .await
            .expect("server failed");
    });

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    format!("http://127.0.0.1:{port}")
}

// ── Inject: no system runtime (503) ─────────────────────────────

#[tokio::test]
async fn inject_returns_503_without_system_runtime() {
    let base = spawn_inject_server(None).await;
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base}/__forge/inject/stuck"))
        .json(&serde_json::json!({"agent": "inspector"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 503);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["error"]
        .as_str()
        .unwrap()
        .contains("no system runtime"));
}

// ── Inject: unknown failure type (400) ──────────────────────────

#[tokio::test]
async fn inject_returns_400_for_unknown_failure_type() {
    let (senders, _rxs) = make_signal_senders(&["inspector"]);
    let base = spawn_inject_server(Some(senders)).await;
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base}/__forge/inject/unknown_thing"))
        .json(&serde_json::json!({"agent": "inspector"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["error"]
        .as_str()
        .unwrap()
        .contains("unknown failure type"));
    assert!(body["valid"].is_array());
}

// ── Inject: invalid body (400) ──────────────────────────────────

#[tokio::test]
async fn inject_returns_400_for_missing_agent_field() {
    let (senders, _rxs) = make_signal_senders(&["inspector"]);
    let base = spawn_inject_server(Some(senders)).await;
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base}/__forge/inject/stuck"))
        .json(&serde_json::json!({"wrong_field": "value"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["error"].as_str().unwrap().contains("missing 'agent'"));
}

#[tokio::test]
async fn inject_returns_400_for_invalid_json() {
    let (senders, _rxs) = make_signal_senders(&["inspector"]);
    let base = spawn_inject_server(Some(senders)).await;
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base}/__forge/inject/stuck"))
        .header("content-type", "application/json")
        .body("not json")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["error"].as_str().unwrap().contains("invalid JSON"));
}

// ── Inject: unknown agent (404) ─────────────────────────────────

#[tokio::test]
async fn inject_returns_404_for_unknown_agent() {
    let (senders, _rxs) = make_signal_senders(&["inspector", "analyst"]);
    let base = spawn_inject_server(Some(senders)).await;
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base}/__forge/inject/stuck"))
        .json(&serde_json::json!({"agent": "nonexistent"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["error"].as_str().unwrap().contains("not found"));
    assert!(body["available_agents"].is_array());
}

// ── Inject: successful injection (200) ──────────────────────────

#[tokio::test]
async fn inject_stuck_sends_signal_and_returns_200() {
    let (senders, mut rxs) = make_signal_senders(&["inspector"]);
    let base = spawn_inject_server(Some(senders)).await;
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base}/__forge/inject/stuck"))
        .json(&serde_json::json!({"agent": "inspector"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["injected"], "stuck");
    assert_eq!(body["agent"], "inspector");

    // Verify the signal was actually sent
    let rx = rxs.get_mut("inspector").unwrap();
    let signal = rx.try_recv().expect("should have received a signal");
    assert!(matches!(signal, AgentSignal::Stuck { agent_name } if agent_name == "inspector"));
}

#[tokio::test]
async fn inject_crash_sends_signal_and_returns_200() {
    let (senders, mut rxs) = make_signal_senders(&["analyst"]);
    let base = spawn_inject_server(Some(senders)).await;
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base}/__forge/inject/crash"))
        .json(&serde_json::json!({"agent": "analyst"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["injected"], "crash");
    assert_eq!(body["agent"], "analyst");

    let rx = rxs.get_mut("analyst").unwrap();
    let signal = rx.try_recv().expect("should have received a signal");
    assert!(matches!(signal, AgentSignal::Crash { agent_name } if agent_name == "analyst"));
}

#[tokio::test]
async fn inject_timeout_sends_signal_and_returns_200() {
    let (senders, mut rxs) = make_signal_senders(&["analyst"]);
    let base = spawn_inject_server(Some(senders)).await;
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base}/__forge/inject/timeout"))
        .json(&serde_json::json!({"agent": "analyst"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["injected"], "timeout");

    let rx = rxs.get_mut("analyst").unwrap();
    let signal = rx.try_recv().expect("should have received a signal");
    assert!(matches!(signal, AgentSignal::Timeout { .. }));
}

#[tokio::test]
async fn inject_hallucination_sends_signal_and_returns_200() {
    let (senders, mut rxs) = make_signal_senders(&["analyst"]);
    let base = spawn_inject_server(Some(senders)).await;
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base}/__forge/inject/hallucination"))
        .json(&serde_json::json!({"agent": "analyst"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["injected"], "hallucination");

    let rx = rxs.get_mut("analyst").unwrap();
    let signal = rx.try_recv().expect("should have received a signal");
    assert!(matches!(signal, AgentSignal::Hallucination { .. }));
}

#[tokio::test]
async fn inject_budget_sends_signal_and_returns_200() {
    let (senders, mut rxs) = make_signal_senders(&["analyst"]);
    let base = spawn_inject_server(Some(senders)).await;
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base}/__forge/inject/budget"))
        .json(&serde_json::json!({"agent": "analyst"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["injected"], "budget");

    let rx = rxs.get_mut("analyst").unwrap();
    let signal = rx.try_recv().expect("should have received a signal");
    assert!(matches!(signal, AgentSignal::BudgetExceeded { .. }));
}
