// FORGE skill types — issue #40
// Core types for the host skill bridge.

use crate::types::CapabilitySignature;

#[derive(Debug, Clone)]
pub struct SkillCapability {
    pub name: String,
    pub signature: CapabilitySignature,
    pub executor: Option<SkillCapabilityExecutor>,
}

#[derive(Debug, Clone)]
pub struct SkillCapabilityExecutor {
    pub params: Vec<String>,
    pub kind: SkillExecutorKind,
    pub argv: Vec<String>,
    pub result: Option<SkillExecutorResult>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillExecutorKind {
    Command,
}

#[derive(Debug, Clone)]
pub struct SkillExecutorResult {
    pub json_path: Option<String>,
    pub success_path: Option<String>,
    pub error_path: Option<String>,
}

/// Metadata about a skill, loaded from SKILL.md frontmatter or config.
#[derive(Debug, Clone)]
pub struct SkillManifest {
    /// Namespace name: "slack"
    pub name: String,
    /// Human-readable description
    pub description: String,
    /// Explicit typed capabilities exposed by this skill.
    pub capabilities: Vec<SkillCapability>,
    /// Legacy single-capability compatibility surface for skills without explicit capabilities.
    pub legacy_signature: Option<CapabilitySignature>,
    /// Default confidence for results (capped at 0.99)
    pub default_confidence: f32,
    /// Timeout in seconds for skill execution
    pub timeout_secs: u64,
    /// Tools the skill is allowed to use during LLM-mediated execution
    pub allowed_tools: Vec<String>,
}

/// A skill loaded from a SKILL.md file.
#[derive(Debug, Clone)]
pub struct LoadedSkill {
    pub manifest: SkillManifest,
    /// The markdown instruction body (L2 content)
    pub instructions: String,
    /// Path to the SKILL.md file
    pub path: std::path::PathBuf,
}

/// Error from skill execution.
#[derive(Debug, thiserror::Error)]
pub enum SkillError {
    #[error("skill not found: {name}")]
    NotFound { name: String },

    #[error("skill method not found: {skill}.{method}")]
    UnknownMethod { skill: String, method: String },

    #[error("skill execution failed: {name}: {reason}")]
    ExecutionFailed { name: String, reason: String },

    #[error("skill timed out after {timeout_secs}s: {name}")]
    Timeout { name: String, timeout_secs: u64 },

    #[error("skill agentic loop exceeded {turns} turns: {name}")]
    MaxTurnsExceeded { name: String, turns: usize },

    #[error("unknown tool requested by LLM: {name}")]
    UnknownTool { name: String },

    #[error("LLM provider error during skill execution: {0}")]
    ProviderError(String),
}
