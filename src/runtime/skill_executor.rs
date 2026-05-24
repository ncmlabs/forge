// FORGE skill executor — issue #40
// Executes skills via an LLM-mediated agentic loop.
// The LLM reads SKILL.md instructions, requests tool calls (bash, HTTP),
// tools execute, results return to LLM, repeat until done.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use crate::llm::registry::ProviderRegistry;
use crate::llm::{CompletionRequest, ToolDefinition};
use crate::runtime::confidence::{ConfidentValue, Value};
use crate::runtime::skill::{
    LoadedSkill, SkillCapabilityExecutor, SkillError, SkillExecutorKind, SkillExecutorResult,
};
use crate::runtime::skill_registry::SharedSkillRegistry;
use crate::tracer::LLMResponseInfo;
use crate::tracer::Tracer;

const SLACK_SECTION_TEXT_LIMIT: usize = 3000;
const SLACK_APPROVAL_SECTION_CHUNK: usize = 2900;
const SLACK_MESSAGE_BLOCK_LIMIT: usize = 50;
const SLACK_APPROVAL_TRUNCATION_NOTE: &str =
    "_Approval body truncated for Slack Block Kit limits._";

/// Executes skills by running an agentic loop:
/// LLM reads skill instructions + args -> requests tool calls ->
/// tools execute -> results returned to LLM -> repeat until done.
pub struct SkillExecutor {
    pub providers: Arc<ProviderRegistry>,
    pub skill_registry: SharedSkillRegistry,
    pub max_turns: usize,
    pub default_timeout: Duration,
    pub tracer: Option<Arc<Tracer>>,
}

impl SkillExecutor {
    pub fn new(providers: Arc<ProviderRegistry>, skill_registry: SharedSkillRegistry) -> Self {
        Self {
            providers,
            skill_registry,
            max_turns: 10,
            default_timeout: Duration::from_secs(30),
            tracer: None,
        }
    }

    pub fn with_tracer(mut self, tracer: Arc<Tracer>) -> Self {
        self.tracer = Some(tracer);
        self
    }

    /// Execute a skill by name with the given arguments.
    pub async fn execute(
        &self,
        skill_name: &str,
        method: &str,
        args: &HashMap<String, ConfidentValue>,
    ) -> Result<ConfidentValue, SkillError> {
        let skill = {
            let registry = self.skill_registry.lock().unwrap();
            registry
                .get(skill_name)
                .cloned()
                .ok_or_else(|| SkillError::NotFound {
                    name: skill_name.to_string(),
                })?
        };

        if skill.manifest.legacy_signature.is_none()
            && !skill
                .manifest
                .capabilities
                .iter()
                .any(|cap| cap.name == method)
        {
            return Err(SkillError::UnknownMethod {
                skill: skill_name.to_string(),
                method: method.to_string(),
            });
        }

        if let Some(ref tracer) = self.tracer {
            tracer.skill_call(&format!("{}.{}", skill_name, method));
        }

        let start = std::time::Instant::now();
        let result = self.execute_with_skill(&skill, method, args).await;
        let elapsed = start.elapsed();

        if let Some(ref tracer) = self.tracer {
            tracer.skill_return(
                &format!("{}.{}", skill_name, method),
                result.is_ok(),
                elapsed.as_millis() as u64,
            );
        }

        result
    }

    async fn execute_with_skill(
        &self,
        skill: &LoadedSkill,
        method: &str,
        args: &HashMap<String, ConfidentValue>,
    ) -> Result<ConfidentValue, SkillError> {
        if let Some(executor) = skill
            .manifest
            .capabilities
            .iter()
            .find(|cap| cap.name == method)
            .and_then(|cap| cap.executor.as_ref())
        {
            return self
                .execute_deterministic(skill, method, executor, args)
                .await;
        }

        let system_prompt = format!(
            "You are executing a skill. Follow these instructions exactly.\n\n{}\n\n\
             Respond with the final result as plain text when done. \
             Do not wrap the result in markdown code blocks.",
            skill.instructions
        );

        let args_text: Vec<String> = args
            .iter()
            .map(|(k, v)| format!("  {}: {}", k, v.value))
            .collect();

        let user_prompt = format!("Execute: {}\nArguments:\n{}", method, args_text.join("\n"));

        // Build tool definitions based on skill's allowed tools
        let tools = self.build_tool_definitions(&skill.manifest.allowed_tools);

        // If no tools are available, do a simple completion
        if tools.is_empty() {
            return self
                .simple_completion(
                    &system_prompt,
                    &user_prompt,
                    &skill.manifest.default_confidence,
                )
                .await;
        }

        // Agentic loop with tool use
        let timeout = Duration::from_secs(skill.manifest.timeout_secs);
        let result = tokio::time::timeout(timeout, async {
            self.agentic_loop(
                &system_prompt,
                &user_prompt,
                &tools,
                &skill.manifest.default_confidence,
            )
            .await
        })
        .await;

        match result {
            Ok(inner) => inner,
            Err(_) => Err(SkillError::Timeout {
                name: skill.manifest.name.clone(),
                timeout_secs: skill.manifest.timeout_secs,
            }),
        }
    }

    async fn execute_deterministic(
        &self,
        skill: &LoadedSkill,
        method: &str,
        executor: &SkillCapabilityExecutor,
        args: &HashMap<String, ConfidentValue>,
    ) -> Result<ConfidentValue, SkillError> {
        match executor.kind {
            SkillExecutorKind::Command => {
                let argv = expand_argv(&executor.argv, &executor.params, args)?;
                let (program, program_args) =
                    argv.split_first()
                        .ok_or_else(|| SkillError::ExecutionFailed {
                            name: format!("{}.{}", skill.manifest.name, method),
                            reason: "executor argv must not be empty".to_string(),
                        })?;

                let output = tokio::time::timeout(
                    Duration::from_secs(skill.manifest.timeout_secs),
                    tokio::process::Command::new(program)
                        .args(program_args)
                        .output(),
                )
                .await
                .map_err(|_| SkillError::Timeout {
                    name: skill.manifest.name.clone(),
                    timeout_secs: skill.manifest.timeout_secs,
                })?
                .map_err(|e| SkillError::ExecutionFailed {
                    name: format!("{}.{}", skill.manifest.name, method),
                    reason: e.to_string(),
                })?;

                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                if !output.status.success() {
                    return Err(SkillError::ExecutionFailed {
                        name: format!("{}.{}", skill.manifest.name, method),
                        reason: if stderr.is_empty() { stdout } else { stderr },
                    });
                }

                let name = format!("{}.{}", skill.manifest.name, method);
                let text = apply_result_mapping(executor.result.as_ref(), &stdout, &name)
                    .map_err(|err| augment_skill_error(err, &name, &stdout, &argv))?;
                Ok(ConfidentValue::from_skill(
                    Value::Text(text),
                    skill.manifest.default_confidence,
                ))
            }
        }
    }

    async fn simple_completion(
        &self,
        system: &str,
        user: &str,
        default_confidence: &f32,
    ) -> Result<ConfidentValue, SkillError> {
        if let Some(ref tracer) = self.tracer {
            tracer.llm_request("skill", user);
        }

        let request = CompletionRequest::simple(user).with_system(system.to_string());
        let response = self
            .providers
            .resolve_and_complete(request, None)
            .await
            .map_err(|e| SkillError::ProviderError(e.to_string()))?;

        let confidence = response.estimate_confidence().min(*default_confidence);

        if let Some(ref tracer) = self.tracer {
            tracer.llm_response(&LLMResponseInfo {
                operation: "skill",
                provider: &response.provider_name,
                model: &response.model_used,
                tokens_in: response.tokens_in,
                tokens_out: response.tokens_out,
                cost_usd: response.cost_usd,
                confidence,
                agent_name: None,
                phase: None,
            });
        }

        Ok(ConfidentValue::from_skill(
            crate::runtime::confidence::Value::Text(response.content),
            confidence,
        ))
    }

    async fn agentic_loop(
        &self,
        system: &str,
        initial_prompt: &str,
        tools: &[ToolDefinition],
        default_confidence: &f32,
    ) -> Result<ConfidentValue, SkillError> {
        let mut prompt = initial_prompt.to_string();

        for turn in 0..self.max_turns {
            if let Some(ref tracer) = self.tracer {
                tracer.llm_request("skill_tool", &prompt);
            }

            let request = CompletionRequest::simple(&prompt)
                .with_system(system.to_string())
                .with_tools(tools.to_vec());
            let response = self
                .providers
                .resolve_and_complete(request, None)
                .await
                .map_err(|e| SkillError::ProviderError(e.to_string()))?;

            if let Some(ref tracer) = self.tracer {
                let confidence = response.estimate_confidence().min(*default_confidence);
                tracer.llm_response(&LLMResponseInfo {
                    operation: "skill_tool",
                    provider: &response.provider_name,
                    model: &response.model_used,
                    tokens_in: response.tokens_in,
                    tokens_out: response.tokens_out,
                    cost_usd: response.cost_usd,
                    confidence,
                    agent_name: None,
                    phase: None,
                });
            }

            // If no tool calls, the LLM is done
            if response.tool_calls.is_empty() {
                let confidence = response.estimate_confidence().min(*default_confidence);
                return Ok(ConfidentValue::from_skill(
                    crate::runtime::confidence::Value::Text(response.content),
                    confidence,
                ));
            }

            // Execute tool calls and build continuation prompt
            let mut tool_results = Vec::new();
            for tool_call in &response.tool_calls {
                let result = self.execute_tool(tool_call).await;
                tool_results.push(format!(
                    "Tool '{}' result: {}",
                    tool_call.name,
                    match &result {
                        Ok(output) => output.clone(),
                        Err(e) => format!("ERROR: {}", e),
                    }
                ));
            }

            // Build next prompt with tool results
            prompt = format!(
                "{}\n\nTool results from turn {}:\n{}",
                initial_prompt,
                turn + 1,
                tool_results.join("\n\n")
            );
        }

        Err(SkillError::MaxTurnsExceeded {
            name: "skill".to_string(),
            turns: self.max_turns,
        })
    }

    async fn execute_tool(
        &self,
        tool_call: &crate::llm::ToolCallRequest,
    ) -> Result<String, SkillError> {
        match tool_call.name.as_str() {
            "bash_exec" => {
                let command = tool_call
                    .arguments
                    .get("command")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| SkillError::ExecutionFailed {
                        name: "bash_exec".to_string(),
                        reason: "missing 'command' argument".to_string(),
                    })?;

                let output = tokio::process::Command::new("sh")
                    .arg("-c")
                    .arg(command)
                    .output()
                    .await
                    .map_err(|e| SkillError::ExecutionFailed {
                        name: "bash_exec".to_string(),
                        reason: e.to_string(),
                    })?;

                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                if stdout.is_empty() {
                    Ok(stderr)
                } else {
                    Ok(stdout)
                }
            }
            "http_request" => {
                let url = tool_call
                    .arguments
                    .get("url")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| SkillError::ExecutionFailed {
                        name: "http_request".to_string(),
                        reason: "missing 'url' argument".to_string(),
                    })?;

                let client = reqwest::Client::new();
                let method = tool_call
                    .arguments
                    .get("method")
                    .and_then(|v| v.as_str())
                    .unwrap_or("GET");

                let resp = match method {
                    "POST" => {
                        let body = tool_call
                            .arguments
                            .get("body")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        client.post(url).body(body.to_string()).send().await
                    }
                    _ => client.get(url).send().await,
                };

                match resp {
                    Ok(r) => r.text().await.map_err(|e| SkillError::ExecutionFailed {
                        name: "http_request".to_string(),
                        reason: e.to_string(),
                    }),
                    Err(e) => Err(SkillError::ExecutionFailed {
                        name: "http_request".to_string(),
                        reason: e.to_string(),
                    }),
                }
            }
            other => Err(SkillError::UnknownTool {
                name: other.to_string(),
            }),
        }
    }

    fn build_tool_definitions(&self, allowed_tools: &[String]) -> Vec<ToolDefinition> {
        let mut tools = Vec::new();

        let has_bash = allowed_tools.is_empty()
            || allowed_tools
                .iter()
                .any(|t| t == "Bash" || t == "bash_exec" || t.starts_with("Bash("));
        let has_http = allowed_tools.is_empty()
            || allowed_tools
                .iter()
                .any(|t| t == "WebFetch" || t == "http_request" || t.starts_with("WebFetch("));

        if has_bash {
            tools.push(ToolDefinition {
                name: "bash_exec".to_string(),
                description: "Execute a shell command and return its output.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "command": {
                            "type": "string",
                            "description": "The shell command to execute"
                        }
                    },
                    "required": ["command"]
                }),
            });
        }

        if has_http {
            tools.push(ToolDefinition {
                name: "http_request".to_string(),
                description: "Make an HTTP request and return the response body.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "url": {
                            "type": "string",
                            "description": "The URL to request"
                        },
                        "method": {
                            "type": "string",
                            "description": "HTTP method (GET, POST)",
                            "default": "GET"
                        },
                        "body": {
                            "type": "string",
                            "description": "Request body (for POST)"
                        }
                    },
                    "required": ["url"]
                }),
            });
        }

        tools
    }
}

fn expand_argv(
    argv: &[String],
    params: &[String],
    args: &HashMap<String, ConfidentValue>,
) -> Result<Vec<String>, SkillError> {
    argv.iter()
        .map(|arg| expand_template(arg, params, args))
        .collect()
}

fn expand_template(
    template: &str,
    params: &[String],
    args: &HashMap<String, ConfidentValue>,
) -> Result<String, SkillError> {
    let mut output = String::new();
    let mut rest = template;

    while let Some(start) = rest.find(['{', '}']) {
        output.push_str(&rest[..start]);
        if rest[start..].starts_with("{{") {
            output.push('{');
            rest = &rest[start + 2..];
            continue;
        }
        if rest[start..].starts_with("}}") {
            output.push('}');
            rest = &rest[start + 2..];
            continue;
        }
        if rest[start..].starts_with('}') {
            output.push('}');
            rest = &rest[start + 1..];
            continue;
        }

        let after_start = &rest[start + 1..];
        let Some(end) = after_start.find('}') else {
            output.push_str(&rest[start..]);
            return Ok(output);
        };
        let key = &after_start[..end];
        output.push_str(&lookup_template_value(key, params, args)?);
        rest = &after_start[end + 1..];
    }
    output.push_str(rest);
    Ok(output)
}

fn lookup_template_value(
    key: &str,
    params: &[String],
    args: &HashMap<String, ConfidentValue>,
) -> Result<String, SkillError> {
    if let Some(inner) = key.strip_prefix("json:") {
        let value = lookup_template_value(inner, params, args)?;
        return serde_json::to_string(&value).map_err(|e| SkillError::ExecutionFailed {
            name: "deterministic_skill".to_string(),
            reason: format!("could not JSON-escape argument '{}': {}", inner, e),
        });
    }

    if let Some(env_name) = key.strip_prefix("env:") {
        return std::env::var(env_name).map_err(|_| SkillError::ExecutionFailed {
            name: "deterministic_skill".to_string(),
            reason: format!("missing environment variable '{}'", env_name),
        });
    }

    if let Some(spec) = key.strip_prefix("slack_approval_blocks:") {
        let (text_key, request_id_key) =
            spec.split_once(':')
                .ok_or_else(|| SkillError::ExecutionFailed {
                    name: "deterministic_skill".to_string(),
                    reason: "slack_approval_blocks requires text and request_id keys".to_string(),
                })?;
        let text = lookup_template_value(text_key, params, args)?;
        let request_id = lookup_template_value(request_id_key, params, args)?;
        return build_slack_approval_blocks(&text, &request_id);
    }

    if let Some(spec) = key.strip_prefix("slack_approval_fallback:") {
        let (text_key, request_id_key) =
            spec.split_once(':')
                .ok_or_else(|| SkillError::ExecutionFailed {
                    name: "deterministic_skill".to_string(),
                    reason: "slack_approval_fallback requires text and request_id keys".to_string(),
                })?;
        let text = lookup_template_value(text_key, params, args)?;
        let request_id = lookup_template_value(request_id_key, params, args)?;
        return Ok(slack_approval_fallback_text(&text, &request_id));
    }

    if let Some(value) = args.get(key) {
        return Ok(value.value.to_string());
    }

    if let Some(index) = params.iter().position(|param| param == key) {
        if let Some(value) = args.get(&format!("_{}", index)) {
            return Ok(value.value.to_string());
        }
    }

    Err(SkillError::ExecutionFailed {
        name: "deterministic_skill".to_string(),
        reason: format!("missing argument '{}'", key),
    })
}

fn apply_result_mapping(
    mapping: Option<&SkillExecutorResult>,
    stdout: &str,
    name: &str,
) -> Result<String, SkillError> {
    let Some(mapping) = mapping else {
        return Ok(stdout.trim().to_string());
    };

    let json: serde_json::Value =
        serde_json::from_str(stdout).map_err(|e| SkillError::ExecutionFailed {
            name: name.to_string(),
            reason: format!("executor output was not valid JSON: {}", e),
        })?;

    if let Some(success_path) = &mapping.success_path {
        if extract_json_path(&json, success_path) == Some(&serde_json::Value::Bool(false)) {
            let reason = mapping
                .error_path
                .as_deref()
                .and_then(|path| extract_json_path(&json, path))
                .map(json_value_to_text)
                .unwrap_or_else(|| "operation returned ok=false".to_string());
            return Err(SkillError::ExecutionFailed {
                name: name.to_string(),
                reason,
            });
        }
    }

    if let Some(path) = &mapping.json_path {
        let Some(value) = extract_json_path(&json, path) else {
            return Err(SkillError::ExecutionFailed {
                name: name.to_string(),
                reason: format!("missing JSON result path '{}'", path),
            });
        };
        return Ok(json_value_to_text(value));
    }

    Ok(stdout.trim().to_string())
}

fn augment_skill_error(err: SkillError, name: &str, stdout: &str, argv: &[String]) -> SkillError {
    if name != "slack.send_approval" {
        return err;
    }

    match err {
        SkillError::ExecutionFailed { name, reason } => SkillError::ExecutionFailed {
            name,
            reason: format!(
                "{}; slack_response={}; block_payload_summary={}",
                reason,
                stdout.trim(),
                summarize_slack_approval_blocks(argv)
            ),
        },
        other => other,
    }
}

fn build_slack_approval_blocks(text: &str, request_id: &str) -> Result<String, SkillError> {
    let mut sections = split_slack_section_text(text, SLACK_APPROVAL_SECTION_CHUNK);
    if sections.is_empty() {
        sections.push("Approval required.".to_string());
    }

    let max_sections = SLACK_MESSAGE_BLOCK_LIMIT - 1;
    if sections.len() > max_sections {
        sections.truncate(max_sections);
        let note_len = SLACK_APPROVAL_TRUNCATION_NOTE.chars().count() + 2;
        let keep = SLACK_APPROVAL_SECTION_CHUNK.saturating_sub(note_len);
        let mut last = truncate_chars(sections.last().map(String::as_str).unwrap_or(""), keep);
        last.push_str("\n\n");
        last.push_str(SLACK_APPROVAL_TRUNCATION_NOTE);
        if let Some(slot) = sections.last_mut() {
            *slot = last;
        }
    }

    let mut blocks = Vec::with_capacity(sections.len() + 1);
    for section in sections {
        blocks.push(serde_json::json!({
            "type": "section",
            "text": {
                "type": "mrkdwn",
                "text": section,
            },
        }));
    }
    blocks.push(serde_json::json!({
        "type": "actions",
        "block_id": "approval_actions",
        "elements": [
            {
                "type": "button",
                "text": {"type": "plain_text", "text": "Approve"},
                "style": "primary",
                "action_id": "approve",
                "value": format!("approved:{request_id}"),
            },
            {
                "type": "button",
                "text": {"type": "plain_text", "text": "Reject"},
                "style": "danger",
                "action_id": "reject",
                "value": format!("rejected:{request_id}"),
            },
        ],
    }));

    serde_json::to_string(&blocks).map_err(|e| SkillError::ExecutionFailed {
        name: "deterministic_skill".to_string(),
        reason: format!("could not build Slack approval blocks: {e}"),
    })
}

fn split_slack_section_text(text: &str, chunk_size: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    let chunk_size = chunk_size.min(SLACK_SECTION_TEXT_LIMIT);
    for ch in text.chars() {
        if current.chars().count() >= chunk_size {
            chunks.push(current);
            current = String::new();
        }
        current.push(ch);
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}

fn slack_approval_fallback_text(text: &str, request_id: &str) -> String {
    let first_line = text
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("Approval required");
    let summary = truncate_chars(first_line.trim(), 180);
    format!("{summary} (request_id: {request_id})")
}

fn summarize_slack_approval_blocks(argv: &[String]) -> String {
    let Some(blocks_arg) = argv.iter().find(|arg| arg.starts_with("blocks=")) else {
        return "blocks_arg=missing".to_string();
    };
    let blocks_json = blocks_arg.trim_start_matches("blocks=");
    let Ok(blocks) = serde_json::from_str::<serde_json::Value>(blocks_json) else {
        return format!(
            "blocks_arg=invalid_json chars={}",
            blocks_json.chars().count()
        );
    };
    let Some(array) = blocks.as_array() else {
        return "blocks_arg=not_array".to_string();
    };

    let mut section_count = 0usize;
    let mut max_section_chars = 0usize;
    let mut total_section_chars = 0usize;
    let mut action_values = Vec::new();
    let mut truncated = false;

    for block in array {
        if block.get("type").and_then(|v| v.as_str()) == Some("section") {
            section_count += 1;
            if let Some(text) = block
                .get("text")
                .and_then(|v| v.get("text"))
                .and_then(|v| v.as_str())
            {
                let chars = text.chars().count();
                max_section_chars = max_section_chars.max(chars);
                total_section_chars += chars;
                if text.contains(SLACK_APPROVAL_TRUNCATION_NOTE) {
                    truncated = true;
                }
            }
        }
        if block.get("type").and_then(|v| v.as_str()) == Some("actions") {
            if let Some(elements) = block.get("elements").and_then(|v| v.as_array()) {
                for element in elements {
                    if let Some(value) = element.get("value").and_then(|v| v.as_str()) {
                        action_values.push(value.to_string());
                    }
                }
            }
        }
    }

    format!(
        "blocks={} sections={} max_section_chars={} total_section_chars={} truncated={} action_values={}",
        array.len(),
        section_count,
        max_section_chars,
        total_section_chars,
        truncated,
        action_values.join("|")
    )
}

fn extract_json_path<'a>(
    value: &'a serde_json::Value,
    path: &str,
) -> Option<&'a serde_json::Value> {
    let mut current = value;
    for segment in path.split('.') {
        current = current.get(segment)?;
    }
    Some(current)
}

fn json_value_to_text(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::confidence::Value;

    // #354: template expansion must preserve CSV labels as a single argv
    // element — commas cannot split the token, because `gh issue create
    // --label` receives the CSV as one argument and splits it itself.
    #[test]
    fn expand_argv_preserves_labels_csv() {
        let argv = vec![
            "gh".to_string(),
            "issue".to_string(),
            "create".to_string(),
            "-R".to_string(),
            "{repo}".to_string(),
            "--title".to_string(),
            "{title}".to_string(),
            "--body".to_string(),
            "{body}".to_string(),
            "--label".to_string(),
            "{labels_csv}".to_string(),
        ];
        let params: Vec<String> = vec![
            "repo".into(),
            "title".into(),
            "body".into(),
            "labels_csv".into(),
        ];
        let mut args: HashMap<String, ConfidentValue> = HashMap::new();
        args.insert(
            "repo".into(),
            ConfidentValue::deterministic(Value::Text("ncmlabs/b".into())),
        );
        args.insert(
            "title".into(),
            ConfidentValue::deterministic(Value::Text("Port auth middleware".into())),
        );
        args.insert(
            "body".into(),
            ConfidentValue::deterministic(Value::Text("Spawned from repo-A task T7".into())),
        );
        args.insert(
            "labels_csv".into(),
            ConfidentValue::deterministic(Value::Text(
                "clone-dev,from:T7,blocks:T7,area:auth".into(),
            )),
        );

        let expanded = expand_argv(&argv, &params, &args).expect("argv expands");

        assert_eq!(
            expanded,
            vec![
                "gh",
                "issue",
                "create",
                "-R",
                "ncmlabs/b",
                "--title",
                "Port auth middleware",
                "--body",
                "Spawned from repo-A task T7",
                "--label",
                "clone-dev,from:T7,blocks:T7,area:auth",
            ]
        );

        // The label CSV must land as one argv element: four labels, one comma-
        // separated string. If template expansion ever split on commas this
        // assertion breaks immediately.
        assert_eq!(
            expanded[10], "clone-dev,from:T7,blocks:T7,area:auth",
            "label CSV must survive as a single argv element"
        );
    }

    // #354: positional fallback — args keyed by `_0`, `_1`, ... must also
    // resolve `{repo}` etc. via the `params` index lookup. This is the
    // callsite used when the resolver lowers positional FORGE arguments.
    #[test]
    fn expand_argv_resolves_positional_args_for_labeled_issue() {
        let argv = vec![
            "gh".to_string(),
            "issue".to_string(),
            "create".to_string(),
            "-R".to_string(),
            "{repo}".to_string(),
            "--label".to_string(),
            "{labels_csv}".to_string(),
        ];
        let params: Vec<String> = vec![
            "repo".into(),
            "title".into(),
            "body".into(),
            "labels_csv".into(),
        ];
        let mut args: HashMap<String, ConfidentValue> = HashMap::new();
        args.insert(
            "_0".into(),
            ConfidentValue::deterministic(Value::Text("ncmlabs/b".into())),
        );
        args.insert(
            "_3".into(),
            ConfidentValue::deterministic(Value::Text("clone-dev,from:T7".into())),
        );

        let expanded = expand_argv(&argv, &params, &args).expect("positional args expand");

        assert_eq!(expanded[4], "ncmlabs/b");
        assert_eq!(expanded[6], "clone-dev,from:T7");
    }

    #[test]
    fn expand_argv_json_escapes_multiline_markdown() {
        let argv = vec![
            "blocks=[{{\"type\":\"section\",\"text\":{{\"type\":\"mrkdwn\",\"text\":{json:text}}}}}]"
                .to_string(),
        ];
        let params: Vec<String> = vec!["text".into()];
        let text = "Plan:\n1. Run `cargo test`\n2. Quote \"ok\"\n3. Path C:\\tmp\\forge\n4. Literal braces {issue_id}";
        let mut args: HashMap<String, ConfidentValue> = HashMap::new();
        args.insert(
            "text".into(),
            ConfidentValue::deterministic(Value::Text(text.into())),
        );

        let expanded = expand_argv(&argv, &params, &args).expect("argv expands");

        let blocks = expanded[0]
            .strip_prefix("blocks=")
            .expect("blocks prefix should remain");
        let parsed: serde_json::Value =
            serde_json::from_str(blocks).expect("blocks JSON should parse");
        assert_eq!(parsed[0]["text"]["text"], text);
    }

    fn expand_slack_send_approval(text: &str, request_id: &str) -> Vec<String> {
        std::env::set_var("SLACK_BOT_TOKEN", "xoxb-test");
        let skills =
            crate::runtime::skill_loader::SkillLoader::load_from_dirs(&[std::path::PathBuf::from(
                "skills/slack",
            )]);
        let slack = skills
            .iter()
            .find(|skill| skill.manifest.name == "slack")
            .expect("slack skill should load");
        let capability = slack
            .manifest
            .capabilities
            .iter()
            .find(|cap| cap.name == "send_approval")
            .expect("send_approval capability should exist");
        let executor = capability
            .executor
            .as_ref()
            .expect("send_approval should be deterministic");
        let mut args: HashMap<String, ConfidentValue> = HashMap::new();
        args.insert(
            "channel".into(),
            ConfidentValue::deterministic(Value::Text("C0123456789".into())),
        );
        args.insert(
            "text".into(),
            ConfidentValue::deterministic(Value::Text(text.into())),
        );
        args.insert(
            "callback_url".into(),
            ConfidentValue::deterministic(Value::Text(
                "http://localhost:3300/webhook/approval".into(),
            )),
        );
        args.insert(
            "request_id".into(),
            ConfidentValue::deterministic(Value::Text(request_id.into())),
        );

        expand_argv(&executor.argv, &executor.params, &args)
            .expect("send_approval argv should expand")
    }

    fn approval_blocks_from_argv(expanded: &[String]) -> serde_json::Value {
        let blocks_arg = expanded
            .iter()
            .find(|arg| arg.starts_with("blocks="))
            .expect("send_approval should send blocks");
        let blocks = blocks_arg
            .strip_prefix("blocks=")
            .expect("blocks prefix should remain");
        serde_json::from_str(blocks).expect("Slack blocks should be valid JSON")
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn slack_send_approval_command_posts_valid_blocks_for_escaped_text() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let fake_curl = tmp.path().join("curl");
        let curl_log = tmp.path().join("curl-argv.json");
        std::fs::write(
            &fake_curl,
            r#"#!/bin/sh
python3 - "$@" <<'PY'
import json
import os
import sys
with open(os.environ["FORGE_FAKE_CURL_LOG"], "w", encoding="utf-8") as fh:
    json.dump(sys.argv[1:], fh)
print('{"ok":true,"ts":"123.456"}')
PY
"#,
        )
        .expect("write fake curl");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = std::fs::metadata(&fake_curl)
                .expect("fake curl metadata")
                .permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(&fake_curl, permissions).expect("chmod fake curl");
        }

        std::env::set_var("FORGE_FAKE_CURL_LOG", &curl_log);

        let skills =
            crate::runtime::skill_loader::SkillLoader::load_from_dirs(&[std::path::PathBuf::from(
                "skills/slack",
            )]);
        let slack = skills
            .iter()
            .find(|skill| skill.manifest.name == "slack")
            .expect("slack skill should load");
        let capability = slack
            .manifest
            .capabilities
            .iter()
            .find(|cap| cap.name == "send_approval")
            .expect("send_approval capability should exist");
        let mut executor = capability
            .executor
            .as_ref()
            .expect("send_approval should be deterministic")
            .clone();
        executor.argv[0] = fake_curl.to_string_lossy().to_string();
        for arg in &mut executor.argv {
            *arg = arg.replace("{env:SLACK_BOT_TOKEN}", "xoxb-test");
        }

        let text =
            "PR ready\nCI: no pull requests found for branch \"clone-dev/x\"\nPath: C:\\tmp\\forge";
        let mut args: HashMap<String, ConfidentValue> = HashMap::new();
        args.insert(
            "channel".into(),
            ConfidentValue::deterministic(Value::Text("C0123456789".into())),
        );
        args.insert(
            "text".into(),
            ConfidentValue::deterministic(Value::Text(text.into())),
        );
        args.insert(
            "callback_url".into(),
            ConfidentValue::deterministic(Value::Text(
                "http://localhost:3300/webhook/approval".into(),
            )),
        );
        args.insert(
            "request_id".into(),
            ConfidentValue::deterministic(Value::Text("smoke-401".into())),
        );

        let skill_executor = SkillExecutor::new(
            std::sync::Arc::new(ProviderRegistry::new("unused")),
            std::sync::Arc::new(std::sync::Mutex::new(
                crate::runtime::skill_registry::SkillRegistry::new(),
            )),
        );
        let result = skill_executor
            .execute_deterministic(slack, "send_approval", &executor, &args)
            .await
            .expect("fake Slack command should succeed");
        assert_eq!(result.value.to_string(), "123.456");

        std::env::remove_var("FORGE_FAKE_CURL_LOG");

        let argv: Vec<String> = serde_json::from_str(
            &std::fs::read_to_string(&curl_log).expect("fake curl should log argv"),
        )
        .expect("fake curl argv should be JSON");
        let blocks_arg = argv
            .iter()
            .find(|arg| arg.starts_with("blocks="))
            .expect("send_approval should pass blocks data");
        let blocks = blocks_arg
            .strip_prefix("blocks=")
            .expect("blocks prefix should remain");
        let parsed: serde_json::Value =
            serde_json::from_str(blocks).expect("Slack blocks should be valid JSON");

        assert_eq!(parsed[0]["text"]["text"], text);
        assert_eq!(parsed[1]["elements"][0]["value"], "approved:smoke-401");
        assert_eq!(parsed[1]["elements"][1]["value"], "rejected:smoke-401");
    }

    #[test]
    fn slack_send_approval_blocks_json_survives_rich_plan_body() {
        let body =
            "Start implementation: 13\n\n```text\ncargo test -- --nocapture\n```\nQuote: \"ship it\"";
        let expanded = expand_slack_send_approval(body, "plan-13");
        let parsed = approval_blocks_from_argv(&expanded);

        assert_eq!(parsed[0]["type"], "section");
        assert_eq!(parsed[0]["text"]["type"], "mrkdwn");
        assert_eq!(parsed[0]["text"]["text"], body);
        assert_eq!(parsed[1]["type"], "actions");
        assert_eq!(parsed[1]["elements"][0]["value"], "approved:plan-13");
        assert_eq!(parsed[1]["elements"][1]["value"], "rejected:plan-13");
    }

    #[test]
    fn slack_send_approval_splits_issue_428_gate_two_plan_body() {
        let issue_33_body = r#"Issue: 33
Title: Make JSON store writes resilient to corrupt data files
Repo: ncmlabs/forge-playground
Branch: clone-dev/33

Plan:
1. Inspect the local JSON store and reproduce malformed JSON behavior.
2. Preserve the API contract while returning a structured 500 response.
3. Implement temp-file-and-rename writes so partial writes cannot corrupt the store.
4. Add tests for quoted errors like "Unexpected token } in JSON at position 42".
5. Run `npm run typecheck`, `npm test`, and `npm run build`.

Acceptance criteria:
- [ ] Malformed JSON produces a structured API 500 response with a safe error code.
- [ ] Writes use a temp-file-and-rename flow so partial writes do not corrupt the store.
- [ ] Tests cover malformed JSON read behavior.
- [ ] Tests cover successful status update still persists after the write change.
"#;
        let quote_heavy_plan = format!(
            "{}\n{}",
            issue_33_body,
            "Review note: quote \"safe_error_code\" and path `data/tasks.json`.\n".repeat(120)
        );

        let expanded = expand_slack_send_approval(&quote_heavy_plan, "plan-33");
        let parsed = approval_blocks_from_argv(&expanded);
        let blocks = parsed.as_array().expect("blocks should be an array");

        assert!(
            blocks.len() > 2,
            "long approval body should split into sections"
        );
        assert!(blocks.len() <= SLACK_MESSAGE_BLOCK_LIMIT);
        for block in blocks.iter().filter(|b| b["type"] == "section") {
            let text = block["text"]["text"].as_str().expect("section text");
            assert!(
                text.chars().count() <= SLACK_SECTION_TEXT_LIMIT,
                "section had {} chars",
                text.chars().count()
            );
        }

        let all_text = blocks
            .iter()
            .filter_map(|b| b["text"]["text"].as_str())
            .collect::<String>();
        assert!(all_text.contains("Malformed JSON produces a structured API 500 response"));
        assert!(all_text.contains("quote \"safe_error_code\""));
        assert_eq!(
            blocks.last().unwrap()["elements"][0]["value"],
            "approved:plan-33"
        );
        assert_eq!(
            blocks.last().unwrap()["elements"][1]["value"],
            "rejected:plan-33"
        );
    }

    #[test]
    fn slack_send_approval_truncates_before_block_limit() {
        let body = "Approval paragraph with \"quotes\" and `code`.\n".repeat(5000);
        let expanded = expand_slack_send_approval(&body, "plan-too-long");
        let parsed = approval_blocks_from_argv(&expanded);
        let blocks = parsed.as_array().expect("blocks should be an array");

        assert_eq!(blocks.len(), SLACK_MESSAGE_BLOCK_LIMIT);
        let last_section = &blocks[SLACK_MESSAGE_BLOCK_LIMIT - 2];
        let text = last_section["text"]["text"]
            .as_str()
            .expect("last section text");
        assert!(text.contains(SLACK_APPROVAL_TRUNCATION_NOTE));
        assert!(text.chars().count() <= SLACK_SECTION_TEXT_LIMIT);
        assert_eq!(
            blocks.last().unwrap()["elements"][0]["value"],
            "approved:plan-too-long"
        );
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn slack_send_approval_invalid_blocks_error_includes_response_and_summary() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let fake_curl = tmp.path().join("curl");
        std::fs::write(
            &fake_curl,
            r#"#!/bin/sh
printf '%s\n' '{"ok":false,"error":"invalid_blocks","response_metadata":{"messages":["section text too long"]}}'
"#,
        )
        .expect("write fake curl");
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = std::fs::metadata(&fake_curl)
                .expect("fake curl metadata")
                .permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(&fake_curl, permissions).expect("chmod fake curl");
        }

        let skills =
            crate::runtime::skill_loader::SkillLoader::load_from_dirs(&[std::path::PathBuf::from(
                "skills/slack",
            )]);
        let slack = skills
            .iter()
            .find(|skill| skill.manifest.name == "slack")
            .expect("slack skill should load");
        let capability = slack
            .manifest
            .capabilities
            .iter()
            .find(|cap| cap.name == "send_approval")
            .expect("send_approval capability should exist");
        let mut executor = capability
            .executor
            .as_ref()
            .expect("send_approval should be deterministic")
            .clone();
        executor.argv[0] = fake_curl.to_string_lossy().to_string();
        for arg in &mut executor.argv {
            *arg = arg.replace("{env:SLACK_BOT_TOKEN}", "xoxb-test");
        }

        let mut args: HashMap<String, ConfidentValue> = HashMap::new();
        args.insert(
            "channel".into(),
            ConfidentValue::deterministic(Value::Text("C0123456789".into())),
        );
        args.insert(
            "text".into(),
            ConfidentValue::deterministic(Value::Text(
                "Start implementation: 33\n\nQuote: \"safe_error_code\"".into(),
            )),
        );
        args.insert(
            "callback_url".into(),
            ConfidentValue::deterministic(Value::Text(
                "http://localhost:3300/webhook/approval".into(),
            )),
        );
        args.insert(
            "request_id".into(),
            ConfidentValue::deterministic(Value::Text("plan-33".into())),
        );

        let skill_executor = SkillExecutor::new(
            std::sync::Arc::new(ProviderRegistry::new("unused")),
            std::sync::Arc::new(std::sync::Mutex::new(
                crate::runtime::skill_registry::SkillRegistry::new(),
            )),
        );
        let err = skill_executor
            .execute_deterministic(slack, "send_approval", &executor, &args)
            .await
            .expect_err("fake Slack invalid_blocks should fail");
        let msg = err.to_string();

        assert!(msg.contains("invalid_blocks"));
        assert!(msg.contains(
            r#"slack_response={"ok":false,"error":"invalid_blocks","response_metadata":{"messages":["section text too long"]}}"#
        ));
        assert!(msg.contains("block_payload_summary=blocks=2 sections=1"));
        assert!(msg.contains("max_section_chars="));
        assert!(msg.contains("action_values=approved:plan-33|rejected:plan-33"));
    }
}
