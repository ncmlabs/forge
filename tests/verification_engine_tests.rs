// Integration tests for the verification engine (issue #204).

use std::collections::HashMap;

use forge::runtime::confidence::{ConfidentValue, Value};
use forge::runtime::verification::*;
use forge::runtime::verification_engine::*;
use forge::types::ConfidenceSource;

fn text_cv(s: &str) -> ConfidentValue {
    ConfidentValue::deterministic(Value::Text(s.to_string()))
}

fn num_cv(n: f64) -> ConfidentValue {
    ConfidentValue::deterministic(Value::Number(n))
}

fn array_cv(items: Vec<&str>) -> ConfidentValue {
    ConfidentValue::deterministic(Value::Array(items.iter().map(|s| text_cv(s)).collect()))
}

fn make_claim(kind: ClaimKind, desc: &str) -> Claim {
    Claim::new(kind, desc, 0.9, ConfidenceSource::Deterministic)
}

// ── Full pipeline: verified scenario ────────────────────────

#[tokio::test]
async fn engine_verifies_valid_agent_result() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("src.rs"), "fn main() {}").unwrap();
    std::fs::create_dir(dir.path().join(".git")).unwrap();

    let mut fields = HashMap::new();
    fields.insert("plan".to_string(), text_cv("implement feature X"));
    fields.insert("confidence".to_string(), num_cv(0.92));
    fields.insert("files_changed".to_string(), array_cv(vec!["src.rs"]));

    let claims = vec![
        make_claim(ClaimKind::TaskInterpretation, "understood task"),
        make_claim(ClaimKind::FilesChanged, "changed src.rs"),
    ];

    // Skip execution validator (no Cargo.toml in temp dir).
    let mut engine = VerificationEngine::new();
    engine.add(Box::new(SchemaValidator));
    engine.add(Box::new(ReferenceValidator));
    engine.add(Box::new(EnvironmentValidator));
    engine.add(Box::new(PolicyValidator));

    let ctx = VerificationContext {
        working_dir: Some(dir.path().to_path_buf()),
        agent_fields: fields,
        claims,
    };

    let result = engine.verify(&ctx).await;

    assert_eq!(result.status, VerificationStatus::Verified);
    assert!(result.contradictions.is_empty());
    assert_eq!(result.risk_class, RiskClass::FileSystem);
    assert!(result.evidence.len() >= 3);
}

// ── Full pipeline: contradicted scenario ────────────────────

#[tokio::test]
async fn engine_contradicts_missing_files() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join(".git")).unwrap();

    let mut fields = HashMap::new();
    fields.insert("plan".to_string(), text_cv("add new module"));
    fields.insert("confidence".to_string(), num_cv(0.88));
    fields.insert(
        "files_changed".to_string(),
        array_cv(vec!["new_module.rs", "tests/test_module.rs"]),
    );

    let claims = vec![make_claim(
        ClaimKind::FilesChanged,
        "changed new_module.rs and tests/test_module.rs",
    )];

    let mut engine = VerificationEngine::new();
    engine.add(Box::new(SchemaValidator));
    engine.add(Box::new(ReferenceValidator));

    let ctx = VerificationContext {
        working_dir: Some(dir.path().to_path_buf()),
        agent_fields: fields,
        claims,
    };

    let result = engine.verify(&ctx).await;

    assert_eq!(result.status, VerificationStatus::Contradicted);
    assert_eq!(result.contradictions.len(), 2); // one per missing file
    assert!(result
        .contradictions
        .iter()
        .all(|c| c.severity == ContradictionSeverity::High));
}

// ── Extract + inject round-trip with pending result ─────────

#[test]
fn extract_inject_resolves_pending_to_verified() {
    // Build an AgentResult with pending verification.
    let mut fields = ConfidentValue::default_agent_result_fields();
    fields.insert("plan".to_string(), text_cv("fix bug"));
    fields.insert("files_changed".to_string(), array_cv(vec!["lib.rs"]));

    let claims = extract_implicit_claims(&fields);
    let mut meta = HashMap::new();
    inject_pending_verification(&mut meta, claims);
    fields.insert(
        "metadata".to_string(),
        ConfidentValue::deterministic(Value::Record(meta)),
    );

    let agent_result = ConfidentValue::deterministic(Value::Record(fields));

    // Extract inputs.
    let (extracted_fields, extracted_claims) = extract_verification_inputs(&agent_result);
    assert!(!extracted_claims.is_empty());
    assert!(extracted_fields.contains_key("plan"));

    // Simulate a resolved verification.
    let resolved = VerificationResult {
        status: VerificationStatus::Verified,
        claims: extracted_claims,
        evidence: vec![Evidence::new(
            EvidenceKind::FileExists,
            "lib.rs exists",
            EvidencePolarity::Supports,
            1.0,
        )],
        contradictions: Vec::new(),
        risk_class: RiskClass::FileSystem,
    };

    let injected = inject_resolved_verification(agent_result, resolved);

    // Verify the metadata now has the resolved status.
    if let Value::Record(ref fields) = injected.value {
        if let Some(meta_cv) = fields.get("metadata") {
            if let Value::Record(ref meta) = meta_cv.value {
                let vr = meta
                    .get("verification")
                    .and_then(|cv| VerificationResult::from_value(&cv.value))
                    .expect("verification result should be present");
                assert_eq!(vr.status, VerificationStatus::Verified);
                assert_eq!(vr.risk_class, RiskClass::FileSystem);
                assert!(!vr.evidence.is_empty());
                return;
            }
        }
    }
    panic!("could not extract resolved verification from injected result");
}

// ── Insufficient when no claims ─────────────────────────────

#[tokio::test]
async fn engine_insufficient_with_empty_result() {
    let engine = VerificationEngine::coding_session();
    let ctx = VerificationContext {
        working_dir: None,
        agent_fields: HashMap::new(),
        claims: Vec::new(),
    };

    let result = engine.verify(&ctx).await;
    assert_eq!(result.status, VerificationStatus::Insufficient);
    assert_eq!(result.risk_class, RiskClass::Informational);
}

// ── Risk classification ─────────────────────────────────────

#[test]
fn classify_risk_from_agent_fields() {
    // Empty → Informational
    assert_eq!(classify_risk(&HashMap::new()), RiskClass::Informational);

    // files_changed → FileSystem
    let mut fields = HashMap::new();
    fields.insert("files_changed".to_string(), array_cv(vec!["main.rs"]));
    assert_eq!(classify_risk(&fields), RiskClass::FileSystem);

    // git_push in metadata → ExternalSideEffect
    let mut fields2 = HashMap::new();
    let mut meta = HashMap::new();
    meta.insert(
        "git_push".to_string(),
        ConfidentValue::deterministic(Value::Bool(true)),
    );
    fields2.insert(
        "metadata".to_string(),
        ConfidentValue::deterministic(Value::Record(meta)),
    );
    assert_eq!(classify_risk(&fields2), RiskClass::ExternalSideEffect);
}
