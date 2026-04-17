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
fn subscribing_agent(name: &str, event_name: &str, filter: Option<Spanned<Expr>>) -> AgentDecl {
    AgentDecl {
        exportable: false,
        name: spanned(name.into()),
        lifecycle: None,
        memory: vec![],
        memory_persistent: false,
        knowledge: None,
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
                spanned(Expr::Template(vec![spanned(TemplatePart::Text(
                    "handled".into(),
                ))])),
                vec![],
            ))],
        })],
        warden_override: Vec::new(),
        stuck_policy: None,
    }
}

/// Build an agent that emits an event from its handler.
fn emitting_agent(name: &str, trigger_event: &str, emit_event: &str) -> AgentDecl {
    AgentDecl {
        exportable: false,
        name: spanned(name.into()),
        lifecycle: None,
        memory: vec![],
        memory_persistent: false,
        knowledge: None,
        timers: vec![],
        subscriptions: vec![],
        handlers: vec![spanned(OnHandler {
            event: spanned(trigger_event.into()),
            params: vec![],
            payload_type: None,
            requires: vec![],
            body: vec![
                spanned(Stmt::Emit(spanned(emit_event.into()), vec![])),
                spanned(Stmt::Give(
                    spanned(Expr::Template(vec![spanned(TemplatePart::Text(
                        "emitted".into(),
                    ))])),
                    vec![],
                )),
            ],
        })],
        warden_override: Vec::new(),
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

    let p = payload_with_fields(
        "MoveEvent",
        "agent-a",
        vec![("room_id", "room-001"), ("player", "Alice")],
    );
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
        decl,
        None,
        mock_registry(),
        None,
        empty_program(),
        None,
        None,
        None,
    )
    .with_event_bus(bus.clone())
    .await;

    let bus_guard = bus.read().await;
    assert_eq!(bus_guard.subscriber_count("MoveEvent"), 1);
}

#[tokio::test]
async fn agent_run_receives_and_dispatches_event() {
    let decl = subscribing_agent("listener", "MoveEvent", None);
    let bus = EventBus::new_shared(None);

    let mut agent = AgentProcess::new(
        decl,
        None,
        mock_registry(),
        None,
        empty_program(),
        None,
        None,
        None,
    )
    .with_event_bus(bus.clone())
    .await;

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
        decl_a,
        None,
        mock_registry(),
        None,
        empty_program(),
        None,
        None,
        None,
    )
    .with_event_bus(bus.clone())
    .await;

    let _agent_b = AgentProcess::new(
        decl_b,
        None,
        mock_registry(),
        None,
        empty_program(),
        None,
        None,
        None,
    )
    .with_event_bus(bus.clone())
    .await;

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
        Box::new(spanned(Expr::Template(vec![spanned(TemplatePart::Text(
            "room-001".into(),
        ))]))),
    ));

    let decl = subscribing_agent("listener", "MoveEvent", Some(filter));
    let bus = EventBus::new_shared(None);

    let mut agent = AgentProcess::new(
        decl,
        None,
        mock_registry(),
        None,
        empty_program(),
        None,
        None,
        None,
    )
    .with_event_bus(bus.clone())
    .await;

    // Publish matching event
    {
        let bus_guard = bus.read().await;
        bus_guard.publish(&payload_with_fields(
            "MoveEvent",
            "emitter",
            vec![("room_id", "room-001")],
        ));
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
        Box::new(spanned(Expr::Template(vec![spanned(TemplatePart::Text(
            "room-001".into(),
        ))]))),
    ));

    let decl = subscribing_agent("listener", "MoveEvent", Some(filter));
    let bus = EventBus::new_shared(None);

    let mut agent = AgentProcess::new(
        decl,
        None,
        mock_registry(),
        None,
        empty_program(),
        None,
        None,
        None,
    )
    .with_event_bus(bus.clone())
    .await;

    // Publish NON-matching event (room-999)
    {
        let bus_guard = bus.read().await;
        bus_guard.publish(&payload_with_fields(
            "MoveEvent",
            "emitter",
            vec![("room_id", "room-999")],
        ));
    }
    bus.write().await.close();

    // run() should skip the event (filter rejects), no dispatch error
    let result = agent.run().await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn agent_filter_can_access_memory() {
    // Regression for #286 sibling: subscribe filters like
    // `subscribe LearnedInsight where category == memory.topic` are
    // documented (README.md, docs/forge-reference.md, forge-sensei/specialist)
    // and must resolve `memory.*` the same way handler bodies do. Prior to
    // the fix, should_handle() did not bind `memory` into the filter env, so
    // any event delivery would error out of filter eval, crash the agent
    // task, and the warden would restart it — with 3 crashes tripping the
    // circuit breaker after a handful of events.

    // Filter: event.category == memory.topic
    let filter = spanned(Expr::BinOp(
        Box::new(spanned(Expr::FieldAccess(
            Box::new(spanned(Expr::Ident("event".into()))),
            spanned("category".into()),
        ))),
        spanned(BinOp::Eq),
        Box::new(spanned(Expr::FieldAccess(
            Box::new(spanned(Expr::Ident("memory".into()))),
            spanned("topic".into()),
        ))),
    ));

    let mut decl = subscribing_agent("specialist_like", "LearnedInsight", Some(filter));
    // Give the agent a memory.topic field (defaults to empty Text).
    decl.memory = vec![spanned(FieldDef {
        name: "topic".into(),
        type_name: spanned(TypeName::Text),
    })];

    let bus = EventBus::new_shared(None);
    let mut agent = AgentProcess::new(
        decl,
        None,
        mock_registry(),
        None,
        empty_program(),
        None,
        None,
        None,
    )
    .with_event_bus(bus.clone())
    .await;

    // Publish a non-matching event (category set, memory.topic still empty).
    // Before the fix this would crash agent.run() because filter eval
    // errored on the unbound `memory` identifier.
    {
        let bus_guard = bus.read().await;
        bus_guard.publish(&payload_with_fields(
            "LearnedInsight",
            "emitter",
            vec![("category", "SYNTAX")],
        ));
    }
    bus.write().await.close();

    let result = agent.run().await;
    assert!(
        result.is_ok(),
        "agent.run() must not error when filter references memory.* — got {:?}",
        result
    );
}

// ── Multi-agent end-to-end scenario ─────────────────────────────────────────

/// Simulates a tic-tac-toe room scenario:
///   1. room_agent handles "join", updates memory.player_count, emits PlayerJoined(player: name)
///   2. observer subscribes to PlayerJoined, receives via bus, dispatches to handler
///
/// Exercises the full pipeline: emit with labeled args → EventSink → drain → bus
/// → subscriber channel → agent run loop → filter → handler dispatch.
#[tokio::test]
async fn multi_agent_event_flow() {
    // ── room_agent: handles "join", emits PlayerJoined ──────────────────
    let room_decl = AgentDecl {
        exportable: false,
        name: spanned("room_agent".into()),
        lifecycle: None,
        memory: vec![spanned(FieldDef {
            name: "player_count".into(),
            type_name: spanned(TypeName::Number),
        })],
        memory_persistent: false,
        knowledge: None,
        timers: vec![],
        subscriptions: vec![],
        handlers: vec![spanned(OnHandler {
            event: spanned("join".into()),
            params: vec![spanned(Param {
                name: "player".into(),
                type_name: spanned(TypeName::Text),
            })],
            payload_type: None,
            requires: vec![],
            body: vec![
                // memory.player_count = memory.player_count + 1
                spanned(Stmt::MemoryUpdate(
                    spanned("player_count".into()),
                    None,
                    spanned(Expr::BinOp(
                        Box::new(spanned(Expr::FieldAccess(
                            Box::new(spanned(Expr::Ident("memory".into()))),
                            spanned("player_count".into()),
                        ))),
                        spanned(BinOp::Add),
                        Box::new(spanned(Expr::NumberLit(1.0))),
                    )),
                )),
                // emit PlayerJoined(player: player)
                spanned(Stmt::Emit(
                    spanned("PlayerJoined".into()),
                    vec![spanned(CallArg {
                        label: Some(spanned("player".into())),
                        value: spanned(Expr::Ident("player".into())),
                    })],
                )),
                spanned(Stmt::Give(
                    spanned(Expr::Template(vec![spanned(TemplatePart::Text(
                        "joined".into(),
                    ))])),
                    vec![],
                )),
            ],
        })],
        warden_override: Vec::new(),
        stuck_policy: None,
    };

    // ── observer: subscribes to PlayerJoined, gives "observed" ──────────
    let observer_decl = subscribing_agent("observer", "PlayerJoined", None);

    // ── Wire both agents to shared bus ──────────────────────────────────
    let bus = EventBus::new_shared(None);

    let room_agent = AgentProcess::new(
        room_decl,
        None,
        mock_registry(),
        None,
        empty_program(),
        None,
        None,
        None,
    )
    .with_event_bus(bus.clone())
    .await;

    let mut observer = AgentProcess::new(
        observer_decl,
        None,
        mock_registry(),
        None,
        empty_program(),
        None,
        None,
        None,
    )
    .with_event_bus(bus.clone())
    .await;

    // Verify observer registered its subscription
    assert_eq!(bus.read().await.subscriber_count("PlayerJoined"), 1);

    // ── Dispatch "join" to room_agent ───────────────────────────────────
    let mut params = HashMap::new();
    params.insert(
        "player".into(),
        ConfidentValue::deterministic(Value::Text("Alice".into())),
    );

    let result = room_agent.dispatch("join", params).await.unwrap();
    assert!(matches!(result, Some(ref v) if matches!(&v.value, Value::Text(s) if s == "joined")));

    // Verify room_agent memory updated
    {
        let ctx = room_agent.context().lock().unwrap();
        match &ctx.memory.get("player_count").unwrap().value {
            Value::Number(n) => assert_eq!(*n, 1.0),
            other => panic!("expected Number, got {:?}", other),
        }
    }

    // Verify room_agent emitted PlayerJoined with labeled field
    {
        let ctx = room_agent.context().lock().unwrap();
        assert_eq!(ctx.event_sink.emitted.len(), 1);
        assert_eq!(ctx.event_sink.emitted[0].name, "PlayerJoined");
        match &ctx.event_sink.emitted[0].fields["player"].value {
            Value::Text(s) => assert_eq!(s, "Alice"),
            other => panic!("expected Text, got {:?}", other),
        }
    }

    // ── Drain room_agent's events through the bus ───────────────────────
    // This simulates what run() does automatically after each handler
    {
        let (emitted, _forwards) = {
            let mut ctx = room_agent.context().lock().unwrap();
            let emitted = std::mem::take(&mut ctx.event_sink.emitted);
            let forwards = std::mem::take(&mut ctx.event_sink.forwards);
            (emitted, forwards)
        };
        let bus_guard = bus.read().await;
        for event in emitted {
            let payload = EventPayload {
                event_name: event.name,
                args: event.args,
                source_agent: "room_agent".to_string(),
                fields: event.fields,
            };
            let delivered = bus_guard.publish(&payload);
            assert_eq!(delivered, 1, "observer should have received the event");
        }
    }

    // ── Close bus so observer.run() terminates after processing ─────────
    bus.write().await.close();

    // ── Observer processes the event via run() ──────────────────────────
    let observer_result = observer.run().await;
    assert!(observer_result.is_ok());
}

/// Regression for #325: an `emit` from inside an agent handler must produce an
/// `event_emit` trace. Before the fix, `forge serve` built the shared bus with
/// a `None` tracer, so `EventBus::publish` — invoked by the agent's drain path —
/// silently skipped tracing while `say` / `HandlerCompleted` continued to work
/// via the executor's own tracer. That made agent-originated fan-out invisible
/// in the SSE event log.
#[tokio::test]
async fn agent_handler_emit_produces_event_emit_trace() {
    use forge::tracer::Tracer;

    // Agent: subscribes to Ping; handler body emits Pong and gives "ok".
    let decl = AgentDecl {
        exportable: false,
        name: spanned("pinger".into()),
        lifecycle: None,
        memory: vec![],
        memory_persistent: false,
        knowledge: None,
        timers: vec![],
        subscriptions: vec![spanned(SubscribeDecl {
            event_name: spanned("Ping".into()),
            filter: None,
        })],
        handlers: vec![spanned(OnHandler {
            event: spanned("Ping".into()),
            params: vec![],
            payload_type: None,
            requires: vec![],
            body: vec![
                spanned(Stmt::Emit(spanned("Pong".into()), vec![])),
                spanned(Stmt::Give(
                    spanned(Expr::Template(vec![spanned(TemplatePart::Text(
                        "ok".into(),
                    ))])),
                    vec![],
                )),
            ],
        })],
        warden_override: Vec::new(),
        stuck_policy: None,
    };

    let tracer = Tracer::with_capture();
    let bus = EventBus::new_shared(Some(tracer.clone()));

    let mut agent = AgentProcess::new(
        decl,
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

    // Publish Ping to the bus → agent handler runs → emits Pong → drain publishes
    // Pong through the bus (tracing event_emit, the path the #325 fix restores).
    {
        let bus_guard = bus.read().await;
        bus_guard.publish(&payload("Ping", "external"));
    }
    bus.write().await.close();

    agent.run().await.expect("agent run");

    let log = tracer.captured_log();
    let pong_emit = log
        .iter()
        .find(|(name, payload)| {
            name == "event_emit"
                && payload["source_agent"] == "pinger"
                && payload["event"] == "Pong"
        })
        .unwrap_or_else(|| {
            panic!(
                "event_emit trace for agent-originated Pong emit missing; log = {:?}",
                log.iter().map(|(n, _)| n).collect::<Vec<_>>()
            )
        });
    // No Pong subscribers registered → subscribers count is 0.
    assert_eq!(pong_emit.1["subscribers"], 0);
}

/// Regression for #325: reproduces the buggy pre-fix wiring by constructing
/// the bus with a `None` tracer (as `forge serve` used to do) while the agent
/// itself has a capturing tracer. The executor's tracer still captures
/// `HandlerStarted`/`HandlerCompleted`/`AgentShutdown`, but — by design of the
/// drain path — the agent-originated `event_emit` trace is absent because it
/// fires via the bus's own tracer. This test locks in *why* the main.rs fix is
/// required: without passing the tracer into the bus, agent emits stay invisible.
#[tokio::test]
async fn agent_handler_emit_is_invisible_when_bus_has_no_tracer() {
    use forge::tracer::Tracer;

    let decl = AgentDecl {
        exportable: false,
        name: spanned("pinger".into()),
        lifecycle: None,
        memory: vec![],
        memory_persistent: false,
        knowledge: None,
        timers: vec![],
        subscriptions: vec![spanned(SubscribeDecl {
            event_name: spanned("Ping".into()),
            filter: None,
        })],
        handlers: vec![spanned(OnHandler {
            event: spanned("Ping".into()),
            params: vec![],
            payload_type: None,
            requires: vec![],
            body: vec![
                spanned(Stmt::Emit(spanned("Pong".into()), vec![])),
                spanned(Stmt::Give(
                    spanned(Expr::Template(vec![spanned(TemplatePart::Text(
                        "ok".into(),
                    ))])),
                    vec![],
                )),
            ],
        })],
        warden_override: Vec::new(),
        stuck_policy: None,
    };

    let tracer = Tracer::with_capture();
    // Buggy pre-fix shape: bus built without a tracer even though the executor has one.
    let bus = EventBus::new_shared(None);

    let mut agent = AgentProcess::new(
        decl,
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

    {
        let bus_guard = bus.read().await;
        bus_guard.publish(&payload("Ping", "external"));
    }
    bus.write().await.close();

    agent.run().await.expect("agent run");

    let log = tracer.captured_log();
    let handler_completed_seen = log.iter().any(|(n, _)| n == "HandlerCompleted");
    let event_emit_seen = log.iter().any(|(n, p)| {
        n == "event_emit" && p["source_agent"] == "pinger" && p["event"] == "Pong"
    });
    assert!(
        handler_completed_seen,
        "executor-side lifecycle traces should still fire even when bus tracer is None"
    );
    assert!(
        !event_emit_seen,
        "without a tracer on the bus, the agent-originated event_emit is silently dropped \
         — this is the #325 symptom that the main.rs fix eliminates by passing the \
         executor's tracer into EventBus::new_shared"
    );
}
