// FORGE introspection API integration tests — issue #139

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use forge::config::ForgeConfig;
use forge::llm::registry::ProviderRegistry;
use forge::runtime::agent::{AgentContext, TimerManager};
use forge::runtime::confidence::{ConfidentValue, Value};
use forge::runtime::event_bus::EventBus;
use forge::runtime::executor::TaskExecutor;
use forge::runtime::http_server::ForgeServer;
use forge::runtime::instance_registry::{InstanceRegistry, SharedInstanceRegistry};
use forge::runtime::memory::AgentMemory;
use forge::runtime::storage::ForgeStorage;
use forge::runtime::system::TopologySnapshot;
use forge::runtime::warded::{SharedWardenSnapshots, WardenSnapshot};

// ── Helpers ──────────────────────────────────────────────────────

fn mock_registry() -> Arc<ProviderRegistry> {
    let config = ForgeConfig::default_mock_config();
    Arc::new(ProviderRegistry::from_config(config).expect("mock registry should build"))
}

fn parse_and_build(source: &str) -> TaskExecutor {
    let program = forge::parser::parse(source).expect("parse failed");
    TaskExecutor::new(program, mock_registry(), None)
}

/// Minimal FORGE source with a dummy endpoint for the server to serve.
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

/// Spawn a server with introspection handles wired up.
async fn spawn_inspect_server(
    registry: Option<SharedInstanceRegistry>,
    storage: Option<Arc<ForgeStorage>>,
    warden_snaps: Option<SharedWardenSnapshots>,
    topology: Option<TopologySnapshot>,
    event_bus: Option<forge::runtime::event_bus::SharedEventBus>,
) -> String {
    let executor = parse_and_build(MINIMAL_SOURCE);
    let mut server = ForgeServer::new(executor, None);

    if let Some(reg) = registry {
        server = server.with_instance_registry(reg);
    }
    if let Some(s) = storage {
        server = server.with_inspect_storage(s);
    }
    if let Some(ws) = warden_snaps {
        server = server.with_warden_snapshots(ws);
    }
    if let Some(t) = topology {
        server = server.with_topology(t);
    }
    if let Some(bus) = event_bus {
        server = server.with_event_bus(bus);
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

// ── /inspect/agents ────────────────────────────────────────────

#[tokio::test]
async fn inspect_agents_empty_without_registry() {
    let base = spawn_inspect_server(None, None, None, None, None).await;
    let resp = reqwest::get(format!("{base}/__forge/inspect/agents"))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body, serde_json::json!([]));
}

#[tokio::test]
async fn inspect_agents_with_populated_registry() {
    let registry: SharedInstanceRegistry =
        Arc::new(tokio::sync::RwLock::new(InstanceRegistry::new()));
    {
        let mut guard = registry.write().await;
        guard.register("git_inspector", Some("inspector"));
        guard.register("analyst", None);
    }

    let base = spawn_inspect_server(Some(registry), None, None, None, None).await;
    let resp = reqwest::get(format!("{base}/__forge/inspect/agents"))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Vec<serde_json::Value> = resp.json().await.unwrap();
    assert_eq!(body.len(), 2);

    // Check fields are present
    for agent in &body {
        assert!(agent.get("id").is_some());
        assert!(agent.get("name").is_some());
        assert!(agent.get("uptime_ms").is_some());
        assert_eq!(agent["status"], "running");
    }
}

// ── /inspect/agents/:id ────────────────────────────────────────

#[tokio::test]
async fn inspect_agent_not_found() {
    let registry: SharedInstanceRegistry =
        Arc::new(tokio::sync::RwLock::new(InstanceRegistry::new()));
    let base = spawn_inspect_server(Some(registry), None, None, None, None).await;

    let resp = reqwest::get(format!(
        "{base}/__forge/inspect/agents/00000000-0000-0000-0000-000000000000"
    ))
    .await
    .unwrap();
    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn inspect_agent_invalid_uuid() {
    let registry: SharedInstanceRegistry =
        Arc::new(tokio::sync::RwLock::new(InstanceRegistry::new()));
    let base = spawn_inspect_server(Some(registry), None, None, None, None).await;

    let resp = reqwest::get(format!("{base}/__forge/inspect/agents/not-a-uuid"))
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn inspect_agent_by_id_returns_summary() {
    let registry: SharedInstanceRegistry =
        Arc::new(tokio::sync::RwLock::new(InstanceRegistry::new()));
    let id = {
        let mut guard = registry.write().await;
        guard.register("test_agent", Some("my_alias"))
    };

    let base = spawn_inspect_server(Some(registry), None, None, None, None).await;
    let resp = reqwest::get(format!("{base}/__forge/inspect/agents/{id}"))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["name"], "test_agent");
    assert_eq!(body["alias"], "my_alias");
    assert_eq!(body["status"], "running");
}

#[tokio::test]
async fn inspect_agent_deep_returns_memory_and_flags() {
    let registry: SharedInstanceRegistry =
        Arc::new(tokio::sync::RwLock::new(InstanceRegistry::new()));

    // Build an AgentContext with memory fields
    let mut memory = AgentMemory::empty();
    memory.set(
        "scan_count",
        ConfidentValue::deterministic(Value::Number(5.0)),
    );
    memory.set(
        "last_health",
        ConfidentValue::deterministic(Value::Text("healthy".to_string())),
    );

    let ctx = AgentContext::new(memory, None, None, TimerManager::empty(), 3);
    let ctx_ref = Arc::new(Mutex::new(ctx));

    let id = {
        let mut guard = registry.write().await;
        guard.register_with_context("git_inspector", Some("inspector"), ctx_ref)
    };

    let base = spawn_inspect_server(Some(registry), None, None, None, None).await;
    let resp = reqwest::get(format!("{base}/__forge/inspect/agents/{id}"))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();

    // Verify agent metadata
    assert_eq!(body["name"], "git_inspector");
    assert_eq!(body["alias"], "inspector");
    assert_eq!(body["status"], "running");

    // Verify deep inspection: memory fields present
    assert!(body.get("memory").is_some(), "missing memory field");
    let mem = &body["memory"];
    assert!(mem.get("scan_count").is_some(), "missing scan_count in memory");
    assert!(mem.get("last_health").is_some(), "missing last_health in memory");

    // Verify stuck/hallucination flags
    assert_eq!(body["stuck"], false);
    assert_eq!(body["hallucinating"], false);

    // Verify event counts
    assert_eq!(body["event_count"], 0);
    assert_eq!(body["escalation_count"], 0);
}

// ── /inspect/topology ──────────────────────────────────────────

#[tokio::test]
async fn inspect_topology_empty() {
    let base = spawn_inspect_server(None, None, None, None, None).await;
    let resp = reqwest::get(format!("{base}/__forge/inspect/topology"))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body, serde_json::json!({}));
}

#[tokio::test]
async fn inspect_topology_with_snapshot() {
    let topo = TopologySnapshot {
        system_name: "test_system".to_string(),
        bindings: vec![
            ("inspector".to_string(), "git_inspector".to_string()),
            ("analyst".to_string(), "code_analyst".to_string()),
        ],
        wiring: vec![vec!["inspector".to_string(), "analyst".to_string()]],
    };

    let bus = EventBus::new_shared(None);
    {
        let mut guard = bus.write().await;
        let _rx = guard.subscribe("ScanComplete", "analyst", None);
        guard.add_route("git_inspector", "code_analyst");
    }

    let base = spawn_inspect_server(None, None, None, Some(topo), Some(bus)).await;
    let resp = reqwest::get(format!("{base}/__forge/inspect/topology"))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["system_name"], "test_system");
    assert_eq!(body["bindings"].as_array().unwrap().len(), 2);
    assert_eq!(body["wiring"].as_array().unwrap().len(), 1);
    assert_eq!(body["subscribers"].as_array().unwrap().len(), 1);
    assert_eq!(body["subscribers"][0]["event"], "ScanComplete");
    assert_eq!(body["routes"]["git_inspector"][0], "code_analyst");
}

// ── /inspect/wardens ───────────────────────────────────────────

#[tokio::test]
async fn inspect_wardens_empty() {
    let base = spawn_inspect_server(None, None, None, None, None).await;
    let resp = reqwest::get(format!("{base}/__forge/inspect/wardens"))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body, serde_json::json!([]));
}

#[tokio::test]
async fn inspect_wardens_with_snapshot() {
    let snaps: SharedWardenSnapshots = Arc::new(tokio::sync::RwLock::new(vec![WardenSnapshot {
        name: "repo_warden".to_string(),
        managed_agents: vec!["inspector".to_string(), "analyst".to_string()],
        degraded_agents: vec![],
        retry_counts: {
            let mut m = HashMap::new();
            m.insert("inspector:Stuck".to_string(), 2);
            m
        },
        circuit_breaker_tripped: false,
    }]));

    let base = spawn_inspect_server(None, None, Some(snaps), None, None).await;
    let resp = reqwest::get(format!("{base}/__forge/inspect/wardens"))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Vec<serde_json::Value> = resp.json().await.unwrap();
    assert_eq!(body.len(), 1);
    assert_eq!(body[0]["name"], "repo_warden");
    assert_eq!(body[0]["managed_agents"].as_array().unwrap().len(), 2);
    assert_eq!(body[0]["retry_counts"]["inspector:Stuck"], 2);
    assert_eq!(body[0]["circuit_breaker_tripped"], false);
}

// ── /inspect/storage ───────────────────────────────────────────

#[tokio::test]
async fn inspect_storage_empty() {
    let base = spawn_inspect_server(None, None, None, None, None).await;
    let resp = reqwest::get(format!("{base}/__forge/inspect/storage"))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body, serde_json::json!([]));
}

#[tokio::test]
async fn inspect_storage_lists_keys_with_sizes() {
    let dir = tempfile::tempdir().unwrap();
    let storage = Arc::new(ForgeStorage::open(&dir.path().join("test.redb")).unwrap());
    storage.store("health:label", "healthy").unwrap();
    storage
        .store("agent:inspector:memory", r#"{"scan_count":5}"#)
        .unwrap();

    let base = spawn_inspect_server(None, Some(storage), None, None, None).await;
    let resp = reqwest::get(format!("{base}/__forge/inspect/storage"))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Vec<serde_json::Value> = resp.json().await.unwrap();
    assert_eq!(body.len(), 2);

    // Check structure
    for entry in &body {
        assert!(entry.get("key").is_some());
        assert!(entry.get("size_bytes").is_some());
    }
}

#[tokio::test]
async fn inspect_storage_with_prefix_filter() {
    let dir = tempfile::tempdir().unwrap();
    let storage = Arc::new(ForgeStorage::open(&dir.path().join("test.redb")).unwrap());
    storage.store("health:label", "healthy").unwrap();
    storage
        .store("agent:inspector:memory", r#"{"scan_count":5}"#)
        .unwrap();
    storage
        .store("agent:analyst:memory", r#"{"insight_count":3}"#)
        .unwrap();

    let base = spawn_inspect_server(None, Some(storage), None, None, None).await;
    let resp = reqwest::get(format!("{base}/__forge/inspect/storage?prefix=agent:"))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Vec<serde_json::Value> = resp.json().await.unwrap();
    assert_eq!(body.len(), 2);
    for entry in &body {
        assert!(entry["key"].as_str().unwrap().starts_with("agent:"));
    }
}
