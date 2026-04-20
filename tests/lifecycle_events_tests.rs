// FORGE lifecycle event tests — issue #255
//
// Verifies that AgentStarted / HandlerStarted / HandlerCompleted / AgentShutdown
// are emitted by AgentProcess and captured by the tracer.

use std::collections::HashMap;
use std::sync::Arc;

use forge::ast::*;
use forge::llm::providers::mock::MockProvider;
use forge::llm::registry::ProviderRegistry;
use forge::runtime::agent::AgentProcess;
use forge::runtime::event_bus::{EventBus, EventPayload};
use forge::tracer::Tracer;

// ── Helpers ──────────────────────────────────────────────────────────────────

fn spanned<T>(node: T) -> Spanned<T> {
    Spanned::new(node, Span { start: 0, end: 0 })
}

fn empty_program() -> Program {
    Program {
        boundary: None,
        items: vec![],
    }
}

fn mock_registry() -> Arc<ProviderRegistry> {
    let mock = MockProvider::new("mock").with_default("mock response");
    let mut reg = ProviderRegistry::new("mock");
    reg.register("mock", Arc::new(mock));
    Arc::new(reg)
}

fn payload(name: &str, source: &str) -> EventPayload {
    EventPayload {
        event_name: name.to_string(),
        args: vec![],
        source_agent: source.to_string(),
        fields: HashMap::new(),
    }
}

/// Minimal agent with a `give "ok"` handler for `Ping`.
fn ping_agent(name: &str) -> AgentDecl {
    AgentDecl {
        exportable: false,
        name: spanned(name.into()),
        lifecycle: None,
        memory: vec![],
        memory_persistent: false,
        knowledge: None,
        timers: vec![],
        schedules: vec![],
        correlates: vec![],
        subscriptions: vec![spanned(SubscribeDecl {
            event_name: spanned("Ping".into()),
            filter: None,
        })],
        handlers: vec![spanned(OnHandler {
            event: spanned("Ping".into()),
            params: vec![],
            payload_type: None,
            requires: vec![],
            body: vec![spanned(Stmt::Give(
                spanned(Expr::Template(vec![spanned(TemplatePart::Text(
                    "ok".into(),
                ))])),
                vec![],
            ))],
        })],
        warden_override: Vec::new(),
        stuck_policy: None,
    }
}

/// Agent whose `Ping` handler has an always-false requires guard with Silent fail.
fn ping_agent_with_failing_requires(name: &str) -> AgentDecl {
    let mut decl = ping_agent(name);
    decl.handlers[0].node.requires = vec![spanned(RequiresClause {
        condition: spanned(Expr::BoolLit(false)),
        on_fail: Some(spanned(FailPolicy::Silent)),
    })];
    decl
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn emits_agent_started_handler_pair_and_shutdown_on_success() {
    let tracer = Tracer::with_capture();
    let bus = EventBus::new_shared(None);

    let mut agent = AgentProcess::new(
        ping_agent("pinger"),
        None,
        mock_registry(),
        Some(tracer.clone()),
        empty_program(),
        None,
        None,
        None,
    )
    .with_event_bus(bus.clone())
    .await;

    // Publish one Ping then close the bus so run() returns.
    {
        let bus_guard = bus.read().await;
        bus_guard.publish(&payload("Ping", "external"));
    }
    bus.write().await.close();

    agent.run().await.expect("agent run");

    let log = tracer.captured_log();
    let names: Vec<&str> = log.iter().map(|(n, _)| n.as_str()).collect();

    // Lifecycle skeleton appears in this order.
    let started_pos = names
        .iter()
        .position(|n| *n == "AgentStarted")
        .expect("AgentStarted missing");
    let handler_started_pos = names
        .iter()
        .position(|n| *n == "HandlerStarted")
        .expect("HandlerStarted missing");
    let handler_done_pos = names
        .iter()
        .position(|n| *n == "HandlerCompleted")
        .expect("HandlerCompleted missing");
    let shutdown_pos = names
        .iter()
        .position(|n| *n == "AgentShutdown")
        .expect("AgentShutdown missing");

    assert!(started_pos < handler_started_pos);
    assert!(handler_started_pos < handler_done_pos);
    assert!(handler_done_pos < shutdown_pos);

    // Inspect HandlerCompleted payload: success + duration + confidence present.
    let (_, completed) = &log[handler_done_pos];
    assert_eq!(completed["event"], "HandlerCompleted");
    assert_eq!(completed["agent"], "pinger");
    assert_eq!(completed["handler"], "Ping");
    assert_eq!(completed["status"], "success");
    assert!(completed["duration_ms"].is_u64());
    assert!(completed["confidence"].is_number());

    // AgentStarted carries agent name and a pid.
    let (_, started) = &log[started_pos];
    assert_eq!(started["agent"], "pinger");
    assert!(started["pid"].is_u64());

    // AgentShutdown reason reflects the channel-closed exit path.
    let (_, shutdown) = &log[shutdown_pos];
    assert_eq!(shutdown["agent"], "pinger");
    assert_eq!(shutdown["reason"], "channel_closed");
}

#[tokio::test]
async fn blocked_by_requires_emits_handler_completed_without_started() {
    let tracer = Tracer::with_capture();
    let agent = AgentProcess::new(
        ping_agent_with_failing_requires("guarded"),
        None,
        mock_registry(),
        Some(tracer.clone()),
        empty_program(),
        None,
        None,
        None,
    );

    // Direct dispatch: requires guard fails, silent fail policy returns Ok(None).
    let out = agent.dispatch("Ping", HashMap::new()).await.unwrap();
    assert!(out.is_none());

    let log = tracer.captured_log();
    let names: Vec<&str> = log.iter().map(|(n, _)| n.as_str()).collect();

    assert!(
        !names.contains(&"HandlerStarted"),
        "HandlerStarted must not fire when requires blocks dispatch"
    );
    let completed = log
        .iter()
        .find(|(n, _)| n == "HandlerCompleted")
        .expect("HandlerCompleted missing");
    assert_eq!(completed.1["status"], "blocked_by_requires");
    assert_eq!(completed.1["duration_ms"], 0);
}
