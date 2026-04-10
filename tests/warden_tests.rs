// FORGE warden tests — issue #24
// Tests for grammar parsing, checker validation, and runtime behavior.

use forge::ast::*;
use forge::checker::warden_checker;
use forge::runtime::warden::*;

fn sp<T>(node: T) -> Spanned<T> {
    Spanned::new(node, Span { start: 0, end: 0 })
}

// ── Helper: build a WardenDecl ──────────────────────────────────────────────

fn basic_warden() -> WardenDecl {
    WardenDecl {
        name: sp("test_warden".to_string()),
        manages: vec![sp("agent_a".to_string()), sp("agent_b".to_string())],
        policies: vec![
            sp(WardPolicy {
                failure_type: sp(FailureType::Stuck),
                response: sp(WardResponse::Nudge),
                scope: sp(WardScope::This),
                after_clauses: vec![
                    sp(AfterClause {
                        count: 3,
                        response: sp(WardResponse::Restart),
                    }),
                    sp(AfterClause {
                        count: 6,
                        response: sp(WardResponse::Escalate),
                    }),
                ],
            }),
            sp(WardPolicy {
                failure_type: sp(FailureType::Crash),
                response: sp(WardResponse::Restart),
                scope: sp(WardScope::All),
                after_clauses: vec![sp(AfterClause {
                    count: 3,
                    response: sp(WardResponse::Escalate),
                })],
            }),
            sp(WardPolicy {
                failure_type: sp(FailureType::Hallucination),
                response: sp(WardResponse::Replace),
                scope: sp(WardScope::Downstream),
                after_clauses: vec![],
            }),
            sp(WardPolicy {
                failure_type: sp(FailureType::Budget),
                response: sp(WardResponse::Escalate),
                scope: sp(WardScope::This),
                after_clauses: vec![],
            }),
            sp(WardPolicy {
                failure_type: sp(FailureType::Timeout),
                response: sp(WardResponse::Restart),
                scope: sp(WardScope::This),
                after_clauses: vec![],
            }),
        ],
        max_retries: Some(sp(MaxRetries {
            count: 5,
            window: sp(Duration {
                value: 60,
                unit: DurationUnit::Seconds,
            }),
        })),
    }
}

// ── Grammar / Parser Tests ──────────────────────────────────────────────────

#[test]
fn parse_basic_warden() {
    let src = r#"warden intake_line
  manages [classifier, router]

  on stuck: nudge, self
  on crash: restart, all
  on hallucination: replace, downstream
  on budget: escalate, self
  on timeout: restart, self

  max_retries 3 per 30s then escalate

agent classifier
  on start
    say "hello"

agent router
  on start
    say "routing"
"#;
    let program = forge::parser::parse(src).expect("parse failed");
    let warden = program.items.iter().find_map(|item| match &item.node {
        TopLevel::Warden(w) => Some(w),
        _ => None,
    });
    assert!(warden.is_some(), "warden not found in parsed program");

    let w = warden.unwrap();
    assert_eq!(w.name.node, "intake_line");
    assert_eq!(w.manages.len(), 2);
    assert_eq!(w.manages[0].node, "classifier");
    assert_eq!(w.manages[1].node, "router");
    assert_eq!(w.policies.len(), 5);
    assert!(w.max_retries.is_some());
    assert_eq!(w.max_retries.as_ref().unwrap().node.count, 3);
}

#[test]
fn parse_warden_with_escalation_ladder() {
    let src = r#"warden factory
  manages [worker]

  on stuck: nudge, self
    after 3: restart
    after 6: escalate

agent worker
  on start
    say "working"
"#;
    let program = forge::parser::parse(src).expect("parse failed");
    let w = program
        .items
        .iter()
        .find_map(|item| match &item.node {
            TopLevel::Warden(w) => Some(w),
            _ => None,
        })
        .unwrap();

    let stuck_policy = &w.policies[0].node;
    assert_eq!(stuck_policy.failure_type.node, FailureType::Stuck);
    assert_eq!(stuck_policy.response.node, WardResponse::Nudge);
    assert_eq!(stuck_policy.scope.node, WardScope::This);
    assert_eq!(stuck_policy.after_clauses.len(), 2);
    assert_eq!(stuck_policy.after_clauses[0].node.count, 3);
    assert_eq!(
        stuck_policy.after_clauses[0].node.response.node,
        WardResponse::Restart
    );
    assert_eq!(stuck_policy.after_clauses[1].node.count, 6);
    assert_eq!(
        stuck_policy.after_clauses[1].node.response.node,
        WardResponse::Escalate
    );
}

#[test]
fn parse_agent_with_warden_override() {
    let src = r#"agent classifier
  warden_override
    on stuck: replace, self

  on start
    say "classifying"
"#;
    let program = forge::parser::parse(src).expect("parse failed");
    let agent = program
        .items
        .iter()
        .find_map(|item| match &item.node {
            TopLevel::Agent(a) => Some(a),
            _ => None,
        })
        .unwrap();

    assert_eq!(agent.warden_override.len(), 1);
    assert_eq!(
        agent.warden_override[0].node.failure_type.node,
        FailureType::Stuck
    );
    assert_eq!(
        agent.warden_override[0].node.response.node,
        WardResponse::Replace
    );
    assert_eq!(agent.warden_override[0].node.scope.node, WardScope::This);
}

#[test]
fn parse_warden_without_max_retries() {
    let src = r#"warden simple
  manages [worker]

  on crash: restart, all

agent worker
  on start
    say "working"
"#;
    let program = forge::parser::parse(src).expect("parse failed");
    let w = program
        .items
        .iter()
        .find_map(|item| match &item.node {
            TopLevel::Warden(w) => Some(w),
            _ => None,
        })
        .unwrap();

    assert!(w.max_retries.is_none());
    assert_eq!(w.policies.len(), 1);
}

// ── Checker Tests ───────────────────────────────────────────────────────────

#[test]
fn checker_warns_incomplete_coverage() {
    let src = r#"warden partial
  manages [worker]

  on crash: restart, all

agent worker
  on start
    say "working"
"#;
    let program = forge::parser::parse(src).expect("parse failed");
    let diagnostics = warden_checker::check(&program, "test.forge");

    let warnings: Vec<_> = diagnostics
        .iter()
        .filter(|d| matches!(d.kind, forge::diagnostic::DiagnosticKind::Warning))
        .collect();
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].message.contains("does not cover"));
    assert!(warnings[0].message.contains("stuck"));
}

#[test]
fn checker_errors_on_unknown_managed_name() {
    let src = r#"warden bad
  manages [nonexistent]

  on crash: restart, all
"#;
    let program = forge::parser::parse(src).expect("parse failed");
    let diagnostics = warden_checker::check(&program, "test.forge");

    let errors: Vec<_> = diagnostics
        .iter()
        .filter(|d| matches!(d.kind, forge::diagnostic::DiagnosticKind::Error))
        .collect();
    assert_eq!(errors.len(), 1);
    assert!(errors[0].message.contains("nonexistent"));
    assert!(errors[0].message.contains("not declared"));
}

#[test]
fn checker_errors_on_non_escalating_ladder() {
    // Build AST directly since this would be unusual to write in syntax
    let warden = WardenDecl {
        name: sp("bad_warden".to_string()),
        manages: vec![],
        policies: vec![sp(WardPolicy {
            failure_type: sp(FailureType::Stuck),
            response: sp(WardResponse::Restart),
            scope: sp(WardScope::This),
            after_clauses: vec![
                // De-escalation: Restart → Nudge (should error)
                sp(AfterClause {
                    count: 3,
                    response: sp(WardResponse::Nudge),
                }),
            ],
        })],
        max_retries: None,
    };

    let program = Program {
        boundary: None,
        items: vec![sp(TopLevel::Warden(warden))],
    };

    let diagnostics = warden_checker::check(&program, "test.forge");
    let errors: Vec<_> = diagnostics
        .iter()
        .filter(|d| matches!(d.kind, forge::diagnostic::DiagnosticKind::Error))
        .collect();
    assert!(
        !errors.is_empty(),
        "expected error for de-escalating ladder"
    );
    assert!(errors[0].message.contains("increase severity"));
}

#[test]
fn checker_full_coverage_no_warnings() {
    let src = r#"warden complete
  manages [worker]

  on stuck: nudge, self
  on crash: restart, all
  on hallucination: replace, downstream
  on contradiction: nudge, self
  on budget: escalate, self
  on timeout: restart, self

agent worker
  on start
    say "working"
"#;
    let program = forge::parser::parse(src).expect("parse failed");
    let diagnostics = warden_checker::check(&program, "test.forge");

    let warnings: Vec<_> = diagnostics
        .iter()
        .filter(|d| matches!(d.kind, forge::diagnostic::DiagnosticKind::Warning))
        .collect();
    assert_eq!(
        warnings.len(),
        0,
        "full coverage should produce no warnings"
    );
}

// ── Runtime: Policy Resolution Tests ────────────────────────────────────────

#[test]
fn resolve_policy_warden_defaults() {
    let decl = basic_warden();
    let no_overrides: Vec<Spanned<WardPolicy>> = vec![];

    let policy = resolve_policy(&decl, &no_overrides, FailureType::Stuck);
    assert!(policy.is_some());
    let p = policy.unwrap();
    assert_eq!(p.response.node, WardResponse::Nudge);
    assert_eq!(p.scope.node, WardScope::This);
}

#[test]
fn resolve_policy_agent_override() {
    let decl = basic_warden();
    let overrides = vec![sp(WardPolicy {
        failure_type: sp(FailureType::Stuck),
        response: sp(WardResponse::Replace),
        scope: sp(WardScope::Downstream),
        after_clauses: vec![],
    })];

    let policy = resolve_policy(&decl, &overrides, FailureType::Stuck);
    assert!(policy.is_some());
    let p = policy.unwrap();
    // Agent override should win
    assert_eq!(p.response.node, WardResponse::Replace);
    assert_eq!(p.scope.node, WardScope::Downstream);
}

#[test]
fn resolve_policy_no_match() {
    let decl = WardenDecl {
        name: sp("empty".to_string()),
        manages: vec![],
        policies: vec![],
        max_retries: None,
    };

    let no_overrides: Vec<Spanned<WardPolicy>> = vec![];
    let policy = resolve_policy(&decl, &no_overrides, FailureType::Crash);
    assert!(policy.is_none());
}

// ── Runtime: Effective Response (Escalation Ladder) ─────────────────────────

#[test]
fn effective_response_base_level() {
    let policy = WardPolicy {
        failure_type: sp(FailureType::Stuck),
        response: sp(WardResponse::Nudge),
        scope: sp(WardScope::This),
        after_clauses: vec![
            sp(AfterClause {
                count: 3,
                response: sp(WardResponse::Restart),
            }),
            sp(AfterClause {
                count: 6,
                response: sp(WardResponse::Escalate),
            }),
        ],
    };

    // Below first threshold → base response
    assert_eq!(effective_response(&policy, 0), WardResponse::Nudge);
    assert_eq!(effective_response(&policy, 1), WardResponse::Nudge);
    assert_eq!(effective_response(&policy, 2), WardResponse::Nudge);
}

#[test]
fn effective_response_first_escalation() {
    let policy = WardPolicy {
        failure_type: sp(FailureType::Stuck),
        response: sp(WardResponse::Nudge),
        scope: sp(WardScope::This),
        after_clauses: vec![
            sp(AfterClause {
                count: 3,
                response: sp(WardResponse::Restart),
            }),
            sp(AfterClause {
                count: 6,
                response: sp(WardResponse::Escalate),
            }),
        ],
    };

    // At first threshold → escalate to restart
    assert_eq!(effective_response(&policy, 3), WardResponse::Restart);
    assert_eq!(effective_response(&policy, 4), WardResponse::Restart);
    assert_eq!(effective_response(&policy, 5), WardResponse::Restart);
}

#[test]
fn effective_response_second_escalation() {
    let policy = WardPolicy {
        failure_type: sp(FailureType::Stuck),
        response: sp(WardResponse::Nudge),
        scope: sp(WardScope::This),
        after_clauses: vec![
            sp(AfterClause {
                count: 3,
                response: sp(WardResponse::Restart),
            }),
            sp(AfterClause {
                count: 6,
                response: sp(WardResponse::Escalate),
            }),
        ],
    };

    // At second threshold → escalate to escalate
    assert_eq!(effective_response(&policy, 6), WardResponse::Escalate);
    assert_eq!(effective_response(&policy, 10), WardResponse::Escalate);
}

// ── Runtime: Retry Tracker ──────────────────────────────────────────────────

#[test]
fn retry_tracker_counts_per_agent_type() {
    let mut tracker = RetryTracker::new();

    assert_eq!(tracker.count("agent_a", FailureType::Stuck), 0);

    let c1 = tracker.record("agent_a", FailureType::Stuck, 1000);
    assert_eq!(c1, 1);

    let c2 = tracker.record("agent_a", FailureType::Stuck, 2000);
    assert_eq!(c2, 2);

    // Different failure type, same agent → separate counter
    let c3 = tracker.record("agent_a", FailureType::Crash, 3000);
    assert_eq!(c3, 1);

    // Different agent → separate counter
    let c4 = tracker.record("agent_b", FailureType::Stuck, 4000);
    assert_eq!(c4, 1);
}

#[test]
fn retry_tracker_group_window() {
    let mut tracker = RetryTracker::new();

    tracker.record("agent_a", FailureType::Stuck, 1000);
    tracker.record("agent_b", FailureType::Crash, 2000);
    tracker.record("agent_a", FailureType::Stuck, 5000);

    // All failures within the last 10 seconds from t=6000
    assert_eq!(tracker.group_count_in_window(6000, 10000), 3);

    // Only failures within the last 2 seconds from t=6000
    assert_eq!(tracker.group_count_in_window(6000, 2000), 1);
}

#[test]
fn retry_tracker_reset() {
    let mut tracker = RetryTracker::new();

    tracker.record("agent_a", FailureType::Stuck, 1000);
    tracker.record("agent_a", FailureType::Stuck, 2000);
    assert_eq!(tracker.count("agent_a", FailureType::Stuck), 2);

    tracker.reset_agent("agent_a", FailureType::Stuck);
    assert_eq!(tracker.count("agent_a", FailureType::Stuck), 0);
}

// ── Runtime: Warden Handle Failure ──────────────────────────────────────────

#[test]
fn warden_handles_failure_with_base_response() {
    let decl = basic_warden();
    let mut warden = Warden::new(decl, None);

    let signal = FailureSignal {
        agent_name: "agent_a".to_string(),
        failure_type: FailureType::Stuck,
        detail: "looping".to_string(),
    };

    let action = warden.handle_failure(&signal, &[], 1000);
    assert!(action.is_some());
    let a = action.unwrap();
    assert_eq!(a.response, WardResponse::Nudge);
    assert_eq!(a.scope, WardScope::This);
    assert_eq!(a.retry_count, 1);
}

#[test]
fn warden_escalates_after_threshold() {
    let decl = basic_warden();
    let mut warden = Warden::new(decl, None);

    let signal = FailureSignal {
        agent_name: "agent_a".to_string(),
        failure_type: FailureType::Stuck,
        detail: "looping".to_string(),
    };

    // Fire 3 failures → should escalate from Nudge to Restart
    for i in 0..3 {
        warden.handle_failure(&signal, &[], i * 1000);
    }
    let action = warden.handle_failure(&signal, &[], 3000).unwrap();
    // 4th failure, count is now 4 which is >= 3 threshold
    assert_eq!(action.response, WardResponse::Restart);
}

#[test]
fn warden_escalates_to_final_level() {
    let decl = basic_warden();
    let mut warden = Warden::new(decl, None);

    let signal = FailureSignal {
        agent_name: "agent_a".to_string(),
        failure_type: FailureType::Stuck,
        detail: "looping".to_string(),
    };

    // Fire 6 failures → should escalate to Escalate
    for i in 0..6 {
        warden.handle_failure(&signal, &[], i * 1000);
    }
    let action = warden.handle_failure(&signal, &[], 7000).unwrap();
    // 7th failure, count is 7 which is >= 6 threshold
    assert_eq!(action.response, WardResponse::Escalate);
}

#[test]
fn warden_respects_agent_overrides() {
    let decl = basic_warden();
    let mut warden = Warden::new(decl, None);

    let overrides = vec![sp(WardPolicy {
        failure_type: sp(FailureType::Stuck),
        response: sp(WardResponse::Replace),
        scope: sp(WardScope::Downstream),
        after_clauses: vec![],
    })];

    let signal = FailureSignal {
        agent_name: "agent_a".to_string(),
        failure_type: FailureType::Stuck,
        detail: "looping".to_string(),
    };

    let action = warden.handle_failure(&signal, &overrides, 1000).unwrap();
    assert_eq!(action.response, WardResponse::Replace);
    assert_eq!(action.scope, WardScope::Downstream);
}

// ── Runtime: Circuit Breaker ────────────────────────────────────────────────

#[test]
fn circuit_breaker_trips_when_threshold_exceeded() {
    let decl = basic_warden(); // max_retries: 5 per 60s
    let mut warden = Warden::new(decl, None);

    let signal = FailureSignal {
        agent_name: "agent_a".to_string(),
        failure_type: FailureType::Crash,
        detail: "crashed".to_string(),
    };

    // Not tripped yet
    assert!(!warden.circuit_breaker_tripped(0));

    // Fire 5 failures within 60s window
    for i in 0..5 {
        warden.handle_failure(&signal, &[], i * 1000);
    }

    // Now tripped (5 failures in window)
    assert!(warden.circuit_breaker_tripped(5000));
}

#[test]
fn circuit_breaker_not_tripped_outside_window() {
    let decl = basic_warden(); // max_retries: 5 per 60s
    let mut warden = Warden::new(decl, None);

    let signal = FailureSignal {
        agent_name: "agent_a".to_string(),
        failure_type: FailureType::Crash,
        detail: "crashed".to_string(),
    };

    // Fire 5 failures spread over 120s (outside 60s window)
    for i in 0..5 {
        warden.handle_failure(&signal, &[], i * 30000);
    }

    // Check at t=150s — only recent failures within last 60s should count
    // Failures at t=60s, t=90s, t=120s are within window from t=150s
    assert!(!warden.circuit_breaker_tripped(150000));
}

#[test]
fn warden_no_circuit_breaker_without_max_retries() {
    let decl = WardenDecl {
        name: sp("no_limit".to_string()),
        manages: vec![],
        policies: vec![],
        max_retries: None,
    };
    let warden = Warden::new(decl, None);
    assert!(!warden.circuit_breaker_tripped(0));
}

// ── Ward Response Ordering ──────────────────────────────────────────────────

#[test]
fn ward_response_ordering() {
    assert!(WardResponse::Nudge < WardResponse::Restart);
    assert!(WardResponse::Restart < WardResponse::Replace);
    assert!(WardResponse::Replace < WardResponse::Escalate);
}

// ── Managed Names ───────────────────────────────────────────────────────────

#[test]
fn warden_managed_names() {
    let decl = basic_warden();
    let warden = Warden::new(decl, None);
    let names = warden.managed_names();
    assert_eq!(names, vec!["agent_a", "agent_b"]);
}

// ── Dynamic adopt/release (#86) ────────────────────────────────────────────

#[test]
fn warden_adopt_adds_agent() {
    let decl = basic_warden();
    let mut warden = Warden::new(decl, None);
    assert_eq!(warden.managed_names().len(), 2);

    warden.adopt("agent_c");
    let names = warden.managed_names();
    assert_eq!(names.len(), 3);
    assert!(names.contains(&"agent_c"));
}

#[test]
fn warden_adopt_no_duplicates() {
    let decl = basic_warden();
    let mut warden = Warden::new(decl, None);

    warden.adopt("agent_a"); // already managed
    assert_eq!(warden.managed_names().len(), 2);
}

#[test]
fn warden_release_removes_agent() {
    let decl = basic_warden();
    let mut warden = Warden::new(decl, None);

    warden.release("agent_b");
    let names = warden.managed_names();
    assert_eq!(names.len(), 1);
    assert_eq!(names[0], "agent_a");
}

#[test]
fn warden_release_clears_retry_tracker() {
    let decl = basic_warden();
    let mut warden = Warden::new(decl, None);

    // Record some failures for agent_b
    let signal = FailureSignal {
        agent_name: "agent_b".to_string(),
        failure_type: FailureType::Stuck,
        detail: "test".to_string(),
    };
    warden.handle_failure(&signal, &[], 1000);
    warden.handle_failure(&signal, &[], 2000);
    assert_eq!(warden.retry_tracker.count("agent_b", FailureType::Stuck), 2);

    // Release clears retry state
    warden.release("agent_b");
    assert_eq!(warden.retry_tracker.count("agent_b", FailureType::Stuck), 0);
}

// ── Issue #64: Downgrade response ──────────────────────────────────────────

#[test]
fn parse_downgrade_response() {
    let src = r#"warden budget_watcher
  manages [worker]

  on budget: downgrade, self
    after 2: escalate

agent worker
  on start
    say "working"
"#;
    let program = forge::parser::parse(src).expect("parse failed");
    let w = program
        .items
        .iter()
        .find_map(|item| match &item.node {
            TopLevel::Warden(w) => Some(w),
            _ => None,
        })
        .unwrap();

    assert_eq!(w.policies.len(), 1);
    let budget_policy = &w.policies[0].node;
    assert_eq!(budget_policy.failure_type.node, FailureType::Budget);
    assert_eq!(budget_policy.response.node, WardResponse::Downgrade);
    assert_eq!(budget_policy.scope.node, WardScope::This);
    assert_eq!(budget_policy.after_clauses.len(), 1);
    assert_eq!(
        budget_policy.after_clauses[0].node.response.node,
        WardResponse::Escalate
    );
}

#[test]
fn ward_response_ordering_with_downgrade() {
    assert!(WardResponse::Nudge < WardResponse::Downgrade);
    assert!(WardResponse::Downgrade < WardResponse::Restart);
    assert!(WardResponse::Restart < WardResponse::Replace);
    assert!(WardResponse::Replace < WardResponse::Escalate);
}

#[test]
fn checker_escalation_ladder_with_downgrade_valid() {
    let src = r#"warden budget_guard
  manages [worker]

  on budget: downgrade, self
    after 2: restart
    after 4: escalate

agent worker
  on start
    say "working"
"#;
    let program = forge::parser::parse(src).expect("parse failed");
    let diagnostics = warden_checker::check(&program, "test.forge");

    let errors: Vec<_> = diagnostics
        .iter()
        .filter(|d| matches!(d.kind, forge::diagnostic::DiagnosticKind::Error))
        .collect();
    assert_eq!(
        errors.len(),
        0,
        "downgrade → restart → escalate should be valid"
    );
}

#[test]
fn checker_escalation_ladder_downgrade_after_restart_invalid() {
    // Build AST directly: restart → downgrade is de-escalation
    let warden = WardenDecl {
        name: sp("bad_warden".to_string()),
        manages: vec![],
        policies: vec![sp(WardPolicy {
            failure_type: sp(FailureType::Budget),
            response: sp(WardResponse::Restart),
            scope: sp(WardScope::This),
            after_clauses: vec![sp(AfterClause {
                count: 3,
                response: sp(WardResponse::Downgrade),
            })],
        })],
        max_retries: None,
    };

    let program = Program {
        boundary: None,
        items: vec![sp(TopLevel::Warden(warden))],
    };

    let diagnostics = warden_checker::check(&program, "test.forge");
    let errors: Vec<_> = diagnostics
        .iter()
        .filter(|d| matches!(d.kind, forge::diagnostic::DiagnosticKind::Error))
        .collect();
    assert!(
        !errors.is_empty(),
        "restart → downgrade should be rejected as de-escalation"
    );
}

#[test]
fn effective_response_downgrade_then_escalate() {
    let policy = WardPolicy {
        failure_type: sp(FailureType::Budget),
        response: sp(WardResponse::Downgrade),
        scope: sp(WardScope::This),
        after_clauses: vec![sp(AfterClause {
            count: 2,
            response: sp(WardResponse::Escalate),
        })],
    };

    // Below threshold → downgrade
    assert_eq!(effective_response(&policy, 0), WardResponse::Downgrade);
    assert_eq!(effective_response(&policy, 1), WardResponse::Downgrade);

    // At threshold → escalate
    assert_eq!(effective_response(&policy, 2), WardResponse::Escalate);
    assert_eq!(effective_response(&policy, 5), WardResponse::Escalate);
}

#[test]
fn parse_wiki_supervisor_three_agents() {
    let src = r#"warden wiki_supervisor
  manages [search_agent, content_manager, qa_agent]

  on hallucination: restart, self
    after 3: escalate

  on stuck: nudge, self
    after 5: restart

  on crash: restart, self
    after 3: escalate

  on timeout: restart, self

  on budget: downgrade, self
    after 2: escalate

  max_retries 10 per 1h then escalate

agent search_agent
  on start
    say "search ready"

agent content_manager
  on start
    say "content ready"

agent qa_agent
  on start
    say "qa ready"
"#;
    let program = forge::parser::parse(src).expect("parse failed");
    let w = program
        .items
        .iter()
        .find_map(|item| match &item.node {
            TopLevel::Warden(w) => Some(w),
            _ => None,
        })
        .unwrap();

    assert_eq!(w.name.node, "wiki_supervisor");
    assert_eq!(w.manages.len(), 3);
    assert_eq!(w.manages[0].node, "search_agent");
    assert_eq!(w.manages[1].node, "content_manager");
    assert_eq!(w.manages[2].node, "qa_agent");
    assert_eq!(w.policies.len(), 5);
    assert!(w.max_retries.is_some());
}

// ── Issue #64: Hallucination detection ─────────────────────────────────────

#[test]
fn stuck_detector_hallucination_detection() {
    use forge::runtime::agent::{StuckDetector, TurnRecord};

    let mut sd = StuckDetector::new(3);

    // Not enough turns yet
    assert!(!sd.is_hallucinating());

    // Add turns with very low confidence
    sd.record_turn(TurnRecord {
        response_text: "I don't know".to_string(),
        confidence: 0.1,
        memory_hash: 1,
    });
    sd.record_turn(TurnRecord {
        response_text: "Maybe something".to_string(),
        confidence: 0.2,
        memory_hash: 2,
    });
    sd.record_turn(TurnRecord {
        response_text: "Not sure at all".to_string(),
        confidence: 0.15,
        memory_hash: 3,
    });

    // All 3 recent turns have confidence < 0.3
    assert!(sd.is_hallucinating());
}

#[test]
fn stuck_detector_not_hallucinating_with_mixed_confidence() {
    use forge::runtime::agent::{StuckDetector, TurnRecord};

    let mut sd = StuckDetector::new(3);

    sd.record_turn(TurnRecord {
        response_text: "Good answer".to_string(),
        confidence: 0.9,
        memory_hash: 1,
    });
    sd.record_turn(TurnRecord {
        response_text: "Unsure".to_string(),
        confidence: 0.2,
        memory_hash: 2,
    });
    sd.record_turn(TurnRecord {
        response_text: "Another good one".to_string(),
        confidence: 0.8,
        memory_hash: 3,
    });

    // Mixed confidence — not hallucinating
    assert!(!sd.is_hallucinating());
}
