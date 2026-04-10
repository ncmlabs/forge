// FORGE verification engine — issue #204
//
// Runtime verification engine that validates agent claims against the real
// environment. Takes the pending VerificationResult from #203 (with extracted
// claims) and resolves it to Verified/Insufficient/Contradicted/Error by
// running a sequence of validator stages.
//
// Validator stages (paper Section 7.2):
//   1. Schema   — AgentResult structural checks
//   2. Reference — claimed files/symbols exist on disk
//   3. Environment — working directory is valid git state
//   4. Execution — tests actually pass (cargo test)
//   5. Policy   — risk class vs verification state
//
// #205 (contradiction events) will emit results to wardens.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;

use crate::runtime::confidence::{ConfidentValue, Value};
use crate::runtime::verification::*;

// ── Context & Result types ───────────────────────────────────

/// Context passed to each validator stage.
pub struct VerificationContext {
    /// Sandbox working directory (from SessionConfig).
    pub working_dir: Option<PathBuf>,
    /// The AgentResult top-level fields (plan, files_changed, etc.).
    pub agent_fields: HashMap<String, ConfidentValue>,
    /// The claims extracted during parse_final().
    pub claims: Vec<Claim>,
}

/// Result from a single validator stage.
pub struct StageResult {
    pub evidence: Vec<Evidence>,
    pub contradictions: Vec<Contradiction>,
    /// If true, the engine stops running further stages.
    pub fatal: bool,
}

impl StageResult {
    pub fn empty() -> Self {
        Self {
            evidence: Vec::new(),
            contradictions: Vec::new(),
            fatal: false,
        }
    }
}

// ── Validator trait ──────────────────────────────────────────

#[async_trait]
pub trait Validator: Send + Sync {
    /// Human-readable name for tracing.
    fn name(&self) -> &str;

    /// Run this validation stage.
    async fn validate(&self, ctx: &VerificationContext) -> StageResult;
}

// ── VerificationEngine ──────────────────────────────────────

pub struct VerificationEngine {
    validators: Vec<Box<dyn Validator>>,
}

impl Default for VerificationEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl VerificationEngine {
    pub fn new() -> Self {
        Self {
            validators: Vec::new(),
        }
    }

    /// Build the default coding-session engine with all 5 stages.
    pub fn coding_session() -> Self {
        let mut engine = Self::new();
        engine.add(Box::new(SchemaValidator));
        engine.add(Box::new(ReferenceValidator));
        engine.add(Box::new(EnvironmentValidator));
        engine.add(Box::new(ExecutionValidator::default()));
        engine.add(Box::new(PolicyValidator));
        engine
    }

    pub fn add(&mut self, validator: Box<dyn Validator>) {
        self.validators.push(validator);
    }

    /// Run all validators in sequence, aggregate evidence/contradictions,
    /// and resolve final VerificationStatus.
    pub async fn verify(&self, ctx: &VerificationContext) -> VerificationResult {
        let mut all_evidence: Vec<Evidence> = Vec::new();
        let mut all_contradictions: Vec<Contradiction> = Vec::new();
        let mut had_error = false;

        for validator in &self.validators {
            let stage = validator.validate(ctx).await;
            all_evidence.extend(stage.evidence);
            all_contradictions.extend(stage.contradictions);
            if stage.fatal {
                had_error = true;
                break;
            }
        }

        let risk_class = classify_risk(&ctx.agent_fields);
        let status = resolve_status(&ctx.claims, &all_evidence, &all_contradictions, had_error);

        VerificationResult {
            status,
            claims: ctx.claims.clone(),
            evidence: all_evidence,
            contradictions: all_contradictions,
            risk_class,
        }
    }
}

// ── Status resolution ───────────────────────────────────────

/// Determine the final VerificationStatus from claims, evidence, and contradictions.
fn resolve_status(
    claims: &[Claim],
    evidence: &[Evidence],
    contradictions: &[Contradiction],
    had_error: bool,
) -> VerificationStatus {
    if had_error {
        return VerificationStatus::Error;
    }
    if !contradictions.is_empty() {
        return VerificationStatus::Contradicted;
    }
    if claims.is_empty() {
        // No claims to verify — nothing to contradict but nothing confirmed either.
        return VerificationStatus::Insufficient;
    }
    // Check that every claim has at least one supporting evidence.
    let has_support = |claim: &Claim| {
        evidence
            .iter()
            .any(|e| e.polarity == EvidencePolarity::Supports && evidence_matches_claim(e, claim))
    };
    if claims.iter().all(has_support) {
        VerificationStatus::Verified
    } else {
        VerificationStatus::Insufficient
    }
}

/// Check whether a piece of evidence is relevant to a specific claim.
fn evidence_matches_claim(evidence: &Evidence, claim: &Claim) -> bool {
    match (evidence.kind, claim.kind) {
        (EvidenceKind::SchemaValidation, _) => true, // schema evidence supports all claims
        (EvidenceKind::FileExists, ClaimKind::FilesChanged) => true,
        (EvidenceKind::SymbolExists, ClaimKind::SymbolReference) => true,
        (EvidenceKind::TestResult, ClaimKind::TestsPass) => true,
        (EvidenceKind::PolicyCheck, _) => false, // policy evidence doesn't support claims directly
        (EvidenceKind::AgentAssessment, ClaimKind::TaskInterpretation) => true,
        (EvidenceKind::AgentAssessment, ClaimKind::TaskComplete) => true,
        _ => false,
    }
}

// ── Risk classification ─────────────────────────────────────

/// Classify the risk level of an agent result from its fields.
pub fn classify_risk(fields: &HashMap<String, ConfidentValue>) -> RiskClass {
    // Check metadata for side-effect markers.
    if let Some(cv) = fields.get("metadata") {
        if let Value::Record(meta) = &cv.value {
            // If metadata explicitly sets a risk class, use it.
            if let Some(rc_cv) = meta.get("risk_class") {
                if let Some(rc) = RiskClass::from_value(&rc_cv.value) {
                    return rc;
                }
            }
            // Check for side-effect indicators.
            for key in &["git_push", "pr_create", "deploy", "merge"] {
                if meta.contains_key(*key) {
                    return RiskClass::ExternalSideEffect;
                }
            }
        }
    }

    // files_changed non-empty → at minimum FileSystem.
    if let Some(cv) = fields.get("files_changed") {
        if let Value::Array(items) = &cv.value {
            if !items.is_empty() {
                return RiskClass::FileSystem;
            }
        }
    }

    // State mutations (memory, knowledge writes).
    if let Some(cv) = fields.get("metadata") {
        if let Value::Record(meta) = &cv.value {
            for key in &["memory_write", "knowledge_write", "state_mutation"] {
                if meta.contains_key(*key) {
                    return RiskClass::StateMutation;
                }
            }
        }
    }

    RiskClass::Informational
}

// ── SchemaValidator ─────────────────────────────────────────

/// Checks AgentResult has expected structural fields.
pub struct SchemaValidator;

#[async_trait]
impl Validator for SchemaValidator {
    fn name(&self) -> &str {
        "schema"
    }

    async fn validate(&self, ctx: &VerificationContext) -> StageResult {
        let mut evidence = Vec::new();
        let mut contradictions = Vec::new();

        // Check plan field exists and is non-empty text.
        let has_plan = ctx
            .agent_fields
            .get("plan")
            .map(|cv| matches!(&cv.value, Value::Text(s) if !s.is_empty()))
            .unwrap_or(false);

        if has_plan {
            evidence.push(Evidence::new(
                EvidenceKind::SchemaValidation,
                "AgentResult contains a non-empty plan",
                EvidencePolarity::Supports,
                1.0,
            ));
        }

        // Check confidence field exists.
        let has_confidence = ctx
            .agent_fields
            .get("confidence")
            .map(|cv| matches!(&cv.value, Value::Number(_)))
            .unwrap_or(false);

        if has_confidence {
            evidence.push(Evidence::new(
                EvidenceKind::SchemaValidation,
                "AgentResult contains a confidence score",
                EvidencePolarity::Supports,
                1.0,
            ));
        }

        // If there are FilesChanged claims, files_changed field must be an array.
        let has_file_claims = ctx.claims.iter().any(|c| c.kind == ClaimKind::FilesChanged);
        if has_file_claims {
            let has_files_array = ctx
                .agent_fields
                .get("files_changed")
                .map(|cv| matches!(&cv.value, Value::Array(_)))
                .unwrap_or(false);

            if has_files_array {
                evidence.push(Evidence::new(
                    EvidenceKind::SchemaValidation,
                    "files_changed field is a valid array",
                    EvidencePolarity::Supports,
                    1.0,
                ));
            } else {
                let e = Evidence::new(
                    EvidenceKind::SchemaValidation,
                    "FilesChanged claim exists but files_changed field is missing or invalid",
                    EvidencePolarity::Contradicts,
                    1.0,
                );
                if let Some(claim) = ctx
                    .claims
                    .iter()
                    .find(|c| c.kind == ClaimKind::FilesChanged)
                {
                    contradictions.push(Contradiction::new(
                        claim.clone(),
                        e.clone(),
                        ContradictionSeverity::Medium,
                    ));
                }
                evidence.push(e);
            }
        }

        StageResult {
            evidence,
            contradictions,
            fatal: false,
        }
    }
}

// ── ReferenceValidator ──────────────────────────────────────

/// Checks claimed files exist on disk in the working directory.
pub struct ReferenceValidator;

#[async_trait]
impl Validator for ReferenceValidator {
    fn name(&self) -> &str {
        "reference"
    }

    async fn validate(&self, ctx: &VerificationContext) -> StageResult {
        let working_dir = match &ctx.working_dir {
            Some(d) => d,
            None => {
                return StageResult {
                    evidence: vec![Evidence::new(
                        EvidenceKind::FileExists,
                        "No working directory configured; skipping file reference checks",
                        EvidencePolarity::Neutral,
                        0.0,
                    )],
                    contradictions: Vec::new(),
                    fatal: false,
                };
            }
        };

        let mut evidence = Vec::new();
        let mut contradictions = Vec::new();

        // Check each file in files_changed.
        if let Some(cv) = ctx.agent_fields.get("files_changed") {
            if let Value::Array(items) = &cv.value {
                for item in items {
                    if let Value::Text(path_str) = &item.value {
                        let full_path = working_dir.join(path_str);
                        if full_path.exists() {
                            evidence.push(Evidence::new(
                                EvidenceKind::FileExists,
                                format!("File exists: {path_str}"),
                                EvidencePolarity::Supports,
                                1.0,
                            ));
                        } else {
                            let e = Evidence::new(
                                EvidenceKind::FileExists,
                                format!("File does not exist: {path_str}"),
                                EvidencePolarity::Contradicts,
                                1.0,
                            );
                            if let Some(claim) = ctx
                                .claims
                                .iter()
                                .find(|c| c.kind == ClaimKind::FilesChanged)
                            {
                                contradictions.push(Contradiction::new(
                                    claim.clone(),
                                    e.clone(),
                                    ContradictionSeverity::High,
                                ));
                            }
                            evidence.push(e);
                        }
                    }
                }
            }
        }

        StageResult {
            evidence,
            contradictions,
            fatal: false,
        }
    }
}

// ── EnvironmentValidator ────────────────────────────────────

/// Checks working directory is a valid git repository.
pub struct EnvironmentValidator;

#[async_trait]
impl Validator for EnvironmentValidator {
    fn name(&self) -> &str {
        "environment"
    }

    async fn validate(&self, ctx: &VerificationContext) -> StageResult {
        let working_dir = match &ctx.working_dir {
            Some(d) => d,
            None => {
                return StageResult {
                    evidence: vec![Evidence::new(
                        EvidenceKind::Other,
                        "No working directory configured; skipping environment checks",
                        EvidencePolarity::Neutral,
                        0.0,
                    )],
                    contradictions: Vec::new(),
                    fatal: false,
                };
            }
        };

        let mut evidence = Vec::new();

        // Check directory exists.
        if working_dir.is_dir() {
            evidence.push(Evidence::new(
                EvidenceKind::Other,
                "Working directory exists",
                EvidencePolarity::Supports,
                1.0,
            ));
        } else {
            evidence.push(Evidence::new(
                EvidenceKind::Other,
                format!(
                    "Working directory does not exist: {}",
                    working_dir.display()
                ),
                EvidencePolarity::Contradicts,
                1.0,
            ));
            return StageResult {
                evidence,
                contradictions: Vec::new(),
                fatal: true,
            };
        }

        // Check for git repository (regular .git dir or worktree marker file).
        let git_path = working_dir.join(".git");
        if git_path.exists() {
            evidence.push(Evidence::new(
                EvidenceKind::Other,
                "Working directory is a git repository",
                EvidencePolarity::Supports,
                1.0,
            ));
        } else {
            evidence.push(Evidence::new(
                EvidenceKind::Other,
                "Working directory is not a git repository",
                EvidencePolarity::Neutral,
                0.5,
            ));
        }

        StageResult {
            evidence,
            contradictions: Vec::new(),
            fatal: false,
        }
    }
}

// ── ExecutionValidator ──────────────────────────────────────

/// Runs tests if TestsPass claims exist.
pub struct ExecutionValidator {
    pub timeout: Duration,
}

impl Default for ExecutionValidator {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(120),
        }
    }
}

#[async_trait]
impl Validator for ExecutionValidator {
    fn name(&self) -> &str {
        "execution"
    }

    async fn validate(&self, ctx: &VerificationContext) -> StageResult {
        // Only run if there is a TestsPass claim.
        let test_claim = ctx.claims.iter().find(|c| c.kind == ClaimKind::TestsPass);
        let test_claim = match test_claim {
            Some(c) => c,
            None => return StageResult::empty(),
        };

        let working_dir = match &ctx.working_dir {
            Some(d) if d.is_dir() => d,
            _ => {
                return StageResult {
                    evidence: vec![Evidence::new(
                        EvidenceKind::TestResult,
                        "No valid working directory; cannot run tests",
                        EvidencePolarity::Neutral,
                        0.0,
                    )],
                    contradictions: Vec::new(),
                    fatal: false,
                };
            }
        };

        // Detect test command.
        let (cmd, args) = if working_dir.join("Cargo.toml").exists() {
            ("cargo", vec!["test"])
        } else if working_dir.join("package.json").exists() {
            ("npm", vec!["test"])
        } else {
            return StageResult {
                evidence: vec![Evidence::new(
                    EvidenceKind::TestResult,
                    "No recognized test runner (Cargo.toml or package.json) found",
                    EvidencePolarity::Neutral,
                    0.0,
                )],
                contradictions: Vec::new(),
                fatal: false,
            };
        };

        // Run tests with timeout.
        let result = tokio::time::timeout(
            self.timeout,
            tokio::process::Command::new(cmd)
                .args(&args)
                .current_dir(working_dir)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .output(),
        )
        .await;

        match result {
            Ok(Ok(output)) => {
                if output.status.success() {
                    StageResult {
                        evidence: vec![Evidence::new(
                            EvidenceKind::TestResult,
                            format!("{cmd} {}", args.join(" ")),
                            EvidencePolarity::Supports,
                            1.0,
                        )],
                        contradictions: Vec::new(),
                        fatal: false,
                    }
                } else {
                    let stderr = String::from_utf8_lossy(&output.stderr)
                        .chars()
                        .take(500)
                        .collect::<String>();
                    let e = Evidence::new(
                        EvidenceKind::TestResult,
                        format!(
                            "Tests failed (exit {}): {}",
                            output.status.code().unwrap_or(-1),
                            stderr.trim()
                        ),
                        EvidencePolarity::Contradicts,
                        1.0,
                    );
                    let contradiction = Contradiction::new(
                        test_claim.clone(),
                        e.clone(),
                        ContradictionSeverity::High,
                    );
                    StageResult {
                        evidence: vec![e],
                        contradictions: vec![contradiction],
                        fatal: false,
                    }
                }
            }
            Ok(Err(err)) => StageResult {
                evidence: vec![Evidence::new(
                    EvidenceKind::TestResult,
                    format!("Failed to spawn test process: {err}"),
                    EvidencePolarity::Contradicts,
                    1.0,
                )],
                contradictions: Vec::new(),
                fatal: true,
            },
            Err(_) => StageResult {
                evidence: vec![Evidence::new(
                    EvidenceKind::TestResult,
                    format!("Test execution timed out after {}s", self.timeout.as_secs()),
                    EvidencePolarity::Contradicts,
                    1.0,
                )],
                contradictions: Vec::new(),
                fatal: true,
            },
        }
    }
}

// ── PolicyValidator ─────────────────────────────────────────

/// Checks risk class against accumulated verification state.
pub struct PolicyValidator;

#[async_trait]
impl Validator for PolicyValidator {
    fn name(&self) -> &str {
        "policy"
    }

    async fn validate(&self, ctx: &VerificationContext) -> StageResult {
        let risk = classify_risk(&ctx.agent_fields);

        match risk {
            RiskClass::ExternalSideEffect => {
                // External side effects require explicit verification — flag as
                // needing attention. The actual blocking happens in downstream
                // consumers via is_actionable().
                StageResult {
                    evidence: vec![Evidence::new(
                        EvidenceKind::PolicyCheck,
                        "Risk class is ExternalSideEffect; requires verified preconditions",
                        EvidencePolarity::Neutral,
                        1.0,
                    )],
                    contradictions: Vec::new(),
                    fatal: false,
                }
            }
            RiskClass::FileSystem => StageResult {
                evidence: vec![Evidence::new(
                    EvidenceKind::PolicyCheck,
                    "Risk class is FileSystem; standard verification sufficient",
                    EvidencePolarity::Supports,
                    1.0,
                )],
                contradictions: Vec::new(),
                fatal: false,
            },
            _ => StageResult {
                evidence: vec![Evidence::new(
                    EvidenceKind::PolicyCheck,
                    format!("Risk class is {}; no elevated policy gate", risk.as_str()),
                    EvidencePolarity::Supports,
                    1.0,
                )],
                contradictions: Vec::new(),
                fatal: false,
            },
        }
    }
}

// ── Helpers for SessionManager integration ──────────────────

/// Extract verification inputs from a completed AgentResult ConfidentValue.
/// Returns the agent fields and the claims from the pending VerificationResult.
pub fn extract_verification_inputs(
    result: &ConfidentValue,
) -> (HashMap<String, ConfidentValue>, Vec<Claim>) {
    let fields = match &result.value {
        Value::Record(f) => f.clone(),
        _ => return (HashMap::new(), Vec::new()),
    };

    // Extract claims from the pending verification result in metadata.
    let claims = fields
        .get("metadata")
        .and_then(|cv| match &cv.value {
            Value::Record(meta) => meta.get("verification"),
            _ => None,
        })
        .and_then(|cv| VerificationResult::from_value(&cv.value))
        .map(|vr| vr.claims)
        .unwrap_or_default();

    (fields, claims)
}

/// Inject a resolved VerificationResult back into the AgentResult ConfidentValue.
pub fn inject_resolved_verification(
    mut result: ConfidentValue,
    vr: VerificationResult,
) -> ConfidentValue {
    if let Value::Record(ref mut fields) = result.value {
        if let Some(meta_cv) = fields.get_mut("metadata") {
            if let Value::Record(ref mut meta) = meta_cv.value {
                meta.insert(
                    "verification".to_string(),
                    ConfidentValue::deterministic(vr.to_value()),
                );
                return result;
            }
        }
        // If metadata doesn't exist as a record, create it.
        let mut meta = HashMap::new();
        meta.insert(
            "verification".to_string(),
            ConfidentValue::deterministic(vr.to_value()),
        );
        fields.insert(
            "metadata".to_string(),
            ConfidentValue::deterministic(Value::Record(meta)),
        );
    }
    result
}

// ── Tests ───────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ConfidenceSource;
    use tempfile::TempDir;

    fn make_context(
        working_dir: Option<PathBuf>,
        fields: HashMap<String, ConfidentValue>,
        claims: Vec<Claim>,
    ) -> VerificationContext {
        VerificationContext {
            working_dir,
            agent_fields: fields,
            claims,
        }
    }

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

    // ── resolve_status ──────────────────────────────────────

    #[test]
    fn resolve_status_error_on_fatal() {
        let status = resolve_status(&[], &[], &[], true);
        assert_eq!(status, VerificationStatus::Error);
    }

    #[test]
    fn resolve_status_contradicted_on_contradictions() {
        let claim = make_claim(ClaimKind::FilesChanged, "changed files");
        let evidence = Evidence::new(
            EvidenceKind::FileExists,
            "file missing",
            EvidencePolarity::Contradicts,
            1.0,
        );
        let contradiction =
            Contradiction::new(claim.clone(), evidence, ContradictionSeverity::High);
        let status = resolve_status(&[claim], &[], &[contradiction], false);
        assert_eq!(status, VerificationStatus::Contradicted);
    }

    #[test]
    fn resolve_status_insufficient_no_claims() {
        let status = resolve_status(&[], &[], &[], false);
        assert_eq!(status, VerificationStatus::Insufficient);
    }

    #[test]
    fn resolve_status_verified_all_supported() {
        let claim = make_claim(ClaimKind::FilesChanged, "changed src/main.rs");
        let evidence = Evidence::new(
            EvidenceKind::FileExists,
            "file exists",
            EvidencePolarity::Supports,
            1.0,
        );
        let status = resolve_status(&[claim], &[evidence], &[], false);
        assert_eq!(status, VerificationStatus::Verified);
    }

    #[test]
    fn resolve_status_insufficient_unsupported_claim() {
        let claim = make_claim(ClaimKind::TestsPass, "all tests pass");
        // No TestResult evidence, only FileExists.
        let evidence = Evidence::new(
            EvidenceKind::FileExists,
            "file exists",
            EvidencePolarity::Supports,
            1.0,
        );
        let status = resolve_status(&[claim], &[evidence], &[], false);
        assert_eq!(status, VerificationStatus::Insufficient);
    }

    // ── classify_risk ───────────────────────────────────────

    #[test]
    fn classify_risk_informational_empty() {
        let fields = HashMap::new();
        assert_eq!(classify_risk(&fields), RiskClass::Informational);
    }

    #[test]
    fn classify_risk_filesystem_with_files() {
        let mut fields = HashMap::new();
        fields.insert("files_changed".to_string(), array_cv(vec!["src/main.rs"]));
        assert_eq!(classify_risk(&fields), RiskClass::FileSystem);
    }

    #[test]
    fn classify_risk_external_with_metadata() {
        let mut fields = HashMap::new();
        let mut meta = HashMap::new();
        meta.insert(
            "git_push".to_string(),
            ConfidentValue::deterministic(Value::Bool(true)),
        );
        fields.insert(
            "metadata".to_string(),
            ConfidentValue::deterministic(Value::Record(meta)),
        );
        assert_eq!(classify_risk(&fields), RiskClass::ExternalSideEffect);
    }

    #[test]
    fn classify_risk_explicit_from_metadata() {
        let mut fields = HashMap::new();
        let mut meta = HashMap::new();
        meta.insert(
            "risk_class".to_string(),
            ConfidentValue::deterministic(Value::Text("state_mutation".to_string())),
        );
        fields.insert(
            "metadata".to_string(),
            ConfidentValue::deterministic(Value::Record(meta)),
        );
        assert_eq!(classify_risk(&fields), RiskClass::StateMutation);
    }

    // ── SchemaValidator ─────────────────────────────────────

    #[tokio::test]
    async fn schema_validator_complete_fields() {
        let mut fields = HashMap::new();
        fields.insert("plan".to_string(), text_cv("implement feature X"));
        fields.insert("confidence".to_string(), num_cv(0.9));
        fields.insert("files_changed".to_string(), array_cv(vec!["src/main.rs"]));
        let claims = vec![make_claim(ClaimKind::FilesChanged, "files changed")];
        let ctx = make_context(None, fields, claims);

        let result = SchemaValidator.validate(&ctx).await;
        assert!(result.contradictions.is_empty());
        assert_eq!(result.evidence.len(), 3); // plan + confidence + files_array
        assert!(result
            .evidence
            .iter()
            .all(|e| e.polarity == EvidencePolarity::Supports));
    }

    #[tokio::test]
    async fn schema_validator_missing_files_array_with_claim() {
        let fields = HashMap::new();
        let claims = vec![make_claim(ClaimKind::FilesChanged, "files changed")];
        let ctx = make_context(None, fields, claims);

        let result = SchemaValidator.validate(&ctx).await;
        assert_eq!(result.contradictions.len(), 1);
    }

    // ── ReferenceValidator ──────────────────────────────────

    #[tokio::test]
    async fn reference_validator_files_exist() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("foo.rs"), "fn main() {}").unwrap();

        let mut fields = HashMap::new();
        fields.insert("files_changed".to_string(), array_cv(vec!["foo.rs"]));
        let claims = vec![make_claim(ClaimKind::FilesChanged, "changed foo.rs")];
        let ctx = make_context(Some(dir.path().to_path_buf()), fields, claims);

        let result = ReferenceValidator.validate(&ctx).await;
        assert!(result.contradictions.is_empty());
        assert_eq!(result.evidence.len(), 1);
        assert_eq!(result.evidence[0].polarity, EvidencePolarity::Supports);
    }

    #[tokio::test]
    async fn reference_validator_file_missing() {
        let dir = TempDir::new().unwrap();
        let mut fields = HashMap::new();
        fields.insert(
            "files_changed".to_string(),
            array_cv(vec!["nonexistent.rs"]),
        );
        let claims = vec![make_claim(
            ClaimKind::FilesChanged,
            "changed nonexistent.rs",
        )];
        let ctx = make_context(Some(dir.path().to_path_buf()), fields, claims);

        let result = ReferenceValidator.validate(&ctx).await;
        assert_eq!(result.contradictions.len(), 1);
        assert_eq!(
            result.contradictions[0].severity,
            ContradictionSeverity::High
        );
    }

    #[tokio::test]
    async fn reference_validator_no_working_dir() {
        let fields = HashMap::new();
        let ctx = make_context(None, fields, Vec::new());

        let result = ReferenceValidator.validate(&ctx).await;
        assert_eq!(result.evidence.len(), 1);
        assert_eq!(result.evidence[0].polarity, EvidencePolarity::Neutral);
    }

    // ── EnvironmentValidator ────────────────────────────────

    #[tokio::test]
    async fn environment_validator_valid_git_dir() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir(dir.path().join(".git")).unwrap();

        let ctx = make_context(Some(dir.path().to_path_buf()), HashMap::new(), Vec::new());
        let result = EnvironmentValidator.validate(&ctx).await;

        assert!(result
            .evidence
            .iter()
            .any(|e| e.description.contains("git repository")));
        assert!(!result.fatal);
    }

    #[tokio::test]
    async fn environment_validator_nonexistent_dir() {
        let ctx = make_context(
            Some(PathBuf::from("/nonexistent/path/xyz")),
            HashMap::new(),
            Vec::new(),
        );
        let result = EnvironmentValidator.validate(&ctx).await;
        assert!(result.fatal);
    }

    #[tokio::test]
    async fn environment_validator_no_working_dir() {
        let ctx = make_context(None, HashMap::new(), Vec::new());
        let result = EnvironmentValidator.validate(&ctx).await;
        assert_eq!(result.evidence[0].polarity, EvidencePolarity::Neutral);
    }

    // ── ExecutionValidator ───────────────────────────────────

    #[tokio::test]
    async fn execution_validator_skips_without_test_claim() {
        let dir = TempDir::new().unwrap();
        let ctx = make_context(Some(dir.path().to_path_buf()), HashMap::new(), Vec::new());
        let result = ExecutionValidator::default().validate(&ctx).await;
        assert!(result.evidence.is_empty());
    }

    #[tokio::test]
    async fn execution_validator_skips_no_test_runner() {
        let dir = TempDir::new().unwrap();
        let claims = vec![make_claim(ClaimKind::TestsPass, "all tests pass")];
        let ctx = make_context(Some(dir.path().to_path_buf()), HashMap::new(), claims);
        let result = ExecutionValidator::default().validate(&ctx).await;
        assert_eq!(result.evidence.len(), 1);
        assert_eq!(result.evidence[0].polarity, EvidencePolarity::Neutral);
    }

    // ── PolicyValidator ─────────────────────────────────────

    #[tokio::test]
    async fn policy_validator_informational() {
        let ctx = make_context(None, HashMap::new(), Vec::new());
        let result = PolicyValidator.validate(&ctx).await;
        assert_eq!(result.evidence.len(), 1);
        assert_eq!(result.evidence[0].polarity, EvidencePolarity::Supports);
    }

    #[tokio::test]
    async fn policy_validator_external_side_effect() {
        let mut fields = HashMap::new();
        let mut meta = HashMap::new();
        meta.insert(
            "git_push".to_string(),
            ConfidentValue::deterministic(Value::Bool(true)),
        );
        fields.insert(
            "metadata".to_string(),
            ConfidentValue::deterministic(Value::Record(meta)),
        );

        let ctx = make_context(None, fields, Vec::new());
        let result = PolicyValidator.validate(&ctx).await;
        assert_eq!(result.evidence[0].polarity, EvidencePolarity::Neutral);
    }

    // ── Full engine pipeline ────────────────────────────────

    #[tokio::test]
    async fn engine_verified_with_existing_files() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("src.rs"), "fn main() {}").unwrap();
        std::fs::create_dir(dir.path().join(".git")).unwrap();

        let mut fields = HashMap::new();
        fields.insert("plan".to_string(), text_cv("implement feature"));
        fields.insert("confidence".to_string(), num_cv(0.9));
        fields.insert("files_changed".to_string(), array_cv(vec!["src.rs"]));

        let claims = vec![
            make_claim(ClaimKind::TaskInterpretation, "interpreted task"),
            make_claim(ClaimKind::FilesChanged, "changed src.rs"),
        ];

        // Use only schema + reference + environment + policy (skip execution — no Cargo.toml).
        let mut engine = VerificationEngine::new();
        engine.add(Box::new(SchemaValidator));
        engine.add(Box::new(ReferenceValidator));
        engine.add(Box::new(EnvironmentValidator));
        engine.add(Box::new(PolicyValidator));

        let ctx = make_context(Some(dir.path().to_path_buf()), fields, claims);
        let result = engine.verify(&ctx).await;

        assert_eq!(result.status, VerificationStatus::Verified);
        assert!(result.contradictions.is_empty());
        assert_eq!(result.risk_class, RiskClass::FileSystem);
    }

    #[tokio::test]
    async fn engine_contradicted_with_missing_files() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir(dir.path().join(".git")).unwrap();

        let mut fields = HashMap::new();
        fields.insert("plan".to_string(), text_cv("implement feature"));
        fields.insert("confidence".to_string(), num_cv(0.9));
        fields.insert("files_changed".to_string(), array_cv(vec!["missing.rs"]));

        let claims = vec![make_claim(ClaimKind::FilesChanged, "changed missing.rs")];

        let mut engine = VerificationEngine::new();
        engine.add(Box::new(SchemaValidator));
        engine.add(Box::new(ReferenceValidator));

        let ctx = make_context(Some(dir.path().to_path_buf()), fields, claims);
        let result = engine.verify(&ctx).await;

        assert_eq!(result.status, VerificationStatus::Contradicted);
        assert!(!result.contradictions.is_empty());
    }

    #[tokio::test]
    async fn engine_insufficient_no_claims() {
        let dir = TempDir::new().unwrap();
        let ctx = make_context(Some(dir.path().to_path_buf()), HashMap::new(), Vec::new());

        let engine = VerificationEngine::coding_session();
        let result = engine.verify(&ctx).await;
        assert_eq!(result.status, VerificationStatus::Insufficient);
    }

    // ── extract/inject helpers ──────────────────────────────

    #[test]
    fn extract_and_inject_round_trip() {
        let mut fields = ConfidentValue::default_agent_result_fields();
        fields.insert("plan".to_string(), text_cv("my plan"));

        let claims = vec![make_claim(ClaimKind::TaskInterpretation, "interpreted")];
        let mut meta = HashMap::new();
        let mut pending = VerificationResult::pending();
        pending.claims = claims.clone();
        meta.insert(
            "verification".to_string(),
            ConfidentValue::deterministic(pending.to_value()),
        );
        fields.insert(
            "metadata".to_string(),
            ConfidentValue::deterministic(Value::Record(meta)),
        );

        let result = ConfidentValue::deterministic(Value::Record(fields));
        let (extracted_fields, extracted_claims) = extract_verification_inputs(&result);

        assert!(!extracted_fields.is_empty());
        assert_eq!(extracted_claims.len(), 1);
        assert_eq!(extracted_claims[0].kind, ClaimKind::TaskInterpretation);

        // Inject a resolved result.
        let resolved = VerificationResult {
            status: VerificationStatus::Verified,
            claims: extracted_claims,
            evidence: vec![Evidence::new(
                EvidenceKind::SchemaValidation,
                "ok",
                EvidencePolarity::Supports,
                1.0,
            )],
            contradictions: Vec::new(),
            risk_class: RiskClass::Informational,
        };

        let injected = inject_resolved_verification(result, resolved);
        let (_, new_claims) = extract_verification_inputs(&injected);
        // After injection the claims come from the resolved result.
        assert_eq!(new_claims.len(), 1);
    }
}
