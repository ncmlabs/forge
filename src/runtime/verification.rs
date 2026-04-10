// FORGE verification contract — issue #203
//
// Runtime types for the claim-evidence-verification model described in
// docs/2026-04-10-forge-confidence-verification-paper.md.
//
// These are runtime-native types (not language-level). They flow through
// AgentResult.metadata as Value::Record and are accessible via field access
// from FORGE code (e.g. result.metadata.verification.status).
//
// The distinction:
//   - confidence controls branching  (sure/unsure — already done)
//   - verification controls trust    (this module)
//   - policy controls action         (approval gate — #204/#205)
//
// #204 (verification engine) will populate these types.
// #205 (contradiction events) will emit them to wardens.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::runtime::confidence::{ConfidentValue, Value};
use crate::types::ConfidenceSource;

// ── RiskClass ──────────────────────────────────────────────────

/// Categorises the kind of action an agent result enables.
/// Ordered from lowest to highest risk for `is_actionable()` comparisons.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum RiskClass {
    /// Pure information, no side effects.
    Informational,
    /// Internal state mutation (memory, knowledge).
    StateMutation,
    /// File system changes within sandbox.
    FileSystem,
    /// External side effects: commit, push, PR, merge, deploy.
    ExternalSideEffect,
}

impl RiskClass {
    pub fn as_str(&self) -> &'static str {
        match self {
            RiskClass::Informational => "informational",
            RiskClass::StateMutation => "state_mutation",
            RiskClass::FileSystem => "file_system",
            RiskClass::ExternalSideEffect => "external_side_effect",
        }
    }

    pub fn parse_str(s: &str) -> Option<Self> {
        match s {
            "informational" => Some(RiskClass::Informational),
            "state_mutation" => Some(RiskClass::StateMutation),
            "file_system" => Some(RiskClass::FileSystem),
            "external_side_effect" => Some(RiskClass::ExternalSideEffect),
            _ => None,
        }
    }

    pub fn to_value(self) -> Value {
        Value::Text(self.as_str().to_string())
    }

    pub fn from_value(v: &Value) -> Option<Self> {
        match v {
            Value::Text(s) => Self::parse_str(s),
            _ => None,
        }
    }
}

// ── ClaimKind ──────────────────────────────────────────────────

/// What category of claim an agent is making.
/// Maps to paper Section 7.1: issue interpretation, target files, symbol
/// assumptions, patch intent, tests claimed to pass, completion, side effects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ClaimKind {
    TaskInterpretation,
    FilesChanged,
    SymbolReference,
    TestsPass,
    TaskComplete,
    SideEffect,
    Other,
}

impl ClaimKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ClaimKind::TaskInterpretation => "task_interpretation",
            ClaimKind::FilesChanged => "files_changed",
            ClaimKind::SymbolReference => "symbol_reference",
            ClaimKind::TestsPass => "tests_pass",
            ClaimKind::TaskComplete => "task_complete",
            ClaimKind::SideEffect => "side_effect",
            ClaimKind::Other => "other",
        }
    }

    pub fn parse_str(s: &str) -> Option<Self> {
        match s {
            "task_interpretation" => Some(ClaimKind::TaskInterpretation),
            "files_changed" => Some(ClaimKind::FilesChanged),
            "symbol_reference" => Some(ClaimKind::SymbolReference),
            "tests_pass" => Some(ClaimKind::TestsPass),
            "task_complete" => Some(ClaimKind::TaskComplete),
            "side_effect" => Some(ClaimKind::SideEffect),
            "other" => Some(ClaimKind::Other),
            _ => None,
        }
    }
}

// ── Claim ──────────────────────────────────────────────────────

/// A structured assertion made by an agent about its output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claim {
    pub kind: ClaimKind,
    pub description: String,
    pub confidence: f32,
    pub source: ConfidenceSource,
}

impl Claim {
    pub fn new(
        kind: ClaimKind,
        description: impl Into<String>,
        confidence: f32,
        source: ConfidenceSource,
    ) -> Self {
        Self {
            kind,
            description: description.into(),
            confidence: confidence.clamp(0.0, 1.0),
            source,
        }
    }

    /// Convenience: extract confidence and source from an existing ConfidentValue.
    pub fn from_confident_value(
        kind: ClaimKind,
        description: impl Into<String>,
        cv: &ConfidentValue,
    ) -> Self {
        Self {
            kind,
            description: description.into(),
            confidence: cv.confidence,
            source: cv.source,
        }
    }

    pub fn to_value(&self) -> Value {
        let mut fields = HashMap::new();
        fields.insert(
            "kind".to_string(),
            ConfidentValue::deterministic(Value::Text(self.kind.as_str().to_string())),
        );
        fields.insert(
            "description".to_string(),
            ConfidentValue::deterministic(Value::Text(self.description.clone())),
        );
        fields.insert(
            "confidence".to_string(),
            ConfidentValue::deterministic(Value::Number(self.confidence as f64)),
        );
        Value::Record(fields)
    }

    pub fn from_value(v: &Value) -> Option<Self> {
        let fields = match v {
            Value::Record(f) => f,
            _ => return None,
        };
        let kind = fields.get("kind").and_then(|cv| match &cv.value {
            Value::Text(s) => ClaimKind::parse_str(s),
            _ => None,
        })?;
        let description = fields
            .get("description")
            .and_then(|cv| match &cv.value {
                Value::Text(s) => Some(s.clone()),
                _ => None,
            })
            .unwrap_or_default();
        let confidence = fields
            .get("confidence")
            .and_then(|cv| match &cv.value {
                Value::Number(n) => Some(*n as f32),
                _ => None,
            })
            .unwrap_or(0.0);
        Some(Self {
            kind,
            description,
            confidence,
            source: ConfidenceSource::Derived(confidence),
        })
    }
}

// ── EvidenceKind ───────────────────────────────────────────────

/// What kind of evidence was gathered.
/// Maps to paper Section 7.2 validator stages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EvidenceKind {
    FileExists,
    SymbolExists,
    TestResult,
    DiffInspection,
    SchemaValidation,
    PolicyCheck,
    AgentAssessment,
    Other,
}

impl EvidenceKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            EvidenceKind::FileExists => "file_exists",
            EvidenceKind::SymbolExists => "symbol_exists",
            EvidenceKind::TestResult => "test_result",
            EvidenceKind::DiffInspection => "diff_inspection",
            EvidenceKind::SchemaValidation => "schema_validation",
            EvidenceKind::PolicyCheck => "policy_check",
            EvidenceKind::AgentAssessment => "agent_assessment",
            EvidenceKind::Other => "other",
        }
    }

    pub fn parse_str(s: &str) -> Option<Self> {
        match s {
            "file_exists" => Some(EvidenceKind::FileExists),
            "symbol_exists" => Some(EvidenceKind::SymbolExists),
            "test_result" => Some(EvidenceKind::TestResult),
            "diff_inspection" => Some(EvidenceKind::DiffInspection),
            "schema_validation" => Some(EvidenceKind::SchemaValidation),
            "policy_check" => Some(EvidenceKind::PolicyCheck),
            "agent_assessment" => Some(EvidenceKind::AgentAssessment),
            "other" => Some(EvidenceKind::Other),
            _ => None,
        }
    }
}

// ── EvidencePolarity ───────────────────────────────────────────

/// Whether evidence supports, contradicts, or is neutral to a claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EvidencePolarity {
    Supports,
    Contradicts,
    Neutral,
}

impl EvidencePolarity {
    pub fn as_str(&self) -> &'static str {
        match self {
            EvidencePolarity::Supports => "supports",
            EvidencePolarity::Contradicts => "contradicts",
            EvidencePolarity::Neutral => "neutral",
        }
    }

    pub fn parse_str(s: &str) -> Option<Self> {
        match s {
            "supports" => Some(EvidencePolarity::Supports),
            "contradicts" => Some(EvidencePolarity::Contradicts),
            "neutral" => Some(EvidencePolarity::Neutral),
            _ => None,
        }
    }
}

// ── Evidence ───────────────────────────────────────────────────

/// A piece of validation output that supports, contradicts, or is neutral to a claim.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evidence {
    pub kind: EvidenceKind,
    pub description: String,
    pub polarity: EvidencePolarity,
    pub confidence: f32,
}

impl Evidence {
    pub fn new(
        kind: EvidenceKind,
        description: impl Into<String>,
        polarity: EvidencePolarity,
        confidence: f32,
    ) -> Self {
        Self {
            kind,
            description: description.into(),
            polarity,
            confidence: confidence.clamp(0.0, 1.0),
        }
    }

    pub fn to_value(&self) -> Value {
        let mut fields = HashMap::new();
        fields.insert(
            "kind".to_string(),
            ConfidentValue::deterministic(Value::Text(self.kind.as_str().to_string())),
        );
        fields.insert(
            "description".to_string(),
            ConfidentValue::deterministic(Value::Text(self.description.clone())),
        );
        fields.insert(
            "polarity".to_string(),
            ConfidentValue::deterministic(Value::Text(self.polarity.as_str().to_string())),
        );
        fields.insert(
            "confidence".to_string(),
            ConfidentValue::deterministic(Value::Number(self.confidence as f64)),
        );
        Value::Record(fields)
    }

    pub fn from_value(v: &Value) -> Option<Self> {
        let fields = match v {
            Value::Record(f) => f,
            _ => return None,
        };
        let kind = fields.get("kind").and_then(|cv| match &cv.value {
            Value::Text(s) => EvidenceKind::parse_str(s),
            _ => None,
        })?;
        let polarity = fields.get("polarity").and_then(|cv| match &cv.value {
            Value::Text(s) => EvidencePolarity::parse_str(s),
            _ => None,
        })?;
        let description = fields
            .get("description")
            .and_then(|cv| match &cv.value {
                Value::Text(s) => Some(s.clone()),
                _ => None,
            })
            .unwrap_or_default();
        let confidence = fields
            .get("confidence")
            .and_then(|cv| match &cv.value {
                Value::Number(n) => Some(*n as f32),
                _ => None,
            })
            .unwrap_or(0.0);
        Some(Self {
            kind,
            description,
            polarity,
            confidence,
        })
    }
}

// ── ContradictionSeverity ──────────────────────────────────────

/// How serious a contradiction is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ContradictionSeverity {
    /// Minor: might be a false positive or low-impact.
    Low,
    /// Moderate: likely real, affects correctness.
    Medium,
    /// Critical: high-confidence contradiction of a high-risk claim.
    High,
}

impl ContradictionSeverity {
    pub fn as_str(&self) -> &'static str {
        match self {
            ContradictionSeverity::Low => "low",
            ContradictionSeverity::Medium => "medium",
            ContradictionSeverity::High => "high",
        }
    }

    pub fn parse_str(s: &str) -> Option<Self> {
        match s {
            "low" => Some(ContradictionSeverity::Low),
            "medium" => Some(ContradictionSeverity::Medium),
            "high" => Some(ContradictionSeverity::High),
            _ => None,
        }
    }
}

// ── Contradiction ──────────────────────────────────────────────

/// A claim contradicted by evidence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contradiction {
    pub claim: Claim,
    pub evidence: Evidence,
    pub severity: ContradictionSeverity,
}

impl Contradiction {
    pub fn new(claim: Claim, evidence: Evidence, severity: ContradictionSeverity) -> Self {
        Self {
            claim,
            evidence,
            severity,
        }
    }

    pub fn to_value(&self) -> Value {
        let mut fields = HashMap::new();
        fields.insert(
            "claim".to_string(),
            ConfidentValue::deterministic(self.claim.to_value()),
        );
        fields.insert(
            "evidence".to_string(),
            ConfidentValue::deterministic(self.evidence.to_value()),
        );
        fields.insert(
            "severity".to_string(),
            ConfidentValue::deterministic(Value::Text(self.severity.as_str().to_string())),
        );
        Value::Record(fields)
    }

    pub fn from_value(v: &Value) -> Option<Self> {
        let fields = match v {
            Value::Record(f) => f,
            _ => return None,
        };
        let claim = fields
            .get("claim")
            .and_then(|cv| Claim::from_value(&cv.value))?;
        let evidence = fields
            .get("evidence")
            .and_then(|cv| Evidence::from_value(&cv.value))?;
        let severity = fields.get("severity").and_then(|cv| match &cv.value {
            Value::Text(s) => ContradictionSeverity::parse_str(s),
            _ => None,
        })?;
        Some(Self {
            claim,
            evidence,
            severity,
        })
    }
}

// ── VerificationStatus ─────────────────────────────────────────

/// Overall outcome of verification.
/// Paper Section 7.2: verified, insufficient, contradicted, error.
/// Plus Pending as the initial state before verification runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum VerificationStatus {
    /// All claims supported by evidence, no contradictions.
    Verified,
    /// Some claims lack sufficient evidence (but no contradictions).
    Insufficient,
    /// At least one claim is contradicted by evidence.
    Contradicted,
    /// Verification itself failed (e.g., couldn't run tests).
    Error,
    /// Not yet verified (initial state).
    Pending,
}

impl VerificationStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            VerificationStatus::Verified => "verified",
            VerificationStatus::Insufficient => "insufficient",
            VerificationStatus::Contradicted => "contradicted",
            VerificationStatus::Error => "error",
            VerificationStatus::Pending => "pending",
        }
    }

    pub fn parse_str(s: &str) -> Option<Self> {
        match s {
            "verified" => Some(VerificationStatus::Verified),
            "insufficient" => Some(VerificationStatus::Insufficient),
            "contradicted" => Some(VerificationStatus::Contradicted),
            "error" => Some(VerificationStatus::Error),
            "pending" => Some(VerificationStatus::Pending),
            _ => None,
        }
    }
}

// ── VerificationResult ─────────────────────────────────────────

/// The complete verification state for an agent result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationResult {
    pub status: VerificationStatus,
    pub claims: Vec<Claim>,
    pub evidence: Vec<Evidence>,
    pub contradictions: Vec<Contradiction>,
    pub risk_class: RiskClass,
}

impl VerificationResult {
    /// Create a pending verification result (initial state).
    pub fn pending() -> Self {
        Self {
            status: VerificationStatus::Pending,
            claims: Vec::new(),
            evidence: Vec::new(),
            contradictions: Vec::new(),
            risk_class: RiskClass::Informational,
        }
    }

    pub fn is_verified(&self) -> bool {
        self.status == VerificationStatus::Verified
    }

    pub fn is_contradicted(&self) -> bool {
        self.status == VerificationStatus::Contradicted
    }

    pub fn is_pending(&self) -> bool {
        self.status == VerificationStatus::Pending
    }

    pub fn has_contradictions(&self) -> bool {
        !self.contradictions.is_empty()
    }

    /// Return only the high-severity contradictions.
    pub fn high_severity_contradictions(&self) -> Vec<&Contradiction> {
        self.contradictions
            .iter()
            .filter(|c| c.severity == ContradictionSeverity::High)
            .collect()
    }

    /// Check if the result is verified AND within the allowed risk level.
    pub fn is_actionable(&self, max_risk: RiskClass) -> bool {
        self.is_verified() && self.risk_class <= max_risk
    }

    pub fn to_value(&self) -> Value {
        let mut fields = HashMap::new();
        fields.insert(
            "status".to_string(),
            ConfidentValue::deterministic(Value::Text(self.status.as_str().to_string())),
        );
        fields.insert(
            "claims".to_string(),
            ConfidentValue::deterministic(Value::Array(
                self.claims
                    .iter()
                    .map(|c| ConfidentValue::deterministic(c.to_value()))
                    .collect(),
            )),
        );
        fields.insert(
            "evidence".to_string(),
            ConfidentValue::deterministic(Value::Array(
                self.evidence
                    .iter()
                    .map(|e| ConfidentValue::deterministic(e.to_value()))
                    .collect(),
            )),
        );
        fields.insert(
            "contradictions".to_string(),
            ConfidentValue::deterministic(Value::Array(
                self.contradictions
                    .iter()
                    .map(|c| ConfidentValue::deterministic(c.to_value()))
                    .collect(),
            )),
        );
        fields.insert(
            "risk_class".to_string(),
            ConfidentValue::deterministic(self.risk_class.to_value()),
        );
        Value::Record(fields)
    }

    pub fn from_value(v: &Value) -> Option<Self> {
        let fields = match v {
            Value::Record(f) => f,
            _ => return None,
        };
        let status = fields.get("status").and_then(|cv| match &cv.value {
            Value::Text(s) => VerificationStatus::parse_str(s),
            _ => None,
        })?;
        let risk_class = fields
            .get("risk_class")
            .and_then(|cv| RiskClass::from_value(&cv.value))
            .unwrap_or(RiskClass::Informational);
        let claims = fields
            .get("claims")
            .and_then(|cv| match &cv.value {
                Value::Array(items) => Some(
                    items
                        .iter()
                        .filter_map(|cv| Claim::from_value(&cv.value))
                        .collect(),
                ),
                _ => None,
            })
            .unwrap_or_default();
        let evidence = fields
            .get("evidence")
            .and_then(|cv| match &cv.value {
                Value::Array(items) => Some(
                    items
                        .iter()
                        .filter_map(|cv| Evidence::from_value(&cv.value))
                        .collect(),
                ),
                _ => None,
            })
            .unwrap_or_default();
        let contradictions = fields
            .get("contradictions")
            .and_then(|cv| match &cv.value {
                Value::Array(items) => Some(
                    items
                        .iter()
                        .filter_map(|cv| Contradiction::from_value(&cv.value))
                        .collect(),
                ),
                _ => None,
            })
            .unwrap_or_default();
        Some(Self {
            status,
            claims,
            evidence,
            contradictions,
            risk_class,
        })
    }
}

// ── Helpers for AgentResult integration ────────────────────────

/// Extract implicit claims from AgentResult fields.
/// Called during parse_final() to seed the verification contract.
pub fn extract_implicit_claims(fields: &HashMap<String, ConfidentValue>) -> Vec<Claim> {
    let mut claims = Vec::new();

    // plan non-empty → TaskInterpretation claim
    if let Some(cv) = fields.get("plan") {
        if let Value::Text(s) = &cv.value {
            if !s.is_empty() {
                claims.push(Claim::from_confident_value(
                    ClaimKind::TaskInterpretation,
                    "Agent provided an implementation plan",
                    cv,
                ));
            }
        }
    }

    // files_changed non-empty → FilesChanged claim
    if let Some(cv) = fields.get("files_changed") {
        if let Value::Array(items) = &cv.value {
            if !items.is_empty() {
                let file_count = items.len();
                claims.push(Claim::from_confident_value(
                    ClaimKind::FilesChanged,
                    format!("Agent claims {file_count} file(s) changed"),
                    cv,
                ));
            }
        }
    }

    // tests_run > 0 && tests_passed == tests_run → TestsPass claim
    let tests_run = fields
        .get("tests_run")
        .and_then(|cv| match &cv.value {
            Value::Number(n) => Some(*n),
            _ => None,
        })
        .unwrap_or(0.0);
    let tests_passed = fields
        .get("tests_passed")
        .and_then(|cv| match &cv.value {
            Value::Number(n) => Some(*n),
            _ => None,
        })
        .unwrap_or(0.0);

    if tests_run > 0.0 {
        let cv = fields.get("tests_run").unwrap();
        if (tests_passed - tests_run).abs() < f64::EPSILON {
            claims.push(Claim::from_confident_value(
                ClaimKind::TestsPass,
                format!("Agent claims all {tests_run} test(s) passed"),
                cv,
            ));
        } else {
            claims.push(Claim::from_confident_value(
                ClaimKind::TestsPass,
                format!("Agent claims {tests_passed}/{tests_run} test(s) passed"),
                cv,
            ));
        }
    }

    claims
}

/// Inject a pending VerificationResult (with optional pre-extracted claims)
/// into AgentResult metadata fields.
pub fn inject_pending_verification(meta: &mut HashMap<String, ConfidentValue>, claims: Vec<Claim>) {
    let mut vr = VerificationResult::pending();
    vr.claims = claims;
    meta.insert(
        "verification".to_string(),
        ConfidentValue::deterministic(vr.to_value()),
    );
}

// ── Tests ──────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // -- RiskClass --

    #[test]
    fn risk_class_ordering() {
        assert!(RiskClass::Informational < RiskClass::StateMutation);
        assert!(RiskClass::StateMutation < RiskClass::FileSystem);
        assert!(RiskClass::FileSystem < RiskClass::ExternalSideEffect);
    }

    #[test]
    fn risk_class_round_trip() {
        for rc in [
            RiskClass::Informational,
            RiskClass::StateMutation,
            RiskClass::FileSystem,
            RiskClass::ExternalSideEffect,
        ] {
            let v = rc.to_value();
            assert_eq!(RiskClass::from_value(&v), Some(rc));
        }
    }

    // -- ClaimKind --

    #[test]
    fn claim_kind_round_trip() {
        for kind in [
            ClaimKind::TaskInterpretation,
            ClaimKind::FilesChanged,
            ClaimKind::SymbolReference,
            ClaimKind::TestsPass,
            ClaimKind::TaskComplete,
            ClaimKind::SideEffect,
            ClaimKind::Other,
        ] {
            let s = kind.as_str();
            assert_eq!(ClaimKind::parse_str(s), Some(kind));
        }
    }

    // -- Claim --

    #[test]
    fn claim_new_clamps_confidence() {
        let c = Claim::new(
            ClaimKind::Other,
            "test",
            1.5,
            ConfidenceSource::Deterministic,
        );
        assert_eq!(c.confidence, 1.0);
        let c = Claim::new(
            ClaimKind::Other,
            "test",
            -0.5,
            ConfidenceSource::Deterministic,
        );
        assert_eq!(c.confidence, 0.0);
    }

    #[test]
    fn claim_from_confident_value() {
        let cv = ConfidentValue::from_llm(Value::Text("plan".into()), 0.85);
        let claim = Claim::from_confident_value(ClaimKind::TaskInterpretation, "has a plan", &cv);
        assert_eq!(claim.kind, ClaimKind::TaskInterpretation);
        assert_eq!(claim.confidence, 0.85);
        assert!(matches!(claim.source, ConfidenceSource::LLMDirect(_)));
    }

    #[test]
    fn claim_to_value_from_value_round_trip() {
        let original = Claim::new(
            ClaimKind::FilesChanged,
            "changed 3 files",
            0.9,
            ConfidenceSource::ExecResult(0.9),
        );
        let v = original.to_value();
        let restored = Claim::from_value(&v).expect("should parse");
        assert_eq!(restored.kind, original.kind);
        assert_eq!(restored.description, original.description);
        assert!((restored.confidence - original.confidence).abs() < 0.01);
    }

    // -- EvidenceKind --

    #[test]
    fn evidence_kind_round_trip() {
        for kind in [
            EvidenceKind::FileExists,
            EvidenceKind::SymbolExists,
            EvidenceKind::TestResult,
            EvidenceKind::DiffInspection,
            EvidenceKind::SchemaValidation,
            EvidenceKind::PolicyCheck,
            EvidenceKind::AgentAssessment,
            EvidenceKind::Other,
        ] {
            let s = kind.as_str();
            assert_eq!(EvidenceKind::parse_str(s), Some(kind));
        }
    }

    // -- EvidencePolarity --

    #[test]
    fn evidence_polarity_round_trip() {
        for p in [
            EvidencePolarity::Supports,
            EvidencePolarity::Contradicts,
            EvidencePolarity::Neutral,
        ] {
            let s = p.as_str();
            assert_eq!(EvidencePolarity::parse_str(s), Some(p));
        }
    }

    // -- Evidence --

    #[test]
    fn evidence_new_clamps_confidence() {
        let e = Evidence::new(
            EvidenceKind::TestResult,
            "all pass",
            EvidencePolarity::Supports,
            1.5,
        );
        assert_eq!(e.confidence, 1.0);
    }

    #[test]
    fn evidence_to_value_from_value_round_trip() {
        let original = Evidence::new(
            EvidenceKind::FileExists,
            "src/main.rs exists",
            EvidencePolarity::Supports,
            1.0,
        );
        let v = original.to_value();
        let restored = Evidence::from_value(&v).expect("should parse");
        assert_eq!(restored.kind, original.kind);
        assert_eq!(restored.polarity, original.polarity);
        assert_eq!(restored.description, original.description);
    }

    // -- ContradictionSeverity --

    #[test]
    fn contradiction_severity_ordering() {
        assert!(ContradictionSeverity::Low < ContradictionSeverity::Medium);
        assert!(ContradictionSeverity::Medium < ContradictionSeverity::High);
    }

    #[test]
    fn contradiction_severity_round_trip() {
        for s in [
            ContradictionSeverity::Low,
            ContradictionSeverity::Medium,
            ContradictionSeverity::High,
        ] {
            assert_eq!(ContradictionSeverity::parse_str(s.as_str()), Some(s));
        }
    }

    // -- Contradiction --

    #[test]
    fn contradiction_to_value_from_value_round_trip() {
        let claim = Claim::new(
            ClaimKind::FilesChanged,
            "changed foo.rs",
            0.9,
            ConfidenceSource::Deterministic,
        );
        let evidence = Evidence::new(
            EvidenceKind::FileExists,
            "foo.rs does not exist",
            EvidencePolarity::Contradicts,
            1.0,
        );
        let original = Contradiction::new(claim, evidence, ContradictionSeverity::High);
        let v = original.to_value();
        let restored = Contradiction::from_value(&v).expect("should parse");
        assert_eq!(restored.claim.kind, ClaimKind::FilesChanged);
        assert_eq!(restored.evidence.polarity, EvidencePolarity::Contradicts);
        assert_eq!(restored.severity, ContradictionSeverity::High);
    }

    // -- VerificationStatus --

    #[test]
    fn verification_status_round_trip() {
        for s in [
            VerificationStatus::Verified,
            VerificationStatus::Insufficient,
            VerificationStatus::Contradicted,
            VerificationStatus::Error,
            VerificationStatus::Pending,
        ] {
            assert_eq!(VerificationStatus::parse_str(s.as_str()), Some(s));
        }
    }

    // -- VerificationResult --

    #[test]
    fn pending_result_defaults() {
        let vr = VerificationResult::pending();
        assert!(vr.is_pending());
        assert!(!vr.is_verified());
        assert!(!vr.is_contradicted());
        assert!(!vr.has_contradictions());
        assert!(vr.claims.is_empty());
        assert!(vr.evidence.is_empty());
        assert!(vr.contradictions.is_empty());
        assert_eq!(vr.risk_class, RiskClass::Informational);
    }

    #[test]
    fn verified_result_is_actionable() {
        let vr = VerificationResult {
            status: VerificationStatus::Verified,
            claims: vec![],
            evidence: vec![],
            contradictions: vec![],
            risk_class: RiskClass::FileSystem,
        };
        assert!(vr.is_verified());
        assert!(vr.is_actionable(RiskClass::FileSystem));
        assert!(vr.is_actionable(RiskClass::ExternalSideEffect));
        assert!(!vr.is_actionable(RiskClass::StateMutation));
    }

    #[test]
    fn contradicted_result_not_actionable() {
        let vr = VerificationResult {
            status: VerificationStatus::Contradicted,
            claims: vec![],
            evidence: vec![],
            contradictions: vec![Contradiction::new(
                Claim::new(
                    ClaimKind::TestsPass,
                    "all pass",
                    0.9,
                    ConfidenceSource::Deterministic,
                ),
                Evidence::new(
                    EvidenceKind::TestResult,
                    "3 failures",
                    EvidencePolarity::Contradicts,
                    1.0,
                ),
                ContradictionSeverity::High,
            )],
            risk_class: RiskClass::Informational,
        };
        assert!(vr.is_contradicted());
        assert!(vr.has_contradictions());
        assert!(!vr.is_actionable(RiskClass::ExternalSideEffect));
    }

    #[test]
    fn high_severity_contradictions_filter() {
        let low = Contradiction::new(
            Claim::new(
                ClaimKind::Other,
                "minor",
                0.5,
                ConfidenceSource::Deterministic,
            ),
            Evidence::new(
                EvidenceKind::Other,
                "minor issue",
                EvidencePolarity::Contradicts,
                0.6,
            ),
            ContradictionSeverity::Low,
        );
        let high = Contradiction::new(
            Claim::new(
                ClaimKind::TestsPass,
                "all pass",
                0.9,
                ConfidenceSource::Deterministic,
            ),
            Evidence::new(
                EvidenceKind::TestResult,
                "3 failures",
                EvidencePolarity::Contradicts,
                1.0,
            ),
            ContradictionSeverity::High,
        );
        let vr = VerificationResult {
            status: VerificationStatus::Contradicted,
            claims: vec![],
            evidence: vec![],
            contradictions: vec![low, high],
            risk_class: RiskClass::Informational,
        };
        let highs = vr.high_severity_contradictions();
        assert_eq!(highs.len(), 1);
        assert_eq!(highs[0].severity, ContradictionSeverity::High);
    }

    #[test]
    fn verification_result_to_value_from_value_round_trip() {
        let vr = VerificationResult {
            status: VerificationStatus::Verified,
            claims: vec![Claim::new(
                ClaimKind::FilesChanged,
                "changed 2 files",
                0.9,
                ConfidenceSource::Deterministic,
            )],
            evidence: vec![Evidence::new(
                EvidenceKind::FileExists,
                "both exist",
                EvidencePolarity::Supports,
                1.0,
            )],
            contradictions: vec![],
            risk_class: RiskClass::FileSystem,
        };
        let v = vr.to_value();
        let restored = VerificationResult::from_value(&v).expect("should parse");
        assert_eq!(restored.status, VerificationStatus::Verified);
        assert_eq!(restored.claims.len(), 1);
        assert_eq!(restored.evidence.len(), 1);
        assert!(restored.contradictions.is_empty());
        assert_eq!(restored.risk_class, RiskClass::FileSystem);
    }

    #[test]
    fn empty_verification_result_to_value() {
        let vr = VerificationResult::pending();
        let v = vr.to_value();
        let restored = VerificationResult::from_value(&v).expect("should parse");
        assert!(restored.is_pending());
        assert!(restored.claims.is_empty());
    }

    // -- extract_implicit_claims --

    #[test]
    fn extract_claims_from_agent_result_fields() {
        let mut fields = HashMap::new();
        fields.insert(
            "plan".to_string(),
            ConfidentValue::from_skill(Value::Text("fix the bug".into()), 0.85),
        );
        fields.insert(
            "files_changed".to_string(),
            ConfidentValue::from_skill(
                Value::Array(vec![
                    ConfidentValue::deterministic(Value::Text("src/main.rs".into())),
                    ConfidentValue::deterministic(Value::Text("src/lib.rs".into())),
                ]),
                0.85,
            ),
        );
        fields.insert(
            "tests_run".to_string(),
            ConfidentValue::deterministic(Value::Number(5.0)),
        );
        fields.insert(
            "tests_passed".to_string(),
            ConfidentValue::deterministic(Value::Number(5.0)),
        );

        let claims = extract_implicit_claims(&fields);
        assert_eq!(claims.len(), 3);

        let kinds: Vec<ClaimKind> = claims.iter().map(|c| c.kind).collect();
        assert!(kinds.contains(&ClaimKind::TaskInterpretation));
        assert!(kinds.contains(&ClaimKind::FilesChanged));
        assert!(kinds.contains(&ClaimKind::TestsPass));
    }

    #[test]
    fn extract_claims_empty_fields_yields_nothing() {
        let fields = ConfidentValue::default_agent_result_fields();
        let claims = extract_implicit_claims(&fields);
        assert!(claims.is_empty());
    }

    #[test]
    fn extract_claims_partial_test_pass() {
        let mut fields = HashMap::new();
        fields.insert(
            "tests_run".to_string(),
            ConfidentValue::deterministic(Value::Number(10.0)),
        );
        fields.insert(
            "tests_passed".to_string(),
            ConfidentValue::deterministic(Value::Number(7.0)),
        );

        let claims = extract_implicit_claims(&fields);
        assert_eq!(claims.len(), 1);
        assert_eq!(claims[0].kind, ClaimKind::TestsPass);
        assert!(claims[0].description.contains("7/10"));
    }

    // -- inject_pending_verification --

    #[test]
    fn inject_pending_verification_populates_metadata() {
        let mut meta = HashMap::new();
        let claims = vec![Claim::new(
            ClaimKind::TaskComplete,
            "done",
            0.9,
            ConfidenceSource::Deterministic,
        )];
        inject_pending_verification(&mut meta, claims);

        assert!(meta.contains_key("verification"));
        let vr = VerificationResult::from_value(&meta["verification"].value).expect("should parse");
        assert!(vr.is_pending());
        assert_eq!(vr.claims.len(), 1);
        assert_eq!(vr.claims[0].kind, ClaimKind::TaskComplete);
    }
}
