use crate::config::ProviderConfig;
use crate::llm::{
    CompletionRequest, CompletionResponse, EmbeddingProvider, EmbeddingRequest, EmbeddingResponse,
    LLMProvider, ProviderCapabilities, ProviderError, QualityTier,
};
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Instant;

pub struct OpenAICompatProvider {
    name: String,
    client: Client,
    base_url: String,
    api_key: String,
    model: String,
    caps: ProviderCapabilities,
    timeout: std::time::Duration,
}

impl OpenAICompatProvider {
    pub fn new(
        name: &str,
        api_key: &str,
        model: &str,
        config: &ProviderConfig,
    ) -> Result<Self, String> {
        let base_url = config
            .base_url
            .clone()
            .ok_or("openai-compat provider requires base_url")?;
        let base_url = base_url.trim_end_matches('/').to_string();

        let is_local = base_url.contains("localhost")
            || base_url.contains("127.0.0.1")
            || base_url.contains("0.0.0.0");

        let caps = Self::build_caps(model, is_local, &config.capabilities);

        Ok(Self {
            name: name.to_string(),
            client: Client::new(),
            base_url,
            api_key: api_key.to_string(),
            model: model.to_string(),
            caps,
            timeout: std::time::Duration::from_secs(config.timeout_secs.unwrap_or(60)),
        })
    }

    fn build_caps(
        model: &str,
        local: bool,
        overrides: &Option<crate::config::CapabilityOverride>,
    ) -> ProviderCapabilities {
        let mut caps = if local {
            ProviderCapabilities {
                max_context_tokens: 8_192,
                quality_tier: QualityTier::Balanced,
                local: true,
                cost_per_1k_input_tokens: 0.0,
                cost_per_1k_output_tokens: 0.0,
                ..Default::default()
            }
        } else {
            let is_fast = model.contains("8b")
                || model.contains("7b")
                || model.contains("3b")
                || model.contains("mini");
            ProviderCapabilities {
                max_context_tokens: 32_768,
                quality_tier: if is_fast {
                    QualityTier::Fast
                } else {
                    QualityTier::Balanced
                },
                local: false,
                cost_per_1k_input_tokens: 0.0001,
                cost_per_1k_output_tokens: 0.0001,
                ..Default::default()
            }
        };

        if let Some(ov) = overrides {
            if let Some(ctx) = ov.max_context_tokens {
                caps.max_context_tokens = ctx;
            }
            if let Some(qt) = &ov.quality_tier {
                caps.quality_tier = qt.clone();
            }
            if let Some(l) = ov.local {
                caps.local = l;
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

// ── OpenAI wire types ─────────────────────────────────────────────────────────

#[derive(Serialize)]
struct OAIRequest<'a> {
    model: &'a str,
    messages: Vec<OAIMessage<'a>>,
    max_tokens: u32,
    temperature: f32,
    #[serde(skip_serializing_if = "<[String]>::is_empty")]
    stop: &'a [String],
    stream: bool,
}

#[derive(Serialize)]
struct OAIMessage<'a> {
    role: &'static str,
    content: &'a str,
}

#[derive(Deserialize)]
struct OAIResponse {
    choices: Vec<OAIChoice>,
    usage: OAIUsage,
    model: String,
}

#[derive(Deserialize)]
struct OAIChoice {
    message: OAIResponseMessage,
}

#[derive(Deserialize)]
struct OAIResponseMessage {
    content: Option<String>,
}

#[derive(Deserialize)]
struct OAIUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
}

#[derive(Deserialize)]
struct OAIError {
    error: OAIErrorBody,
}

#[derive(Deserialize)]
struct OAIErrorBody {
    message: String,
    code: Option<String>,
}

// ── Implementation ────────────────────────────────────────────────────────────

#[async_trait]
impl LLMProvider for OpenAICompatProvider {
    fn name(&self) -> &str {
        &self.name
    }
    fn capabilities(&self) -> &ProviderCapabilities {
        &self.caps
    }

    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, ProviderError> {
        let mut messages: Vec<OAIMessage> = vec![];
        if let Some(sys) = &req.system {
            messages.push(OAIMessage {
                role: "system",
                content: sys,
            });
        }
        messages.push(OAIMessage {
            role: "user",
            content: &req.prompt,
        });

        let body = OAIRequest {
            model: &self.model,
            messages,
            max_tokens: req.max_tokens,
            temperature: req.temperature,
            stop: &req.stop_sequences,
            stream: false,
        };

        let url = format!("{}/chat/completions", self.base_url);
        let start = Instant::now();

        let http_resp = self
            .client
            .post(&url)
            .header("authorization", format!("Bearer {}", self.api_key))
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
                } else if e.is_connect() {
                    ProviderError::Unavailable {
                        provider: self.name.clone(),
                        reason: format!("cannot connect to {}: {}", self.base_url, e),
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
                .unwrap_or(30);
            return Err(ProviderError::RateLimited {
                provider: self.name.clone(),
                retry_after_secs: retry_after,
            });
        }

        if !status.is_success() {
            let err: OAIError = http_resp.json().await.unwrap_or(OAIError {
                error: OAIErrorBody {
                    message: format!("HTTP {}", status),
                    code: None,
                },
            });

            if err.error.code.as_deref() == Some("context_length_exceeded") {
                return Err(ProviderError::ContextTooLong {
                    tokens: 0,
                    max: self.caps.max_context_tokens,
                });
            }

            return Err(ProviderError::Rejected {
                provider: self.name.clone(),
                reason: err.error.message,
            });
        }

        let resp: OAIResponse =
            http_resp
                .json()
                .await
                .map_err(|e| ProviderError::InvalidResponse {
                    provider: self.name.clone(),
                    reason: e.to_string(),
                })?;

        let content = resp
            .choices
            .into_iter()
            .next()
            .and_then(|c| c.message.content)
            .unwrap_or_default();

        let tokens_in = resp.usage.prompt_tokens;
        let tokens_out = resp.usage.completion_tokens;
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

// ── OpenAI-compatible Embedding Provider ─────────────────────────────────────

pub struct OpenAICompatEmbeddingProvider {
    name: String,
    client: Client,
    base_url: String,
    api_key: String,
    model: String,
    dimensions: usize,
    cost_per_1k_tokens: f32,
    timeout: std::time::Duration,
}

impl OpenAICompatEmbeddingProvider {
    pub fn new(
        name: &str,
        api_key: &str,
        model: &str,
        base_url: &str,
        dimensions: usize,
        cost_per_1k_tokens: f32,
        timeout_secs: u64,
    ) -> Self {
        Self {
            name: name.to_string(),
            client: Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.to_string(),
            model: model.to_string(),
            dimensions,
            cost_per_1k_tokens,
            timeout: std::time::Duration::from_secs(timeout_secs),
        }
    }
}

#[derive(Serialize)]
struct OAIEmbeddingRequest<'a> {
    model: &'a str,
    input: &'a [String],
}

#[derive(Deserialize)]
struct OAIEmbeddingResponse {
    data: Vec<OAIEmbeddingData>,
    usage: OAIEmbeddingUsage,
    model: String,
}

#[derive(Deserialize)]
struct OAIEmbeddingData {
    embedding: Vec<f32>,
    #[allow(dead_code)]
    index: usize,
}

#[derive(Deserialize)]
struct OAIEmbeddingUsage {
    prompt_tokens: u32,
    #[allow(dead_code)]
    total_tokens: u32,
}

#[async_trait]
impl EmbeddingProvider for OpenAICompatEmbeddingProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn embedding_dimensions(&self) -> usize {
        self.dimensions
    }

    async fn embed(&self, request: EmbeddingRequest) -> Result<EmbeddingResponse, ProviderError> {
        let body = OAIEmbeddingRequest {
            model: request.model.as_deref().unwrap_or(&self.model),
            input: &request.texts,
        };

        let url = format!("{}/embeddings", self.base_url);

        let http_resp = self
            .client
            .post(&url)
            .header("authorization", format!("Bearer {}", self.api_key))
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
                } else if e.is_connect() {
                    ProviderError::Unavailable {
                        provider: self.name.clone(),
                        reason: format!("cannot connect to {}: {}", self.base_url, e),
                    }
                } else {
                    ProviderError::Network(e.to_string())
                }
            })?;

        let status = http_resp.status();

        if status == 429 {
            let retry_after = http_resp
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(30);
            return Err(ProviderError::RateLimited {
                provider: self.name.clone(),
                retry_after_secs: retry_after,
            });
        }

        if !status.is_success() {
            let err: OAIError = http_resp.json().await.unwrap_or(OAIError {
                error: OAIErrorBody {
                    message: format!("HTTP {}", status),
                    code: None,
                },
            });
            return Err(ProviderError::Rejected {
                provider: self.name.clone(),
                reason: err.error.message,
            });
        }

        let resp: OAIEmbeddingResponse =
            http_resp
                .json()
                .await
                .map_err(|e| ProviderError::InvalidResponse {
                    provider: self.name.clone(),
                    reason: e.to_string(),
                })?;

        let tokens_used = resp.usage.prompt_tokens;
        let cost_usd = (tokens_used as f32 / 1000.0) * self.cost_per_1k_tokens;

        let mut embeddings: Vec<Vec<f32>> = resp.data.into_iter().map(|d| d.embedding).collect();

        // Sort by index to ensure order matches input
        // (OpenAI API may return embeddings out of order)
        embeddings.truncate(request.texts.len());

        Ok(EmbeddingResponse {
            embeddings,
            model_used: resp.model,
            tokens_used,
            cost_usd,
        })
    }
}
