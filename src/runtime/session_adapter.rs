// FORGE session adapter — config-driven CLI driver for external agents (issue #191)
//
// A single generic SessionDriver implementation that reads AdapterConfig
// (parsed from ADAPTER.toml) and handles process spawning, stdin piping,
// stdout parsing, and result extraction for any CLI agent.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;

use crate::runtime::adapter_loader::{
    extract_json_path, AdapterConfig, OutputFormat, PromptDelivery,
};
use crate::runtime::confidence::{ConfidentValue, Value};
use crate::runtime::session_manager::{
    SessionConfig, SessionController, SessionDriver, SessionDriverEvent, SessionRuntimeHandle,
    SessionState,
};

// ── Command building ─────────────────────────────────────────

/// A CLI command ready to be spawned.
#[derive(Debug, Clone)]
pub struct CliCommand {
    pub program: String,
    pub args: Vec<String>,
    pub stdin_payload: Option<String>,
    /// Working directory for sandbox isolation (issue #194).
    pub working_dir: Option<String>,
}

/// Build the CLI command for starting a new session.
pub fn build_command(adapter: &AdapterConfig, config: &SessionConfig) -> CliCommand {
    let mut args = adapter.args.clone();

    // Map SessionConfig fields to adapter flags
    if !config.tools.is_empty() {
        if let Some(flag) = adapter.flags.get("tools") {
            args.push(flag.clone());
            args.push(config.tools.join(","));
        }
    }

    if let Some(budget) = config.budget_usd {
        if let Some(flag) = adapter.flags.get("budget") {
            args.push(flag.clone());
            args.push(format!("{}", budget));
        }
    }

    // Permission mode selection
    if let Some(ref perms) = adapter.permissions {
        let has_write_tools = config
            .tools
            .iter()
            .any(|t| perms.readwrite_tools.iter().any(|rw| t == rw));

        if has_write_tools {
            args.extend(perms.readwrite_args.iter().cloned());
        } else {
            args.extend(perms.readonly_args.iter().cloned());
        }
    }

    // Prompt delivery
    let stdin_payload = match &adapter.prompt_delivery {
        PromptDelivery::Stdin => config.prompt.clone(),
        PromptDelivery::Positional => {
            if let Some(ref prompt) = config.prompt {
                args.push(prompt.clone());
            }
            None
        }
        PromptDelivery::Flag(flag) => {
            if let Some(ref prompt) = config.prompt {
                args.push(flag.clone());
                args.push(prompt.clone());
            }
            None
        }
    };

    CliCommand {
        program: adapter.command.clone(),
        args,
        stdin_payload,
        working_dir: config.working_dir.clone(),
    }
}

/// Build the CLI command for resuming an existing session.
pub fn build_resume_command(adapter: &AdapterConfig, state: &SessionState) -> Option<CliCommand> {
    let resume = adapter.resume.as_ref()?;
    let external_id = state.external_session_id.as_ref()?;

    let mut args = Vec::new();

    if let Some(ref resume_args) = resume.args {
        // Positional resume: e.g., codex exec resume <id>
        args.extend(resume_args.iter().cloned());
        args.push(external_id.clone());
        // Add output format flags from base args (e.g., --json)
        for arg in &adapter.args {
            if arg.starts_with("--") {
                args.push(arg.clone());
            }
        }
    } else if let Some(ref flag) = resume.flag {
        // Flag-based resume: e.g., claude --resume <id>
        args.extend(adapter.args.iter().cloned());
        args.push(flag.clone());
        args.push(external_id.clone());
    } else {
        return None;
    }

    // If there's a follow-up prompt, add it
    let stdin_payload = match &adapter.prompt_delivery {
        PromptDelivery::Stdin => state.config.prompt.clone(),
        PromptDelivery::Positional => {
            if let Some(ref prompt) = state.config.prompt {
                args.push(prompt.clone());
            }
            None
        }
        PromptDelivery::Flag(flag) => {
            if let Some(ref prompt) = state.config.prompt {
                args.push(flag.clone());
                args.push(prompt.clone());
            }
            None
        }
    };

    Some(CliCommand {
        program: adapter.command.clone(),
        args,
        stdin_payload,
        working_dir: state.config.working_dir.clone(),
    })
}

// ── Output parsing ───────────────────────────────────────────

/// Parse a single stdout line for progress events.
pub fn parse_line(adapter: &AdapterConfig, line: &str) -> Option<SessionDriverEvent> {
    let progress = adapter.progress.as_ref()?;

    let json: serde_json::Value = serde_json::from_str(line).ok()?;
    let event_type = json.get(&progress.event_field)?.as_str()?;

    if progress.progress_types.iter().any(|t| t == event_type) {
        let payload = ConfidentValue::from_skill(
            Value::Text(line.to_string()),
            adapter.result_mapping.confidence_default,
        );
        return Some(SessionDriverEvent::Progress {
            payload,
            cost_delta_usd: 0.0,
        });
    }

    None
}

/// Parse the final collected output after the process exits.
pub fn parse_final(
    adapter: &AdapterConfig,
    stdout: &str,
    _stderr: &str,
    exit_code: Option<i32>,
) -> Result<(ConfidentValue, Option<String>), String> {
    // Non-zero exit code is a failure — try to extract a meaningful error
    // message from the output before returning the generic code error.
    if let Some(code) = exit_code {
        if code != 0 {
            // Try to find an error message in stdout JSONL events
            let error_msg = extract_error_from_output(stdout, _stderr);
            return Err(match error_msg {
                Some(msg) => format!("adapter '{}' failed: {}", adapter.name, msg),
                None => format!("adapter '{}' exited with code {}", adapter.name, code),
            });
        }
    }

    // Find the result JSON based on output format
    let result_json = match adapter.output_format {
        OutputFormat::Json => {
            // Single JSON object
            serde_json::from_str::<serde_json::Value>(stdout.trim()).ok()
        }
        OutputFormat::StreamJson => {
            // JSONL with typed events — find the result event
            find_result_event(adapter, stdout)
        }
        OutputFormat::Jsonl => {
            // JSONL — find the last matching result event
            find_result_event(adapter, stdout)
        }
        OutputFormat::Text => None,
    };

    let mut fields: HashMap<String, ConfidentValue> = HashMap::new();
    let rm = &adapter.result_mapping;
    let conf = rm.confidence_default.min(0.99); // Principle I cap

    // Extract fields from JSON if available
    let mut external_session_id = None;

    if let Some(ref json) = result_json {
        // Check success condition
        if let Some(ref progress) = adapter.progress {
            if let (Some(ref sf), Some(ref sv)) = (&progress.success_field, &progress.success_value)
            {
                if let Some(actual) = json.get(sf).and_then(|v| v.as_str()) {
                    if actual != sv.as_str() {
                        return Err(format!(
                            "adapter '{}': {} = '{}' (expected '{}')",
                            adapter.name, sf, actual, sv
                        ));
                    }
                }
            }
        }

        // Map result fields to AgentResult
        if let Some(ref path) = rm.plan {
            if let Some(val) = extract_json_path(json, path) {
                if let Some(s) = val.as_str() {
                    fields.insert(
                        "plan".to_string(),
                        ConfidentValue::from_skill(Value::Text(s.to_string()), conf),
                    );
                }
            }
        }

        if let Some(ref path) = rm.patch_summary {
            if let Some(val) = extract_json_path(json, path) {
                if let Some(s) = val.as_str() {
                    fields.insert(
                        "patch_summary".to_string(),
                        ConfidentValue::from_skill(Value::Text(s.to_string()), conf),
                    );
                }
            }
        }

        if let Some(ref path) = rm.files_changed {
            if let Some(val) = extract_json_path(json, path) {
                if let Some(arr) = val.as_array() {
                    let items: Vec<ConfidentValue> = arr
                        .iter()
                        .filter_map(|v| v.as_str())
                        .map(|s| ConfidentValue::from_skill(Value::Text(s.to_string()), conf))
                        .collect();
                    fields.insert(
                        "files_changed".to_string(),
                        ConfidentValue::from_skill(Value::Array(items), conf),
                    );
                }
            }
        }

        if let Some(ref path) = rm.cost_usd {
            if let Some(val) = extract_json_path(json, path) {
                if let Some(n) = val.as_f64() {
                    fields.insert(
                        "cost_usd".to_string(),
                        ConfidentValue::from_skill(Value::Number(n), conf),
                    );
                }
            }
        }

        if let Some(ref path) = rm.session_id {
            if let Some(val) = extract_json_path(json, path) {
                if let Some(s) = val.as_str() {
                    external_session_id = Some(s.to_string());
                }
            }
        }

        // Metadata fields
        let mut meta_fields: HashMap<String, ConfidentValue> = HashMap::new();
        for (key, path) in &rm.metadata {
            if let Some(val) = extract_json_path(json, path) {
                let cv = match val {
                    serde_json::Value::Number(n) => {
                        ConfidentValue::from_skill(Value::Number(n.as_f64().unwrap_or(0.0)), conf)
                    }
                    serde_json::Value::String(s) => {
                        ConfidentValue::from_skill(Value::Text(s), conf)
                    }
                    serde_json::Value::Bool(b) => ConfidentValue::from_skill(Value::Bool(b), conf),
                    _ => ConfidentValue::from_skill(Value::Text(val.to_string()), conf),
                };
                meta_fields.insert(key.clone(), cv);
            }
        }
        // Inject pending verification contract with implicit claims (#203)
        let implicit_claims = crate::runtime::verification::extract_implicit_claims(&fields);
        crate::runtime::verification::inject_pending_verification(
            &mut meta_fields,
            implicit_claims,
        );

        fields.insert(
            "metadata".to_string(),
            ConfidentValue::from_skill(Value::Record(meta_fields), conf),
        );
    } else {
        // Text fallback: entire stdout becomes the plan field
        if !stdout.trim().is_empty() {
            fields.insert(
                "plan".to_string(),
                ConfidentValue::from_skill(Value::Text(stdout.trim().to_string()), conf),
            );
        }
    }

    // Set confidence
    fields.insert(
        "confidence".to_string(),
        ConfidentValue::from_skill(Value::Number(conf as f64), conf),
    );

    let result = ConfidentValue::from_agent_result(fields);
    Ok((result, external_session_id))
}

/// Find the result event in JSONL output based on adapter progress config.
fn find_result_event(adapter: &AdapterConfig, stdout: &str) -> Option<serde_json::Value> {
    let progress = adapter.progress.as_ref()?;
    let result_type = progress.result_type.as_ref()?;

    let mut last_match = None;
    for line in stdout.lines() {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(line) {
            if let Some(event_type) = json.get(&progress.event_field).and_then(|v| v.as_str()) {
                if event_type == result_type {
                    last_match = Some(json);
                }
            }
        }
    }
    last_match
}

/// Try to extract a human-readable error message from CLI output.
///
/// Checks JSONL events for `"type": "error"` or `"type": "turn.failed"`,
/// then falls back to stderr content.
fn extract_error_from_output(stdout: &str, stderr: &str) -> Option<String> {
    // Check JSONL events for error messages
    for line in stdout.lines() {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(line) {
            // Direct error event: {"type": "error", "message": "..."}
            if json.get("type").and_then(|v| v.as_str()) == Some("error") {
                if let Some(msg) = json.get("message").and_then(|v| v.as_str()) {
                    return Some(msg.to_string());
                }
            }
            // Nested error: {"type": "turn.failed", "error": {"message": "..."}}
            if let Some(err_obj) = json.get("error") {
                if let Some(msg) = err_obj.get("message").and_then(|v| v.as_str()) {
                    return Some(msg.to_string());
                }
                // Error as string
                if let Some(msg) = err_obj.as_str() {
                    return Some(msg.to_string());
                }
            }
        }
    }

    // Fall back to stderr if non-empty
    let trimmed = stderr.trim();
    if !trimmed.is_empty() {
        // Take first meaningful line
        return trimmed.lines().next().map(|s| s.to_string());
    }

    None
}

// ── Process controller ───────────────────────────────────────

/// Controls a running CLI process (cancel via SIGINT, kill via SIGKILL).
pub struct ProcessController {
    child: Arc<Mutex<Option<Child>>>,
}

impl ProcessController {
    pub fn new(child: Child) -> Self {
        Self {
            child: Arc::new(Mutex::new(Some(child))),
        }
    }
}

#[async_trait]
impl SessionController for ProcessController {
    async fn request_cancel(&self) -> Result<(), String> {
        let pid = {
            let guard = self.child.lock().map_err(|e| e.to_string())?;
            guard.as_ref().and_then(|c| c.id())
        };
        if let Some(pid) = pid {
            #[cfg(unix)]
            {
                // SAFETY: sending SIGINT to a process we own
                unsafe {
                    libc::kill(pid as i32, libc::SIGINT);
                }
            }
            #[cfg(not(unix))]
            {
                // On non-Unix, fall through to force_kill
                let _ = pid;
            }
        }
        Ok(())
    }

    async fn force_kill(&self) -> Result<(), String> {
        let maybe_child = {
            let mut guard = self.child.lock().map_err(|e| e.to_string())?;
            guard.take()
        };
        if let Some(mut child) = maybe_child {
            child.kill().await.map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

// ── Config-driven session driver ─────────────────────────────

/// A SessionDriver that uses AdapterConfig to drive any CLI agent.
pub struct ConfigDrivenDriver {
    pub config: AdapterConfig,
}

impl ConfigDrivenDriver {
    pub fn new(config: AdapterConfig) -> Self {
        Self { config }
    }

    /// Spawn a CLI process and return a handle with event channel.
    async fn spawn_process(&self, cmd: CliCommand) -> Result<SessionRuntimeHandle, String> {
        let mut command = Command::new(&cmd.program);
        command
            .args(&cmd.args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(if cmd.stdin_payload.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            });

        // Apply sandbox working directory (issue #194)
        if let Some(ref wd) = cmd.working_dir {
            command.current_dir(wd);
        }

        let mut child = command
            .spawn()
            .map_err(|e| format!("failed to spawn '{}': {}", cmd.program, e))?;

        let process_id = child.id();

        // Write prompt to stdin if needed, then close
        if let Some(payload) = cmd.stdin_payload {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(payload.as_bytes()).await;
                let _ = stdin.shutdown().await;
                // stdin is dropped here, closing the pipe
            }
        }

        let stdout = child.stdout.take().ok_or("failed to capture stdout")?;
        let stderr = child.stderr.take().ok_or("failed to capture stderr")?;

        let controller = Arc::new(ProcessController::new(child));
        let (tx, rx) = mpsc::unbounded_channel();

        // Background task: read stdout, parse lines, collect output
        let adapter_config = self.config.clone();
        let bg_controller = controller.clone();
        tokio::spawn(async move {
            let mut stdout_reader = BufReader::new(stdout).lines();
            let mut stderr_reader = BufReader::new(stderr);
            let mut collected_stdout = String::new();
            let mut collected_stderr = String::new();

            // Read stdout line by line
            while let Ok(Some(line)) = stdout_reader.next_line().await {
                collected_stdout.push_str(&line);
                collected_stdout.push('\n');

                // Try to parse as progress event
                if let Some(event) = parse_line(&adapter_config, &line) {
                    let _ = tx.send(event);
                }
            }

            // Read remaining stderr
            let mut stderr_buf = String::new();
            use tokio::io::AsyncReadExt;
            let _ = stderr_reader.read_to_string(&mut stderr_buf).await;
            collected_stderr.push_str(&stderr_buf);

            // Wait for child exit — take the child out to avoid holding
            // the MutexGuard across an await point.
            let exit_code = {
                let maybe_child = bg_controller.child.lock().unwrap().take();
                if let Some(mut child) = maybe_child {
                    child.wait().await.ok().and_then(|s| s.code())
                } else {
                    None
                }
            };

            // Parse final output
            match parse_final(
                &adapter_config,
                &collected_stdout,
                &collected_stderr,
                exit_code,
            ) {
                Ok((result, _external_id)) => {
                    let _ = tx.send(SessionDriverEvent::Completed { result });
                }
                Err(error) => {
                    let _ = tx.send(SessionDriverEvent::Failed { error });
                }
            }
        });

        Ok(SessionRuntimeHandle {
            external_session_id: None, // Set after first progress event with session_id
            process_id,
            events: rx,
            controller: controller as Arc<dyn SessionController>,
        })
    }
}

#[async_trait]
impl SessionDriver for ConfigDrivenDriver {
    async fn start(
        &self,
        _session_id: &str,
        config: &SessionConfig,
    ) -> Result<SessionRuntimeHandle, String> {
        let cmd = build_command(&self.config, config);
        self.spawn_process(cmd).await
    }

    async fn resume(&self, state: &SessionState) -> Result<Option<SessionRuntimeHandle>, String> {
        match build_resume_command(&self.config, state) {
            Some(cmd) => {
                let handle = self.spawn_process(cmd).await?;
                Ok(Some(handle))
            }
            None => Ok(None),
        }
    }
}

// ── Tests ────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::adapter_loader::{generic_fallback_adapter, parse_adapter_toml_str};
    use std::path::Path;

    fn claude_config() -> AdapterConfig {
        let content = include_str!("../../adapters/claude/ADAPTER.toml");
        parse_adapter_toml_str(content, Path::new("claude/ADAPTER.toml")).unwrap()
    }

    fn codex_config() -> AdapterConfig {
        let content = include_str!("../../adapters/codex/ADAPTER.toml");
        parse_adapter_toml_str(content, Path::new("codex/ADAPTER.toml")).unwrap()
    }

    // ── build_command tests ──────────────────────────────────

    #[test]
    fn claude_build_command_basic() {
        let adapter = claude_config();
        let session = SessionConfig::new("test", "claude");
        let cmd = build_command(&adapter, &session);

        assert_eq!(cmd.program, "claude");
        assert!(cmd.args.contains(&"--print".to_string()));
        assert!(cmd.args.contains(&"--bare".to_string()));
        assert!(cmd.args.contains(&"--output-format".to_string()));
        // No stdin payload without a prompt
        assert!(cmd.stdin_payload.is_none());
    }

    #[test]
    fn claude_build_command_with_prompt_and_tools() {
        let adapter = claude_config();
        let mut session = SessionConfig::new("test", "claude");
        session.prompt = Some("implement login page".to_string());
        session.tools = vec!["Read".to_string(), "Edit".to_string(), "Bash".to_string()];
        session.budget_usd = Some(5.0);

        let cmd = build_command(&adapter, &session);

        // Prompt goes to stdin
        assert_eq!(cmd.stdin_payload.as_deref(), Some("implement login page"));

        // Tools flag
        let tools_idx = cmd.args.iter().position(|a| a == "--allowedTools").unwrap();
        assert_eq!(cmd.args[tools_idx + 1], "Read,Edit,Bash");

        // Budget flag
        let budget_idx = cmd
            .args
            .iter()
            .position(|a| a == "--max-budget-usd")
            .unwrap();
        assert_eq!(cmd.args[budget_idx + 1], "5");

        // Has write tools → acceptEdits
        assert!(cmd.args.contains(&"--permission-mode".to_string()));
        assert!(cmd.args.contains(&"acceptEdits".to_string()));
    }

    #[test]
    fn claude_readonly_tools_get_plan_mode() {
        let adapter = claude_config();
        let mut session = SessionConfig::new("test", "claude");
        session.tools = vec!["Read".to_string(), "Glob".to_string()];

        let cmd = build_command(&adapter, &session);
        assert!(cmd.args.contains(&"plan".to_string()));
        assert!(!cmd.args.contains(&"acceptEdits".to_string()));
    }

    #[test]
    fn codex_build_command_with_prompt() {
        let adapter = codex_config();
        let mut session = SessionConfig::new("test", "codex");
        session.prompt = Some("fix the bug".to_string());
        session.tools = vec!["Edit".to_string()];

        let cmd = build_command(&adapter, &session);

        assert_eq!(cmd.program, "codex");
        // Prompt is positional, not stdin
        assert!(cmd.stdin_payload.is_none());
        assert!(cmd.args.contains(&"fix the bug".to_string()));
        // Has write tools → full-auto
        assert!(cmd.args.contains(&"--full-auto".to_string()));
    }

    #[test]
    fn codex_readonly_gets_sandbox() {
        let adapter = codex_config();
        let mut session = SessionConfig::new("test", "codex");
        session.tools = vec!["Read".to_string()];

        let cmd = build_command(&adapter, &session);
        assert!(cmd.args.contains(&"--sandbox".to_string()));
        assert!(cmd.args.contains(&"read-only".to_string()));
    }

    #[test]
    fn generic_fallback_build_command() {
        let adapter = generic_fallback_adapter("opencode");
        let mut session = SessionConfig::new("test", "opencode");
        session.prompt = Some("hello".to_string());

        let cmd = build_command(&adapter, &session);
        assert_eq!(cmd.program, "opencode");
        assert_eq!(cmd.stdin_payload.as_deref(), Some("hello"));
        assert!(cmd.args.is_empty()); // No base args for generic
    }

    // ── build_resume_command tests ───────────────────────────

    #[test]
    fn claude_resume_command() {
        let adapter = claude_config();
        let config = SessionConfig::new("test", "claude");
        let mut state = SessionState::new_for_test("s1".into(), config);
        state.external_session_id = Some("ext-123".to_string());

        let cmd = build_resume_command(&adapter, &state).unwrap();
        assert_eq!(cmd.program, "claude");
        assert!(cmd.args.contains(&"--resume".to_string()));
        assert!(cmd.args.contains(&"ext-123".to_string()));
    }

    #[test]
    fn codex_resume_command() {
        let adapter = codex_config();
        let config = SessionConfig::new("test", "codex");
        let mut state = SessionState::new_for_test("s1".into(), config);
        state.external_session_id = Some("thread-abc".to_string());

        let cmd = build_resume_command(&adapter, &state).unwrap();
        assert_eq!(cmd.program, "codex");
        // Should have: exec resume thread-abc
        assert!(cmd.args.contains(&"exec".to_string()));
        assert!(cmd.args.contains(&"resume".to_string()));
        assert!(cmd.args.contains(&"thread-abc".to_string()));
    }

    #[test]
    fn generic_no_resume() {
        let adapter = generic_fallback_adapter("opencode");
        let config = SessionConfig::new("test", "opencode");
        let mut state = SessionState::new_for_test("s1".into(), config);
        state.external_session_id = Some("ext-123".to_string());

        assert!(build_resume_command(&adapter, &state).is_none());
    }

    // ── parse_line tests ─────────────────────────────────────

    #[test]
    fn claude_parse_progress_line() {
        let adapter = claude_config();
        let line = r#"{"type":"assistant","message":"working..."}"#;
        let event = parse_line(&adapter, line);
        assert!(event.is_some());
        assert!(matches!(
            event.unwrap(),
            SessionDriverEvent::Progress { .. }
        ));
    }

    #[test]
    fn claude_parse_result_line_not_progress() {
        let adapter = claude_config();
        // "result" type is not in progress_types, only in result_type
        let line = r#"{"type":"result","subtype":"success","result":"done"}"#;
        let event = parse_line(&adapter, line);
        assert!(event.is_none());
    }

    #[test]
    fn parse_line_invalid_json() {
        let adapter = claude_config();
        let event = parse_line(&adapter, "not json at all");
        assert!(event.is_none());
    }

    #[test]
    fn parse_line_no_progress_config() {
        let adapter = generic_fallback_adapter("test");
        let line = r#"{"type":"progress"}"#;
        let event = parse_line(&adapter, line);
        assert!(event.is_none());
    }

    // ── parse_final tests ────────────────────────────────────

    #[test]
    fn claude_parse_final_success() {
        let adapter = claude_config();
        let stdout = r#"{"type":"system.init","session_id":"sess-1"}
{"type":"assistant","content":"thinking..."}
{"type":"result","subtype":"success","result":"Login page implemented successfully","total_cost_usd":0.12,"session_id":"sess-1","usage":{"input_tokens":5000,"output_tokens":2000},"num_turns":3,"duration_ms":45000}
"#;
        let (result, ext_id) = parse_final(&adapter, stdout, "", Some(0)).unwrap();

        // Should have extracted session_id
        assert_eq!(ext_id.as_deref(), Some("sess-1"));

        // Check AgentResult fields
        if let Value::Record(ref fields) = result.value {
            // plan
            if let Some(cv) = fields.get("plan") {
                if let Value::Text(ref s) = cv.value {
                    assert_eq!(s, "Login page implemented successfully");
                } else {
                    panic!("plan should be Text");
                }
            }
            // cost_usd
            if let Some(cv) = fields.get("cost_usd") {
                if let Value::Number(n) = cv.value {
                    assert!((n - 0.12).abs() < 0.001);
                }
            }
            // confidence capped at 0.99
            assert!(result.confidence <= 0.99);

            // metadata should have tokens
            if let Some(cv) = fields.get("metadata") {
                if let Value::Record(ref meta) = cv.value {
                    assert!(meta.contains_key("tokens_in"));
                    assert!(meta.contains_key("tokens_out"));
                }
            }
        } else {
            panic!("result should be Record");
        }
    }

    #[test]
    fn claude_parse_final_failure_subtype() {
        let adapter = claude_config();
        let stdout = r#"{"type":"result","subtype":"error","error":"rate limited"}
"#;
        let err = parse_final(&adapter, stdout, "", Some(0)).unwrap_err();
        assert!(err.contains("error"));
    }

    #[test]
    fn parse_final_nonzero_exit() {
        let adapter = claude_config();
        let err = parse_final(&adapter, "", "", Some(1)).unwrap_err();
        assert!(err.contains("exited with code 1"));
    }

    #[test]
    fn text_fallback_parse_final() {
        let adapter = generic_fallback_adapter("test");
        let stdout = "Here is the implementation plan:\n1. Create files\n2. Write tests";
        let (result, ext_id) = parse_final(&adapter, stdout, "", Some(0)).unwrap();

        assert!(ext_id.is_none());
        if let Value::Record(ref fields) = result.value {
            if let Some(cv) = fields.get("plan") {
                if let Value::Text(ref s) = cv.value {
                    assert!(s.contains("implementation plan"));
                }
            }
        }
    }

    #[test]
    fn empty_stdout_parse_final() {
        let adapter = generic_fallback_adapter("test");
        let (result, _) = parse_final(&adapter, "", "", Some(0)).unwrap();
        // Should succeed with default fields
        assert!(matches!(result.value, Value::Record(_)));
    }

    // ── error extraction tests ───────────────────────────────

    #[test]
    fn codex_error_extraction_from_jsonl() {
        // Real Codex output when hitting usage limit
        let stdout = r#"{"type":"thread.started","thread_id":"019d77ba-58b8-7411-be37-5c4e70330777"}
{"type":"turn.started"}
{"type":"error","message":"You've hit your usage limit. Upgrade to Pro."}
{"type":"turn.failed","error":{"message":"You've hit your usage limit. Upgrade to Pro."}}
"#;
        let adapter = codex_config();
        let err = parse_final(&adapter, stdout, "", Some(1)).unwrap_err();
        assert!(
            err.contains("usage limit"),
            "error should contain the actual message, got: {}",
            err
        );
    }

    #[test]
    fn error_extraction_from_stderr() {
        let adapter = generic_fallback_adapter("test");
        let err = parse_final(&adapter, "", "command not found: test", Some(127)).unwrap_err();
        assert!(
            err.contains("command not found"),
            "error should contain stderr, got: {}",
            err
        );
    }

    #[test]
    fn nonzero_exit_with_no_output() {
        let adapter = generic_fallback_adapter("test");
        let err = parse_final(&adapter, "", "", Some(1)).unwrap_err();
        assert!(err.contains("exited with code 1"));
    }
}
