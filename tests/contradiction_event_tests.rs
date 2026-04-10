// Integration tests for contradiction events and warden integration (issue #205).
//
// Tests cover:
// - Grammar/parser: `on contradiction:` in warden declarations
// - Checker: coverage warning includes "contradiction"
// - Warden runtime: default contradiction policy, escalation ladder
// - Session manager: ContradictionSummary persistence, event emission
// - Executor: verification gate blocks contradicted actions
// - EventBus: `session.contradiction` payload shape

use std::collections::HashMap;

use forge::ast::*;
use forge::checker::warden_checker;
use forge::runtime::confidence::{ConfidentValue, Value};
use forge::runtime::executor::TaskExecutor;
use forge::runtime::session_manager::ContradictionSummary;
use forge::runtime::verification::*;
use forge::runtime::warden::*;
use forge::types::ConfidenceSource;

fn sp<T>(node: T) -> Spanned<T> {
    Spanned::new(node, Span { start: 0, end: 0 })
}

// ── Grammar / Parser Tests ─────────────────────────────────────

#[test]
fn parse_contradiction_failure_type() {
    let src = r#"warden code_guardian
  manages [coder]

  on contradiction: nudge, self
    after 2: restart
    after 4: escalate

agent coder
  on start
    say "coding"
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
    let policy = &w.policies[0].node;
    assert_eq!(policy.failure_type.node, FailureType::Contradiction);
    assert_eq!(policy.response.node, WardResponse::Nudge);
    assert_eq!(policy.scope.node, WardScope::This);
    assert_eq!(policy.after_clauses.len(), 2);
    assert_eq!(policy.after_clauses[0].node.count, 2);
    assert_eq!(
        policy.after_clauses[0].node.response.node,
        WardResponse::Restart
    );
    assert_eq!(policy.after_clauses[1].node.count, 4);
    assert_eq!(
        policy.after_clauses[1].node.response.node,
        WardResponse::Escalate
    );
}

#[test]
fn parse_warden_with_all_six_failure_types() {
    let src = r#"warden full_guard
  manages [worker]

  on stuck: nudge, self
  on crash: restart, all
  on hallucination: replace, downstream
  on contradiction: nudge, self
    after 2: restart
    after 4: escalate
  on budget: escalate, self
  on timeout: restart, self

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

    assert_eq!(w.policies.len(), 6);
    let types: Vec<FailureType> = w
        .policies
        .iter()
        .map(|p| p.node.failure_type.node)
        .collect();
    assert!(types.contains(&FailureType::Contradiction));
}

#[test]
fn parse_agent_with_contradiction_warden_override() {
    let src = r#"agent coder
  warden_override
    on contradiction: escalate, self

  on start
    say "coding"
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
        FailureType::Contradiction
    );
    assert_eq!(
        agent.warden_override[0].node.response.node,
        WardResponse::Escalate
    );
}

// ── Checker Tests ──────────────────────────────────────────────

#[test]
fn checker_warns_missing_contradiction_coverage() {
    let src = r#"warden old_style
  manages [worker]

  on stuck: nudge, self
  on crash: restart, all
  on hallucination: replace, downstream
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
    assert_eq!(warnings.len(), 1, "should warn about missing contradiction");
    assert!(
        warnings[0].message.contains("contradiction"),
        "warning should mention 'contradiction', got: {}",
        warnings[0].message
    );
}

#[test]
fn checker_full_six_type_coverage_no_warnings() {
    let src = r#"warden complete_guard
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
        "full six-type coverage should produce no warnings"
    );
}

// ── Default Contradiction Policy ───────────────────────────────

#[test]
fn default_contradiction_policy_shape() {
    let policy = default_contradiction_policy();
    assert_eq!(policy.failure_type.node, FailureType::Contradiction);
    assert_eq!(policy.response.node, WardResponse::Nudge);
    assert_eq!(policy.scope.node, WardScope::This);
    assert_eq!(policy.after_clauses.len(), 2);
    assert_eq!(policy.after_clauses[0].node.count, 2);
    assert_eq!(
        policy.after_clauses[0].node.response.node,
        WardResponse::Restart
    );
    assert_eq!(policy.after_clauses[1].node.count, 4);
    assert_eq!(
        policy.after_clauses[1].node.response.node,
        WardResponse::Escalate
    );
}

#[test]
fn default_contradiction_policy_escalation_ladder() {
    let policy = default_contradiction_policy();

    // First failure: nudge
    assert_eq!(effective_response(&policy, 1), WardResponse::Nudge);

    // At threshold 2: restart
    assert_eq!(effective_response(&policy, 2), WardResponse::Restart);
    assert_eq!(effective_response(&policy, 3), WardResponse::Restart);

    // At threshold 4: escalate
    assert_eq!(effective_response(&policy, 4), WardResponse::Escalate);
    assert_eq!(effective_response(&policy, 10), WardResponse::Escalate);
}

// ── Warden: Contradiction Handling with Default Fallback ───────

#[test]
fn warden_handles_contradiction_with_default_fallback() {
    // Warden with NO explicit contradiction policy — should use built-in default
    let decl = WardenDecl {
        name: sp("test_warden".to_string()),
        manages: vec![sp("coder".to_string())],
        policies: vec![sp(WardPolicy {
            failure_type: sp(FailureType::Stuck),
            response: sp(WardResponse::Nudge),
            scope: sp(WardScope::This),
            after_clauses: vec![],
        })],
        max_retries: None,
    };
    let mut warden = Warden::new(decl, None);

    let signal = FailureSignal {
        agent_name: "coder".to_string(),
        failure_type: FailureType::Contradiction,
        detail: "[high] 2 contradictions found".to_string(),
    };

    // First contradiction — should get nudge from default policy
    let action = warden.handle_failure(&signal, &[], 1000);
    assert!(
        action.is_some(),
        "should fall back to default contradiction policy"
    );
    let a = action.unwrap();
    assert_eq!(a.response, WardResponse::Nudge);
    assert_eq!(a.scope, WardScope::This);
    assert_eq!(a.retry_count, 1);
}

#[test]
fn warden_contradiction_escalates_via_default_policy() {
    let decl = WardenDecl {
        name: sp("test_warden".to_string()),
        manages: vec![sp("coder".to_string())],
        policies: vec![], // No explicit policies at all
        max_retries: None,
    };
    let mut warden = Warden::new(decl, None);

    let signal = FailureSignal {
        agent_name: "coder".to_string(),
        failure_type: FailureType::Contradiction,
        detail: "test".to_string(),
    };

    // Fire 2 failures — should escalate to restart
    warden.handle_failure(&signal, &[], 1000);
    let action = warden.handle_failure(&signal, &[], 2000).unwrap();
    assert_eq!(action.response, WardResponse::Restart);

    // Fire 2 more — should escalate to escalate (count = 4)
    warden.handle_failure(&signal, &[], 3000);
    let action = warden.handle_failure(&signal, &[], 4000).unwrap();
    assert_eq!(action.response, WardResponse::Escalate);
}

#[test]
fn warden_uses_explicit_contradiction_policy_over_default() {
    let decl = WardenDecl {
        name: sp("strict_warden".to_string()),
        manages: vec![sp("coder".to_string())],
        policies: vec![sp(WardPolicy {
            failure_type: sp(FailureType::Contradiction),
            response: sp(WardResponse::Escalate),
            scope: sp(WardScope::All),
            after_clauses: vec![],
        })],
        max_retries: None,
    };
    let mut warden = Warden::new(decl, None);

    let signal = FailureSignal {
        agent_name: "coder".to_string(),
        failure_type: FailureType::Contradiction,
        detail: "test".to_string(),
    };

    // Should use explicit policy (immediate escalate), not default (nudge)
    let action = warden.handle_failure(&signal, &[], 1000).unwrap();
    assert_eq!(action.response, WardResponse::Escalate);
    assert_eq!(action.scope, WardScope::All);
}

#[test]
fn warden_release_clears_contradiction_retry_state() {
    let decl = WardenDecl {
        name: sp("test_warden".to_string()),
        manages: vec![sp("coder".to_string())],
        policies: vec![],
        max_retries: None,
    };
    let mut warden = Warden::new(decl, None);

    let signal = FailureSignal {
        agent_name: "coder".to_string(),
        failure_type: FailureType::Contradiction,
        detail: "test".to_string(),
    };

    warden.handle_failure(&signal, &[], 1000);
    warden.handle_failure(&signal, &[], 2000);
    assert_eq!(
        warden
            .retry_tracker
            .count("coder", FailureType::Contradiction),
        2
    );

    warden.release("coder");
    assert_eq!(
        warden
            .retry_tracker
            .count("coder", FailureType::Contradiction),
        0
    );
}

// ── ContradictionSummary Persistence ───────────────────────────

#[test]
fn contradiction_summary_serde_roundtrip() {
    let summary = ContradictionSummary {
        count: 3,
        high_severity_count: 1,
        max_severity: "high".to_string(),
        verification_status: "contradicted".to_string(),
        risk_class: "file_system".to_string(),
    };

    let json = serde_json::to_string(&summary).unwrap();
    let deserialized: ContradictionSummary = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.count, 3);
    assert_eq!(deserialized.high_severity_count, 1);
    assert_eq!(deserialized.max_severity, "high");
    assert_eq!(deserialized.verification_status, "contradicted");
    assert_eq!(deserialized.risk_class, "file_system");
}

#[test]
fn session_state_backward_compat_without_contradiction_summary() {
    // Simulates loading a session JSON that was persisted before #205
    // (no contradiction_summary field). The #[serde(default)] on the
    // contradiction_summary field means old JSON files deserialize cleanly.
    use forge::runtime::session_manager::SessionState;

    // Build a minimal session state JSON without the contradiction_summary field.
    let json = r#"{
        "id": "session-123",
        "config": {
            "name": "test",
            "agent": "coder",
            "prompt": "fix bug",
            "tools": [],
            "timeout_secs": null,
            "budget_usd": null,
            "gives": null,
            "cancel_timeout_secs": 30
        },
        "status": "Done",
        "external_session_id": null,
        "process_id": null,
        "started_at": "2026-04-10T00:00:00Z",
        "updated_at": "2026-04-10T00:00:00Z",
        "cost_usd": 0.0,
        "budget_exceeded": false,
        "latest_progress": null,
        "progress_events": [],
        "output": null,
        "error": null
    }"#;

    // Should deserialize cleanly with None for contradiction_summary
    let loaded: SessionState = serde_json::from_str(json).unwrap();
    assert!(loaded.contradiction_summary.is_none());
    assert_eq!(loaded.id, "session-123");
}

// ── Executor: Verification Gate ────────────────────────────────

fn make_agent_result_with_verification(
    status: VerificationStatus,
    risk: RiskClass,
) -> ConfidentValue {
    let vr = VerificationResult {
        status,
        claims: vec![],
        evidence: vec![],
        contradictions: if status == VerificationStatus::Contradicted {
            vec![Contradiction::new(
                Claim::new(
                    ClaimKind::TaskComplete,
                    "task done",
                    0.9,
                    ConfidenceSource::LLMDirect(0.9),
                ),
                Evidence::new(
                    EvidenceKind::TestResult,
                    "tests failed",
                    EvidencePolarity::Contradicts,
                    1.0,
                ),
                ContradictionSeverity::High,
            )]
        } else {
            vec![]
        },
        risk_class: risk,
    };

    let mut meta = HashMap::new();
    meta.insert(
        "verification".to_string(),
        ConfidentValue::deterministic(vr.to_value()),
    );

    let mut fields = HashMap::new();
    fields.insert(
        "plan".to_string(),
        ConfidentValue::from_skill(Value::Text("fix bug".into()), 0.85),
    );
    fields.insert(
        "metadata".to_string(),
        ConfidentValue::deterministic(Value::Record(meta)),
    );

    ConfidentValue::from_agent_result(fields)
}

#[test]
fn verification_gate_allows_verified_result() {
    let result =
        make_agent_result_with_verification(VerificationStatus::Verified, RiskClass::FileSystem);
    assert!(TaskExecutor::check_verification_gate(&result, RiskClass::FileSystem).is_ok());
}

#[test]
fn verification_gate_allows_verified_lower_risk() {
    let result =
        make_agent_result_with_verification(VerificationStatus::Verified, RiskClass::Informational);
    assert!(TaskExecutor::check_verification_gate(&result, RiskClass::FileSystem).is_ok());
}

#[test]
fn verification_gate_blocks_contradicted() {
    let result = make_agent_result_with_verification(
        VerificationStatus::Contradicted,
        RiskClass::FileSystem,
    );
    let err = TaskExecutor::check_verification_gate(&result, RiskClass::ExternalSideEffect);
    assert!(err.is_err());
    let msg = format!("{}", err.unwrap_err());
    assert!(
        msg.contains("contradiction"),
        "error should mention contradiction: {}",
        msg
    );
}

#[test]
fn verification_gate_blocks_insufficient_for_external_side_effect() {
    let result = make_agent_result_with_verification(
        VerificationStatus::Insufficient,
        RiskClass::FileSystem,
    );
    let err = TaskExecutor::check_verification_gate(&result, RiskClass::ExternalSideEffect);
    assert!(err.is_err());
    let msg = format!("{}", err.unwrap_err());
    assert!(msg.contains("insufficient"), "error: {}", msg);
}

#[test]
fn verification_gate_allows_insufficient_for_lower_risk() {
    let result = make_agent_result_with_verification(
        VerificationStatus::Insufficient,
        RiskClass::FileSystem,
    );
    // FileSystem < ExternalSideEffect, so lower risk actions are allowed with insufficient
    assert!(TaskExecutor::check_verification_gate(&result, RiskClass::FileSystem).is_ok());
}

#[test]
fn verification_gate_allows_pending() {
    let result =
        make_agent_result_with_verification(VerificationStatus::Pending, RiskClass::Informational);
    assert!(TaskExecutor::check_verification_gate(&result, RiskClass::ExternalSideEffect).is_ok());
}

#[test]
fn verification_gate_blocks_error_status() {
    let result =
        make_agent_result_with_verification(VerificationStatus::Error, RiskClass::Informational);
    let err = TaskExecutor::check_verification_gate(&result, RiskClass::FileSystem);
    assert!(err.is_err());
}

#[test]
fn verification_gate_allows_no_verification_metadata() {
    // Plain result without any verification metadata — backward compat
    let mut fields = HashMap::new();
    fields.insert(
        "plan".to_string(),
        ConfidentValue::from_skill(Value::Text("fix bug".into()), 0.85),
    );
    let result = ConfidentValue::from_agent_result(fields);

    assert!(TaskExecutor::check_verification_gate(&result, RiskClass::ExternalSideEffect).is_ok());
}

#[test]
fn verification_gate_blocks_verified_but_high_risk() {
    // Verified but risk_class exceeds allowed max
    let result = make_agent_result_with_verification(
        VerificationStatus::Verified,
        RiskClass::ExternalSideEffect,
    );
    // Gate allows up to FileSystem, but result is ExternalSideEffect
    let err = TaskExecutor::check_verification_gate(&result, RiskClass::FileSystem);
    assert!(err.is_err());
    let msg = format!("{}", err.unwrap_err());
    assert!(msg.contains("risk class"), "error: {}", msg);
}

// ── EventBus Payload Shape ─────────────────────────────────────

#[test]
fn contradiction_detected_event_payload() {
    use forge::runtime::session_manager::SessionEvent;

    let event = SessionEvent::ContradictionDetected {
        session_id: "sess-001".to_string(),
        contradiction_count: 2,
        high_severity_count: 1,
        max_severity: "high".to_string(),
        verification_status: "contradicted".to_string(),
        risk_class: "file_system".to_string(),
    };

    // Verify the event carries all expected fields
    match &event {
        SessionEvent::ContradictionDetected {
            session_id,
            contradiction_count,
            high_severity_count,
            max_severity,
            verification_status,
            risk_class,
        } => {
            assert_eq!(session_id, "sess-001");
            assert_eq!(*contradiction_count, 2);
            assert_eq!(*high_severity_count, 1);
            assert_eq!(max_severity, "high");
            assert_eq!(verification_status, "contradicted");
            assert_eq!(risk_class, "file_system");
        }
        _ => panic!("wrong event variant"),
    }
}
