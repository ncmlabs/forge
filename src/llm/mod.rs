// FORGE LLM backend abstraction
// See issue #8 and providers.md for full specification

pub mod cost_tracker;
pub mod providers;
pub mod registry;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use thiserror::Error;

// ── Capability hints ──────────────────────────────────────────────────────────
// Used in FORGE source: `reason "..." with quality: high, local_only: true`
// The runtime finds the best provider that satisfies these constraints.

#[derive(Debug, Clone, Default)]
pub struct CapabilityHint {
    pub min_context_tokens: Option<u32>,
    pub quality: Option<QualityTier>,
    pub local_only: bool,
    pub max_cost_per_call: Option<f32>,
    pub provider_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, PartialOrd, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QualityTier {
    Fast,
    Balanced,
    High,
}

// ── Provider capabilities ─────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ProviderCapabilities {
    pub max_context_tokens: u32,
    pub quality_tier: QualityTier,
    pub local: bool,
    pub cost_per_1k_input_tokens: f32,
    pub cost_per_1k_output_tokens: f32,
    pub supports_streaming: bool,
    pub supports_function_calling: bool,
    pub supports_json_mode: bool,
    pub max_output_tokens: u32,
}

impl Default for ProviderCapabilities {
    fn default() -> Self {
        Self {
            max_context_tokens: 8_192,
            quality_tier: QualityTier::Balanced,
            local: false,
            cost_per_1k_input_tokens: 0.001,
            cost_per_1k_output_tokens: 0.001,
            supports_streaming: true,
            supports_function_calling: true,
            supports_json_mode: true,
            max_output_tokens: 4_096,
        }
    }
}

// ── Request / Response ────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct CompletionRequest {
    pub prompt: String,
    pub system: Option<String>,
    pub max_tokens: u32,
    pub temperature: f32,
    pub stop_sequences: Vec<String>,
    pub json_mode: bool,
}

impl CompletionRequest {
    pub fn simple(prompt: impl Into<String>) -> Self {
        Self {
            prompt: prompt.into(),
            system: None,
            max_tokens: 4096,
            temperature: 0.7,
            stop_sequences: vec![],
            json_mode: false,
        }
    }

    pub fn with_system(mut self, system: impl Into<String>) -> Self {
        self.system = Some(system.into());
        self
    }

    pub fn with_temperature(mut self, t: f32) -> Self {
        self.temperature = t;
        self
    }

    pub fn with_max_tokens(mut self, n: u32) -> Self {
        self.max_tokens = n;
        self
    }
}

#[derive(Debug, Clone)]
pub struct CompletionResponse {
    pub content: String,
    pub tokens_in: u32,
    pub tokens_out: u32,
    pub latency_ms: u64,
    pub model_used: String,
    pub provider_name: String,
    pub cost_usd: f32,
}

impl CompletionResponse {
    /// Heuristic confidence estimate for POC — replace with logprobs when available.
    pub fn estimate_confidence(&self) -> f32 {
        let hedging_phrases = [
            "i'm not sure",
            "i think",
            "possibly",
            "might be",
            "i cannot",
            "i don't know",
            "unclear",
            "it depends",
        ];
        let lower = self.content.to_lowercase();
        let hedge_count = hedging_phrases
            .iter()
            .filter(|p| lower.contains(*p))
            .count();

        (0.85 - (hedge_count as f32 * 0.08)).max(0.3)
    }
}

// ── Error types ───────────────────────────────────────────────────────────────

#[derive(Debug, Error, Clone)]
pub enum ProviderError {
    #[error("provider '{provider}' is unavailable: {reason}")]
    Unavailable { provider: String, reason: String },

    #[error("rate limited by '{provider}', retry after {retry_after_secs}s")]
    RateLimited {
        provider: String,
        retry_after_secs: u64,
    },

    #[error("request exceeded context window ({tokens} tokens, max {max})")]
    ContextTooLong { tokens: u32, max: u32 },

    #[error("provider '{provider}' rejected request: {reason}")]
    Rejected { provider: String, reason: String },

    #[error("cost limit exceeded: estimated ${estimate:.4}, budget ${budget:.4}")]
    BudgetExceeded { estimate: f32, budget: f32 },

    #[error("no provider satisfies capability requirements: {requirements}")]
    NoSatisfyingProvider { requirements: String },

    #[error("provider '{provider}' returned invalid response: {reason}")]
    InvalidResponse { provider: String, reason: String },

    #[error("request timed out after {secs}s")]
    Timeout { secs: u64 },

    #[error("network error: {0}")]
    Network(String),
}

// ── Tool-use types (issue #40 — skill bridge) ───────────────────────────────

/// Definition of a tool the LLM can call during skill execution.
#[derive(Debug, Clone, Serialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

/// A tool call requested by the LLM.
#[derive(Debug, Clone, Deserialize)]
pub struct ToolCallRequest {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

/// Response from an LLM call that may include tool use requests.
#[derive(Debug, Clone)]
pub struct CompletionWithToolsResponse {
    pub content: String,
    pub tool_calls: Vec<ToolCallRequest>,
    pub tokens_in: u32,
    pub tokens_out: u32,
    pub latency_ms: u64,
    pub cost_usd: f32,
    pub model_used: String,
    pub provider_name: String,
}

impl CompletionWithToolsResponse {
    /// Heuristic confidence (same as CompletionResponse).
    pub fn estimate_confidence(&self) -> f32 {
        let hedging_phrases = [
            "i'm not sure",
            "i think",
            "possibly",
            "might be",
            "i cannot",
            "i don't know",
            "unclear",
            "it depends",
        ];
        let lower = self.content.to_lowercase();
        let hedge_count = hedging_phrases
            .iter()
            .filter(|p| lower.contains(*p))
            .count();
        (0.85 - (hedge_count as f32 * 0.08)).max(0.3)
    }
}

// ── The trait every provider implements ────────────────────────────────────────

#[async_trait]
pub trait LLMProvider: Send + Sync {
    fn name(&self) -> &str;
    fn capabilities(&self) -> &ProviderCapabilities;

    async fn complete(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, ProviderError>;

    /// Complete with tool-use support (for skill execution).
    /// Default implementation falls back to regular complete() with no tool calls.
    async fn complete_with_tools(
        &self,
        request: CompletionRequest,
        _tools: &[ToolDefinition],
    ) -> Result<CompletionWithToolsResponse, ProviderError> {
        let resp = self.complete(request).await?;
        Ok(CompletionWithToolsResponse {
            content: resp.content,
            tool_calls: vec![],
            tokens_in: resp.tokens_in,
            tokens_out: resp.tokens_out,
            latency_ms: resp.latency_ms,
            cost_usd: resp.cost_usd,
            model_used: resp.model_used,
            provider_name: resp.provider_name,
        })
    }

    async fn health_check(&self) -> Result<(), ProviderError> {
        self.complete(CompletionRequest::simple("ping").with_max_tokens(1))
            .await?;
        Ok(())
    }

    fn estimate_cost(&self, prompt_tokens: u32, max_output_tokens: u32) -> f32 {
        let caps = self.capabilities();
        let input_cost = (prompt_tokens as f32 / 1000.0) * caps.cost_per_1k_input_tokens;
        let output_cost = (max_output_tokens as f32 / 1000.0) * caps.cost_per_1k_output_tokens;
        input_cost + output_cost
    }
}

pub type BoxedProvider = Arc<dyn LLMProvider>;
