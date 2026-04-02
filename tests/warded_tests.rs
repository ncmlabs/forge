// FORGE warded runtime tests — issue #24
// Integration tests for agent lifecycle management: spawn, crash detection,
// restart, escalation ladder, scope enforcement, circuit breaker.

use std::sync::Arc;

use forge::ast::*;
use forge::llm::registry::ProviderRegistry;
use forge::runtime::warded::WardedRuntime;
use forge::runtime::warden::*;

fn sp<T>(node: T) -> Spanned<T> {
    Spanned::new(node, Span { start: 0, end: 0 })
}

// ── Test Helpers ────────────────────────────────────────────────────────────

/// Build a minimal agent that handles "start" by saying something.
fn simple_agent(name: &str) -> AgentDecl {
    AgentDecl {
        name: sp(name.to_string()),
        lifecycle: None,
        memory: vec![],
        timers: vec![],
        subscriptions: vec![],
        warden_override: vec![],
        handlers: vec![sp(OnHandler {
            event: sp("start".to_string()),
            params: vec![],
            payload_type: None,
            requires: vec![],
            body: vec![sp(Stmt::Say(sp(Expr::Template(vec![
                sp(TemplatePart::Text("running".to_string())),
            ]))))],
        })],
        stuck_policy: None,
    }
}

/// Build a warden that manages given agent names.
fn test_warden(
    name: &str,
    agent_names: Vec<&str>,
    policies: Vec<WardPolicy>,
    max_retries: Option<MaxRetries>,
) -> WardenDecl {
    WardenDecl {
        name: sp(name.to_string()),
        manages: agent_names.iter().map(|n| sp(n.to_string())).collect(),
        policies: policies.into_iter().map(|p| sp(p)).collect(),
        max_retries: max_retries.map(|mr| sp(mr)),
    }
}

fn mock_registry() -> Arc<ProviderRegistry> {
    Arc::new(ProviderRegistry::new("mock"))
}

fn make_program(items: Vec<TopLevel>) -> Program {
    Program {
        boundary: None,
        items: items.into_iter().map(|i| sp(i)).collect(),
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[tokio::test]
async fn warded_runtime_constructs_blueprints() {
    let agent_a = simple_agent("agent_a");
    let agent_b = simple_agent("agent_b");

    let warden_decl = test_warden(
        "test_warden",
        vec!["agent_a", "agent_b"],
        vec![WardPolicy {
            failure_type: sp(FailureType::Crash),
            response: sp(WardResponse::Restart),
            scope: sp(WardScope::This),
            after_clauses: vec![],
        }],
        None,
    );

    let program = make_program(vec![
        TopLevel::Agent(agent_a),
        TopLevel::Agent(agent_b),
    ]);

    let runtime = WardedRuntime::new(
        warden_decl,
        &program,
        mock_registry(),
        None,
    );

    let names = runtime.warden.managed_names();
    assert_eq!(names.len(), 2);
    assert!(names.contains(&"agent_a"));
    assert!(names.contains(&"agent_b"));
}

#[tokio::test]
async fn warded_runtime_spawns_agents() {
    let agent = simple_agent("worker");

    let warden_decl = test_warden(
        "boss",
        vec!["worker"],
        vec![WardPolicy {
            failure_type: sp(FailureType::Crash),
            response: sp(WardResponse::Restart),
            scope: sp(WardScope::This),
            after_clauses: vec![],
        }],
        None,
    );

    let program = make_program(vec![TopLevel::Agent(agent)]);

    let mut runtime = WardedRuntime::new(
        warden_decl,
        &program,
        mock_registry(),
        None,
    );

    // spawn_all should succeed without errors
    let result = runtime.spawn_all().await;
    assert!(result.is_ok(), "spawn_all failed: {:?}", result.err());
}

#[tokio::test]
async fn warded_runtime_agents_exit_cleanly() {
    // Agents with no subscriptions will exit immediately (no events to receive)
    let agent = simple_agent("worker");

    let warden_decl = test_warden(
        "boss",
        vec!["worker"],
        vec![WardPolicy {
            failure_type: sp(FailureType::Crash),
            response: sp(WardResponse::Restart),
            scope: sp(WardScope::This),
            after_clauses: vec![],
        }],
        None,
    );

    let program = make_program(vec![TopLevel::Agent(agent)]);

    let mut runtime = WardedRuntime::new(
        warden_decl,
        &program,
        mock_registry(),
        None,
    );

    runtime.spawn_all().await.unwrap();

    // run() should complete when agents exit normally
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        runtime.run(),
    ).await;

    assert!(result.is_ok(), "runtime timed out");
    assert!(result.unwrap().is_ok(), "runtime returned error");
}

#[tokio::test]
async fn retry_tracker_integrates_with_warden() {
    let warden_decl = test_warden(
        "boss",
        vec![],
        vec![WardPolicy {
            failure_type: sp(FailureType::Crash),
            response: sp(WardResponse::Nudge),
            scope: sp(WardScope::This),
            after_clauses: vec![
                sp(AfterClause { count: 3, response: sp(WardResponse::Restart) }),
                sp(AfterClause { count: 5, response: sp(WardResponse::Escalate) }),
            ],
        }],
        None,
    );

    let program = make_program(vec![]);
    let mut runtime = WardedRuntime::new(
        warden_decl,
        &program,
        mock_registry(),
        None,
    );

    // Simulate failures directly through the warden
    let signal = FailureSignal {
        agent_name: "test_agent".to_string(),
        failure_type: FailureType::Crash,
        detail: "test crash".to_string(),
    };

    // First failure → Nudge (count=1, below threshold 3)
    let action = runtime.warden.handle_failure(&signal, &[], 1000).unwrap();
    assert_eq!(action.response, WardResponse::Nudge);

    // 2nd and 3rd failures → still Nudge
    let action = runtime.warden.handle_failure(&signal, &[], 2000).unwrap();
    assert_eq!(action.response, WardResponse::Nudge);
    let action = runtime.warden.handle_failure(&signal, &[], 3000).unwrap();
    assert_eq!(action.response, WardResponse::Restart); // count=3, hits threshold

    // 4th failure → Restart
    let action = runtime.warden.handle_failure(&signal, &[], 4000).unwrap();
    assert_eq!(action.response, WardResponse::Restart);

    // 5th failure → Escalate
    let action = runtime.warden.handle_failure(&signal, &[], 5000).unwrap();
    assert_eq!(action.response, WardResponse::Escalate);
}

#[tokio::test]
async fn circuit_breaker_integration() {
    let warden_decl = test_warden(
        "boss",
        vec![],
        vec![WardPolicy {
            failure_type: sp(FailureType::Crash),
            response: sp(WardResponse::Restart),
            scope: sp(WardScope::This),
            after_clauses: vec![],
        }],
        Some(MaxRetries {
            count: 3,
            window: sp(Duration { value: 10, unit: DurationUnit::Seconds }),
        }),
    );

    let program = make_program(vec![]);
    let mut runtime = WardedRuntime::new(
        warden_decl,
        &program,
        mock_registry(),
        None,
    );

    // Fire 3 crashes within 10s window
    let signal = FailureSignal {
        agent_name: "agent_a".to_string(),
        failure_type: FailureType::Crash,
        detail: "crash".to_string(),
    };

    runtime.warden.handle_failure(&signal, &[], 1000);
    runtime.warden.handle_failure(&signal, &[], 2000);
    runtime.warden.handle_failure(&signal, &[], 3000);

    // Circuit breaker should trip at t=4s (3 failures in 10s window)
    assert!(runtime.warden.circuit_breaker_tripped(4000));
}

#[tokio::test]
async fn warded_runtime_skips_unresolved_agents() {
    // Warden manages "nonexistent" but no AgentDecl in program.
    // The checker catches this at compile time. At runtime,
    // no blueprint is created so spawn_all has nothing to spawn.
    let warden_decl = test_warden(
        "boss",
        vec!["nonexistent"],
        vec![],
        None,
    );

    let program = make_program(vec![]);

    let mut runtime = WardedRuntime::new(
        warden_decl,
        &program,
        mock_registry(),
        None,
    );

    // spawn_all succeeds vacuously — no blueprints to spawn
    let result = runtime.spawn_all().await;
    assert!(result.is_ok());

    // run() exits immediately — no agents to monitor
    let result = runtime.run().await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn policy_with_agent_overrides_in_runtime() {
    let mut agent = simple_agent("special_agent");
    agent.warden_override = vec![sp(WardPolicy {
        failure_type: sp(FailureType::Crash),
        response: sp(WardResponse::Escalate),
        scope: sp(WardScope::All),
        after_clauses: vec![],
    })];

    let warden_decl = test_warden(
        "boss",
        vec!["special_agent"],
        vec![WardPolicy {
            failure_type: sp(FailureType::Crash),
            response: sp(WardResponse::Restart),
            scope: sp(WardScope::This),
            after_clauses: vec![],
        }],
        None,
    );

    let program = make_program(vec![TopLevel::Agent(agent.clone())]);

    let mut runtime = WardedRuntime::new(
        warden_decl,
        &program,
        mock_registry(),
        None,
    );

    // Simulate crash — agent override should win (Escalate instead of Restart)
    let signal = FailureSignal {
        agent_name: "special_agent".to_string(),
        failure_type: FailureType::Crash,
        detail: "test".to_string(),
    };

    let action = runtime.warden.handle_failure(
        &signal,
        &agent.warden_override,
        1000,
    ).unwrap();

    assert_eq!(action.response, WardResponse::Escalate);
    assert_eq!(action.scope, WardScope::All);
}
