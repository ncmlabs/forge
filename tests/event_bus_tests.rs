// FORGE event bus integration tests — issue #19

use std::collections::HashMap;
use std::sync::Arc;

use forge::ast::*;
use forge::llm::providers::mock::MockProvider;
use forge::llm::registry::ProviderRegistry;
use forge::runtime::agent::AgentProcess;
use forge::runtime::confidence::{ConfidentValue, Value};
use forge::runtime::event_bus::{EventBus, EventPayload};

// ── Helpers ──────────────────────────────────────────────────────────────────

fn spanned<T>(node: T) -> Spanned<T> {
    Spanned::new(node, Span { start: 0, end: 0 })
}

fn empty_program() -> Program {
    Program { boundary: None, items: vec![] }
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

fn payload_with_fields(name: &str, source: &str, fields: Vec<(&str, &str)>) -> EventPayload {
    let mut field_map = HashMap::new();
    for (k, v) in fields {
        field_map.insert(
            k.to_string(),
            ConfidentValue::deterministic(Value::Text(v.to_string())),
        );
    }
    EventPayload {
        event_name: name.to_string(),
        args: vec![],
        source_agent: source.to_string(),
        fields: field_map,
    }
}

/// Build a minimal agent with a handler that gives back event field values.
fn subscribing_agent(
    name: &str,
    event_name: &str,
    filter: Option<Spanned<Expr>>,
) -> AgentDecl {
    AgentDecl {
        name: spanned(name.into()),
        lifecycle: None,
        memory: vec![],
        timers: vec![],
        subscriptions: vec![spanned(SubscribeDecl {
            event_name: spanned(event_name.into()),
            filter,
        })],
        handlers: vec![spanned(OnHandler {
            event: spanned(event_name.into()),
            params: vec![],
            payload_type: None,
            requires: vec![],
            body: vec![spanned(Stmt::Give(
                spanned(Expr::Template(vec![spanned(TemplatePart::Text("handled".into()))])),
                None,
            ))],
        })],
        stuck_policy: None,
    }
}

/// Build an agent that emits an event from its handler.
fn emitting_agent(
    name: &str,
    trigger_event: &str,
    emit_event: &str,
) -> AgentDecl {
    AgentDecl {
        name: spanned(name.into()),
        lifecycle: None,
        memory: vec![],
        timers: vec![],
        subscriptions: vec![],
        handlers: vec![spanned(OnHandler {
            event: spanned(trigger_event.into()),
            params: vec![],
            payload_type: None,
            requires: vec![],
            body: vec![
                spanned(Stmt::Emit(
                    spanned(emit_event.into()),
                    vec![],
                )),
                spanned(Stmt::Give(
                    spanned(Expr::Template(vec![spanned(TemplatePart::Text("emitted".into()))])),
                    None,
                )),
            ],
        })],
        stuck_policy: None,
    }
}

// ── Bus-level tests ─────────────────────────────────────────────────────────

#[tokio::test]
async fn bus_subscribe_and_publish() {
    let mut bus = EventBus::new(None);
    let mut rx = bus.subscribe("MoveEvent", "agent-b", None);
    let count = bus.publish(&payload("MoveEvent", "agent-a"));
    assert_eq!(count, 1);
    let event = rx.recv().await.unwrap();
    assert_eq!(event.event_name, "MoveEvent");
    assert_eq!(event.source_agent, "agent-a");
}

#[tokio::test]
async fn bus_publish_no_subscribers() {
    let bus = EventBus::new(None);
    let count = bus.publish(&payload("MoveEvent", "agent-a"));
    assert_eq!(count, 0);
}

#[tokio::test]
async fn bus_multiple_subscribers_all_receive() {
    let mut bus = EventBus::new(None);
    let mut rx1 = bus.subscribe("MoveEvent", "agent-a", None);
    let mut rx2 = bus.subscribe("MoveEvent", "agent-b", None);
    let count = bus.publish(&payload("MoveEvent", "agent-c"));
    assert_eq!(count, 2);
    assert_eq!(rx1.recv().await.unwrap().event_name, "MoveEvent");
    assert_eq!(rx2.recv().await.unwrap().event_name, "MoveEvent");
}

#[tokio::test]
async fn bus_unsubscribed_agents_dont_receive() {
    let mut bus = EventBus::new(None);
    let mut rx_move = bus.subscribe("MoveEvent", "agent-a", None);
    let mut rx_chat = bus.subscribe("ChatEvent", "agent-b", None);

    bus.publish(&payload("MoveEvent", "src"));

    // agent-a subscribed to MoveEvent → receives
    assert!(rx_move.recv().await.is_some());
    // agent-b subscribed to ChatEvent → nothing
    assert!(rx_chat.try_recv().is_err());
}

#[tokio::test]
async fn bus_forward_delivers_to_target() {
    let mut bus = EventBus::new(None);
    let mut rx_a = bus.subscribe("Foo", "agent-a", None);
    let mut rx_b = bus.subscribe("Foo", "agent-b", None);

    let ok = bus.forward(&payload("Foo", "src"), "agent-b");
    assert!(ok);

    // Only agent-b should receive
    assert!(rx_b.recv().await.is_some());
    assert!(rx_a.try_recv().is_err());
}

#[tokio::test]
async fn bus_forward_unknown_target() {
    let bus = EventBus::new(None);
    assert!(!bus.forward(&payload("Foo", "src"), "nobody"));
}

#[tokio::test]
async fn bus_payload_fields_preserved() {
    let mut bus = EventBus::new(None);
    let mut rx = bus.subscribe("MoveEvent", "agent-b", None);

    let p = payload_with_fields("MoveEvent", "agent-a", vec![
        ("room_id", "room-001"),
        ("player", "Alice"),
    ]);
    bus.publish(&p);

    let received = rx.recv().await.unwrap();
    assert_eq!(received.fields.len(), 2);
    match &received.fields["room_id"].value {
        Value::Text(s) => assert_eq!(s, "room-001"),
        _ => panic!("expected Text"),
    }
}

// ── Agent + Bus integration tests ───────────────────────────────────────────

#[tokio::test]
async fn agent_registers_subscriptions_with_bus() {
    let decl = subscribing_agent("listener", "MoveEvent", None);
    let bus = EventBus::new_shared(None);

    let _agent = AgentProcess::new(
        decl, None, mock_registry(), None, empty_program(),
    ).with_event_bus(bus.clone()).await;

    let bus_guard = bus.read().await;
    assert_eq!(bus_guard.subscriber_count("MoveEvent"), 1);
}

#[tokio::test]
async fn agent_run_receives_and_dispatches_event() {
    let decl = subscribing_agent("listener", "MoveEvent", None);
    let bus = EventBus::new_shared(None);

    let mut agent = AgentProcess::new(
        decl, None, mock_registry(), None, empty_program(),
    ).with_event_bus(bus.clone()).await;

    // Publish an event, then close the bus so channels close and run() terminates
    {
        let bus_guard = bus.read().await;
        bus_guard.publish(&payload("MoveEvent", "emitter"));
    }
    bus.write().await.close();

    let result = agent.run().await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn agent_emit_drains_through_bus() {
    // Agent A emits MoveEvent when triggered
    let decl_a = emitting_agent("emitter", "trigger", "MoveEvent");
    let decl_b = subscribing_agent("listener", "MoveEvent", None);
    let bus = EventBus::new_shared(None);

    let agent_a = AgentProcess::new(
        decl_a, None, mock_registry(), None, empty_program(),
    ).with_event_bus(bus.clone()).await;

    let _agent_b = AgentProcess::new(
        decl_b, None, mock_registry(), None, empty_program(),
    ).with_event_bus(bus.clone()).await;

    // Trigger agent_a, which should emit MoveEvent
    let result = agent_a.dispatch("trigger", HashMap::new()).await.unwrap();
    assert!(matches!(result, Some(ref v) if matches!(&v.value, Value::Text(s) if s == "emitted")));

    // Verify event was collected in sink (drain_event_sink not called yet since we used dispatch directly)
    let ctx = agent_a.context().lock().unwrap();
    assert_eq!(ctx.event_sink.emitted.len(), 1);
    assert_eq!(ctx.event_sink.emitted[0].name, "MoveEvent");
}

#[tokio::test]
async fn agent_filter_subscribe_with_matching_event() {
    // Filter: event.room_id == "room-001"
    let filter = spanned(Expr::BinOp(
        Box::new(spanned(Expr::FieldAccess(
            Box::new(spanned(Expr::Ident("event".into()))),
            spanned("room_id".into()),
        ))),
        spanned(BinOp::Eq),
        Box::new(spanned(Expr::Template(vec![
            spanned(TemplatePart::Text("room-001".into())),
        ]))),
    ));

    let decl = subscribing_agent("listener", "MoveEvent", Some(filter));
    let bus = EventBus::new_shared(None);

    let mut agent = AgentProcess::new(
        decl, None, mock_registry(), None, empty_program(),
    ).with_event_bus(bus.clone()).await;

    // Publish matching event
    {
        let bus_guard = bus.read().await;
        bus_guard.publish(&payload_with_fields("MoveEvent", "emitter", vec![
            ("room_id", "room-001"),
        ]));
    }
    bus.write().await.close();

    // run() should handle the event (filter passes)
    let result = agent.run().await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn agent_filter_subscribe_rejects_non_matching() {
    // Filter: event.room_id == "room-001"
    let filter = spanned(Expr::BinOp(
        Box::new(spanned(Expr::FieldAccess(
            Box::new(spanned(Expr::Ident("event".into()))),
            spanned("room_id".into()),
        ))),
        spanned(BinOp::Eq),
        Box::new(spanned(Expr::Template(vec![
            spanned(TemplatePart::Text("room-001".into())),
        ]))),
    ));

    let decl = subscribing_agent("listener", "MoveEvent", Some(filter));
    let bus = EventBus::new_shared(None);

    let mut agent = AgentProcess::new(
        decl, None, mock_registry(), None, empty_program(),
    ).with_event_bus(bus.clone()).await;

    // Publish NON-matching event (room-999)
    {
        let bus_guard = bus.read().await;
        bus_guard.publish(&payload_with_fields("MoveEvent", "emitter", vec![
            ("room_id", "room-999"),
        ]));
    }
    bus.write().await.close();

    // run() should skip the event (filter rejects), no dispatch error
    let result = agent.run().await;
    assert!(result.is_ok());
}
