// Integration tests for the claim/evidence/verification contract (issue #203).

use std::collections::HashMap;

use forge::runtime::confidence::{ConfidentValue, Value};
use forge::runtime::verification::*;
use forge::types::ConfidenceSource;

// ── AgentResult integration ────────────────────────────────────

#[test]
fn agent_result_with_verification_metadata() {
    let mut fields = HashMap::new();
    fields.insert(
        "plan".to_string(),
        ConfidentValue::from_skill(Value::Text("fix login bug".into()), 0.85),
    );
    fields.insert(
        "files_changed".to_string(),
        ConfidentValue::from_skill(
            Value::Array(vec![ConfidentValue::deterministic(Value::Text(
                "src/auth.rs".into(),
            ))]),
            0.85,
        ),
    );

    // Build metadata with verification
    let mut meta = HashMap::new();
    let claims = extract_implicit_claims(&fields);
    inject_pending_verification(&mut meta, claims);

    fields.insert(
        "metadata".to_string(),
        ConfidentValue::deterministic(Value::Record(meta)),
    );

    let result = ConfidentValue::from_agent_result(fields);

    // Verify we can traverse into metadata.verification.status
    if let Value::Record(top) = &result.value {
        let meta_cv = &top["metadata"];
        if let Value::Record(meta_fields) = &meta_cv.value {
            let ver_cv = &meta_fields["verification"];
            let vr =
                VerificationResult::from_value(&ver_cv.value).expect("should parse verification");
            assert!(vr.is_pending());
            assert_eq!(vr.claims.len(), 2); // plan + files_changed
            assert!(vr
                .claims
                .iter()
                .any(|c| c.kind == ClaimKind::TaskInterpretation));
            assert!(vr.claims.iter().any(|c| c.kind == ClaimKind::FilesChanged));
        } else {
            panic!("metadata should be Record");
        }
    } else {
        panic!("result should be Record");
    }
}

// ── Implicit claim extraction ──────────────────────────────────

#[test]
fn extract_implicit_claims_all_fields() {
    let mut fields = ConfidentValue::default_agent_result_fields();
    fields.insert(
        "plan".to_string(),
        ConfidentValue::from_skill(Value::Text("implement feature".into()), 0.9),
    );
    fields.insert(
        "files_changed".to_string(),
        ConfidentValue::from_skill(
            Value::Array(vec![
                ConfidentValue::deterministic(Value::Text("a.rs".into())),
                ConfidentValue::deterministic(Value::Text("b.rs".into())),
            ]),
            0.9,
        ),
    );
    fields.insert(
        "tests_run".to_string(),
        ConfidentValue::deterministic(Value::Number(10.0)),
    );
    fields.insert(
        "tests_passed".to_string(),
        ConfidentValue::deterministic(Value::Number(10.0)),
    );

    let claims = extract_implicit_claims(&fields);
    assert_eq!(claims.len(), 3);

    let kinds: Vec<ClaimKind> = claims.iter().map(|c| c.kind).collect();
    assert!(kinds.contains(&ClaimKind::TaskInterpretation));
    assert!(kinds.contains(&ClaimKind::FilesChanged));
    assert!(kinds.contains(&ClaimKind::TestsPass));

    // FilesChanged claim should mention count
    let fc = claims
        .iter()
        .find(|c| c.kind == ClaimKind::FilesChanged)
        .unwrap();
    assert!(fc.description.contains("2 file(s)"));
}

// ── Serde round-trip via Value ─────────────────────────────────

#[test]
fn verification_result_full_round_trip() {
    let claim1 = Claim::new(
        ClaimKind::FilesChanged,
        "changed src/main.rs",
        0.9,
        ConfidenceSource::ExecResult(0.9),
    );
    let claim2 = Claim::new(
        ClaimKind::TestsPass,
        "all 5 tests pass",
        0.95,
        ConfidenceSource::Deterministic,
    );
    let ev1 = Evidence::new(
        EvidenceKind::FileExists,
        "src/main.rs exists",
        EvidencePolarity::Supports,
        1.0,
    );
    let ev2 = Evidence::new(
        EvidenceKind::TestResult,
        "5/5 pass",
        EvidencePolarity::Supports,
        1.0,
    );
    let vr = VerificationResult {
        status: VerificationStatus::Verified,
        claims: vec![claim1, claim2],
        evidence: vec![ev1, ev2],
        contradictions: vec![],
        risk_class: RiskClass::FileSystem,
    };

    let value = vr.to_value();
    let restored = VerificationResult::from_value(&value).expect("should parse");

    assert_eq!(restored.status, VerificationStatus::Verified);
    assert_eq!(restored.claims.len(), 2);
    assert_eq!(restored.evidence.len(), 2);
    assert!(restored.contradictions.is_empty());
    assert_eq!(restored.risk_class, RiskClass::FileSystem);
    assert!(restored.is_verified());
    assert!(restored.is_actionable(RiskClass::FileSystem));
}

// ── Contradiction in VerificationResult ────────────────────────

#[test]
fn verification_result_with_contradictions() {
    let claim = Claim::new(
        ClaimKind::FilesChanged,
        "changed foo.rs",
        0.9,
        ConfidenceSource::Deterministic,
    );
    let evidence = Evidence::new(
        EvidenceKind::FileExists,
        "foo.rs not found",
        EvidencePolarity::Contradicts,
        1.0,
    );
    let contradiction = Contradiction::new(claim, evidence, ContradictionSeverity::High);

    let vr = VerificationResult {
        status: VerificationStatus::Contradicted,
        claims: vec![],
        evidence: vec![],
        contradictions: vec![contradiction],
        risk_class: RiskClass::FileSystem,
    };

    // Round-trip
    let value = vr.to_value();
    let restored = VerificationResult::from_value(&value).expect("should parse");
    assert!(restored.is_contradicted());
    assert!(restored.has_contradictions());
    assert_eq!(restored.high_severity_contradictions().len(), 1);
    assert!(!restored.is_actionable(RiskClass::ExternalSideEffect));
}

// ── Event payload compatibility ────────────────────────────────

#[test]
fn contradiction_fits_event_payload_fields() {
    // Simulates what #205 will do: putting contradiction data in event fields
    let claim = Claim::new(
        ClaimKind::SymbolReference,
        "uses foo::bar",
        0.8,
        ConfidenceSource::LLMDirect(0.8),
    );
    let evidence = Evidence::new(
        EvidenceKind::SymbolExists,
        "foo::bar not found in crate",
        EvidencePolarity::Contradicts,
        1.0,
    );
    let contradiction = Contradiction::new(claim, evidence, ContradictionSeverity::Medium);

    // Build fields like EventPayload.fields
    let mut fields: HashMap<String, ConfidentValue> = HashMap::new();
    fields.insert(
        "contradiction".to_string(),
        ConfidentValue::deterministic(contradiction.to_value()),
    );
    fields.insert(
        "verification_status".to_string(),
        ConfidentValue::deterministic(Value::Text("contradicted".into())),
    );

    // Verify we can reconstruct from the fields
    let c = Contradiction::from_value(&fields["contradiction"].value).expect("should parse");
    assert_eq!(c.claim.kind, ClaimKind::SymbolReference);
    assert_eq!(c.evidence.kind, EvidenceKind::SymbolExists);
    assert_eq!(c.severity, ContradictionSeverity::Medium);
}
