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
        exportable: false,
        name: spanned("test_agent".into()),
        lifecycle: None,
        memory: memory_fields,
        memory_persistent: false,
        knowledge: None,
        timers: vec![],
        schedules: vec![],
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
    let agent = AgentProcess::new(
        decl,
        None,
        mock_registry(),
        None,
        empty_program(),
        None,
        None,
        None,
    );
    let result = agent.dispatch("greet", HashMap::new()).await.unwrap();
    assert!(matches!(result, Some(ref v) if matches!(&v.value, Value::Text(s) if s == "hello")));
}

#[tokio::test]
async fn dispatch_unknown_event_errors() {
    let decl = simple_agent(vec![], vec![], None);
    let agent = AgentProcess::new(
        decl,
        None,
        mock_registry(),
        None,
        empty_program(),
        None,
        None,
        None,
    );
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
    let agent = AgentProcess::new(
        decl,
        None,
        mock_registry(),
        None,
        empty_program(),
        None,
        None,
        None,
    );
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
    let agent = AgentProcess::new(
        decl,
        None,
        mock_registry(),
        None,
        empty_program(),
        None,
        None,
        None,
    );

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
    let agent = AgentProcess::new(
        decl,
        None,
        mock_registry(),
        None,
        empty_program(),
        None,
        None,
        None,
    );
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
    let agent = AgentProcess::new(
        decl,
        None,
        mock_registry(),
        None,
        empty_program(),
        None,
        None,
        None,
    );
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
    let agent = AgentProcess::new(
        decl,
        None,
        mock_registry(),
        None,
        empty_program(),
        None,
        None,
        None,
    );
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
    let agent = AgentProcess::new(
        decl,
        None,
        mock_registry(),
        None,
        empty_program(),
        None,
        None,
        None,
    );
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
    let agent = AgentProcess::new(
        decl,
        None,
        mock_registry(),
        None,
        empty_program(),
        None,
        None,
        None,
    );
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
    let agent = AgentProcess::new(
        decl,
        None,
        mock_registry(),
        None,
        empty_program(),
        None,
        None,
        None,
    );
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
    let agent = AgentProcess::new(
        decl,
        Some(&states),
        mock_registry(),
        None,
        empty_program(),
        None,
        None,
        None,
    );

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
    let agent = AgentProcess::new(
        decl,
        Some(&states),
        mock_registry(),
        None,
        empty_program(),
        None,
        None,
        None,
    );
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

    let agent = AgentProcess::new(
        decl,
        None,
        mock_registry(),
        None,
        empty_program(),
        None,
        None,
        None,
    );
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

    let agent = AgentProcess::new(
        decl,
        None,
        mock_registry(),
        None,
        empty_program(),
        None,
        None,
        None,
    );
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
    let agent = AgentProcess::new(
        decl,
        None,
        mock_registry(),
        None,
        empty_program(),
        None,
        None,
        None,
    );
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
    let agent = AgentProcess::new(
        decl,
        None,
        mock_registry(),
        None,
        empty_program(),
        None,
        None,
        None,
    );
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
    let agent = AgentProcess::new(
        decl,
        None,
        mock_registry(),
        None,
        empty_program(),
        None,
        None,
        None,
    );

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

#[tokio::test]
async fn no_give_handler_does_not_trip_stuck() {
    // Regression for #286: a handler that runs side effects but never
    // `give`s — typical of `on LearnedInsight` absorption handlers that
    // call `learn`, mutate memory, and `say` a receipt — must NOT feed the
    // stuck detector. Otherwise a handful of event-subscription dispatches
    // trivially hit Jaccard=1.0 on empty response_texts and the warden's
    // circuit breaker trips after bounded, healthy absorption work.
    let handler = spanned(OnHandler {
        event: spanned("message".into()),
        params: vec![],
        payload_type: None,
        requires: vec![],
        body: vec![spanned(Stmt::Say(spanned(Expr::Template(vec![spanned(
            TemplatePart::Text("absorbed".into()),
        )]))))],
        // no Give statement
    });

    let stuck_policy = spanned(StuckPolicy {
        turns: Some(3),
        body: vec![spanned(Stmt::Escalate(spanned("human".into())))],
    });

    let decl = simple_agent(vec![], vec![handler], Some(stuck_policy));
    let agent = AgentProcess::new(
        decl,
        None,
        mock_registry(),
        None,
        empty_program(),
        None,
        None,
        None,
    );

    // Five dispatches — well past the default stuck threshold of 3.
    for _ in 0..5 {
        agent.dispatch("message", HashMap::new()).await.unwrap();
    }

    let ctx = agent.context().lock().unwrap();
    assert!(
        ctx.event_sink.escalations.is_empty(),
        "no-give handler must not trigger stuck escalation"
    );
    assert!(
        !ctx.stuck_detector.is_stuck(),
        "no-give handler must not feed stuck detector at all"
    );
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
            TopLevel::Agent(a) => Some(a.as_ref().clone()),
            _ => None,
        })
        .expect("no agent in program");

    let agent = AgentProcess::new(
        agent_decl,
        None,
        mock_registry(),
        None,
        program,
        None,
        None,
        None,
    );

    // Dispatch three pings — count should increment
    let r1 = agent.dispatch("ping", HashMap::new()).await.unwrap();
    assert!(matches!(r1, Some(ref v) if matches!(&v.value, Value::Number(n) if *n == 1.0)));

    let r2 = agent.dispatch("ping", HashMap::new()).await.unwrap();
    assert!(matches!(r2, Some(ref v) if matches!(&v.value, Value::Number(n) if *n == 2.0)));

    let r3 = agent.dispatch("ping", HashMap::new()).await.unwrap();
    assert!(matches!(r3, Some(ref v) if matches!(&v.value, Value::Number(n) if *n == 3.0)));
}

// ── Issue #311: dev-cycle bug fixes ─────────────────────────────────────────

/// Bug 1: `escalate` does NOT exit a handler — only `give` does.
/// After `escalate to lead`, execution must NOT fall through to the
/// emit below the if block.  The fix adds `give "escalated"` right
/// after `escalate to lead`.
#[tokio::test]
async fn escalate_then_give_exits_handler_no_fallthrough() {
    let source = r#"
agent fixer
  memory
    iteration: Number

  on fail(failures: Text)
    memory.iteration = memory.iteration + 1
    if memory.iteration >= 3
      say "max iterations reached"
      escalate to lead
      give "escalated"
    say "fixing"
    emit FixReady(issue: "123")
"#;
    let program = forge::parser::parse(source).expect("parse failed");
    let agent_decl = program
        .items
        .iter()
        .find_map(|item| match &item.node {
            TopLevel::Agent(a) => Some(a.as_ref().clone()),
            _ => None,
        })
        .expect("no agent");

    let agent = AgentProcess::new(
        agent_decl,
        None,
        mock_registry(),
        None,
        program,
        None,
        None,
        None,
    );

    let params = || {
        let mut p = HashMap::new();
        p.insert(
            "failures".into(),
            ConfidentValue::deterministic(Value::Text("test error".into())),
        );
        p
    };

    // Iterations 1 and 2 — should emit FixReady
    agent.dispatch("fail", params()).await.unwrap();
    agent.dispatch("fail", params()).await.unwrap();
    {
        let ctx = agent.context().lock().unwrap();
        assert_eq!(
            ctx.event_sink.emitted.len(),
            2,
            "first 2 iterations should emit FixReady"
        );
        assert!(
            ctx.event_sink.escalations.is_empty(),
            "no escalation before iteration 3"
        );
    }

    // Iteration 3 — should escalate + give, NOT emit FixReady
    let result = agent.dispatch("fail", params()).await.unwrap();
    assert!(
        matches!(result, Some(ref v) if matches!(&v.value, Value::Text(s) if s == "escalated")),
        "handler must return 'escalated' from give"
    );
    {
        let ctx = agent.context().lock().unwrap();
        assert_eq!(
            ctx.event_sink.emitted.len(),
            2,
            "iteration 3 must NOT emit FixReady (give exits before emit)"
        );
        assert_eq!(
            ctx.event_sink.escalations,
            vec!["lead"],
            "escalation must be recorded"
        );
    }

    // Iteration 4 — also escalate, still no additional emit
    let result = agent.dispatch("fail", params()).await.unwrap();
    assert!(
        matches!(result, Some(ref v) if matches!(&v.value, Value::Text(s) if s == "escalated")),
    );
    {
        let ctx = agent.context().lock().unwrap();
        assert_eq!(
            ctx.event_sink.emitted.len(),
            2,
            "iteration 4 must still not emit (give exits)"
        );
    }
}

/// Bug 1 regression: WITHOUT the `give` after `escalate`, execution
/// falls through and emits an event on every iteration past the cap.
#[tokio::test]
async fn escalate_without_give_falls_through() {
    let source = r#"
agent fixer_broken
  memory
    iteration: Number

  on fail(failures: Text)
    memory.iteration = memory.iteration + 1
    if memory.iteration >= 3
      escalate to lead
    emit FixReady(issue: "123")
"#;
    let program = forge::parser::parse(source).expect("parse failed");
    let agent_decl = program
        .items
        .iter()
        .find_map(|item| match &item.node {
            TopLevel::Agent(a) => Some(a.as_ref().clone()),
            _ => None,
        })
        .expect("no agent");

    let agent = AgentProcess::new(
        agent_decl,
        None,
        mock_registry(),
        None,
        program,
        None,
        None,
        None,
    );

    let params = || {
        let mut p = HashMap::new();
        p.insert(
            "failures".into(),
            ConfidentValue::deterministic(Value::Text("err".into())),
        );
        p
    };

    // Three iterations — all three emit because escalate does NOT exit
    for _ in 0..3 {
        agent.dispatch("fail", params()).await.unwrap();
    }
    let ctx = agent.context().lock().unwrap();
    assert_eq!(
        ctx.event_sink.emitted.len(),
        3,
        "without give, escalate falls through and emits on ALL iterations"
    );
    assert_eq!(
        ctx.event_sink.escalations,
        vec!["lead"],
        "escalation recorded on iteration 3"
    );
}

/// Bug 2: The dev-cycle workflow file must parse with test_cmd threaded
/// through events, handlers, and the endpoint.
#[tokio::test]
async fn dev_cycle_workflow_parses_with_test_cmd() {
    let source = std::fs::read_to_string("workflows/dev-cycle/main.forge")
        .expect("could not read dev-cycle workflow");
    let program = forge::parser::parse(&source);
    assert!(
        program.is_ok(),
        "dev-cycle/main.forge must parse: {:?}",
        program.err()
    );
    let program = program.unwrap();

    // Verify test_cmd field exists in the IssueAssigned event
    let issue_assigned = program.items.iter().find_map(|item| match &item.node {
        TopLevel::Event(e) if e.name.node == "IssueAssigned" => Some(e.clone()),
        _ => None,
    });
    assert!(issue_assigned.is_some(), "IssueAssigned event must exist");
    let fields: Vec<&str> = issue_assigned
        .as_ref()
        .unwrap()
        .fields
        .iter()
        .map(|f| f.node.name.as_str())
        .collect();
    assert!(
        fields.contains(&"test_cmd"),
        "IssueAssigned must have test_cmd field, got: {:?}",
        fields
    );

    // Verify test_cmd in ImplementationReady, TestsFailed, AcceptanceMet
    for event_name in &["ImplementationReady", "TestsFailed", "AcceptanceMet"] {
        let evt = program.items.iter().find_map(|item| match &item.node {
            TopLevel::Event(e) if e.name.node == *event_name => Some(e.clone()),
            _ => None,
        });
        assert!(evt.is_some(), "{event_name} event must exist");
        let fields: Vec<&str> = evt
            .as_ref()
            .unwrap()
            .fields
            .iter()
            .map(|f| f.node.name.as_str())
            .collect();
        assert!(
            fields.contains(&"test_cmd"),
            "{event_name} must have test_cmd field, got: {:?}",
            fields
        );
    }

    // Verify endpoint dev_cycle has test_cmd param
    let endpoint = program.items.iter().find_map(|item| match &item.node {
        TopLevel::Endpoint(e) if e.name.node == "dev_cycle" => Some(e.clone()),
        _ => None,
    });
    assert!(endpoint.is_some(), "dev_cycle endpoint must exist");
    let params: Vec<&str> = endpoint
        .as_ref()
        .unwrap()
        .params
        .iter()
        .map(|p| p.node.name.as_str())
        .collect();
    assert!(
        params.contains(&"test_cmd"),
        "dev_cycle endpoint must have test_cmd param, got: {:?}",
        params
    );
}

/// Bug 3 / notification throttling: in the iteration path (TestsFailed
/// handler), the implementer should only `say` (log) the fix status, NOT
/// emit any Slack-bound events.  The only emitted event should be
/// ImplementationReady to re-trigger the tester.
#[tokio::test]
async fn iteration_path_emits_only_implementation_ready() {
    let source = r#"
agent impl_agent
  memory
    iteration: Number

  on TestsFailed(issue_id: Text, failures: Text)
    memory.iteration = memory.iteration + 1
    if memory.iteration >= 3
      say "escalating"
      escalate to lead
      give "escalated"
    say "fixing iteration"
    say "fix pushed — re-running tests"
    emit ImplementationReady(issue_id: issue_id)
"#;
    let program = forge::parser::parse(source).expect("parse failed");
    let agent_decl = program
        .items
        .iter()
        .find_map(|item| match &item.node {
            TopLevel::Agent(a) => Some(a.as_ref().clone()),
            _ => None,
        })
        .expect("no agent");

    let agent = AgentProcess::new(
        agent_decl,
        None,
        mock_registry(),
        None,
        program,
        None,
        None,
        None,
    );

    let params = || {
        let mut p = HashMap::new();
        p.insert(
            "issue_id".into(),
            ConfidentValue::deterministic(Value::Text("42".into())),
        );
        p.insert(
            "failures".into(),
            ConfidentValue::deterministic(Value::Text("error".into())),
        );
        p
    };

    // Two iterations — each emits exactly one ImplementationReady, nothing else
    agent.dispatch("TestsFailed", params()).await.unwrap();
    agent.dispatch("TestsFailed", params()).await.unwrap();
    {
        let ctx = agent.context().lock().unwrap();
        assert_eq!(ctx.event_sink.emitted.len(), 2);
        for evt in &ctx.event_sink.emitted {
            assert_eq!(
                evt.name, "ImplementationReady",
                "only ImplementationReady should be emitted, not Slack notifications"
            );
        }
        assert!(ctx.event_sink.escalations.is_empty());
    }

    // Third iteration — escalates, no ImplementationReady emitted
    let result = agent.dispatch("TestsFailed", params()).await.unwrap();
    assert!(
        matches!(result, Some(ref v) if matches!(&v.value, Value::Text(s) if s == "escalated"))
    );
    {
        let ctx = agent.context().lock().unwrap();
        assert_eq!(
            ctx.event_sink.emitted.len(),
            2,
            "iteration 3 must not emit (give exits before emit)"
        );
        assert_eq!(ctx.event_sink.escalations, vec!["lead"]);
    }
}

// ── T2.1 (#296): Implementer iteration loop tests ──────────────────────────

/// T2.1: iteration_log accumulates a diagnosis entry per TestsFailed dispatch.
#[tokio::test]
async fn iteration_loop_tracks_diagnosis_log() {
    let source = r#"
agent impl_agent
  memory
    iteration: Number
    iteration_log: Text
    last_diagnosis: Text
    plan: Text

  on TestsFailed(issue_id: Text, failures: Text)
    memory.iteration = memory.iteration + 1
    memory.last_diagnosis = "diag for iteration"
    memory.iteration_log = memory.iteration_log + "\n--- Iteration {memory.iteration} ---\ndiag for iteration"
    emit ImplementationReady(issue_id: issue_id)
"#;
    let program = forge::parser::parse(source).expect("parse failed");
    let agent_decl = program
        .items
        .iter()
        .find_map(|item| match &item.node {
            TopLevel::Agent(a) => Some(a.as_ref().clone()),
            _ => None,
        })
        .expect("no agent");

    let agent = AgentProcess::new(
        agent_decl,
        None,
        mock_registry(),
        None,
        program,
        None,
        None,
        None,
    );

    let params = || {
        let mut p = HashMap::new();
        p.insert(
            "issue_id".into(),
            ConfidentValue::deterministic(Value::Text("42".into())),
        );
        p.insert(
            "failures".into(),
            ConfidentValue::deterministic(Value::Text("test error".into())),
        );
        p
    };

    // Two iterations
    agent.dispatch("TestsFailed", params()).await.unwrap();
    agent.dispatch("TestsFailed", params()).await.unwrap();
    {
        let ctx = agent.context().lock().unwrap();
        let log = ctx.memory.get("iteration_log").unwrap();
        let log_text = match &log.value {
            Value::Text(s) => s.clone(),
            _ => panic!("iteration_log should be Text"),
        };
        assert!(
            log_text.contains("--- Iteration 1 ---"),
            "log must contain iteration 1 marker, got: {log_text}"
        );
        assert!(
            log_text.contains("--- Iteration 2 ---"),
            "log must contain iteration 2 marker, got: {log_text}"
        );
        let diagnosis = ctx.memory.get("last_diagnosis").unwrap();
        assert!(
            matches!(&diagnosis.value, Value::Text(s) if !s.is_empty()),
            "last_diagnosis must be set"
        );
    }
}

/// T2.1: configurable max_iterations — cap at 2 instead of default 3.
#[tokio::test]
async fn configurable_max_iterations_cap() {
    let source = r#"
agent impl_agent
  memory
    iteration: Number
    max_iterations: Number

  on start
    memory.max_iterations = 2

  on TestsFailed(issue_id: Text, failures: Text)
    memory.iteration = memory.iteration + 1
    if memory.iteration >= memory.max_iterations
      escalate to lead
      give "escalated"
    emit ImplementationReady(issue_id: issue_id)
"#;
    let program = forge::parser::parse(source).expect("parse failed");
    let agent_decl = program
        .items
        .iter()
        .find_map(|item| match &item.node {
            TopLevel::Agent(a) => Some(a.as_ref().clone()),
            _ => None,
        })
        .expect("no agent");

    let agent = AgentProcess::new(
        agent_decl,
        None,
        mock_registry(),
        None,
        program,
        None,
        None,
        None,
    );

    let params = || {
        let mut p = HashMap::new();
        p.insert(
            "issue_id".into(),
            ConfidentValue::deterministic(Value::Text("42".into())),
        );
        p.insert(
            "failures".into(),
            ConfidentValue::deterministic(Value::Text("error".into())),
        );
        p
    };

    // Fire on start to initialize max_iterations = 2
    agent.dispatch("start", HashMap::new()).await.unwrap();

    // Iteration 1 — below cap, should emit
    agent.dispatch("TestsFailed", params()).await.unwrap();
    {
        let ctx = agent.context().lock().unwrap();
        assert_eq!(ctx.event_sink.emitted.len(), 1, "iteration 1 should emit");
        assert!(ctx.event_sink.escalations.is_empty());
    }

    // Iteration 2 — hits cap (>= 2), should escalate
    let result = agent.dispatch("TestsFailed", params()).await.unwrap();
    assert!(
        matches!(result, Some(ref v) if matches!(&v.value, Value::Text(s) if s == "escalated")),
        "iteration 2 must escalate at cap of 2"
    );
    {
        let ctx = agent.context().lock().unwrap();
        assert_eq!(
            ctx.event_sink.emitted.len(),
            1,
            "iteration 2 must not emit (give exits before emit)"
        );
        assert_eq!(ctx.event_sink.escalations, vec!["lead"]);
    }
}

/// T2.1: on escalation, memory preserves iteration count and accumulated log
/// for structured escalation context.
#[tokio::test]
async fn escalation_preserves_iteration_state() {
    let source = r#"
agent impl_agent
  memory
    iteration: Number
    max_iterations: Number
    iteration_log: Text
    last_diagnosis: Text

  on start
    memory.max_iterations = 1

  on TestsFailed(issue_id: Text, failures: Text)
    memory.iteration = memory.iteration + 1
    memory.last_diagnosis = "root cause: {failures}"
    memory.iteration_log = memory.iteration_log + "\n--- Iteration {memory.iteration} ---\nroot cause: {failures}"
    if memory.iteration >= memory.max_iterations
      escalate to lead
      give "escalated"
    emit ImplementationReady(issue_id: issue_id)
"#;
    let program = forge::parser::parse(source).expect("parse failed");
    let agent_decl = program
        .items
        .iter()
        .find_map(|item| match &item.node {
            TopLevel::Agent(a) => Some(a.as_ref().clone()),
            _ => None,
        })
        .expect("no agent");

    let agent = AgentProcess::new(
        agent_decl,
        None,
        mock_registry(),
        None,
        program,
        None,
        None,
        None,
    );

    let params = || {
        let mut p = HashMap::new();
        p.insert(
            "issue_id".into(),
            ConfidentValue::deterministic(Value::Text("99".into())),
        );
        p.insert(
            "failures".into(),
            ConfidentValue::deterministic(Value::Text("assertion failed".into())),
        );
        p
    };

    // Fire on start to initialize max_iterations = 1
    agent.dispatch("start", HashMap::new()).await.unwrap();

    // Cap at 1 — first dispatch escalates
    let result = agent.dispatch("TestsFailed", params()).await.unwrap();
    assert!(
        matches!(result, Some(ref v) if matches!(&v.value, Value::Text(s) if s == "escalated"))
    );
    {
        let ctx = agent.context().lock().unwrap();
        // Verify memory has the full iteration state for escalation message
        let iteration = ctx.memory.get("iteration").unwrap();
        assert!(
            matches!(&iteration.value, Value::Number(n) if *n == 1.0),
            "iteration count must be 1"
        );
        let log = ctx.memory.get("iteration_log").unwrap();
        assert!(
            matches!(&log.value, Value::Text(s) if s.contains("--- Iteration 1 ---")),
            "iteration_log must contain the iteration marker"
        );
        let diag = ctx.memory.get("last_diagnosis").unwrap();
        assert!(
            matches!(&diag.value, Value::Text(s) if !s.is_empty()),
            "last_diagnosis must be populated"
        );
        assert_eq!(ctx.event_sink.escalations, vec!["lead"]);
    }
}

/// T2.1: enhanced main.forge parses and contains the new task + memory fields.
#[tokio::test]
async fn dev_cycle_main_forge_has_iteration_loop_enhancements() {
    let source = std::fs::read_to_string("workflows/dev-cycle/main.forge")
        .expect("could not read dev-cycle workflow");
    let program = forge::parser::parse(&source);
    assert!(
        program.is_ok(),
        "dev-cycle/main.forge must parse: {:?}",
        program.err()
    );
    let program = program.unwrap();

    // Verify diagnose_failures task exists
    let diag_task = program.items.iter().find_map(|item| match &item.node {
        TopLevel::Task(t) if t.name.node == "diagnose_failures" => Some(t.clone()),
        _ => None,
    });
    assert!(
        diag_task.is_some(),
        "diagnose_failures task must exist in main.forge"
    );
    let diag_task = diag_task.unwrap();
    let param_names: Vec<&str> = diag_task
        .needs
        .iter()
        .map(|p| p.node.name.as_str())
        .collect();
    assert!(
        param_names.contains(&"failures")
            && param_names.contains(&"plan")
            && param_names.contains(&"iteration"),
        "diagnose_failures must take failures, plan, iteration params, got: {:?}",
        param_names
    );

    // Verify implementer agent has new memory fields
    let implementer = program.items.iter().find_map(|item| match &item.node {
        TopLevel::Agent(a) if a.name.node == "implementer" => Some(a.clone()),
        _ => None,
    });
    assert!(implementer.is_some(), "implementer agent must exist");
    let impl_memory: Vec<&str> = implementer
        .as_ref()
        .unwrap()
        .memory
        .iter()
        .map(|f| f.node.name.as_str())
        .collect();
    for field in &["max_iterations", "iteration_log", "last_diagnosis"] {
        assert!(
            impl_memory.contains(field),
            "implementer must have {field} memory field, got: {:?}",
            impl_memory
        );
    }
}

/// T5.1 (#302): dev-cycle declares TaskCompleted and LessonExtracted events
/// with the signal schema from the issue's definition of done.
#[tokio::test]
async fn dev_cycle_has_task_completed_and_lesson_extracted_events() {
    let source = std::fs::read_to_string("workflows/dev-cycle/main.forge")
        .expect("could not read dev-cycle workflow");
    let program = forge::parser::parse(&source).expect("dev-cycle must parse");

    let task_completed = program.items.iter().find_map(|item| match &item.node {
        TopLevel::Event(e) if e.name.node == "TaskCompleted" => Some(e.clone()),
        _ => None,
    });
    assert!(
        task_completed.is_some(),
        "TaskCompleted event must exist (T5.1)"
    );
    let tc_fields: Vec<&str> = task_completed
        .as_ref()
        .unwrap()
        .fields
        .iter()
        .map(|f| f.node.name.as_str())
        .collect();
    for field in &[
        "task_id",
        "outcome",
        "ci_passed_first_try",
        "review_rounds",
        "time_to_merge",
        "reverted_within_7d",
    ] {
        assert!(
            tc_fields.contains(field),
            "TaskCompleted must have {field} field, got: {:?}",
            tc_fields
        );
    }

    let lesson = program.items.iter().find_map(|item| match &item.node {
        TopLevel::Event(e) if e.name.node == "LessonExtracted" => Some(e.clone()),
        _ => None,
    });
    assert!(lesson.is_some(), "LessonExtracted event must exist (T5.1)");
    let le_fields: Vec<&str> = lesson
        .as_ref()
        .unwrap()
        .fields
        .iter()
        .map(|f| f.node.name.as_str())
        .collect();
    for field in &["task_id", "agent_id", "category", "content", "confidence"] {
        assert!(
            le_fields.contains(field),
            "LessonExtracted must have {field} field, got: {:?}",
            le_fields
        );
    }
}

/// T5.1 (#302): each of the five specialists has an `on TaskCompleted` handler
/// — the subscription point where LessonExtracted is emitted and lessons are
/// written to the knowledge store.
#[tokio::test]
async fn dev_cycle_five_specialists_handle_task_completed() {
    let source = std::fs::read_to_string("workflows/dev-cycle/main.forge")
        .expect("could not read dev-cycle workflow");
    let program = forge::parser::parse(&source).expect("dev-cycle must parse");

    for agent_name in &[
        "planner",
        "implementer",
        "tester",
        "reviewer",
        "release_manager",
    ] {
        let agent = program
            .items
            .iter()
            .find_map(|item| match &item.node {
                TopLevel::Agent(a) if a.name.node == *agent_name => Some(a.clone()),
                _ => None,
            })
            .unwrap_or_else(|| panic!("{agent_name} agent must exist"));

        let has_handler = agent
            .handlers
            .iter()
            .any(|h| h.node.event.node == "TaskCompleted");
        assert!(
            has_handler,
            "{agent_name} must have `on TaskCompleted` handler (T5.1)"
        );

        // Each specialist must declare a knowledge store so the runtime
        // wires the shared instance and `learn` is usable in the handler.
        assert!(
            agent.knowledge.is_some(),
            "{agent_name} must declare a knowledge store for lesson persistence"
        );
    }
}

/// T5.1 (#302): signals flow through events so release_manager's TaskCompleted
/// payload is populated from real pipeline state, not hardcoded values.
#[tokio::test]
async fn dev_cycle_threads_signals_through_acceptance_and_prmerged() {
    let source = std::fs::read_to_string("workflows/dev-cycle/main.forge")
        .expect("could not read dev-cycle workflow");
    let program = forge::parser::parse(&source).expect("dev-cycle must parse");

    let acceptance_met = program
        .items
        .iter()
        .find_map(|item| match &item.node {
            TopLevel::Event(e) if e.name.node == "AcceptanceMet" => Some(e.clone()),
            _ => None,
        })
        .expect("AcceptanceMet event must exist");
    let am_fields: Vec<&str> = acceptance_met
        .fields
        .iter()
        .map(|f| f.node.name.as_str())
        .collect();
    assert!(
        am_fields.contains(&"ci_passed_first_try"),
        "AcceptanceMet must carry ci_passed_first_try for T5.1, got: {:?}",
        am_fields
    );

    let pr_merged = program
        .items
        .iter()
        .find_map(|item| match &item.node {
            TopLevel::Event(e) if e.name.node == "PRMerged" => Some(e.clone()),
            _ => None,
        })
        .expect("PRMerged event must exist");
    let pm_fields: Vec<&str> = pr_merged
        .fields
        .iter()
        .map(|f| f.node.name.as_str())
        .collect();
    for field in &["ci_passed_first_try", "review_rounds"] {
        assert!(
            pm_fields.contains(field),
            "PRMerged must carry {field} for T5.1, got: {:?}",
            pm_fields
        );
    }
}

/// T5.1 (#302): tester tracks `test_runs` and reviewer tracks `review_rounds`
/// in memory so `ci_passed_first_try` and review_rounds signals are real.
#[tokio::test]
async fn dev_cycle_tester_and_reviewer_track_signal_counters() {
    let source = std::fs::read_to_string("workflows/dev-cycle/main.forge")
        .expect("could not read dev-cycle workflow");
    let program = forge::parser::parse(&source).expect("dev-cycle must parse");

    let tester = program
        .items
        .iter()
        .find_map(|item| match &item.node {
            TopLevel::Agent(a) if a.name.node == "tester" => Some(a.clone()),
            _ => None,
        })
        .expect("tester agent must exist");
    let tester_memory: Vec<&str> = tester.memory.iter().map(|f| f.node.name.as_str()).collect();
    assert!(
        tester_memory.contains(&"test_runs"),
        "tester must track test_runs for ci_passed_first_try signal, got: {:?}",
        tester_memory
    );

    let reviewer = program
        .items
        .iter()
        .find_map(|item| match &item.node {
            TopLevel::Agent(a) if a.name.node == "reviewer" => Some(a.clone()),
            _ => None,
        })
        .expect("reviewer agent must exist");
    let reviewer_memory: Vec<&str> = reviewer
        .memory
        .iter()
        .map(|f| f.node.name.as_str())
        .collect();
    assert!(
        reviewer_memory.contains(&"review_rounds"),
        "reviewer must track review_rounds for review_rounds signal, got: {:?}",
        reviewer_memory
    );
}

/// T5.2 (#303): SwarmMastery states, MasterySignal + MasteryUpdated events,
/// and scoring helpers exist in the dev-cycle program.
#[tokio::test]
async fn dev_cycle_has_swarm_mastery_schema() {
    let source = std::fs::read_to_string("workflows/dev-cycle/main.forge")
        .expect("could not read dev-cycle workflow");
    let program = forge::parser::parse(&source).expect("dev-cycle must parse");

    let swarm_states = program.items.iter().find_map(|item| match &item.node {
        TopLevel::States(s) if s.name.node == "SwarmMastery" => Some(s.clone()),
        _ => None,
    });
    assert!(
        swarm_states.is_some(),
        "SwarmMastery states block must exist (T5.2)"
    );
    let state_names: Vec<&str> = swarm_states
        .as_ref()
        .unwrap()
        .transitions
        .iter()
        .flat_map(|t| vec![t.node.from.node.as_str(), t.node.to.node.as_str()])
        .collect();
    for state in &["novice", "apprentice", "journeyman", "expert"] {
        assert!(
            state_names.contains(state),
            "SwarmMastery must reach state `{state}`, got transitions involving: {:?}",
            state_names
        );
    }

    for event_name in &["MasterySignal", "MasteryUpdated"] {
        let found = program.items.iter().any(|item| match &item.node {
            TopLevel::Event(e) => e.name.node == *event_name,
            _ => false,
        });
        assert!(found, "event `{event_name}` must exist (T5.2)");
    }

    for fn_name in &[
        "compute_swarm_score",
        "determine_swarm_level",
        "reviewer_clean_signal",
        "reviewer_regress_signal",
    ] {
        let found = program.items.iter().any(|item| match &item.node {
            TopLevel::Pure(p) => p.name.node == *fn_name,
            _ => false,
        });
        assert!(found, "pure function `{fn_name}` must exist (T5.2)");
    }
}

/// T5.2 (#303): swarm_mastery_coordinator subscribes to TaskCompleted, and
/// swarm_mastery_tuple subscribes to MasterySignal with a per-tuple filter.
#[tokio::test]
async fn dev_cycle_swarm_mastery_agents_subscribe_correctly() {
    let source = std::fs::read_to_string("workflows/dev-cycle/main.forge")
        .expect("could not read dev-cycle workflow");
    let program = forge::parser::parse(&source).expect("dev-cycle must parse");

    let coordinator = program
        .items
        .iter()
        .find_map(|item| match &item.node {
            TopLevel::Agent(a) if a.name.node == "swarm_mastery_coordinator" => Some(a.clone()),
            _ => None,
        })
        .expect("swarm_mastery_coordinator agent must exist");
    let coord_events: Vec<&str> = coordinator
        .subscriptions
        .iter()
        .map(|s| s.node.event_name.node.as_str())
        .collect();
    assert!(
        coord_events.contains(&"TaskCompleted"),
        "swarm_mastery_coordinator must subscribe to TaskCompleted, got: {:?}",
        coord_events
    );

    let tuple = program
        .items
        .iter()
        .find_map(|item| match &item.node {
            TopLevel::Agent(a) if a.name.node == "swarm_mastery_tuple" => Some(a.clone()),
            _ => None,
        })
        .expect("swarm_mastery_tuple agent must exist");
    let tuple_events: Vec<&str> = tuple
        .subscriptions
        .iter()
        .map(|s| s.node.event_name.node.as_str())
        .collect();
    assert!(
        tuple_events.contains(&"MasterySignal"),
        "swarm_mastery_tuple must subscribe to MasterySignal, got: {:?}",
        tuple_events
    );
    // Tuple must have a filter — it's per-(specialist, project), not global.
    let has_filter = tuple
        .subscriptions
        .iter()
        .any(|s| s.node.event_name.node == "MasterySignal" && s.node.filter.is_some());
    assert!(
        has_filter,
        "swarm_mastery_tuple's MasterySignal subscription must have a where-filter"
    );

    // Tuple must declare the SwarmMastery lifecycle.
    let lifecycle = tuple
        .lifecycle
        .as_ref()
        .map(|l| l.node.as_str())
        .unwrap_or("");
    assert_eq!(
        lifecycle, "SwarmMastery",
        "swarm_mastery_tuple must use SwarmMastery lifecycle"
    );
}

// ── Slack adapter (#298 T3.1) ────────────────────────────────────────────────

/// The slack-adapter agent declares the seven typed outbound events, the five
/// template tasks, the `slack_adapter` agent with matching handlers, a pool of
/// 1 worker, and its warden. Parses from source.
#[tokio::test]
async fn slack_adapter_main_forge_has_seven_events_and_handlers() {
    let source = std::fs::read_to_string("examples/agents/slack-adapter/main.forge")
        .expect("could not read slack-adapter main.forge");
    let program =
        forge::parser::parse(&source).expect("slack-adapter main.forge must parse cleanly");

    let event_names: Vec<&str> = program
        .items
        .iter()
        .filter_map(|item| match &item.node {
            TopLevel::Event(e) => Some(e.name.node.as_str()),
            _ => None,
        })
        .collect();
    for event in &[
        "PostApproval",
        "PostApprovalResult",
        "PostMessage",
        "PostThreadReply",
        "AddReaction",
        "RequestHuman",
        "WardenEscalation",
    ] {
        assert!(
            event_names.contains(event),
            "slack-adapter must declare {event} event, got: {:?}",
            event_names
        );
    }

    let adapter = program
        .items
        .iter()
        .find_map(|item| match &item.node {
            TopLevel::Agent(a) if a.name.node == "slack_adapter" => Some(a.clone()),
            _ => None,
        })
        .expect("slack_adapter agent must exist");

    let handler_events: Vec<&str> = adapter
        .handlers
        .iter()
        .map(|h| h.node.event.node.as_str())
        .collect();
    for event in &[
        "start",
        "PostApproval",
        "PostApprovalResult",
        "PostMessage",
        "PostThreadReply",
        "AddReaction",
        "RequestHuman",
        "WardenEscalation",
        "status",
    ] {
        assert!(
            handler_events.contains(event),
            "slack_adapter must have `on {event}` handler, got: {:?}",
            handler_events
        );
    }

    let memory: Vec<&str> = adapter
        .memory
        .iter()
        .map(|f| f.node.name.as_str())
        .collect();
    for field in &[
        "sent_count",
        "failed_count",
        "default_escalation_channel",
        "last_error",
    ] {
        assert!(
            memory.contains(field),
            "slack_adapter must have {field} memory field, got: {:?}",
            memory
        );
    }
}

/// The slack-adapter declares a pool of 1 worker with `strategy: fastest`
/// — the rate-limit-backpressure shape called for in #298.
#[tokio::test]
async fn slack_adapter_declares_single_worker_pool() {
    let source = std::fs::read_to_string("examples/agents/slack-adapter/main.forge")
        .expect("could not read slack-adapter main.forge");
    let program = forge::parser::parse(&source).expect("slack-adapter must parse");

    let pool = program
        .items
        .iter()
        .find_map(|item| match &item.node {
            TopLevel::Pool(p) if p.name.node == "slack_adapter_pool" => Some(p.clone()),
            _ => None,
        })
        .expect("slack_adapter_pool must exist");
    assert_eq!(
        pool.worker_count.node, 1.0,
        "slack_adapter_pool must declare exactly 1 worker for serialized Slack posting"
    );
}

/// The template tasks exist and render Text; they are the shared Block Kit
/// template library the issue calls for.
#[tokio::test]
async fn slack_adapter_has_five_template_tasks() {
    let source = std::fs::read_to_string("examples/agents/slack-adapter/main.forge")
        .expect("could not read slack-adapter main.forge");
    let program = forge::parser::parse(&source).expect("slack-adapter must parse");

    let task_names: Vec<&str> = program
        .items
        .iter()
        .filter_map(|item| match &item.node {
            TopLevel::Task(t) => Some(t.name.node.as_str()),
            _ => None,
        })
        .collect();
    for task in &[
        "build_approval_block",
        "build_progress_block",
        "build_error_block",
        "build_merge_confirmation_block",
        "build_rejection_block",
        "build_escalation_block",
    ] {
        assert!(
            task_names.contains(task),
            "slack-adapter must declare `{task}` template task, got: {:?}",
            task_names
        );
    }
}

/// After migration, pr-review-bot no longer calls `skill.slack.*`; it emits
/// `PostApproval` and `PostMessage` events for the adapter to handle.
#[tokio::test]
async fn pr_review_bot_emits_events_instead_of_calling_slack_skill() {
    let source = std::fs::read_to_string("examples/agents/pr-review-bot/server.forge")
        .expect("could not read pr-review-bot server.forge");
    assert!(
        !source.contains("skill.slack"),
        "pr-review-bot must not reference skill.slack after migration"
    );

    let program = forge::parser::parse(&source).expect("pr-review-bot must parse");
    let event_names: Vec<&str> = program
        .items
        .iter()
        .filter_map(|item| match &item.node {
            TopLevel::Event(e) => Some(e.name.node.as_str()),
            _ => None,
        })
        .collect();
    for event in &["PostApproval", "PostMessage"] {
        assert!(
            event_names.contains(event),
            "pr-review-bot must declare {event} locally to emit it, got: {:?}",
            event_names
        );
    }
}

/// After migration, approval-gate no longer calls `skill.slack.*`; it emits
/// `PostApproval`, `PostMessage`, and `WardenEscalation`.
#[tokio::test]
async fn approval_gate_emits_events_instead_of_calling_slack_skill() {
    let source = std::fs::read_to_string("examples/agents/approval-gate/main.forge")
        .expect("could not read approval-gate main.forge");
    assert!(
        !source.contains("skill.slack"),
        "approval-gate must not reference skill.slack after migration"
    );

    let program = forge::parser::parse(&source).expect("approval-gate must parse");
    let event_names: Vec<&str> = program
        .items
        .iter()
        .filter_map(|item| match &item.node {
            TopLevel::Event(e) => Some(e.name.node.as_str()),
            _ => None,
        })
        .collect();
    for event in &["PostApproval", "PostMessage", "WardenEscalation"] {
        assert!(
            event_names.contains(event),
            "approval-gate must declare {event} locally to emit it, got: {:?}",
            event_names
        );
    }
}

// ── Clone-dev task graph (#299 T4.1) ────────────────────────────────────────

/// T4.1 (#299): the clone-dev skeleton declares `type TaskNode` with the
/// five fields the DoD specifies — the mastermind's `task_graph` DAG node.
#[tokio::test]
async fn clone_dev_skeleton_declares_task_node_type() {
    let source = std::fs::read_to_string("examples/agents/clone-dev-skeleton/main.forge")
        .expect("could not read clone-dev-skeleton main.forge");
    let program = forge::parser::parse(&source).expect("skeleton must parse (T4.1)");

    let task_node = program
        .items
        .iter()
        .find_map(|item| match &item.node {
            TopLevel::TypeDef(t) if t.name.node == "TaskNode" => Some(t.clone()),
            _ => None,
        })
        .expect("type TaskNode must exist (T4.1)");

    let field_names: Vec<&str> = task_node
        .fields
        .iter()
        .map(|f| f.node.name.as_str())
        .collect();
    for field in &["task_id", "status", "blocked_on", "specialist", "project"] {
        assert!(
            field_names.contains(field),
            "TaskNode must declare {field} field, got: {:?}",
            field_names
        );
    }

    let blocked_on_ty = &task_node
        .fields
        .iter()
        .find(|f| f.node.name == "blocked_on")
        .expect("blocked_on field")
        .node
        .type_name
        .node;
    assert!(
        matches!(blocked_on_ty, TypeName::Array(inner, None) if matches!(**inner, TypeName::Text)),
        "TaskNode.blocked_on must be Text[], got {:?}",
        blocked_on_ty
    );
}

/// T4.1 (#299): the skeleton declares the four graph events the mastermind
/// consumes and emits.
#[tokio::test]
async fn clone_dev_skeleton_declares_task_graph_events() {
    let source = std::fs::read_to_string("examples/agents/clone-dev-skeleton/main.forge")
        .expect("could not read clone-dev-skeleton main.forge");
    let program = forge::parser::parse(&source).expect("skeleton must parse (T4.1)");

    let events: HashMap<String, EventDecl> = program
        .items
        .iter()
        .filter_map(|item| match &item.node {
            TopLevel::Event(e) => Some((e.name.node.clone(), e.clone())),
            _ => None,
        })
        .collect();

    for event in &[
        "TaskBlocked",
        "TaskCompleted",
        "UnblockTask",
        "CycleDetected",
    ] {
        assert!(
            events.contains_key(*event),
            "event {event} must exist (T4.1)"
        );
    }

    let blocked_fields: Vec<&str> = events["TaskBlocked"]
        .fields
        .iter()
        .map(|f| f.node.name.as_str())
        .collect();
    assert!(blocked_fields.contains(&"task_id"));
    assert!(blocked_fields.contains(&"blocked_on"));

    let unblock_fields: Vec<&str> = events["UnblockTask"]
        .fields
        .iter()
        .map(|f| f.node.name.as_str())
        .collect();
    assert!(unblock_fields.contains(&"task_id"));
    assert!(unblock_fields.contains(&"unblocked_by"));

    let cycle_fields: Vec<&str> = events["CycleDetected"]
        .fields
        .iter()
        .map(|f| f.node.name.as_str())
        .collect();
    assert!(cycle_fields.contains(&"task_id"));
    assert!(cycle_fields.contains(&"blocker_id"));
}

/// T4.1 (#299): the graph-manipulation pure functions exist. They're
/// fragmented (one per op) to stay within FORGE's i3→i4 nesting limit —
/// the test enforces the full set so future refactors don't silently
/// drop one.
#[tokio::test]
async fn clone_dev_skeleton_declares_task_graph_pure_functions() {
    let source = std::fs::read_to_string("examples/agents/clone-dev-skeleton/main.forge")
        .expect("could not read clone-dev-skeleton main.forge");
    let program = forge::parser::parse(&source).expect("skeleton must parse (T4.1)");

    let pure_names: Vec<&str> = program
        .items
        .iter()
        .filter_map(|item| match &item.node {
            TopLevel::Pure(p) => Some(p.name.node.as_str()),
            _ => None,
        })
        .collect();

    for fn_name in &[
        "text_in_list",
        "get_task_blockers",
        "would_create_cycle",
        "first_cycling_blocker",
        "find_newly_unblocked",
        "update_task_node",
        "apply_completion",
    ] {
        assert!(
            pure_names.contains(fn_name),
            "pure function `{fn_name}` must exist (T4.1), got: {:?}",
            pure_names
        );
    }
}

/// T4.1 (#299): mastermind's `task_graph` is typed as `TaskNode[]` (not
/// the pre-T4.1 pipe-delimited `Text[]`), and the agent subscribes to
/// both graph-affecting events.
#[tokio::test]
async fn clone_dev_mastermind_has_typed_graph_and_handlers() {
    let source = std::fs::read_to_string("examples/agents/clone-dev-skeleton/main.forge")
        .expect("could not read clone-dev-skeleton main.forge");
    let program = forge::parser::parse(&source).expect("skeleton must parse (T4.1)");

    let mastermind = program
        .items
        .iter()
        .find_map(|item| match &item.node {
            TopLevel::Agent(a) if a.name.node == "mastermind" => Some(a.clone()),
            _ => None,
        })
        .expect("mastermind agent must exist");

    assert!(
        mastermind.memory_persistent,
        "mastermind.memory must be persistent (leverages #57)"
    );

    let task_graph = mastermind
        .memory
        .iter()
        .find(|f| f.node.name == "task_graph")
        .expect("mastermind.memory.task_graph must exist");
    assert!(
        matches!(&task_graph.node.type_name.node,
            TypeName::Array(inner, None) if matches!(&**inner, TypeName::Custom(name) if name == "TaskNode")
        ),
        "mastermind.memory.task_graph must be TaskNode[] (T4.1), got {:?}",
        task_graph.node.type_name.node
    );

    let sub_events: Vec<&str> = mastermind
        .subscriptions
        .iter()
        .map(|s| s.node.event_name.node.as_str())
        .collect();
    for evt in &["ClonedevTaskInbound", "TaskBlocked", "TaskCompleted"] {
        assert!(
            sub_events.contains(evt),
            "mastermind must subscribe to {evt}, got: {:?}",
            sub_events
        );
    }

    let handler_events: Vec<&str> = mastermind
        .handlers
        .iter()
        .map(|h| h.node.event.node.as_str())
        .collect();
    for evt in &[
        "start",
        "ClonedevTaskInbound",
        "TaskBlocked",
        "TaskCompleted",
    ] {
        assert!(
            handler_events.contains(evt),
            "mastermind must have `on {evt}` handler, got: {:?}",
            handler_events
        );
    }
}

/// T4.1 (#299): smoke endpoints exist so a live server can drive the
/// task-graph flows end-to-end (see skeleton README /task_blocked and
/// /task_completed). These also give integration harnesses a stable
/// contract to hit without plumbing real producers.
#[tokio::test]
async fn clone_dev_skeleton_exposes_graph_smoke_endpoints() {
    let source = std::fs::read_to_string("examples/agents/clone-dev-skeleton/main.forge")
        .expect("could not read clone-dev-skeleton main.forge");
    let program = forge::parser::parse(&source).expect("skeleton must parse (T4.1)");

    let endpoints: Vec<&str> = program
        .items
        .iter()
        .filter_map(|item| match &item.node {
            TopLevel::Endpoint(e) => Some(e.name.node.as_str()),
            _ => None,
        })
        .collect();
    for ep in &["task_blocked", "task_completed"] {
        assert!(
            endpoints.contains(ep),
            "endpoint /{ep} must exist (T4.1), got: {:?}",
            endpoints
        );
    }
}
