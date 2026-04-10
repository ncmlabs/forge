// FORGE skill executor — issue #40
// Executes skills via an LLM-mediated agentic loop.
// The LLM reads SKILL.md instructions, requests tool calls (bash, HTTP),
// tools execute, results return to LLM, repeat until done.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use crate::llm::registry::ProviderRegistry;
use crate::llm::{CompletionRequest, ToolDefinition};
use crate::runtime::confidence::ConfidentValue;
use crate::runtime::skill::{LoadedSkill, SkillError};
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
                .any(|t| t == "Bash" || t == "bash_exec");
        let has_http = allowed_tools.is_empty()
            || allowed_tools
                .iter()
                .any(|t| t == "WebFetch" || t == "http_request");

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
