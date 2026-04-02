use crate::llm::{
    CompletionRequest, CompletionResponse, LLMProvider, ProviderCapabilities, ProviderError,
    QualityTier,
};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

pub struct MockProvider {
    name: String,
    caps: ProviderCapabilities,
    responses: HashMap<String, String>,
    default_response: String,
    sequence: Option<(Vec<String>, Arc<AtomicUsize>)>,
}

impl MockProvider {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            caps: ProviderCapabilities {
                max_context_tokens: 1_000_000,
                quality_tier: QualityTier::High,
                local: true,
                cost_per_1k_input_tokens: 0.0,
                cost_per_1k_output_tokens: 0.0,
                ..Default::default()
            },
            responses: HashMap::new(),
            default_response: "mock response".to_string(),
            sequence: None,
        }
    }

    /// Add a pattern match: if prompt contains `pattern`, return `response`
    pub fn with_response(mut self, pattern: &str, response: &str) -> Self {
        self.responses
            .insert(pattern.to_string(), response.to_string());
        self
    }

    pub fn with_default(mut self, response: &str) -> Self {
        self.default_response = response.to_string();
        self
    }

    /// Return responses in round-robin order from the given sequence.
    /// Takes priority over pattern-based and default responses.
    pub fn with_responses_sequence(mut self, responses: Vec<String>) -> Self {
        self.sequence = Some((responses, Arc::new(AtomicUsize::new(0))));
        self
    }
}

#[async_trait]
impl LLMProvider for MockProvider {
    fn name(&self) -> &str {
        &self.name
    }
    fn capabilities(&self) -> &ProviderCapabilities {
        &self.caps
    }

    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, ProviderError> {
        let content = if let Some((ref seq, ref counter)) = self.sequence {
            let idx = counter.fetch_add(1, Ordering::Relaxed) % seq.len();
            seq[idx].clone()
        } else {
            self.responses
                .iter()
                .find(|(pattern, _)| req.prompt.contains(pattern.as_str()))
                .map(|(_, resp)| resp.clone())
                .unwrap_or_else(|| self.default_response.clone())
        };

        Ok(CompletionResponse {
            tokens_in: (req.prompt.len() / 4) as u32,
            tokens_out: (content.len() / 4) as u32,
            content,
            latency_ms: 1,
            model_used: "mock-model".to_string(),
            provider_name: self.name.clone(),
            cost_usd: 0.0,
        })
    }

    async fn health_check(&self) -> Result<(), ProviderError> {
        Ok(())
    }
}
