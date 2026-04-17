// Live regression for #325 — agent-originated `emit` must land on the
// `/__forge/events` SSE trace stream.
//
// This is not a unit test on the bus; it stands up the same `forge serve`
// topology that main.rs wires up (TaskExecutor + shared EventBus +
// SystemRuntime + live Tracer routed into a broadcast channel) and asserts
// that an emit from inside an agent handler produces an `event_emit` trace
// frame attributed to that agent. Before the fix, the bus was constructed
// with a `None` tracer and the frame was silently dropped.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use forge::compose;
use forge::runtime::confidence::{ConfidentValue, Value};
use forge::runtime::event_bus::EventBus;
use forge::runtime::executor::TaskExecutor;
use forge::runtime::instance_registry::InstanceRegistry;

const PING_PONG_SRC: &str = r#"#! boundary: server

event Ping
  id: Text

event Pong
  id: Text

agent pinger
  memory
    count: Number
  subscribe Ping

  on start
    memory.count = 0

  on Ping(id: Text)
    memory.count = memory.count + 1
    emit Pong(id: id)

warden w
  manages [pinger]
  on stuck: nudge, self
  on timeout: restart, self
  on crash: restart, self
  on hallucination: nudge, self
  on contradiction: nudge, self
  on budget: nudge, self

system ping_system
  use
    p: pinger

endpoint ping(id: Text) -> Text
  emit Ping(id: id)
  give "queued"
"#;

fn mock_registry() -> Arc<forge::llm::registry::ProviderRegistry> {
    let config = forge::config::ForgeConfig::default_mock_config();
    Arc::new(
        forge::llm::registry::ProviderRegistry::from_config(config)
            .expect("mock registry should build"),
    )
}

#[tokio::test]
async fn agent_emit_reaches_sse_broadcast_like_forge_serve() {
    let program = forge::parser::parse(PING_PONG_SRC).expect("parse ping/pong");
    // Run checkers the same way the serve path does, to make sure the test
    // exercises a program `forge serve` would accept.
    let sources = [compose::SourceFile {
        path: "ping_pong.forge".to_string(),
        source: PING_PONG_SRC.to_string(),
        program: program.clone(),
    }];
    let composed = compose::merge_programs(&sources).expect("merge");
    let diagnostics = forge::checker::check_all(&composed.program, "ping_pong.forge");
    let errors: Vec<_> = diagnostics
        .iter()
        .filter(|d| d.kind == forge::diagnostic::DiagnosticKind::Error)
        .collect();
    assert!(errors.is_empty(), "checker errors: {:?}", errors);

    // Same wiring shape as `src/main.rs` serve mode: broadcast channel feeds
    // both SSE subscribers and the tracer. A clone of the receiver is what
    // `/__forge/events` would stream to an HTTP client.
    let (events_tx, mut events_rx) = tokio::sync::broadcast::channel::<String>(256);
    let tracer = forge::tracer::Tracer::with_live(events_tx.clone());

    let executor = TaskExecutor::new(composed.program, mock_registry(), Some(tracer.clone()))
        .with_config(forge::config::ForgeConfig::default_mock_config());

    // The fix: bus is constructed with the executor's tracer, not `None`.
    // Without `executor.tracer().cloned()` here, agent-originated emits would
    // never reach `events_tx` and this test would fail — that is the #325 bug.
    let event_bus = EventBus::new_shared(executor.tracer().cloned());
    let instance_registry = Arc::new(tokio::sync::RwLock::new(InstanceRegistry::new()));

    let system_runtime = executor
        .build_system_runtime()
        .expect("build system runtime")
        .expect("ping_system should produce a runtime")
        .with_shared_infrastructure(event_bus.clone(), instance_registry.clone());

    // ForgeServer normally injects the bus into the executor clone on every
    // request (`http_server.rs:153`). For this test we're calling `exec_endpoint`
    // directly, so we have to do the injection ourselves.
    let executor = executor.with_event_bus(event_bus.clone());

    tokio::spawn(async move {
        let _ = system_runtime.start().await;
    });

    // Let the system runtime register subscriptions + run `on start`.
    tokio::time::sleep(Duration::from_millis(300)).await;

    let mut args = HashMap::new();
    args.insert(
        "id".to_string(),
        ConfidentValue::deterministic(Value::Text("ABC".to_string())),
    );
    let result = executor
        .exec_endpoint("ping", args, None)
        .await
        .expect("endpoint dispatch");
    match &result.value.value {
        Value::Text(s) => assert_eq!(s, "queued"),
        other => panic!("expected queued, got {:?}", other),
    }

    // Drain every frame the broadcast channel has produced within a short
    // window. The agent handler runs asynchronously in its own task, so we
    // give it a beat to drain Ping → run handler → emit Pong → drain again.
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

    let parsed: Vec<serde_json::Value> = frames
        .iter()
        .filter_map(|f| serde_json::from_str(f).ok())
        .collect();

    let find_emit = |source: &str, name: &str| {
        parsed.iter().find(|v| {
            v["event"] == name && v["source_agent"] == source
        })
    };

    let ping = find_emit("endpoint", "Ping").unwrap_or_else(|| {
        panic!(
            "endpoint Ping emit missing from SSE broadcast; frames = {:#?}",
            frames
        )
    });
    let pong = find_emit("pinger", "Pong").unwrap_or_else(|| {
        panic!(
            "pinger Pong emit missing from SSE broadcast (this is the #325 symptom); \
             frames = {:#?}",
            frames
        )
    });

    assert_eq!(ping["subscribers"], 1, "Ping should reach pinger");
    assert_eq!(pong["subscribers"], 0, "no one subscribes to Pong");

    // Exactly one Ping emit frame — verifies the removal of the redundant
    // explicit executor-side `event_emit` that used to shadow the bus's trace
    // and would otherwise produce two frames per endpoint emit now that the
    // bus has a tracer.
    let ping_count = parsed
        .iter()
        .filter(|v| v["event"] == "Ping" && v["source_agent"] == "endpoint")
        .count();
    assert_eq!(
        ping_count, 1,
        "endpoint Ping must trace exactly once; frames = {:#?}",
        frames
    );
}
