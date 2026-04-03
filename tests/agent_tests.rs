// FORGE agent integration tests — issue #11

use std::collections::HashMap;
use std::sync::Arc;

use forge::ast::*;
use forge::llm::providers::mock::MockProvider;
use forge::llm::registry::ProviderRegistry;
use forge::runtime::agent::*;
use forge::runtime::confidence::{ConfidentValue, Value};
use forge::runtime::memory::AgentMemory;

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

/// Build a minimal agent decl with given handlers and memory fields.
fn simple_agent(
    memory_fields: Vec<Spanned<FieldDef>>,
    handlers: Vec<Spanned<OnHandler>>,
    stuck_policy: Option<Spanned<StuckPolicy>>,
) -> AgentDecl {
    AgentDecl {
        name: spanned("test_agent".into()),
        lifecycle: None,
        memory: memory_fields,
        knowledge: None,
        timers: vec![],
        subscriptions: vec![],
        warden_override: Vec::new(),
        handlers,
        stuck_policy,
    }
}

fn text_field(name: &str) -> Spanned<FieldDef> {
    spanned(FieldDef {
        name: name.to_string(),
        type_name: spanned(TypeName::Text),
    })
}

fn number_field(name: &str) -> Spanned<FieldDef> {
    spanned(FieldDef {
        name: name.to_string(),
        type_name: spanned(TypeName::Number),
    })
}

// ── Memory tests ─────────────────────────────────────────────────────────────

#[test]
fn memory_init_from_fields() {
    let fields = vec![text_field("topic"), number_field("count")];
    let mem = AgentMemory::new(&fields);
    assert!(matches!(mem.get("topic").unwrap().value, Value::Text(ref s) if s.is_empty()));
    assert!(matches!(mem.get("count").unwrap().value, Value::Number(n) if n == 0.0));
    assert!(mem.get("nonexistent").is_none());
}

#[test]
fn memory_record_for_env() {
    let fields = vec![text_field("name"), number_field("score")];
    let mut mem = AgentMemory::new(&fields);
    mem.set(
        "name",
        ConfidentValue::deterministic(Value::Text("Alice".into())),
    );
    mem.set("score", ConfidentValue::deterministic(Value::Number(95.0)));
    match mem.to_record() {
        Value::Record(map) => {
            assert!(matches!(map["name"].value, Value::Text(ref s) if s == "Alice"));
            assert!(matches!(map["score"].value, Value::Number(n) if n == 95.0));
        }
        _ => panic!("expected Record"),
    }
}

// ── Handler dispatch tests ───────────────────────────────────────────────────

#[tokio::test]
async fn dispatch_selects_correct_handler() {
    // Handler that gives a static value
    let handler = spanned(OnHandler {
        event: spanned("greet".into()),
        params: vec![],
        payload_type: None,
        requires: vec![],
        body: vec![spanned(Stmt::Give(
            spanned(Expr::Template(vec![spanned(TemplatePart::Text(
                "hello".into(),
            ))])),
            vec![],
        ))],
    });

    let decl = simple_agent(vec![], vec![handler], None);
    let agent = AgentProcess::new(decl, None, mock_registry(), None, empty_program());
    let result = agent.dispatch("greet", HashMap::new()).await.unwrap();
    assert!(matches!(result, Some(ref v) if matches!(&v.value, Value::Text(s) if s == "hello")));
}

#[tokio::test]
async fn dispatch_unknown_event_errors() {
    let decl = simple_agent(vec![], vec![], None);
    let agent = AgentProcess::new(decl, None, mock_registry(), None, empty_program());
    let result = agent.dispatch("nonexistent", HashMap::new()).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn dispatch_binds_params() {
    // Handler that gives the "name" param back
    let handler = spanned(OnHandler {
        event: spanned("greet".into()),
        params: vec![spanned(Param {
            name: "name".into(),
            type_name: spanned(TypeName::Text),
        })],
        payload_type: None,
        requires: vec![],
        body: vec![spanned(Stmt::Give(
            spanned(Expr::Ident("name".into())),
            vec![],
        ))],
    });

    let decl = simple_agent(vec![], vec![handler], None);
    let agent = AgentProcess::new(decl, None, mock_registry(), None, empty_program());
    let mut params = HashMap::new();
    params.insert(
        "name".into(),
        ConfidentValue::deterministic(Value::Text("World".into())),
    );
    let result = agent.dispatch("greet", params).await.unwrap();
    assert!(matches!(result, Some(ref v) if matches!(&v.value, Value::Text(s) if s == "World")));
}

// ── Memory update in handler ─────────────────────────────────────────────────

#[tokio::test]
async fn memory_update_persists_across_dispatches() {
    // Handler that sets memory.topic then gives it back
    let handler = spanned(OnHandler {
        event: spanned("set_topic".into()),
        params: vec![spanned(Param {
            name: "t".into(),
            type_name: spanned(TypeName::Text),
        })],
        payload_type: None,
        requires: vec![],
        body: vec![spanned(Stmt::MemoryUpdate(
            spanned("topic".into()),
            None,
            spanned(Expr::Ident("t".into())),
        ))],
    });

    let read_handler = spanned(OnHandler {
        event: spanned("get_topic".into()),
        params: vec![],
        payload_type: None,
        requires: vec![],
        body: vec![spanned(Stmt::Give(
            spanned(Expr::FieldAccess(
                Box::new(spanned(Expr::Ident("memory".into()))),
                spanned("topic".into()),
            )),
            vec![],
        ))],
    });

    let decl = simple_agent(vec![text_field("topic")], vec![handler, read_handler], None);
    let agent = AgentProcess::new(decl, None, mock_registry(), None, empty_program());

    // Set topic
    let mut params = HashMap::new();
    params.insert(
        "t".into(),
        ConfidentValue::deterministic(Value::Text("billing".into())),
    );
    agent.dispatch("set_topic", params).await.unwrap();

    // Read topic back
    let result = agent.dispatch("get_topic", HashMap::new()).await.unwrap();
    assert!(matches!(result, Some(ref v) if matches!(&v.value, Value::Text(s) if s == "billing")));
}

// ── Requires guard tests ─────────────────────────────────────────────────────

#[tokio::test]
async fn requires_pass_executes_handler() {
    let handler = spanned(OnHandler {
        event: spanned("action".into()),
        params: vec![],
        payload_type: None,
        requires: vec![spanned(RequiresClause {
            condition: spanned(Expr::BoolLit(true)),
            on_fail: None,
        })],
        body: vec![spanned(Stmt::Give(
            spanned(Expr::Template(vec![spanned(TemplatePart::Text(
                "ok".into(),
            ))])),
            vec![],
        ))],
    });

    let decl = simple_agent(vec![], vec![handler], None);
    let agent = AgentProcess::new(decl, None, mock_registry(), None, empty_program());
    let result = agent.dispatch("action", HashMap::new()).await.unwrap();
    assert!(matches!(result, Some(ref v) if matches!(&v.value, Value::Text(s) if s == "ok")));
}

#[tokio::test]
async fn requires_fail_silent_skips() {
    let handler = spanned(OnHandler {
        event: spanned("action".into()),
        params: vec![],
        payload_type: None,
        requires: vec![spanned(RequiresClause {
            condition: spanned(Expr::BoolLit(false)),
            on_fail: Some(spanned(FailPolicy::Silent)),
        })],
        body: vec![spanned(Stmt::Give(
            spanned(Expr::Template(vec![spanned(TemplatePart::Text(
                "should not reach".into(),
            ))])),
            vec![],
        ))],
    });

    let decl = simple_agent(vec![], vec![handler], None);
    let agent = AgentProcess::new(decl, None, mock_registry(), None, empty_program());
    let result = agent.dispatch("action", HashMap::new()).await.unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn requires_fail_give_returns_value() {
    let handler = spanned(OnHandler {
        event: spanned("action".into()),
        params: vec![],
        payload_type: None,
        requires: vec![spanned(RequiresClause {
            condition: spanned(Expr::BoolLit(false)),
            on_fail: Some(spanned(FailPolicy::Give(spanned(Expr::Template(vec![
                spanned(TemplatePart::Text("denied".into())),
            ]))))),
        })],
        body: vec![spanned(Stmt::Give(
            spanned(Expr::Template(vec![spanned(TemplatePart::Text(
                "ok".into(),
            ))])),
            vec![],
        ))],
    });

    let decl = simple_agent(vec![], vec![handler], None);
    let agent = AgentProcess::new(decl, None, mock_registry(), None, empty_program());
    let result = agent.dispatch("action", HashMap::new()).await.unwrap();
    assert!(matches!(result, Some(ref v) if matches!(&v.value, Value::Text(s) if s == "denied")));
}

#[tokio::test]
async fn requires_fail_crash_returns_error() {
    let handler = spanned(OnHandler {
        event: spanned("action".into()),
        params: vec![],
        payload_type: None,
        requires: vec![spanned(RequiresClause {
            condition: spanned(Expr::BoolLit(false)),
            on_fail: Some(spanned(FailPolicy::Crash)),
        })],
        body: vec![],
    });

    let decl = simple_agent(vec![], vec![handler], None);
    let agent = AgentProcess::new(decl, None, mock_registry(), None, empty_program());
    let result = agent.dispatch("action", HashMap::new()).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn requires_fail_log_rejects() {
    let handler = spanned(OnHandler {
        event: spanned("action".into()),
        params: vec![],
        payload_type: None,
        requires: vec![spanned(RequiresClause {
            condition: spanned(Expr::BoolLit(false)),
            on_fail: Some(spanned(FailPolicy::Log)),
        })],
        body: vec![spanned(Stmt::Give(
            spanned(Expr::Template(vec![spanned(TemplatePart::Text(
                "should not reach".into(),
            ))])),
            vec![],
        ))],
    });

    let decl = simple_agent(vec![], vec![handler], None);
    let agent = AgentProcess::new(decl, None, mock_registry(), None, empty_program());
    let result = agent.dispatch("action", HashMap::new()).await.unwrap();
    // on fail: log rejects the handler (returns None) and logs to stderr
    assert!(result.is_none());
}

#[tokio::test]
async fn requires_short_circuits_on_first_failure() {
    // Two requires: first fails (false), second passes (true).
    // Handler body should NOT execute because first guard fails.
    // This proves evaluation order and short-circuit behavior.
    let handler = spanned(OnHandler {
        event: spanned("action".into()),
        params: vec![],
        payload_type: None,
        requires: vec![
            spanned(RequiresClause {
                condition: spanned(Expr::BoolLit(false)),
                on_fail: Some(spanned(FailPolicy::Give(spanned(Expr::Template(vec![
                    spanned(TemplatePart::Text("first_failed".into())),
                ]))))),
            }),
            spanned(RequiresClause {
                condition: spanned(Expr::BoolLit(true)),
                on_fail: Some(spanned(FailPolicy::Give(spanned(Expr::Template(vec![
                    spanned(TemplatePart::Text("second_failed".into())),
                ]))))),
            }),
        ],
        body: vec![spanned(Stmt::Give(
            spanned(Expr::Template(vec![spanned(TemplatePart::Text(
                "body_reached".into(),
            ))])),
            vec![],
        ))],
    });

    let decl = simple_agent(vec![], vec![handler], None);
    let agent = AgentProcess::new(decl, None, mock_registry(), None, empty_program());
    let result = agent.dispatch("action", HashMap::new()).await.unwrap();
    // Should get "first_failed" — proves first guard ran and short-circuited
    assert!(
        matches!(result, Some(ref v) if matches!(&v.value, Value::Text(s) if s == "first_failed"))
    );
}

// ── State machine via agent ──────────────────────────────────────────────────

#[tokio::test]
async fn state_machine_transition_via_handler() {
    let handler = spanned(OnHandler {
        event: spanned("activate".into()),
        params: vec![],
        payload_type: None,
        requires: vec![],
        body: vec![spanned(Stmt::TransitionTo(spanned("active".into())))],
    });

    let states = StatesDecl {
        name: spanned("Phase".into()),
        transitions: vec![spanned(StateTransition {
            from: spanned("idle".into()),
            to: spanned("active".into()),
            condition: None,
        })],
    };

    let decl = simple_agent(vec![], vec![handler], None);
    let agent = AgentProcess::new(decl, Some(&states), mock_registry(), None, empty_program());

    agent.dispatch("activate", HashMap::new()).await.unwrap();
    let ctx = agent.context().lock().unwrap();
    assert_eq!(ctx.state_machine.as_ref().unwrap().current, "active");
}

#[tokio::test]
async fn state_machine_invalid_transition_errors() {
    let handler = spanned(OnHandler {
        event: spanned("jump".into()),
        params: vec![],
        payload_type: None,
        requires: vec![],
        body: vec![spanned(Stmt::TransitionTo(spanned("done".into())))],
    });

    let states = StatesDecl {
        name: spanned("Phase".into()),
        transitions: vec![spanned(StateTransition {
            from: spanned("idle".into()),
            to: spanned("active".into()),
            condition: None,
        })],
    };

    let decl = simple_agent(vec![], vec![handler], None);
    let agent = AgentProcess::new(decl, Some(&states), mock_registry(), None, empty_program());
    let result = agent.dispatch("jump", HashMap::new()).await;
    assert!(result.is_err());
}

// ── Timer tests ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn timer_start_via_handler() {
    let handler = spanned(OnHandler {
        event: spanned("begin".into()),
        params: vec![],
        payload_type: None,
        requires: vec![],
        body: vec![spanned(Stmt::StartTimer {
            name: spanned("timeout".into()),
            context: None,
        })],
    });

    let mut decl = simple_agent(vec![], vec![handler], None);
    decl.timers = vec![spanned(TimerField {
        name: spanned("timeout".into()),
        duration: spanned(Duration {
            value: 10,
            unit: DurationUnit::Minutes,
        }),
    })];

    let agent = AgentProcess::new(decl, None, mock_registry(), None, empty_program());
    agent.dispatch("begin", HashMap::new()).await.unwrap();

    let ctx = agent.context().lock().unwrap();
    assert_eq!(
        ctx.timer_manager.state("timeout"),
        Some(&TimerState::Running)
    );
}

#[tokio::test]
async fn timer_cancel_via_handler() {
    let start_handler = spanned(OnHandler {
        event: spanned("begin".into()),
        params: vec![],
        payload_type: None,
        requires: vec![],
        body: vec![spanned(Stmt::StartTimer {
            name: spanned("timeout".into()),
            context: None,
        })],
    });

    let cancel_handler = spanned(OnHandler {
        event: spanned("stop".into()),
        params: vec![],
        payload_type: None,
        requires: vec![],
        body: vec![spanned(Stmt::CancelTimer {
            name: spanned("timeout".into()),
            context: None,
        })],
    });

    let mut decl = simple_agent(vec![], vec![start_handler, cancel_handler], None);
    decl.timers = vec![spanned(TimerField {
        name: spanned("timeout".into()),
        duration: spanned(Duration {
            value: 10,
            unit: DurationUnit::Minutes,
        }),
    })];

    let agent = AgentProcess::new(decl, None, mock_registry(), None, empty_program());
    agent.dispatch("begin", HashMap::new()).await.unwrap();
    agent.dispatch("stop", HashMap::new()).await.unwrap();

    let ctx = agent.context().lock().unwrap();
    assert_eq!(ctx.timer_manager.state("timeout"), Some(&TimerState::Idle));
}

// ── Event emit tests ─────────────────────────────────────────────────────────

#[tokio::test]
async fn emit_collected_in_event_sink() {
    let handler = spanned(OnHandler {
        event: spanned("resolve".into()),
        params: vec![],
        payload_type: None,
        requires: vec![],
        body: vec![spanned(Stmt::Emit(
            spanned("Resolved".into()),
            vec![spanned(CallArg {
                label: Some(spanned("summary".into())),
                value: spanned(Expr::Template(vec![spanned(TemplatePart::Text(
                    "done".into(),
                ))])),
            })],
        ))],
    });

    let decl = simple_agent(vec![], vec![handler], None);
    let agent = AgentProcess::new(decl, None, mock_registry(), None, empty_program());
    agent.dispatch("resolve", HashMap::new()).await.unwrap();

    let ctx = agent.context().lock().unwrap();
    assert_eq!(ctx.event_sink.emitted.len(), 1);
    assert_eq!(ctx.event_sink.emitted[0].name, "Resolved");
}

// ── Escalate tests ───────────────────────────────────────────────────────────

#[tokio::test]
async fn escalate_collected_in_event_sink() {
    let handler = spanned(OnHandler {
        event: spanned("help".into()),
        params: vec![],
        payload_type: None,
        requires: vec![],
        body: vec![spanned(Stmt::Escalate(spanned("human".into())))],
    });

    let decl = simple_agent(vec![], vec![handler], None);
    let agent = AgentProcess::new(decl, None, mock_registry(), None, empty_program());
    agent.dispatch("help", HashMap::new()).await.unwrap();

    let ctx = agent.context().lock().unwrap();
    assert_eq!(ctx.event_sink.escalations, vec!["human"]);
}

// ── Stuck detection integration ──────────────────────────────────────────────

#[tokio::test]
async fn stuck_detection_triggers_policy() {
    // Handler that gives a static response (will trigger stuck after 3 identical turns)
    let handler = spanned(OnHandler {
        event: spanned("message".into()),
        params: vec![],
        payload_type: None,
        requires: vec![],
        body: vec![spanned(Stmt::Give(
            spanned(Expr::Template(vec![spanned(TemplatePart::Text(
                "I cannot help with that".into(),
            ))])),
            vec![],
        ))],
    });

    // Stuck policy escalates
    let stuck_policy = spanned(StuckPolicy {
        turns: Some(3),
        body: vec![spanned(Stmt::Escalate(spanned("human".into())))],
    });

    let decl = simple_agent(vec![], vec![handler], Some(stuck_policy));
    let agent = AgentProcess::new(decl, None, mock_registry(), None, empty_program());

    // First two turns — not stuck yet
    agent.dispatch("message", HashMap::new()).await.unwrap();
    agent.dispatch("message", HashMap::new()).await.unwrap();
    {
        let ctx = agent.context().lock().unwrap();
        assert!(ctx.event_sink.escalations.is_empty());
    }

    // Third turn — stuck, policy triggers
    agent.dispatch("message", HashMap::new()).await.unwrap();
    {
        let ctx = agent.context().lock().unwrap();
        assert!(!ctx.event_sink.escalations.is_empty());
    }
}

// ── Full integration: parsed FORGE agent ─────────────────────────────────────

#[tokio::test]
async fn parsed_agent_memory_and_dispatch() {
    // This test parses actual FORGE syntax and runs the agent
    let source = r#"
agent test_bot
  memory
    count: Number

  on ping
    memory.count = memory.count + 1
    give memory.count
"#;
    let program = forge::parser::parse(source).expect("parse failed");

    // Extract the agent decl
    let agent_decl = program
        .items
        .iter()
        .find_map(|item| match &item.node {
            TopLevel::Agent(a) => Some(a.clone()),
            _ => None,
        })
        .expect("no agent in program");

    let agent = AgentProcess::new(agent_decl, None, mock_registry(), None, program);

    // Dispatch three pings — count should increment
    let r1 = agent.dispatch("ping", HashMap::new()).await.unwrap();
    assert!(matches!(r1, Some(ref v) if matches!(&v.value, Value::Number(n) if *n == 1.0)));

    let r2 = agent.dispatch("ping", HashMap::new()).await.unwrap();
    assert!(matches!(r2, Some(ref v) if matches!(&v.value, Value::Number(n) if *n == 2.0)));

    let r3 = agent.dispatch("ping", HashMap::new()).await.unwrap();
    assert!(matches!(r3, Some(ref v) if matches!(&v.value, Value::Number(n) if *n == 3.0)));
}
