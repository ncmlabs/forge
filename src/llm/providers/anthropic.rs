use crate::config::ProviderConfig;
use crate::llm::{
    CompletionRequest, CompletionResponse, LLMProvider, ProviderCapabilities, ProviderError,
    QualityTier,
};
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Instant;

pub struct AnthropicProvider {
    name: String,
    client: Client,
    api_key: String,
    model: String,
    caps: ProviderCapabilities,
    timeout: std::time::Duration,
}

impl AnthropicProvider {
    pub fn new(
        name: &str,
        api_key: &str,
        model: &str,
        config: &ProviderConfig,
    ) -> Result<Self, String> {
        let caps = Self::caps_for_model(model, &config.capabilities);
        Ok(Self {
            name: name.to_string(),
            client: Client::new(),
            api_key: api_key.to_string(),
            model: model.to_string(),
            caps,
            timeout: std::time::Duration::from_secs(config.timeout_secs.unwrap_or(30)),
        })
    }

    fn caps_for_model(
        model: &str,
        overrides: &Option<crate::config::CapabilityOverride>,
    ) -> ProviderCapabilities {
        let mut caps = match model {
            m if m.contains("haiku") => ProviderCapabilities {
                max_context_tokens: 200_000,
                quality_tier: QualityTier::Fast,
                cost_per_1k_input_tokens: 0.00025,
                cost_per_1k_output_tokens: 0.00125,
                ..Default::default()
            },
            m if m.contains("sonnet") => ProviderCapabilities {
                max_context_tokens: 200_000,
                quality_tier: QualityTier::Balanced,
                cost_per_1k_input_tokens: 0.003,
                cost_per_1k_output_tokens: 0.015,
                ..Default::default()
            },
            m if m.contains("opus") => ProviderCapabilities {
                max_context_tokens: 200_000,
                quality_tier: QualityTier::High,
                cost_per_1k_input_tokens: 0.015,
                cost_per_1k_output_tokens: 0.075,
                ..Default::default()
            },
            _ => ProviderCapabilities::default(),
        };

        if let Some(ov) = overrides {
            if let Some(ctx) = ov.max_context_tokens {
                caps.max_context_tokens = ctx;
            }
            if let Some(qt) = &ov.quality_tier {
                caps.quality_tier = qt.clone();
            }
            if let Some(c) = ov.cost_per_1k_input {
                caps.cost_per_1k_input_tokens = c;
            }
            if let Some(c) = ov.cost_per_1k_output {
                caps.cost_per_1k_output_tokens = c;
            }
        }

        caps
    }
}

// ── Anthropic wire types ──────────────────────────────────────────────────────

#[derive(Serialize)]
struct AnthropicRequest<'a> {
    model: &'a str,
    max_tokens: u32,
    messages: Vec<AnthropicMessage<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<&'a str>,
    temperature: f32,
    #[serde(skip_serializing_if = "<[String]>::is_empty")]
    stop_sequences: &'a [String],
}

#[derive(Serialize)]
struct AnthropicMessage<'a> {
    role: &'static str,
    content: &'a str,
}

#[derive(Deserialize)]
struct AnthropicResponse {
    content: Vec<AnthropicContent>,
    usage: AnthropicUsage,
    model: String,
}

#[derive(Deserialize)]
struct AnthropicContent {
    #[serde(rename = "type")]
    type_: String,
    text: Option<String>,
}

#[derive(Deserialize)]
struct AnthropicUsage {
    input_tokens: u32,
    output_tokens: u32,
}

#[derive(Deserialize)]
struct AnthropicError {
    error: AnthropicErrorBody,
}

#[derive(Deserialize)]
struct AnthropicErrorBody {
    #[serde(rename = "type")]
    #[allow(dead_code)]
    type_: String,
    message: String,
}

// ── Implementation ────────────────────────────────────────────────────────────

#[async_trait]
impl LLMProvider for AnthropicProvider {
    fn name(&self) -> &str {
        &self.name
    }
    fn capabilities(&self) -> &ProviderCapabilities {
        &self.caps
    }

    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, ProviderError> {
        let body = AnthropicRequest {
            model: &self.model,
            max_tokens: req.max_tokens,
            messages: vec![AnthropicMessage {
                role: "user",
                content: &req.prompt,
            }],
            system: req.system.as_deref(),
            temperature: req.temperature,
            stop_sequences: &req.stop_sequences,
        };

        let start = Instant::now();

        let http_resp = self
            .client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .timeout(self.timeout)
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    ProviderError::Timeout {
                        secs: self.timeout.as_secs(),
                    }
                } else {
                    ProviderError::Network(e.to_string())
                }
            })?;

        let status = http_resp.status();
        let latency_ms = start.elapsed().as_millis() as u64;

        if status == 429 {
            let retry_after = http_resp
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(60);
            return Err(ProviderError::RateLimited {
                provider: self.name.clone(),
                retry_after_secs: retry_after,
            });
        }

        if !status.is_success() {
            let err: AnthropicError = http_resp.json().await.unwrap_or(AnthropicError {
                error: AnthropicErrorBody {
                    type_: "unknown".to_string(),
                    message: format!("HTTP {}", status),
                },
            });
            return Err(ProviderError::Rejected {
                provider: self.name.clone(),
                reason: err.error.message,
            });
        }

        let resp: AnthropicResponse =
            http_resp
                .json()
                .await
                .map_err(|e| ProviderError::InvalidResponse {
                    provider: self.name.clone(),
                    reason: e.to_string(),
                })?;

        let content = resp
            .content
            .into_iter()
            .find(|c| c.type_ == "text")
            .and_then(|c| c.text)
            .unwrap_or_default();

        let tokens_in = resp.usage.input_tokens;
        let tokens_out = resp.usage.output_tokens;
        let cost_usd = (tokens_in as f32 / 1000.0) * self.caps.cost_per_1k_input_tokens
            + (tokens_out as f32 / 1000.0) * self.caps.cost_per_1k_output_tokens;

        Ok(CompletionResponse {
            content,
            tool_calls: vec![],
            tokens_in,
            tokens_out,
            latency_ms,
            model_used: resp.model,
            provider_name: self.name.clone(),
            cost_usd,
        })
    }
}
