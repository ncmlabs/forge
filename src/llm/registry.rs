use crate::config::ForgeConfig;
use crate::llm::providers::build_provider;
use crate::llm::{
    BoxedProvider, CapabilityHint, CompletionRequest, CompletionResponse, LLMProvider,
    ProviderError,
};
use std::collections::HashMap;

pub struct ProviderRegistry {
    providers: HashMap<String, BoxedProvider>,
    fallbacks: HashMap<String, String>,
    default: String,
    /// Phase → ordered provider chain (#361). When a `CapabilityHint.phase`
    /// matches a key here, the chain is tried in order; on failure we fall
    /// through to the per-provider `fallbacks` link from the last attempted
    /// chain entry. Bare `reason`/`classify` calls (no phase) still resolve
    /// via the legacy quality-tier path.
    routing: HashMap<String, Vec<String>>,
}

impl ProviderRegistry {
    pub fn from_config(mut config: ForgeConfig) -> Result<Self, ProviderError> {
        config.resolve_env_vars();
        // forge.config.toml's [llm.routing] is single-provider per phase. The
        // chain shape is reserved for clone-dev's overlay (issue #361 wires
        // that path through `set_routing`).
        let routing = config
            .llm
            .routing
            .clone()
            .unwrap_or_default()
            .into_iter()
            .map(|(phase, name)| (phase, vec![name]))
            .collect();
        let mut registry = Self {
            providers: HashMap::new(),
            fallbacks: HashMap::new(),
            default: config.llm.default.clone(),
            routing,
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

        // T8.6 (#361) — clone-dev startup overlay. When the runtime is
        // launched with `$FORGE_CLONEDEV_CONFIG=<path>`, the resolved
        // CloneDevConfig's [llm.routing] (primary + fallback) overrides
        // anything from forge.config.toml's own `[llm.routing]`. A missing
        // file or parse error is a warning rather than a hard failure —
        // the registry stays usable for non-clone-dev workflows that
        // happen to share the same forge.config.toml.
        if let Ok(path) = std::env::var("FORGE_CLONEDEV_CONFIG") {
            match crate::runtime::clone_dev_config::load(std::path::Path::new(&path)) {
                Ok(cfg) => {
                    let table = cfg.routing_table();
                    if !table.is_empty() {
                        registry.routing = table;
                    }
                }
                Err(e) => {
                    eprintln!(
                        "[forge] FORGE_CLONEDEV_CONFIG='{}' present but unloadable: {}. \
                         Runtime continues with forge.config.toml routing.",
                        path, e
                    );
                }
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

    /// Replace the phase routing table. Used by the clone-dev startup overlay
    /// (#361) when `$FORGE_CLONEDEV_CONFIG` is set; the resolved
    /// `CloneDevConfig` rebuilds this map keyed by phase name.
    pub fn set_routing(&mut self, table: HashMap<String, Vec<String>>) {
        self.routing = table;
    }

    /// Set the chain for a single phase. Convenience for tests and granular
    /// updates without rebuilding the whole table.
    pub fn set_phase_chain(&mut self, phase: &str, chain: Vec<String>) {
        self.routing.insert(phase.to_string(), chain);
    }

    pub async fn resolve_and_complete(
        &self,
        request: CompletionRequest,
        hint: Option<&CapabilityHint>,
    ) -> Result<CompletionResponse, ProviderError> {
        let chain = self.choose_chain(hint)?;
        self.try_chain(&chain, request).await
    }

    /// Resolve the ordered provider chain for a hint:
    /// 1. Explicit `provider_name` pin always wins (single-element chain).
    /// 2. Phase-keyed chain from the routing table when `hint.phase` matches.
    /// 3. Capability-based `find_best` when other hint fields are set.
    /// 4. Default provider as a last resort.
    ///
    /// Unknown phase keys fall through to (4) — config-only typos shouldn't
    /// stop the runtime; the diagnostic surfaces in the warning logs.
    fn choose_chain(&self, hint: Option<&CapabilityHint>) -> Result<Vec<String>, ProviderError> {
        match hint {
            Some(h) if h.provider_name.is_some() => {
                let name = h.provider_name.as_ref().unwrap();
                if self.providers.contains_key(name) {
                    Ok(vec![name.clone()])
                } else {
                    Err(ProviderError::NoSatisfyingProvider {
                        requirements: format!("explicitly requested '{}' but not configured", name),
                    })
                }
            }
            Some(h) if h.phase.is_some() => {
                let phase = h.phase.as_ref().unwrap();
                match self.routing.get(phase) {
                    Some(chain) if !chain.is_empty() => {
                        for name in chain {
                            if !self.providers.contains_key(name) {
                                return Err(ProviderError::NoSatisfyingProvider {
                                    requirements: format!(
                                        "phase '{}' chain references unknown provider '{}'",
                                        phase, name
                                    ),
                                });
                            }
                        }
                        Ok(chain.clone())
                    }
                    _ => {
                        eprintln!(
                            "[forge] phase '{}' has no routing entry; using default '{}'",
                            phase, self.default
                        );
                        Ok(vec![self.default.clone()])
                    }
                }
            }
            Some(h) => self.find_best(h).map(|name| vec![name]),
            None => Ok(vec![self.default.clone()]),
        }
    }

    /// Walk the chain in order; for each entry, delegate to the per-provider
    /// `fallbacks` linked-list. The chain composes with per-provider fallback:
    /// each chain step is "try this provider with its own fallback", and we
    /// only advance to the next chain step after that nested chain is fully
    /// exhausted. This keeps existing per-provider `fallback` config working
    /// untouched while letting clone-dev declare richer per-phase chains.
    async fn try_chain(
        &self,
        chain: &[String],
        request: CompletionRequest,
    ) -> Result<CompletionResponse, ProviderError> {
        let mut last_err: Option<ProviderError> = None;
        for (idx, name) in chain.iter().enumerate() {
            match self.try_with_fallback(name, request.clone()).await {
                Ok(resp) => return Ok(resp),
                Err(e) => {
                    if idx + 1 < chain.len() {
                        eprintln!(
                            "[forge] chain step {}/{} ('{}') failed: {}. Advancing chain.",
                            idx + 1,
                            chain.len(),
                            name,
                            e
                        );
                    }
                    last_err = Some(e);
                }
            }
        }
        Err(
            last_err.unwrap_or_else(|| ProviderError::NoSatisfyingProvider {
                requirements: "empty provider chain".to_string(),
            }),
        )
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
                    tool_calls: vec![],
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

    // ── #361 — phase-keyed routing ─────────────────────────────────────

    #[tokio::test]
    async fn phase_routing_dispatches_to_configured_provider() {
        let sonnet = MockProvider::new("sonnet").with_default("from sonnet");
        let gpt4o = MockProvider::new("gpt-4o").with_default("from gpt-4o");
        let ollama = MockProvider::new("ollama").with_default("from ollama");

        let mut registry = ProviderRegistry::new("sonnet");
        registry.register("sonnet", Arc::new(sonnet));
        registry.register("gpt-4o", Arc::new(gpt4o));
        registry.register("ollama", Arc::new(ollama));
        registry.set_phase_chain("plan", vec!["sonnet".into()]);
        registry.set_phase_chain("implement", vec!["gpt-4o".into()]);
        registry.set_phase_chain("ops_investigate", vec!["ollama".into()]);

        let plan = CapabilityHint {
            phase: Some("plan".into()),
            ..Default::default()
        };
        let imp = CapabilityHint {
            phase: Some("implement".into()),
            ..Default::default()
        };
        let ops = CapabilityHint {
            phase: Some("ops_investigate".into()),
            ..Default::default()
        };

        let plan_resp = registry
            .resolve_and_complete(CompletionRequest::simple("p"), Some(&plan))
            .await
            .unwrap();
        let imp_resp = registry
            .resolve_and_complete(CompletionRequest::simple("i"), Some(&imp))
            .await
            .unwrap();
        let ops_resp = registry
            .resolve_and_complete(CompletionRequest::simple("o"), Some(&ops))
            .await
            .unwrap();

        assert_eq!(plan_resp.provider_name, "sonnet");
        assert_eq!(imp_resp.provider_name, "gpt-4o");
        assert_eq!(ops_resp.provider_name, "ollama");
    }

    #[tokio::test]
    async fn phase_routing_walks_chain_on_failure() {
        // Primary fails; chain advances to the next entry. Cost is charged
        // to whichever provider actually served (#361 DoD).
        let primary = FailingProvider::new("primary");
        let secondary = FailingProvider::new("secondary");
        let tertiary = MockProvider::new("tertiary").with_default("served");

        let mut registry = ProviderRegistry::new("tertiary");
        registry.register("primary", Arc::new(primary));
        registry.register("secondary", Arc::new(secondary));
        registry.register("tertiary", Arc::new(tertiary));
        registry.set_phase_chain(
            "plan",
            vec!["primary".into(), "secondary".into(), "tertiary".into()],
        );

        let hint = CapabilityHint {
            phase: Some("plan".into()),
            ..Default::default()
        };
        let resp = registry
            .resolve_and_complete(CompletionRequest::simple("test"), Some(&hint))
            .await
            .unwrap();
        assert_eq!(resp.provider_name, "tertiary");
        assert_eq!(resp.content, "served");
    }

    #[tokio::test]
    async fn unknown_phase_falls_back_to_default() {
        let default_p = MockProvider::new("default-p").with_default("default served");
        let mut registry = ProviderRegistry::new("default-p");
        registry.register("default-p", Arc::new(default_p));
        // No routing configured.

        let hint = CapabilityHint {
            phase: Some("does_not_exist".into()),
            ..Default::default()
        };
        let resp = registry
            .resolve_and_complete(CompletionRequest::simple("x"), Some(&hint))
            .await
            .unwrap();
        assert_eq!(resp.provider_name, "default-p");
    }

    #[tokio::test]
    async fn phase_chain_referencing_unknown_provider_errors() {
        let mock = MockProvider::new("real");
        let mut registry = ProviderRegistry::new("real");
        registry.register("real", Arc::new(mock));
        // Chain references a provider that was never registered.
        registry.set_phase_chain("plan", vec!["ghost".into()]);

        let hint = CapabilityHint {
            phase: Some("plan".into()),
            ..Default::default()
        };
        let result = registry
            .resolve_and_complete(CompletionRequest::simple("x"), Some(&hint))
            .await;
        assert!(matches!(
            result,
            Err(ProviderError::NoSatisfyingProvider { .. })
        ));
    }

    #[tokio::test]
    async fn from_config_applies_clone_dev_overlay_when_env_set() {
        // The startup overlay path: when $FORGE_CLONEDEV_CONFIG points at a
        // valid clone-dev.toml with [llm.routing], its phase chains land on
        // the registry without any explicit caller plumbing.
        use std::io::Write;

        let dir = std::env::temp_dir().join(format!("forge-361-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let path = dir.join("clone-dev.toml");
        {
            let mut f = std::fs::File::create(&path).expect("create");
            writeln!(
                f,
                "[llm.routing]\nplan = \"sonnet\"\nimplement = \"gpt-4o\"\n\n[llm.routing.fallback]\nplan = [\"sonnet\", \"gpt-4o\"]\n"
            )
            .expect("write");
        }

        // Build a minimal ForgeConfig with two named providers so the
        // overlay's chain references resolve.
        let toml = r#"
[llm]
default = "sonnet"

[providers.sonnet]
type = "mock"

[providers."gpt-4o"]
type = "mock"
"#;
        let config: crate::config::ForgeConfig = toml::from_str(toml).expect("config parse");

        // Set the env var across the build, then unset to avoid leaking
        // into other tests that share this process.
        std::env::set_var("FORGE_CLONEDEV_CONFIG", &path);
        let registry = ProviderRegistry::from_config(config).expect("registry build");
        std::env::remove_var("FORGE_CLONEDEV_CONFIG");
        let _ = std::fs::remove_dir_all(&dir);

        let chain = registry
            .routing
            .get("plan")
            .cloned()
            .expect("plan chain present");
        assert_eq!(chain, vec!["sonnet".to_string(), "gpt-4o".into()]);
        let imp = registry
            .routing
            .get("implement")
            .cloned()
            .expect("implement chain present");
        assert_eq!(imp, vec!["gpt-4o".to_string()]);
    }

    #[tokio::test]
    async fn explicit_provider_pin_wins_over_phase() {
        // hint.provider_name set together with hint.phase: pin still wins.
        // Lets agent code force a specific provider for one call without
        // touching config.
        let pinned = MockProvider::new("pinned").with_default("from pinned");
        let phase_target = MockProvider::new("phase-target").with_default("phase response");

        let mut registry = ProviderRegistry::new("phase-target");
        registry.register("pinned", Arc::new(pinned));
        registry.register("phase-target", Arc::new(phase_target));
        registry.set_phase_chain("plan", vec!["phase-target".into()]);

        let hint = CapabilityHint {
            phase: Some("plan".into()),
            provider_name: Some("pinned".into()),
            ..Default::default()
        };
        let resp = registry
            .resolve_and_complete(CompletionRequest::simple("x"), Some(&hint))
            .await
            .unwrap();
        assert_eq!(resp.provider_name, "pinned");
    }
}
