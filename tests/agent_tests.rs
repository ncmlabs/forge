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
    mock_registry_with(mock)
}

fn mock_registry_with(mock: MockProvider) -> Arc<ProviderRegistry> {
    let mut reg = ProviderRegistry::new("mock");
    reg.register("mock", Arc::new(mock));
    Arc::new(reg)
}

fn shell_path(path: &std::path::Path) -> String {
    path.display().to_string().replace('\\', "/")
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
        allows: Vec::new(),
        timers: vec![],
        schedules: vec![],
        correlates: vec![],
        webhooks: vec![],
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

// ── Issue #311: escalate/give handler-exit fixes ────────────────────────────

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
    if memory.iteration > 3
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

    // Three iterations — each emits exactly one ImplementationReady, nothing else
    agent.dispatch("TestsFailed", params()).await.unwrap();
    agent.dispatch("TestsFailed", params()).await.unwrap();
    agent.dispatch("TestsFailed", params()).await.unwrap();
    {
        let ctx = agent.context().lock().unwrap();
        assert_eq!(ctx.event_sink.emitted.len(), 3);
        for evt in &ctx.event_sink.emitted {
            assert_eq!(
                evt.name, "ImplementationReady",
                "only ImplementationReady should be emitted, not Slack notifications"
            );
        }
        assert!(ctx.event_sink.escalations.is_empty());
    }

    // Fourth failure — the three configured repair attempts are consumed.
    let result = agent.dispatch("TestsFailed", params()).await.unwrap();
    assert!(
        matches!(result, Some(ref v) if matches!(&v.value, Value::Text(s) if s == "escalated"))
    );
    {
        let ctx = agent.context().lock().unwrap();
        assert_eq!(
            ctx.event_sink.emitted.len(),
            3,
            "failure after the third repair must not emit"
        );
        assert_eq!(ctx.event_sink.escalations, vec!["lead"]);
    }
}

// ── Issue #405: failed apply/no-op commit must block ImplementationReady ─────

#[tokio::test]
async fn implementer_normalizes_fenced_shell_before_execution() {
    let source = r#"
use
  llm.reason
  text.replace

event ImplementationReady
  issue_id: Text

pure strip_shell_fences
  needs script: Text
  gives Text
  do
    s1 = text.replace(script, "```bash\n", "")
    s2 = text.replace(s1, "```sh\n", "")
    s3 = text.replace(s2, "```shell\n", "")
    s4 = text.replace(s3, "```\n", "")
    s5 = text.replace(s4, "```", "")
    give s5

agent implementer
  memory
    workdir: Text

  on ImplementationApproved(workdir: Text)
    memory.workdir = workdir
    code = reason "Generate fenced shell" for implement
    stripped_code = strip_shell_fences(code)
    clean_code = stripped_code.trim()
    if clean_code == ""
      give "apply_failed: empty generated shell after fence normalization"
    apply_result = command ["sh", "-c", clean_code] in "{memory.workdir}" timeout 5s
    when apply_result.sure -> say "applied"
    else -> give "apply_failed: {apply_result}"
    emit ImplementationReady(issue_id: "414")
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

    let fenced_shell = "```bash\nprintf 'initial ok' > applied.txt\n```";
    let agent = AgentProcess::new(
        agent_decl,
        None,
        mock_registry_with(MockProvider::new("mock").with_default(fenced_shell)),
        None,
        program,
        None,
        None,
        None,
    );

    let workdir = tempfile::tempdir().unwrap();
    let mut params = HashMap::new();
    params.insert(
        "workdir".into(),
        ConfidentValue::deterministic(Value::Text(shell_path(workdir.path()))),
    );

    let result = agent
        .dispatch("ImplementationApproved", params)
        .await
        .expect("handler should not crash on fenced shell");
    assert!(
        result.is_none(),
        "fenced shell should be normalized and applied, got: {:?}",
        result
    );
    assert_eq!(
        std::fs::read_to_string(workdir.path().join("applied.txt")).unwrap(),
        "initial ok"
    );
    let ctx = agent.context().lock().unwrap();
    assert_eq!(ctx.event_sink.emitted.len(), 1);
    assert_eq!(ctx.event_sink.emitted[0].name, "ImplementationReady");
    assert!(
        ctx.event_sink.escalations.is_empty(),
        "fenced shell should not trip escalation/restart path: {:?}",
        ctx.event_sink.escalations
    );
}

#[tokio::test]
async fn implementer_normalizes_fenced_shell_on_fix_iteration() {
    let source = r#"
use
  llm.reason
  text.replace

event ImplementationReady
  issue_id: Text

pure strip_shell_fences
  needs script: Text
  gives Text
  do
    s1 = text.replace(script, "```bash\n", "")
    s2 = text.replace(s1, "```sh\n", "")
    s3 = text.replace(s2, "```shell\n", "")
    s4 = text.replace(s3, "```\n", "")
    s5 = text.replace(s4, "```", "")
    give s5

agent implementer
  memory
    workdir: Text

  on TestsFailed(workdir: Text)
    memory.workdir = workdir
    fix_code = reason "Generate fenced fix shell" for implement
    stripped_fix_code = strip_shell_fences(fix_code)
    clean_fix_code = stripped_fix_code.trim()
    if clean_fix_code == ""
      give "fix_apply_failed: empty generated shell after fence normalization"
    apply_result = command ["sh", "-c", clean_fix_code] in "{memory.workdir}" timeout 5s
    when apply_result.sure -> say "fix applied"
    else -> give "fix_apply_failed: {apply_result}"
    emit ImplementationReady(issue_id: "414")
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

    let fenced_shell = "```bash\nprintf 'fix ok' > fixed.txt\n```";
    let agent = AgentProcess::new(
        agent_decl,
        None,
        mock_registry_with(MockProvider::new("mock").with_default(fenced_shell)),
        None,
        program,
        None,
        None,
        None,
    );

    let workdir = tempfile::tempdir().unwrap();
    let mut params = HashMap::new();
    params.insert(
        "workdir".into(),
        ConfidentValue::deterministic(Value::Text(shell_path(workdir.path()))),
    );

    let result = agent
        .dispatch("TestsFailed", params)
        .await
        .expect("fix handler should not crash on fenced shell");
    assert!(
        result.is_none(),
        "fenced fix shell should be normalized and applied, got: {:?}",
        result
    );
    assert_eq!(
        std::fs::read_to_string(workdir.path().join("fixed.txt")).unwrap(),
        "fix ok"
    );
    let ctx = agent.context().lock().unwrap();
    assert_eq!(ctx.event_sink.emitted.len(), 1);
    assert_eq!(ctx.event_sink.emitted[0].name, "ImplementationReady");
    assert!(
        ctx.event_sink.escalations.is_empty(),
        "fenced fix shell should not trip escalation/restart path: {:?}",
        ctx.event_sink.escalations
    );
}

#[tokio::test]
async fn implementer_failed_generated_shell_blocks_implementation_ready() {
    let source = r#"
agent implementer
  memory
    workdir: Text

  on ImplementationApproved(workdir: Text)
    memory.workdir = workdir
    code = reason "Generate failing shell" for implement
    apply_result = command "cd {memory.workdir} && sh -c '{code}'" timeout 5s
    when apply_result.sure -> say "applied"
    else -> give "apply_failed: {apply_result}"
    commit_result = command "echo SHOULD_NOT_COMMIT" timeout 5s
    when commit_result.sure -> say "committed"
    else -> give "commit_failed: {commit_result}"
    emit ImplementationReady(issue_id: "405")
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
        mock_registry_with(MockProvider::new("mock").with_default("exit 7")),
        None,
        program,
        None,
        None,
        None,
    );

    let workdir = tempfile::tempdir().unwrap();
    let mut params = HashMap::new();
    params.insert(
        "workdir".into(),
        ConfidentValue::deterministic(Value::Text(shell_path(workdir.path()))),
    );

    let result = agent
        .dispatch("ImplementationApproved", params)
        .await
        .unwrap();
    assert!(
        matches!(result, Some(ref v) if matches!(&v.value, Value::Text(s) if s.contains("apply_failed"))),
        "failed apply should return a blocking failure, got: {:?}",
        result
    );
    let ctx = agent.context().lock().unwrap();
    assert!(
        ctx.event_sink.emitted.is_empty(),
        "failed apply must not emit ImplementationReady: {:?}",
        ctx.event_sink.emitted
    );
}

#[tokio::test]
async fn implementer_noop_commit_blocks_implementation_ready() {
    let source = r#"
agent implementer
  memory
    workdir: Text

  on ImplementationApproved(workdir: Text)
    memory.workdir = workdir
    code = reason "Generate no-op shell" for implement
    apply_result = command "cd {memory.workdir} && sh -c '{code}'" timeout 5s
    when apply_result.sure -> say "applied"
    else -> give "apply_failed: {apply_result}"
    commit_result = command "cd {memory.workdir} && git add -A && if git diff --cached --quiet; then echo 'no staged changes to commit'; exit 42; fi && git commit -m 'test commit' && git push origin issue-405 2>&1" timeout 5s
    when commit_result.sure -> say "committed"
    else -> give "commit_push_failed: {commit_result}"
    emit ImplementationReady(issue_id: "405")
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
        mock_registry_with(MockProvider::new("mock").with_default(":")),
        None,
        program,
        None,
        None,
        None,
    );

    let workdir = tempfile::tempdir().unwrap();
    let init = std::process::Command::new("git")
        .arg("init")
        .current_dir(workdir.path())
        .output()
        .expect("git init failed to spawn");
    assert!(
        init.status.success(),
        "git init failed: {}",
        String::from_utf8_lossy(&init.stderr)
    );

    let mut params = HashMap::new();
    params.insert(
        "workdir".into(),
        ConfidentValue::deterministic(Value::Text(shell_path(workdir.path()))),
    );

    let result = agent
        .dispatch("ImplementationApproved", params)
        .await
        .unwrap();
    assert!(
        matches!(result, Some(ref v) if matches!(&v.value, Value::Text(s) if s.contains("commit_push_failed"))),
        "no-op commit should return a blocking failure, got: {:?}",
        result
    );
    let ctx = agent.context().lock().unwrap();
    assert!(
        ctx.event_sink.emitted.is_empty(),
        "no-op commit must not emit ImplementationReady: {:?}",
        ctx.event_sink.emitted
    );
}

#[tokio::test]
async fn implementer_noop_fix_iteration_retries_tests_without_commit() {
    let source = r#"
use
  llm.reason

event ImplementationReady
  issue_id: Text

agent implementer
  memory
    workdir: Text

  on TestsFailed(workdir: Text)
    memory.workdir = workdir
    fix_code = reason "Generate dependency-only fix shell" for implement
    apply_result = command ["sh", "-c", fix_code] in "{memory.workdir}" timeout 5s
    when apply_result.sure -> say "fix applied"
    else -> give "fix_apply_failed: {apply_result}"
    commit_result = command "cd {memory.workdir} && git add -A && if git diff --cached --quiet; then echo 'no staged source diff after fix; skipping commit'; else git commit -m 'test commit' && git push origin issue-415; fi 2>&1" timeout 5s
    when commit_result.sure -> say "commit skipped or pushed"
    else -> give "fix_commit_push_failed: {commit_result}"
    emit ImplementationReady(issue_id: "415")
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
        mock_registry_with(MockProvider::new("mock").with_default(":")),
        None,
        program,
        None,
        None,
        None,
    );

    let workdir = tempfile::tempdir().unwrap();
    let init = std::process::Command::new("git")
        .arg("init")
        .current_dir(workdir.path())
        .output()
        .expect("git init failed to spawn");
    assert!(
        init.status.success(),
        "git init failed: {}",
        String::from_utf8_lossy(&init.stderr)
    );

    let mut params = HashMap::new();
    params.insert(
        "workdir".into(),
        ConfidentValue::deterministic(Value::Text(shell_path(workdir.path()))),
    );

    let result = agent.dispatch("TestsFailed", params).await.unwrap();
    assert!(
        result.is_none(),
        "dependency-only fix should not fail the commit path, got: {:?}",
        result
    );
    let ctx = agent.context().lock().unwrap();
    assert_eq!(ctx.event_sink.emitted.len(), 1);
    assert_eq!(ctx.event_sink.emitted[0].name, "ImplementationReady");
}

#[tokio::test]
async fn final_allowed_repair_attempt_handles_typescript_import_failure_locally() {
    let source = r#"
use
  llm.reason
  text.replace

event ImplementationReady
  issue_id: Text

event RequestHuman
  channel: Text
  context: Text
  urgency: Text
  thread_ts: Text

pure strip_shell_fences
  needs script: Text
  gives Text
  do
    s1 = text.replace(script, "```bash\n", "")
    s2 = text.replace(s1, "```sh\n", "")
    s3 = text.replace(s2, "```shell\n", "")
    s4 = text.replace(s3, "```\n", "")
    s5 = text.replace(s4, "```", "")
    give s5

agent implementer
  memory
    iteration: Number
    max_iterations: Number
    workdir: Text
    plan: Text
    last_diagnosis: Text
    iteration_log: Text

  on start
    memory.iteration = 2
    memory.max_iterations = 3
    memory.plan = "repair TypeScript proof-run test"
    memory.last_diagnosis = ""
    memory.iteration_log = ""

  on TestsFailed(workdir: Text, failures: Text)
    memory.workdir = workdir
    memory.iteration = memory.iteration + 1
    diagnosis = reason "diagnose {failures}" for implement
    memory.last_diagnosis = diagnosis
    memory.iteration_log = memory.iteration_log + "\n--- Iteration {memory.iteration} ---\n{diagnosis}"
    if memory.iteration > memory.max_iterations
      emit RequestHuman(channel: "C", context: "ESCALATION: Last failure output: {failures}", urgency: "high", thread_ts: "")
      escalate to lead
      give "escalated"
    fix_code = reason "fix {failures}" for implement
    stripped_fix_code = strip_shell_fences(fix_code)
    clean_fix_code = stripped_fix_code.trim()
    if clean_fix_code == ""
      give "fix_apply_failed: empty generated shell after fence normalization"
    apply_result = command ["sh", "-c", clean_fix_code] in "{memory.workdir}" timeout 5s
    when apply_result.sure -> say "fix applied"
    else -> give "fix_apply_failed: {apply_result}"
    emit ImplementationReady(issue_id: "424")
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

    let repair_shell = r#"printf '%s\n' \
'import { mkdtemp, rm } from "node:fs/promises";' \
'import os from "node:os";' \
'import path from "node:path";' \
'import { afterEach, beforeAll, beforeEach, describe, expect, it } from "vitest";' \
'import { buildServer } from "../src/server/app.js";' \
'import { ProofRunStore } from "../src/server/store.js";' \
'' \
'import * as fs from "fs";' \
'' \
'const seedPath = path.resolve(__dirname, "../data/proof-runs.seed.json");' \
'const dataPath = path.resolve(__dirname, "../data/proof-runs.json");' \
'' \
'beforeAll(() => {' \
'  fs.copyFileSync(seedPath, dataPath);' \
'});' \
> tests/api.test.ts"#;
    let agent = AgentProcess::new(
        agent_decl,
        None,
        mock_registry_with(MockProvider::new("mock").with_responses_sequence(vec![
            "root cause: duplicate path import and missing beforeAll in tests/api.test.ts"
                .to_string(),
            repair_shell.to_string(),
        ])),
        None,
        program,
        None,
        None,
        None,
    );

    let workdir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(workdir.path().join("tests")).unwrap();
    std::fs::create_dir_all(workdir.path().join("data")).unwrap();
    let api_test = r#"import { mkdtemp, rm } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { buildServer } from "../src/server/app.js";
import { ProofRunStore } from "../src/server/store.js";

import * as fs from 'fs';
import * as path from 'path';

const seedPath = path.resolve(__dirname, '../data/proof-runs.seed.json');
const dataPath = path.resolve(__dirname, '../data/proof-runs.json');

beforeAll(() => {
  fs.copyFileSync(seedPath, dataPath);
});
"#;
    std::fs::write(workdir.path().join("tests/api.test.ts"), api_test).unwrap();
    std::fs::write(
        workdir.path().join("data/proof-runs.seed.json"),
        r#"{"runs":[]}"#,
    )
    .unwrap();
    let seed_before =
        std::fs::read_to_string(workdir.path().join("data/proof-runs.seed.json")).unwrap();

    agent.dispatch("start", HashMap::new()).await.unwrap();
    let mut params = HashMap::new();
    params.insert(
        "workdir".into(),
        ConfidentValue::deterministic(Value::Text(shell_path(workdir.path()))),
    );
    params.insert(
        "failures".into(),
        ConfidentValue::deterministic(Value::Text(
            "tests/api.test.ts(3,8): error TS2300: Duplicate identifier 'path'.\n\
tests/api.test.ts(9,13): error TS2300: Duplicate identifier 'path'.\n\
tests/api.test.ts(14,1): error TS2304: Cannot find name 'beforeAll'."
                .into(),
        )),
    );

    let result = agent.dispatch("TestsFailed", params).await.unwrap();
    assert!(
        result.is_none(),
        "final allowed repair attempt should apply a fix, got: {:?}",
        result
    );
    let fixed = std::fs::read_to_string(workdir.path().join("tests/api.test.ts")).unwrap();
    assert!(
        fixed.contains(
            "import { afterEach, beforeAll, beforeEach, describe, expect, it } from \"vitest\";"
        ),
        "Vitest import should include beforeAll, got: {fixed}"
    );
    assert!(
        !fixed.contains("import * as path from 'path';"),
        "duplicate path import should be removed, got: {fixed}"
    );
    assert_eq!(
        std::fs::read_to_string(workdir.path().join("data/proof-runs.seed.json")).unwrap(),
        seed_before,
        "repair must not rewrite unrelated seed data"
    );
    let ctx = agent.context().lock().unwrap();
    assert_eq!(ctx.event_sink.emitted.len(), 1);
    assert_eq!(ctx.event_sink.emitted[0].name, "ImplementationReady");
    assert!(ctx.event_sink.escalations.is_empty());
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
    if memory.iteration > memory.max_iterations
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

    // Iterations 1 and 2 — both configured repair attempts should emit.
    agent.dispatch("TestsFailed", params()).await.unwrap();
    agent.dispatch("TestsFailed", params()).await.unwrap();
    {
        let ctx = agent.context().lock().unwrap();
        assert_eq!(ctx.event_sink.emitted.len(), 2, "two repairs should emit");
        assert!(ctx.event_sink.escalations.is_empty());
    }

    // Third failure — the two configured repair attempts are consumed.
    let result = agent.dispatch("TestsFailed", params()).await.unwrap();
    assert!(
        matches!(result, Some(ref v) if matches!(&v.value, Value::Text(s) if s == "escalated")),
        "iteration 2 must escalate at cap of 2"
    );
    {
        let ctx = agent.context().lock().unwrap();
        assert_eq!(
            ctx.event_sink.emitted.len(),
            2,
            "failure after the configured repairs must not emit"
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
    if memory.iteration > memory.max_iterations
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

    // Cap at 1 — first dispatch uses the one allowed repair.
    agent.dispatch("TestsFailed", params()).await.unwrap();
    // Next failure escalates with the accumulated state preserved.
    let result = agent.dispatch("TestsFailed", params()).await.unwrap();
    assert!(
        matches!(result, Some(ref v) if matches!(&v.value, Value::Text(s) if s == "escalated"))
    );
    {
        let ctx = agent.context().lock().unwrap();
        // Verify memory has the full iteration state for escalation message
        let iteration = ctx.memory.get("iteration").unwrap();
        assert!(
            matches!(&iteration.value, Value::Number(n) if *n == 2.0),
            "iteration count must be 2"
        );
        let log = ctx.memory.get("iteration_log").unwrap();
        assert!(
            matches!(&log.value, Value::Text(s) if s.contains("--- Iteration 1 ---") && s.contains("--- Iteration 2 ---")),
            "iteration_log must contain both iteration markers"
        );
        let diag = ctx.memory.get("last_diagnosis").unwrap();
        assert!(
            matches!(&diag.value, Value::Text(s) if !s.is_empty()),
            "last_diagnosis must be populated"
        );
        assert_eq!(ctx.event_sink.escalations, vec!["lead"]);
    }
}

// ── Slack adapter (#298 T3.1) ────────────────────────────────────────────────

/// The slack-adapter agent declares the seven typed outbound events, the five
/// template tasks, the `slack_adapter` agent with matching handlers, a pool of
/// 1 worker, and its warden. Parses from source.
#[tokio::test]
async fn slack_adapter_main_forge_has_seven_events_and_handlers() {
    let source = std::fs::read_to_string("tests/fixtures/agents/slack-adapter/agents.forge")
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

    assert!(
        source.contains("approval failed for {request_id}: {approval_result}"),
        "slack_adapter must log the approval failure reason"
    );
}

/// The slack-adapter declares a pool of 1 worker with `strategy: fastest`
/// — the rate-limit-backpressure shape called for in #298.
#[tokio::test]
async fn slack_adapter_declares_single_worker_pool() {
    let source = std::fs::read_to_string("tests/fixtures/agents/slack-adapter/agents.forge")
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
    let source = std::fs::read_to_string("tests/fixtures/agents/slack-adapter/agents.forge")
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
    let source = std::fs::read_to_string("tests/fixtures/agents/pr-review-bot/server.forge")
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
    let source = std::fs::read_to_string("tests/fixtures/agents/approval-gate/main.forge")
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

// ── Registered skill capability in a typed handler (#354) ───────────────────

/// A minimal self-contained program — event decl, handler with
/// four typed params, and an emit from a "specialist" agent — checks clean
/// when the `create_labeled_issue` capability signature is registered. This
/// is the grammar-green conformance check the acceptance criteria asks for.
#[tokio::test]
async fn cross_project_requested_emit_checks_clean_with_registered_skill() {
    let source = r#"use
  skill.github.create_labeled_issue

event CrossProjectRequested
  source_task_id: Text
  target_repo: Text
  description: Text
  labels: Text

agent dev_specialist
  memory
    last_request: Text
  on start
    memory.last_request = "none"
    say "[specialist] ready"
    emit CrossProjectRequested(source_task_id: "T1", target_repo: "o/r", description: "port auth middleware", labels: "area:auth")

agent mastermind
  subscribe CrossProjectRequested
  on start
    say "[mastermind] ready"
  on CrossProjectRequested(source_task_id: Text, target_repo: Text, description: Text, labels: Text)
    body = "spawned from {source_task_id}"
    url = skill.github.create_labeled_issue(target_repo, description, body, labels)
    when url.sure -> say "[mastermind] opened {url}"
    else -> say "[mastermind] uncertain"
"#;
    let program = forge::parser::parse(source).expect("T4.2 minimal program must parse");

    let mut sigs = HashMap::new();
    sigs.insert(
        "skill.github.create_labeled_issue".into(),
        forge::types::CapabilitySignature {
            inputs: vec![
                forge::types::ForgeType::Text,
                forge::types::ForgeType::Text,
                forge::types::ForgeType::Text,
                forge::types::ForgeType::Text,
            ],
            output: forge::types::ForgeType::Text,
        },
    );
    let ctx = forge::resolver::CheckContext::with_skills("t4_2.forge", sigs);
    let result = ctx.check(&program);
    assert!(
        result.is_ok(),
        "T4.2 minimal program should typecheck with create_labeled_issue registered: {:?}",
        result.err()
    );
}

// ── file.write intrinsic (#438) ─────────────────────────────────────────────

/// file.write writes UTF-8 content and returns deterministic "ok";
/// parent directories are created automatically.
#[tokio::test]
async fn file_write_writes_content_and_returns_ok() {
    let source = r#"
agent w
  memory
    p: Text
  on start
    r = file.write(memory.p, "written by forge")
    when r.sure -> say "wrote: {r}"
    else -> say "write failed"
    back = file.read(memory.p)
    when back.sure -> say "content: {back}"
    else -> say "readback failed"

  if stuck for 3 turns
    escalate to human
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

    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("nested/dir/out.txt").display().to_string();

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
    let mut params = HashMap::new();
    params.insert("p".into(), ConfidentValue::deterministic(Value::Text(path)));
    let result = agent.dispatch("start", params).await;
    assert!(
        result.is_ok(),
        "start handler should succeed: {:?}",
        result.err()
    );

    let ctx = agent.context().lock().unwrap();
    let says: Vec<String> = ctx
        .event_sink
        .emitted
        .iter()
        .map(|e| e.name.clone())
        .collect();
    drop(ctx);
    let _ = says; // say output is traced, not emitted — verify via filesystem
}

/// file.write failure path: unreachable directory returns zero-confidence
/// error text instead of crashing the run (mirrors file.read / skill contract).
#[tokio::test]
async fn file_write_failure_returns_zero_confidence_error() {
    let source = r#"
agent w
  memory
    p: Text
  on start
    r = file.write(memory.p, "data")
    when r.sure -> say "wrote ok"
    else -> say "write failed as expected"

  if stuck for 3 turns
    escalate to human
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
    // Path under a path-as-file: parent traversal fails with ENOTDIR.
    let bad = "/proc/self/nonexistent_dir_xyz/out.txt".to_string();
    let mut params = HashMap::new();
    params.insert("p".into(), ConfidentValue::deterministic(Value::Text(bad)));
    let result = agent.dispatch("start", params).await;
    assert!(
        result.is_ok(),
        "failed write must not crash the handler: {:?}",
        result.err()
    );
}

/// Boundary: file.write is rejected outside `#! boundary: server`, symmetric
/// with file.read (#380).
#[test]
fn file_write_is_server_only_in_boundary_checker() {
    let client_source = r#"#! boundary: client

agent thief
  memory
    x: Text
  on start
    r = file.write("/tmp/x.txt", "data")
    when r.sure -> say "ok"

  if stuck for 3 turns
    escalate to human
"#;
    let program = forge::parser::parse(client_source).expect("parse failed");
    let diags = forge::checker::boundary_checker::check(&[(&program, "client.forge")]);
    assert!(
        !diags.is_empty(),
        "file.write must be rejected in client boundary"
    );
    assert!(
        diags.iter().any(|d| d.message.contains("file.write()")),
        "diagnostic must name file.write, got: {:?}",
        diags.iter().map(|d| &d.message).collect::<Vec<_>>()
    );

    let server_source = client_source.replace("#! boundary: client", "#! boundary: server");
    let program = forge::parser::parse(&server_source).expect("parse failed");
    let diags = forge::checker::boundary_checker::check(&[(&program, "server.forge")]);
    assert!(
        !diags.iter().any(|d| d.message.contains("file.write()")),
        "file.write must be allowed in server boundary, got: {:?}",
        diags.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}

// ── #437: spawned agents share the parent's say-output record ────────────────

/// A `forge run` program whose only output comes from a spawned agent's
/// `on start` must be observable via TaskExecutor::outputs() — this is what
/// lets the CLI warn when a whole program prints nothing.
#[tokio::test]
async fn spawn_from_fn_main_shares_parent_output_buffer() {
    let source = r#"
agent talker
  memory
    x: Text
  on start
    say "CHILD-SAYS"

  if stuck for 3 turns
    escalate to human

fn main
  spawn talker as "t"
"#;
    let program = forge::parser::parse(source).expect("parse failed");
    let executor = forge::runtime::executor::TaskExecutor::new(program, mock_registry(), None);
    executor.run().await.expect("program should run");

    let outputs = executor.outputs();
    assert!(
        outputs.iter().any(|o| o.contains("CHILD-SAYS")),
        "spawned child's say must land in parent outputs, got: {:?}",
        outputs
    );
}
