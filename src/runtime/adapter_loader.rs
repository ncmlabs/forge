// FORGE adapter loader — parses ADAPTER.toml files into runtime configs (issue #191)
//
// Adapter definitions are declarative TOML that describe how to invoke an
// external CLI agent and map its output to AgentResult. This module loads
// and validates those definitions.

use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

// ── Adapter config types ─────────────────────────────────────

/// How the prompt is delivered to the CLI agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptDelivery {
    /// Pipe prompt to stdin.
    Stdin,
    /// Append prompt as the last positional argument.
    Positional,
    /// Pass prompt via a named flag.
    Flag(String),
}

/// What output format the CLI agent produces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutputFormat {
    /// Single JSON object on stdout.
    Json,
    /// Newline-delimited JSON events.
    Jsonl,
    /// Claude-style streaming JSON (JSONL with typed events).
    StreamJson,
    /// Plain text (fallback).
    Text,
}

/// Resume configuration.
#[derive(Debug, Clone)]
pub struct ResumeConfig {
    /// Flag that accepts the external session ID (e.g., "--resume").
    pub flag: Option<String>,
    /// Positional args prepended before the session ID (e.g., ["exec", "resume"]).
    pub args: Option<Vec<String>>,
}

/// Permission/sandbox mode configuration.
#[derive(Debug, Clone)]
pub struct PermissionConfig {
    /// Args appended when all tools are read-only.
    pub readonly_args: Vec<String>,
    /// Args appended when tools include write/exec capabilities.
    pub readwrite_args: Vec<String>,
    /// Tool names that trigger readwrite mode.
    pub readwrite_tools: Vec<String>,
}

/// Maps CLI output fields to AgentResult fields via dot-paths.
#[derive(Debug, Clone)]
pub struct ResultMappingConfig {
    /// Dot-path for AgentResult.plan field.
    pub plan: Option<String>,
    /// Dot-path for AgentResult.patch_summary field.
    pub patch_summary: Option<String>,
    /// Dot-path for AgentResult.files_changed field.
    pub files_changed: Option<String>,
    /// Dot-path for AgentResult.cost_usd field.
    pub cost_usd: Option<String>,
    /// Dot-path for the external session ID (for resume).
    pub session_id: Option<String>,
    /// Default confidence when not present in output.
    pub confidence_default: f32,
    /// Extra metadata field mappings (key = metadata field name, value = dot-path).
    pub metadata: HashMap<String, String>,
}

/// Streaming progress event detection configuration.
#[derive(Debug, Clone)]
pub struct ProgressConfig {
    /// JSON field that identifies the event type.
    pub event_field: String,
    /// Event type values that indicate progress.
    pub progress_types: Vec<String>,
    /// Event type value for the final result.
    pub result_type: Option<String>,
    /// Field to check for success status.
    pub success_field: Option<String>,
    /// Expected value of the success field.
    pub success_value: Option<String>,
}

/// Fully parsed adapter configuration, ready for the runtime.
#[derive(Debug, Clone)]
pub struct AdapterConfig {
    pub name: String,
    pub description: String,
    pub command: String,
    pub args: Vec<String>,
    pub prompt_delivery: PromptDelivery,
    pub output_format: OutputFormat,
    pub flags: HashMap<String, String>,
    pub resume: Option<ResumeConfig>,
    pub permissions: Option<PermissionConfig>,
    pub result_mapping: ResultMappingConfig,
    pub progress: Option<ProgressConfig>,
    pub path: PathBuf,
}

// ── TOML deserialization types ───────────────────────────────

#[derive(Debug, Deserialize)]
struct AdapterToml {
    adapter: AdapterSection,
}

#[derive(Debug, Deserialize)]
struct AdapterSection {
    name: String,
    #[serde(default)]
    description: Option<String>,
    command: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default = "default_prompt_delivery")]
    prompt_delivery: String,
    #[serde(default)]
    prompt_flag: Option<String>,
    #[serde(default = "default_output_format")]
    output_format: String,
    #[serde(default)]
    flags: HashMap<String, String>,
    #[serde(default)]
    resume: Option<ResumeSection>,
    #[serde(default)]
    permissions: Option<PermissionSection>,
    #[serde(default)]
    result_mapping: Option<ResultMappingSection>,
    #[serde(default)]
    progress: Option<ProgressSection>,
}

fn default_prompt_delivery() -> String {
    "stdin".to_string()
}
fn default_output_format() -> String {
    "text".to_string()
}

#[derive(Debug, Deserialize)]
struct ResumeSection {
    flag: Option<String>,
    args: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct PermissionSection {
    #[serde(default)]
    readonly_args: Vec<String>,
    #[serde(default)]
    readwrite_args: Vec<String>,
    #[serde(default)]
    readwrite_tools: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ResultMappingSection {
    plan: Option<String>,
    patch_summary: Option<String>,
    files_changed: Option<String>,
    cost_usd: Option<String>,
    session_id: Option<String>,
    #[serde(default = "default_confidence")]
    confidence_default: f32,
    #[serde(default)]
    metadata: HashMap<String, String>,
}

fn default_confidence() -> f32 {
    0.5
}

#[derive(Debug, Deserialize)]
struct ProgressSection {
    #[serde(default = "default_event_field")]
    event_field: String,
    #[serde(default)]
    progress_types: Vec<String>,
    result_type: Option<String>,
    success_field: Option<String>,
    success_value: Option<String>,
}

fn default_event_field() -> String {
    "type".to_string()
}

// ── Loading ──────────────────────────────────────────────────

/// Parse an ADAPTER.toml file into an AdapterConfig.
pub fn parse_adapter_toml(path: &Path) -> Result<AdapterConfig, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {}", path.display(), e))?;

    parse_adapter_toml_str(&content, path)
}

/// Parse ADAPTER.toml content from a string (useful for testing).
pub fn parse_adapter_toml_str(content: &str, path: &Path) -> Result<AdapterConfig, String> {
    let toml: AdapterToml = toml::from_str(content)
        .map_err(|e| format!("invalid ADAPTER.toml {}: {}", path.display(), e))?;

    let a = toml.adapter;

    let prompt_delivery = match a.prompt_delivery.as_str() {
        "stdin" => PromptDelivery::Stdin,
        "positional" => PromptDelivery::Positional,
        "flag" => {
            let flag = a.prompt_flag.ok_or_else(|| {
                format!(
                    "adapter '{}': prompt_delivery = \"flag\" requires prompt_flag",
                    a.name
                )
            })?;
            PromptDelivery::Flag(flag)
        }
        other => {
            return Err(format!(
                "adapter '{}': unknown prompt_delivery '{}' (expected stdin, positional, or flag)",
                a.name, other
            ));
        }
    };

    let output_format = match a.output_format.as_str() {
        "json" => OutputFormat::Json,
        "jsonl" => OutputFormat::Jsonl,
        "stream-json" => OutputFormat::StreamJson,
        "text" => OutputFormat::Text,
        other => {
            return Err(format!(
                "adapter '{}': unknown output_format '{}' (expected json, jsonl, stream-json, or text)",
                a.name, other
            ));
        }
    };

    let resume = a.resume.map(|r| ResumeConfig {
        flag: r.flag,
        args: r.args,
    });

    let permissions = a.permissions.map(|p| PermissionConfig {
        readonly_args: p.readonly_args,
        readwrite_args: p.readwrite_args,
        readwrite_tools: p.readwrite_tools,
    });

    let result_mapping = match a.result_mapping {
        Some(rm) => ResultMappingConfig {
            plan: rm.plan,
            patch_summary: rm.patch_summary,
            files_changed: rm.files_changed,
            cost_usd: rm.cost_usd,
            session_id: rm.session_id,
            confidence_default: rm.confidence_default,
            metadata: rm.metadata,
        },
        None => ResultMappingConfig {
            plan: None,
            patch_summary: None,
            files_changed: None,
            cost_usd: None,
            session_id: None,
            confidence_default: 0.5,
            metadata: HashMap::new(),
        },
    };

    let progress = a.progress.map(|p| ProgressConfig {
        event_field: p.event_field,
        progress_types: p.progress_types,
        result_type: p.result_type,
        success_field: p.success_field,
        success_value: p.success_value,
    });

    Ok(AdapterConfig {
        name: a.name,
        description: a.description.unwrap_or_default(),
        command: a.command,
        args: a.args,
        prompt_delivery,
        output_format,
        flags: a.flags,
        resume,
        permissions,
        result_mapping,
        progress,
        path: path.to_path_buf(),
    })
}

/// Load all adapters from resolved paths.
pub fn load_adapters(resolved: &[(String, PathBuf)]) -> Result<Vec<AdapterConfig>, String> {
    let mut configs = Vec::new();
    for (name, path) in resolved {
        let mut config = parse_adapter_toml(path)?;
        // The manifest key overrides the TOML name for consistency
        config.name = name.clone();
        configs.push(config);
    }
    Ok(configs)
}

/// Create a generic fallback adapter config for an unknown agent name.
pub fn generic_fallback_adapter(agent_name: &str) -> AdapterConfig {
    AdapterConfig {
        name: agent_name.to_string(),
        description: format!("Generic adapter for '{}'", agent_name),
        command: agent_name.to_string(),
        args: Vec::new(),
        prompt_delivery: PromptDelivery::Stdin,
        output_format: OutputFormat::Text,
        flags: HashMap::new(),
        resume: None,
        permissions: None,
        result_mapping: ResultMappingConfig {
            plan: None,
            patch_summary: None,
            files_changed: None,
            cost_usd: None,
            session_id: None,
            confidence_default: 0.5,
            metadata: HashMap::new(),
        },
        progress: None,
        path: PathBuf::new(),
    }
}

// ── JSON dot-path extraction ─────────────────────────────────

/// Extract a value from a JSON object using a dot-separated path.
///
/// Example: `extract_json_path(value, "usage.input_tokens")` walks
/// `value["usage"]["input_tokens"]`.
pub fn extract_json_path(value: &serde_json::Value, path: &str) -> Option<serde_json::Value> {
    let mut current = value;
    for segment in path.split('.') {
        current = current.get(segment)?;
    }
    Some(current.clone())
}

// ── Tests ────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_claude_adapter() {
        let content = include_str!("../../adapters/claude/ADAPTER.toml");
        let config =
            parse_adapter_toml_str(content, Path::new("adapters/claude/ADAPTER.toml")).unwrap();

        assert_eq!(config.name, "claude");
        assert_eq!(config.command, "claude");
        assert_eq!(config.prompt_delivery, PromptDelivery::Stdin);
        assert_eq!(config.output_format, OutputFormat::StreamJson);
        assert!(config.args.contains(&"--print".to_string()));
        assert!(config.args.contains(&"--bare".to_string()));

        // Flags
        assert_eq!(config.flags.get("tools").unwrap(), "--allowedTools");
        assert_eq!(config.flags.get("budget").unwrap(), "--max-budget-usd");

        // Resume
        let resume = config.resume.as_ref().unwrap();
        assert_eq!(resume.flag.as_deref(), Some("--resume"));

        // Permissions
        let perms = config.permissions.as_ref().unwrap();
        assert_eq!(perms.readonly_args, vec!["--permission-mode", "plan"]);
        assert_eq!(
            perms.readwrite_args,
            vec!["--permission-mode", "acceptEdits"]
        );
        assert!(perms.readwrite_tools.contains(&"Edit".to_string()));

        // Result mapping
        assert_eq!(config.result_mapping.plan.as_deref(), Some("result"));
        assert_eq!(
            config.result_mapping.cost_usd.as_deref(),
            Some("total_cost_usd")
        );
        assert_eq!(
            config.result_mapping.session_id.as_deref(),
            Some("session_id")
        );
        assert!((config.result_mapping.confidence_default - 0.85).abs() < 0.01);

        // Metadata mappings
        assert_eq!(
            config.result_mapping.metadata.get("tokens_in").unwrap(),
            "usage.input_tokens"
        );

        // Progress
        let progress = config.progress.as_ref().unwrap();
        assert_eq!(progress.event_field, "type");
        assert!(progress.progress_types.contains(&"assistant".to_string()));
        assert_eq!(progress.result_type.as_deref(), Some("result"));
        assert_eq!(progress.success_field.as_deref(), Some("subtype"));
        assert_eq!(progress.success_value.as_deref(), Some("success"));
    }

    #[test]
    fn parse_codex_adapter() {
        let content = include_str!("../../adapters/codex/ADAPTER.toml");
        let config =
            parse_adapter_toml_str(content, Path::new("adapters/codex/ADAPTER.toml")).unwrap();

        assert_eq!(config.name, "codex");
        assert_eq!(config.command, "codex");
        assert_eq!(config.prompt_delivery, PromptDelivery::Positional);
        assert_eq!(config.output_format, OutputFormat::Jsonl);
        assert_eq!(config.args, vec!["exec", "--json"]);

        // Resume uses positional args, not flag
        let resume = config.resume.as_ref().unwrap();
        assert!(resume.flag.is_none());
        assert_eq!(
            resume.args.as_ref().unwrap(),
            &vec!["exec".to_string(), "resume".to_string()]
        );

        assert!((config.result_mapping.confidence_default - 0.80).abs() < 0.01);
    }

    #[test]
    fn parse_minimal_adapter() {
        let content = r#"
[adapter]
name = "myagent"
command = "myagent"
"#;
        let config = parse_adapter_toml_str(content, Path::new("ADAPTER.toml")).unwrap();

        assert_eq!(config.name, "myagent");
        assert_eq!(config.command, "myagent");
        assert_eq!(config.prompt_delivery, PromptDelivery::Stdin);
        assert_eq!(config.output_format, OutputFormat::Text);
        assert!(config.flags.is_empty());
        assert!(config.resume.is_none());
        assert!(config.permissions.is_none());
        assert!(config.progress.is_none());
        assert!((config.result_mapping.confidence_default - 0.5).abs() < 0.01);
    }

    #[test]
    fn parse_flag_prompt_delivery() {
        let content = r#"
[adapter]
name = "custom"
command = "custom-agent"
prompt_delivery = "flag"
prompt_flag = "--prompt"
"#;
        let config = parse_adapter_toml_str(content, Path::new("ADAPTER.toml")).unwrap();
        assert_eq!(
            config.prompt_delivery,
            PromptDelivery::Flag("--prompt".to_string())
        );
    }

    #[test]
    fn flag_delivery_without_flag_name_errors() {
        let content = r#"
[adapter]
name = "bad"
command = "bad"
prompt_delivery = "flag"
"#;
        let err = parse_adapter_toml_str(content, Path::new("ADAPTER.toml")).unwrap_err();
        assert!(err.contains("requires prompt_flag"));
    }

    #[test]
    fn unknown_prompt_delivery_errors() {
        let content = r#"
[adapter]
name = "bad"
command = "bad"
prompt_delivery = "magic"
"#;
        let err = parse_adapter_toml_str(content, Path::new("ADAPTER.toml")).unwrap_err();
        assert!(err.contains("unknown prompt_delivery"));
    }

    #[test]
    fn generic_fallback() {
        let config = generic_fallback_adapter("opencode");
        assert_eq!(config.name, "opencode");
        assert_eq!(config.command, "opencode");
        assert_eq!(config.prompt_delivery, PromptDelivery::Stdin);
        assert_eq!(config.output_format, OutputFormat::Text);
        assert!((config.result_mapping.confidence_default - 0.5).abs() < 0.01);
    }

    #[test]
    fn extract_json_path_nested() {
        let json: serde_json::Value = serde_json::json!({
            "usage": {
                "input_tokens": 1234,
                "output_tokens": 567
            },
            "result": "hello world",
            "total_cost_usd": 0.05
        });

        assert_eq!(
            extract_json_path(&json, "usage.input_tokens"),
            Some(serde_json::json!(1234))
        );
        assert_eq!(
            extract_json_path(&json, "result"),
            Some(serde_json::json!("hello world"))
        );
        assert_eq!(
            extract_json_path(&json, "total_cost_usd"),
            Some(serde_json::json!(0.05))
        );
        assert_eq!(extract_json_path(&json, "missing.field"), None);
    }

    #[test]
    fn extract_json_path_deeply_nested() {
        let json: serde_json::Value = serde_json::json!({
            "a": { "b": { "c": 42 } }
        });
        assert_eq!(
            extract_json_path(&json, "a.b.c"),
            Some(serde_json::json!(42))
        );
    }
}
