use crate::config::ForgeConfig;
use crate::llm::providers::build_provider;
use crate::llm::{
    BoxedProvider, CapabilityHint, CompletionRequest, CompletionResponse,
    CompletionWithToolsResponse, LLMProvider, ProviderError, ToolDefinition,
};
use std::collections::HashMap;

pub struct ProviderRegistry {
    providers: HashMap<String, BoxedProvider>,
    fallbacks: HashMap<String, String>,
    default: String,
    #[allow(dead_code)]
    routing: HashMap<String, String>,
}

impl ProviderRegistry {
    pub fn from_config(mut config: ForgeConfig) -> Result<Self, ProviderError> {
        config.resolve_env_vars();
        let mut registry = Self {
            providers: HashMap::new(),
            fallbacks: HashMap::new(),
            default: config.llm.default.clone(),
            routing: config.llm.routing.unwrap_or_default(),
        };

        for (name, provider_config) in &config.providers {
            let provider =
                build_provider(name, provider_config).map_err(|e| ProviderError::Unavailable {
                    provider: name.clone(),
                    reason: e.to_string(),
                })?;
            registry.providers.insert(name.clone(), provider);

            if let Some(fb) = &provider_config.fallback {
                registry.fallbacks.insert(name.clone(), fb.clone());
            }
        }

        Ok(registry)
    }

    /// Manual builder for tests
    pub fn new(default: &str) -> Self {
        Self {
            providers: HashMap::new(),
            fallbacks: HashMap::new(),
            default: default.to_string(),
            routing: HashMap::new(),
        }
    }

    pub fn register(&mut self, name: &str, provider: BoxedProvider) {
        self.providers.insert(name.to_string(), provider);
    }

    pub fn set_fallback(&mut self, from: &str, to: &str) {
        self.fallbacks.insert(from.to_string(), to.to_string());
    }

    pub async fn resolve_and_complete(
        &self,
        request: CompletionRequest,
        hint: Option<&CapabilityHint>,
    ) -> Result<CompletionResponse, ProviderError> {
        let provider_name = self.choose_provider(hint)?;
        self.try_with_fallback(&provider_name, request).await
    }

    fn choose_provider(&self, hint: Option<&CapabilityHint>) -> Result<String, ProviderError> {
        match hint {
            Some(h) if h.provider_name.is_some() => {
                let name = h.provider_name.as_ref().unwrap();
                if self.providers.contains_key(name) {
                    Ok(name.clone())
                } else {
                    Err(ProviderError::NoSatisfyingProvider {
                        requirements: format!("explicitly requested '{}' but not configured", name),
                    })
                }
            }
            Some(h) => self.find_best(h),
            None => Ok(self.default.clone()),
        }
    }

    fn find_best(&self, hint: &CapabilityHint) -> Result<String, ProviderError> {
        let mut candidates: Vec<(&String, &BoxedProvider)> = self
            .providers
            .iter()
            .filter(|(_, p)| self.satisfies(p.as_ref(), hint))
            .collect();

        if candidates.is_empty() {
            return Err(ProviderError::NoSatisfyingProvider {
                requirements: format!("{:?}", hint),
            });
        }

        candidates.sort_by(|(_, a), (_, b)| {
            let cost_a = a.capabilities().cost_per_1k_input_tokens;
            let cost_b = b.capabilities().cost_per_1k_input_tokens;
            cost_a.partial_cmp(&cost_b).unwrap()
        });

        Ok(candidates[0].0.clone())
    }

    fn satisfies(&self, provider: &dyn LLMProvider, hint: &CapabilityHint) -> bool {
        let caps = provider.capabilities();

        if hint.local_only && !caps.local {
            return false;
        }

        if let Some(min_ctx) = hint.min_context_tokens {
            if caps.max_context_tokens < min_ctx {
                return false;
            }
        }

        if let Some(required) = &hint.quality {
            if caps.quality_tier < *required {
                return false;
            }
        }

        true
    }

    async fn try_with_fallback(
        &self,
        start_name: &str,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, ProviderError> {
        let mut current_name = start_name.to_string();

        loop {
            let provider =
                self.providers
                    .get(&current_name)
                    .ok_or_else(|| ProviderError::Unavailable {
                        provider: current_name.clone(),
                        reason: "not found in registry".to_string(),
                    })?;

            match provider.complete(request.clone()).await {
                Ok(resp) => return Ok(resp),
                Err(e) => {
                    eprintln!(
                        "[forge] provider '{}' failed: {}. {}",
                        current_name,
                        e,
                        self.fallbacks
                            .get(&current_name)
                            .map(|f| format!("Trying fallback '{}'.", f))
                            .unwrap_or_else(|| "No fallback configured.".to_string())
                    );

                    match self.fallbacks.get(&current_name) {
                        Some(fallback) => current_name = fallback.clone(),
                        None => return Err(e),
                    }
                }
            }
        }
    }

    /// Resolve provider and complete with tool-use support (for skill execution).
    pub async fn resolve_and_complete_with_tools(
        &self,
        request: CompletionRequest,
        tools: &[ToolDefinition],
        hint: Option<&CapabilityHint>,
    ) -> Result<CompletionWithToolsResponse, ProviderError> {
        let provider_name = self.choose_provider(hint)?;
        let provider =
            self.providers
                .get(&provider_name)
                .ok_or_else(|| ProviderError::Unavailable {
                    provider: provider_name.clone(),
                    reason: "not found in registry".to_string(),
                })?;
        provider.complete_with_tools(request, tools).await
    }

    pub fn get(&self, name: &str) -> Option<&BoxedProvider> {
        self.providers.get(name)
    }

    pub async fn health_check_all(&self) -> HashMap<String, Result<(), ProviderError>> {
        let mut results = HashMap::new();
        for (name, provider) in &self.providers {
            results.insert(name.clone(), provider.health_check().await);
        }
        results
    }

    pub fn provider_names(&self) -> Vec<&str> {
        self.providers.keys().map(|s| s.as_str()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::providers::mock::MockProvider;
    use crate::llm::{ProviderCapabilities, QualityTier};
    use std::sync::Arc;

    /// A provider that always fails — used to test fallback chains
    struct FailingProvider {
        name: String,
        caps: ProviderCapabilities,
    }

    impl FailingProvider {
        fn new(name: &str) -> Self {
            Self {
                name: name.to_string(),
                caps: ProviderCapabilities::default(),
            }
        }
    }

    #[async_trait::async_trait]
    impl LLMProvider for FailingProvider {
        fn name(&self) -> &str {
            &self.name
        }
        fn capabilities(&self) -> &ProviderCapabilities {
            &self.caps
        }

        async fn complete(
            &self,
            _req: CompletionRequest,
        ) -> Result<CompletionResponse, ProviderError> {
            Err(ProviderError::Unavailable {
                provider: self.name.clone(),
                reason: "always fails".to_string(),
            })
        }
    }

    #[tokio::test]
    async fn default_provider_selected() {
        let mock = MockProvider::new("default").with_default("hello");
        let mut registry = ProviderRegistry::new("default");
        registry.register("default", Arc::new(mock));

        let resp = registry
            .resolve_and_complete(CompletionRequest::simple("test"), None)
            .await
            .unwrap();
        assert_eq!(resp.content, "hello");
        assert_eq!(resp.provider_name, "default");
    }

    #[tokio::test]
    async fn fallback_on_failure() {
        let primary = FailingProvider::new("primary");
        let fallback = MockProvider::new("fallback").with_default("fallback response");

        let mut registry = ProviderRegistry::new("primary");
        registry.register("primary", Arc::new(primary));
        registry.register("fallback", Arc::new(fallback));
        registry.set_fallback("primary", "fallback");

        let resp = registry
            .resolve_and_complete(CompletionRequest::simple("test"), None)
            .await
            .unwrap();
        assert_eq!(resp.content, "fallback response");
        assert_eq!(resp.provider_name, "fallback");
    }

    #[tokio::test]
    async fn explicit_provider_pin() {
        let mock = MockProvider::new("pinned").with_default("pinned response");
        let mut registry = ProviderRegistry::new("other");
        registry.register("pinned", Arc::new(mock));

        let hint = CapabilityHint {
            provider_name: Some("pinned".to_string()),
            ..Default::default()
        };
        let resp = registry
            .resolve_and_complete(CompletionRequest::simple("test"), Some(&hint))
            .await
            .unwrap();
        assert_eq!(resp.provider_name, "pinned");
    }

    #[tokio::test]
    async fn local_only_routing() {
        // Cloud provider (not local)
        struct CloudProvider(ProviderCapabilities);
        #[async_trait::async_trait]
        impl LLMProvider for CloudProvider {
            fn name(&self) -> &str {
                "cloud"
            }
            fn capabilities(&self) -> &ProviderCapabilities {
                &self.0
            }
            async fn complete(
                &self,
                _req: CompletionRequest,
            ) -> Result<CompletionResponse, ProviderError> {
                Ok(CompletionResponse {
                    content: "cloud".to_string(),
                    tokens_in: 1,
                    tokens_out: 1,
                    latency_ms: 1,
                    model_used: "cloud".to_string(),
                    provider_name: "cloud".to_string(),
                    cost_usd: 0.01,
                })
            }
        }

        let cloud = CloudProvider(ProviderCapabilities {
            local: false,
            cost_per_1k_input_tokens: 0.01,
            ..Default::default()
        });
        let local = MockProvider::new("local").with_default("local response");

        let mut registry = ProviderRegistry::new("cloud");
        registry.register("cloud", Arc::new(cloud));
        registry.register("local", Arc::new(local));

        let hint = CapabilityHint {
            local_only: true,
            ..Default::default()
        };
        let resp = registry
            .resolve_and_complete(CompletionRequest::simple("test"), Some(&hint))
            .await
            .unwrap();
        assert_eq!(resp.provider_name, "local");
    }

    #[tokio::test]
    async fn no_satisfying_provider_error() {
        let mock = MockProvider::new("mock");
        let mut registry = ProviderRegistry::new("mock");
        registry.register("mock", Arc::new(mock));

        let hint = CapabilityHint {
            quality: Some(QualityTier::High),
            min_context_tokens: Some(999_999_999),
            ..Default::default()
        };
        let result = registry
            .resolve_and_complete(CompletionRequest::simple("test"), Some(&hint))
            .await;
        assert!(matches!(
            result,
            Err(ProviderError::NoSatisfyingProvider { .. })
        ));
    }
}
