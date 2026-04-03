// FORGE warded runtime tests — issue #24
// Integration tests for agent lifecycle management: spawn, crash detection,
// restart, escalation ladder, scope enforcement, circuit breaker.

use std::collections::HashMap;
use std::sync::Arc;

use forge::ast::*;
use forge::llm::registry::ProviderRegistry;
use forge::runtime::event_bus::EventPayload;
use forge::runtime::warded::WardedRuntime;
use forge::runtime::warden::*;

fn sp<T>(node: T) -> Spanned<T> {
    Spanned::new(node, Span { start: 0, end: 0 })
}

// ── Test Helpers ────────────────────────────────────────────────────────────

/// Build a minimal agent that handles "start" by saying something.
/// No subscriptions → agent exits immediately from run() (channels close).
fn simple_agent(name: &str) -> AgentDecl {
    AgentDecl {
        name: sp(name.to_string()),
        lifecycle: None,
        memory: vec![],
        knowledge: None,
        timers: vec![],
        subscriptions: vec![],
        warden_override: vec![],
        handlers: vec![sp(OnHandler {
            event: sp("start".to_string()),
            params: vec![],
            payload_type: None,
            requires: vec![],
            body: vec![sp(Stmt::Say(sp(Expr::Template(vec![sp(
                TemplatePart::Text("running".to_string()),
            )]))))],
        })],
        stuck_policy: None,
    }
}

/// Build an agent that subscribes to "trigger" and crashes when it receives it.
/// The handler body references an undefined variable → RuntimeError::UndefinedVariable.
fn crashing_agent(name: &str) -> AgentDecl {
    AgentDecl {
        name: sp(name.to_string()),
        lifecycle: None,
        memory: vec![],
        knowledge: None,
        timers: vec![],
        subscriptions: vec![sp(SubscribeDecl {
            event_name: sp("trigger".to_string()),
            filter: None,
        })],
        warden_override: vec![],
        handlers: vec![sp(OnHandler {
            event: sp("trigger".to_string()),
            params: vec![],
            payload_type: None,
            requires: vec![],
            body: vec![
                // Reference undefined variable → crash
                sp(Stmt::Say(sp(Expr::Ident("nonexistent_var".to_string())))),
            ],
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
        policies: policies.into_iter().map(sp).collect(),
        max_retries: max_retries.map(sp),
    }
}

fn mock_registry() -> Arc<ProviderRegistry> {
    Arc::new(ProviderRegistry::new("mock"))
}

fn make_program(items: Vec<TopLevel>) -> Program {
    Program {
        boundary: None,
        items: items.into_iter().map(sp).collect(),
    }
}

fn crash_restart_policy() -> WardPolicy {
    WardPolicy {
        failure_type: sp(FailureType::Crash),
        response: sp(WardResponse::Restart),
        scope: sp(WardScope::This),
        after_clauses: vec![],
    }
}

/// Publish a "trigger" event on the bus to make crashing agents crash.
async fn publish_trigger(bus: &forge::runtime::event_bus::SharedEventBus) {
    let bus_guard = bus.read().await;
    bus_guard.publish(&EventPayload {
        event_name: "trigger".to_string(),
        args: vec![],
        source_agent: "test".to_string(),
        fields: HashMap::new(),
    });
}

// ── Construction Tests ──────────────────────────────────────────────────────

#[tokio::test]
async fn constructs_blueprints_for_managed_agents() {
    let program = make_program(vec![
        TopLevel::Agent(simple_agent("agent_a")),
        TopLevel::Agent(simple_agent("agent_b")),
    ]);

    let runtime = WardedRuntime::new(
        test_warden(
            "boss",
            vec!["agent_a", "agent_b"],
            vec![crash_restart_policy()],
            None,
        ),
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
async fn spawns_agents_as_tokio_tasks() {
    let program = make_program(vec![TopLevel::Agent(simple_agent("worker"))]);
    let mut runtime = WardedRuntime::new(
        test_warden("boss", vec!["worker"], vec![crash_restart_policy()], None),
        &program,
        mock_registry(),
        None,
    );

    let result = runtime.spawn_all().await;
    assert!(result.is_ok(), "spawn_all failed: {:?}", result.err());
}

#[tokio::test]
async fn agents_exit_cleanly_when_no_events() {
    // Agents with no subscriptions exit immediately (no events to receive)
    let program = make_program(vec![TopLevel::Agent(simple_agent("worker"))]);
    let mut runtime = WardedRuntime::new(
        test_warden("boss", vec!["worker"], vec![crash_restart_policy()], None),
        &program,
        mock_registry(),
        None,
    );

    runtime.spawn_all().await.unwrap();

    let result = tokio::time::timeout(std::time::Duration::from_secs(2), runtime.run()).await;

    assert!(result.is_ok(), "runtime timed out");
    assert!(result.unwrap().is_ok(), "runtime returned error");
}

#[tokio::test]
async fn skips_unresolved_agents_gracefully() {
    // Warden manages "nonexistent" but no AgentDecl in program.
    // Checker catches this at compile time; runtime has no blueprint.
    let program = make_program(vec![]);
    let mut runtime = WardedRuntime::new(
        test_warden("boss", vec!["nonexistent"], vec![], None),
        &program,
        mock_registry(),
        None,
    );

    // spawn_all succeeds vacuously — no blueprints to spawn
    assert!(runtime.spawn_all().await.is_ok());
    // run() exits immediately — no agents to monitor
    assert!(runtime.run().await.is_ok());
}

// ── Live Agent Crash Detection ──────────────────────────────────────────────

#[tokio::test]
async fn detects_agent_crash_and_restarts() {
    // Agent subscribes to "trigger", crashes when it receives it.
    // Warden policy: on crash → restart, self.
    // After restart, agent is alive again (subscribed to trigger again).
    let program = make_program(vec![TopLevel::Agent(crashing_agent("crasher"))]);

    let warden_decl = test_warden(
        "boss",
        vec!["crasher"],
        vec![WardPolicy {
            failure_type: sp(FailureType::Crash),
            response: sp(WardResponse::Restart),
            scope: sp(WardScope::This),
            after_clauses: vec![sp(AfterClause {
                count: 2,
                response: sp(WardResponse::Escalate),
            })],
        }],
        None,
    );

    let mut runtime = WardedRuntime::new(warden_decl, &program, mock_registry(), None);

    runtime.spawn_all().await.unwrap();
    let bus = runtime.event_bus().clone();

    // Run the warden in a background task
    let handle = tokio::spawn(async move { runtime.run().await });

    // Give agents time to subscribe
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Trigger crash #1 — warden should restart (count=1, below threshold 2)
    publish_trigger(&bus).await;
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Trigger crash #2 — warden should escalate (count=2, hits threshold)
    publish_trigger(&bus).await;

    // Wait for warden to finish (escalation returns error)
    let result = tokio::time::timeout(std::time::Duration::from_secs(3), handle).await;

    assert!(result.is_ok(), "warden timed out");
    let inner = result.unwrap().unwrap();
    // Escalation returns an error
    assert!(inner.is_err(), "expected escalation error");
    let err_msg = format!("{:?}", inner.unwrap_err());
    assert!(
        err_msg.contains("escalated"),
        "expected escalation, got: {}",
        err_msg
    );
}

// ── Scope: self (one_for_one equivalent) ────────────────────────────────────

#[tokio::test]
async fn scope_self_only_restarts_crashed_agent() {
    // Two agents: crasher subscribes to "trigger", stable has no subscriptions.
    // Policy: on crash → restart, self.
    // Only crasher should be affected; stable exits normally.
    let program = make_program(vec![
        TopLevel::Agent(crashing_agent("crasher")),
        TopLevel::Agent(simple_agent("stable")),
    ]);

    let warden_decl = test_warden(
        "boss",
        vec!["crasher", "stable"],
        vec![WardPolicy {
            failure_type: sp(FailureType::Crash),
            response: sp(WardResponse::Restart),
            scope: sp(WardScope::This),
            after_clauses: vec![sp(AfterClause {
                count: 1,
                response: sp(WardResponse::Escalate),
            })],
        }],
        None,
    );

    let mut runtime = WardedRuntime::new(warden_decl, &program, mock_registry(), None);

    runtime.spawn_all().await.unwrap();
    let bus = runtime.event_bus().clone();

    let handle = tokio::spawn(async move { runtime.run().await });

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Trigger crash — only crasher should restart, then escalate
    publish_trigger(&bus).await;

    let result = tokio::time::timeout(std::time::Duration::from_secs(3), handle).await;

    assert!(result.is_ok(), "warden timed out");
    // Eventually escalates
    let inner = result.unwrap().unwrap();
    assert!(inner.is_err());
}

// ── Scope: all (one_for_all equivalent) ─────────────────────────────────────

#[tokio::test]
async fn scope_all_restarts_entire_group() {
    // Policy: on crash → restart, all. When crasher crashes, ALL agents restart.
    let program = make_program(vec![
        TopLevel::Agent(crashing_agent("crasher")),
        TopLevel::Agent(simple_agent("bystander")),
    ]);

    let warden_decl = test_warden(
        "boss",
        vec!["crasher", "bystander"],
        vec![WardPolicy {
            failure_type: sp(FailureType::Crash),
            response: sp(WardResponse::Restart),
            scope: sp(WardScope::All),
            after_clauses: vec![sp(AfterClause {
                count: 1,
                response: sp(WardResponse::Escalate),
            })],
        }],
        None,
    );

    let mut runtime = WardedRuntime::new(warden_decl, &program, mock_registry(), None);

    runtime.spawn_all().await.unwrap();
    let bus = runtime.event_bus().clone();

    let handle = tokio::spawn(async move { runtime.run().await });

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    publish_trigger(&bus).await;

    let result = tokio::time::timeout(std::time::Duration::from_secs(3), handle).await;

    assert!(result.is_ok(), "warden timed out");
    // Escalates after 1 crash
    let inner = result.unwrap().unwrap();
    assert!(inner.is_err());
    assert!(format!("{:?}", inner.unwrap_err()).contains("escalated"));
}

// ── Escalation Ladder ───────────────────────────────────────────────────────

#[tokio::test]
async fn escalation_ladder_nudge_to_restart_to_escalate() {
    let warden_decl = test_warden(
        "boss",
        vec![],
        vec![WardPolicy {
            failure_type: sp(FailureType::Crash),
            response: sp(WardResponse::Nudge),
            scope: sp(WardScope::This),
            after_clauses: vec![
                sp(AfterClause {
                    count: 3,
                    response: sp(WardResponse::Restart),
                }),
                sp(AfterClause {
                    count: 5,
                    response: sp(WardResponse::Escalate),
                }),
            ],
        }],
        None,
    );

    let program = make_program(vec![]);
    let mut runtime = WardedRuntime::new(warden_decl, &program, mock_registry(), None);

    let signal = FailureSignal {
        agent_name: "test".to_string(),
        failure_type: FailureType::Crash,
        detail: "crash".to_string(),
    };

    // Failures 1-2 → Nudge
    assert_eq!(
        runtime
            .warden
            .handle_failure(&signal, &[], 1000)
            .unwrap()
            .response,
        WardResponse::Nudge
    );
    assert_eq!(
        runtime
            .warden
            .handle_failure(&signal, &[], 2000)
            .unwrap()
            .response,
        WardResponse::Nudge
    );

    // Failure 3 → Restart (hits first threshold)
    assert_eq!(
        runtime
            .warden
            .handle_failure(&signal, &[], 3000)
            .unwrap()
            .response,
        WardResponse::Restart
    );

    // Failure 4 → still Restart
    assert_eq!(
        runtime
            .warden
            .handle_failure(&signal, &[], 4000)
            .unwrap()
            .response,
        WardResponse::Restart
    );

    // Failure 5 → Escalate (hits second threshold)
    assert_eq!(
        runtime
            .warden
            .handle_failure(&signal, &[], 5000)
            .unwrap()
            .response,
        WardResponse::Escalate
    );

    // Failure 6+ → still Escalate
    assert_eq!(
        runtime
            .warden
            .handle_failure(&signal, &[], 6000)
            .unwrap()
            .response,
        WardResponse::Escalate
    );
}

// ── Circuit Breaker ─────────────────────────────────────────────────────────

#[tokio::test]
async fn circuit_breaker_trips_on_rapid_failures() {
    let warden_decl = test_warden(
        "boss",
        vec![],
        vec![crash_restart_policy()],
        Some(MaxRetries {
            count: 3,
            window: sp(Duration {
                value: 10,
                unit: DurationUnit::Seconds,
            }),
        }),
    );

    let program = make_program(vec![]);
    let mut runtime = WardedRuntime::new(warden_decl, &program, mock_registry(), None);

    let signal = FailureSignal {
        agent_name: "agent_a".to_string(),
        failure_type: FailureType::Crash,
        detail: "crash".to_string(),
    };

    // Not tripped before any failures
    assert!(!runtime.warden.circuit_breaker_tripped(0));

    // Fire 3 crashes within window
    runtime.warden.handle_failure(&signal, &[], 1000);
    runtime.warden.handle_failure(&signal, &[], 2000);
    runtime.warden.handle_failure(&signal, &[], 3000);

    // Circuit breaker trips
    assert!(runtime.warden.circuit_breaker_tripped(4000));
}

#[tokio::test]
async fn circuit_breaker_respects_time_window() {
    let warden_decl = test_warden(
        "boss",
        vec![],
        vec![crash_restart_policy()],
        Some(MaxRetries {
            count: 3,
            window: sp(Duration {
                value: 5,
                unit: DurationUnit::Seconds,
            }),
        }),
    );

    let program = make_program(vec![]);
    let mut runtime = WardedRuntime::new(warden_decl, &program, mock_registry(), None);

    let signal = FailureSignal {
        agent_name: "agent_a".to_string(),
        failure_type: FailureType::Crash,
        detail: "crash".to_string(),
    };

    // Failures spread over 12s (outside 5s window)
    runtime.warden.handle_failure(&signal, &[], 1000);
    runtime.warden.handle_failure(&signal, &[], 5000);
    runtime.warden.handle_failure(&signal, &[], 12000);

    // At t=13s, only the last failure is within the 5s window
    assert!(!runtime.warden.circuit_breaker_tripped(13000));
}

// ── Agent Override in Live Runtime ──────────────────────────────────────────

#[tokio::test]
async fn agent_override_takes_precedence() {
    let mut agent = simple_agent("special");
    agent.warden_override = vec![sp(WardPolicy {
        failure_type: sp(FailureType::Crash),
        response: sp(WardResponse::Escalate),
        scope: sp(WardScope::All),
        after_clauses: vec![],
    })];

    let program = make_program(vec![TopLevel::Agent(agent.clone())]);

    let mut runtime = WardedRuntime::new(
        test_warden("boss", vec!["special"], vec![crash_restart_policy()], None),
        &program,
        mock_registry(),
        None,
    );

    // Warden default is Restart/This, but agent overrides to Escalate/All
    let signal = FailureSignal {
        agent_name: "special".to_string(),
        failure_type: FailureType::Crash,
        detail: "test".to_string(),
    };

    let action = runtime
        .warden
        .handle_failure(&signal, &agent.warden_override, 1000)
        .unwrap();
    assert_eq!(action.response, WardResponse::Escalate);
    assert_eq!(action.scope, WardScope::All);
}

// ── All Five Failure Types ──────────────────────────────────────────────────

#[tokio::test]
async fn all_failure_types_resolve_correctly() {
    let warden_decl = test_warden(
        "boss",
        vec![],
        vec![
            WardPolicy {
                failure_type: sp(FailureType::Stuck),
                response: sp(WardResponse::Nudge),
                scope: sp(WardScope::This),
                after_clauses: vec![],
            },
            WardPolicy {
                failure_type: sp(FailureType::Crash),
                response: sp(WardResponse::Restart),
                scope: sp(WardScope::All),
                after_clauses: vec![],
            },
            WardPolicy {
                failure_type: sp(FailureType::Hallucination),
                response: sp(WardResponse::Replace),
                scope: sp(WardScope::Downstream),
                after_clauses: vec![],
            },
            WardPolicy {
                failure_type: sp(FailureType::Budget),
                response: sp(WardResponse::Escalate),
                scope: sp(WardScope::This),
                after_clauses: vec![],
            },
            WardPolicy {
                failure_type: sp(FailureType::Timeout),
                response: sp(WardResponse::Restart),
                scope: sp(WardScope::This),
                after_clauses: vec![],
            },
        ],
        None,
    );

    let program = make_program(vec![]);
    let mut runtime = WardedRuntime::new(warden_decl, &program, mock_registry(), None);

    let cases = vec![
        (FailureType::Stuck, WardResponse::Nudge, WardScope::This),
        (FailureType::Crash, WardResponse::Restart, WardScope::All),
        (
            FailureType::Hallucination,
            WardResponse::Replace,
            WardScope::Downstream,
        ),
        (FailureType::Budget, WardResponse::Escalate, WardScope::This),
        (FailureType::Timeout, WardResponse::Restart, WardScope::This),
    ];

    for (ft, expected_resp, expected_scope) in cases {
        let signal = FailureSignal {
            agent_name: "test".to_string(),
            failure_type: ft,
            detail: "test".to_string(),
        };
        let action = runtime.warden.handle_failure(&signal, &[], 1000).unwrap();
        assert_eq!(
            action.response, expected_resp,
            "wrong response for {:?}",
            ft
        );
        assert_eq!(action.scope, expected_scope, "wrong scope for {:?}", ft);
    }
}
