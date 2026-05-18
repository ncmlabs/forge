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

                let text = apply_result_mapping(
                    executor.result.as_ref(),
                    &stdout,
                    &format!("{}.{}", skill.manifest.name, method),
                )?;
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
        let text = "Plan:\n1. Run `cargo test`\n2. Quote \"ok\"\n3. Literal braces {issue_id}";
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

    #[test]
    fn slack_send_approval_blocks_json_survives_rich_plan_body() {
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
            ConfidentValue::deterministic(Value::Text(
                "Start implementation: 13\n\n```text\ncargo test -- --nocapture\n```\nQuote: \"ship it\""
                    .into(),
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
            ConfidentValue::deterministic(Value::Text("plan-13".into())),
        );

        let expanded = expand_argv(&executor.argv, &executor.params, &args)
            .expect("send_approval argv should expand");
        let blocks_arg = expanded
            .iter()
            .find(|arg| arg.starts_with("blocks="))
            .expect("send_approval should send blocks");
        let blocks = blocks_arg
            .strip_prefix("blocks=")
            .expect("blocks prefix should remain");
        let parsed: serde_json::Value =
            serde_json::from_str(blocks).expect("Slack blocks should be valid JSON");

        assert_eq!(parsed[0]["type"], "section");
        assert_eq!(parsed[0]["text"]["type"], "mrkdwn");
        assert_eq!(
            parsed[0]["text"]["text"],
            "Start implementation: 13\n\n```text\ncargo test -- --nocapture\n```\nQuote: \"ship it\""
        );
        assert_eq!(parsed[1]["type"], "actions");
        assert_eq!(parsed[1]["elements"][0]["value"], "approved:plan-13");
        assert_eq!(parsed[1]["elements"][1]["value"], "rejected:plan-13");
    }
}
