use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Deserialize, Clone)]
pub struct ForgeConfig {
    pub llm:       LLMConfig,
    pub providers: HashMap<String, ProviderConfig>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct LLMConfig {
    pub default:  String,
    pub routing:  Option<HashMap<String, String>>,
    pub budget:   Option<BudgetConfig>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct BudgetConfig {
    pub max_cost_usd:     Option<f32>,
    pub max_total_tokens: Option<u32>,
    pub alert_at_pct:     Option<u32>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ProviderConfig {
    #[serde(rename = "type")]
    pub type_:        String,
    pub model:        Option<String>,
    pub api_key:      Option<String>,
    pub base_url:     Option<String>,
    pub fallback:     Option<String>,
    pub capabilities: Option<CapabilityOverride>,
    pub headers:      Option<HashMap<String, String>>,
    pub timeout_secs: Option<u64>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct CapabilityOverride {
    pub max_context_tokens: Option<u32>,
    pub quality_tier:       Option<crate::llm::QualityTier>,
    pub local:              Option<bool>,
    pub cost_per_1k_input:  Option<f32>,
    pub cost_per_1k_output: Option<f32>,
}

impl ForgeConfig {
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| ConfigError::FileNotFound(path.display().to_string(), e.to_string()))?;
        let config: ForgeConfig = toml::from_str(&content)
            .map_err(|e| ConfigError::ParseError(e.to_string()))?;
        config.validate()?;
        Ok(config)
    }

    pub fn load_or_default() -> Self {
        let quiet = std::env::var("FORGE_LOG_LEVEL")
            .map(|v| v == "quiet")
            .unwrap_or(false);
        let explicit_mock = std::env::var("FORGE_MOCK")
            .map(|v| v == "1")
            .unwrap_or(false);

        // Check env override for config path
        if let Ok(path) = std::env::var("FORGE_CONFIG") {
            if let Ok(config) = Self::load(Path::new(&path)) {
                return Self::apply_env_overrides(config);
            }
        }

        // Search standard paths
        let search_paths = [
            Some(std::path::PathBuf::from("forge.config.toml")),
            dirs::home_dir().map(|d| d.join(".forge/config.toml")),
        ];
        for path in search_paths.iter().flatten() {
            if path.exists() {
                if let Ok(config) = Self::load(path) {
                    return Self::apply_env_overrides(config);
                }
            }
        }

        if !quiet && !explicit_mock {
            eprintln!("warning: no forge.config.toml found, using mock provider");
            eprintln!("  hint: create forge.config.toml or set FORGE_CONFIG=/path/to/config.toml");
        }

        Self::apply_env_overrides(Self::default_mock_config())
    }

    fn apply_env_overrides(mut config: ForgeConfig) -> ForgeConfig {
        config.resolve_env_vars();

        // FORGE_MOCK=1 or FORGE_PROVIDER=mock
        if std::env::var("FORGE_MOCK").map(|v| v == "1").unwrap_or(false) {
            config.llm.default = "mock".to_string();
            if !config.providers.contains_key("mock") {
                config.providers.insert("mock".to_string(), ProviderConfig {
                    type_:        "mock".to_string(),
                    model:        Some("mock-model".to_string()),
                    api_key:      None,
                    base_url:     None,
                    fallback:     None,
                    capabilities: None,
                    headers:      None,
                    timeout_secs: None,
                });
            }
        } else if let Ok(provider) = std::env::var("FORGE_PROVIDER") {
            config.llm.default = provider;
        }

        // FORGE_BUDGET override
        if let Ok(budget) = std::env::var("FORGE_BUDGET") {
            if let Ok(val) = budget.parse::<f32>() {
                let budget_config = config.llm.budget.get_or_insert(BudgetConfig {
                    max_cost_usd:     None,
                    max_total_tokens: None,
                    alert_at_pct:     None,
                });
                budget_config.max_cost_usd = Some(val);
            }
        }

        config
    }

    fn validate(&self) -> Result<(), ConfigError> {
        if !self.providers.contains_key(&self.llm.default) {
            return Err(ConfigError::UnknownProvider(
                format!("default provider '{}' not defined in [providers]", self.llm.default)
            ));
        }
        self.check_fallback_cycles()?;
        Ok(())
    }

    fn check_fallback_cycles(&self) -> Result<(), ConfigError> {
        for (name, config) in &self.providers {
            let mut visited = vec![name.as_str()];
            let mut current = config.fallback.as_deref();
            while let Some(next) = current {
                if visited.contains(&next) {
                    return Err(ConfigError::CircularFallback(name.clone()));
                }
                visited.push(next);
                current = self.providers.get(next)
                    .and_then(|c| c.fallback.as_deref());
            }
        }
        Ok(())
    }

    pub fn resolve_env_vars(&mut self) {
        for config in self.providers.values_mut() {
            if let Some(key) = &config.api_key {
                config.api_key = Some(expand_env_var(key));
            }
            if let Some(url) = &config.base_url {
                config.base_url = Some(expand_env_var(url));
            }
        }
    }

    pub fn default_mock_config() -> Self {
        let mut providers = HashMap::new();
        providers.insert("mock".to_string(), ProviderConfig {
            type_:        "mock".to_string(),
            model:        Some("mock-model".to_string()),
            api_key:      None,
            base_url:     None,
            fallback:     None,
            capabilities: None,
            headers:      None,
            timeout_secs: None,
        });
        Self {
            llm: LLMConfig {
                default: "mock".to_string(),
                routing: None,
                budget:  None,
            },
            providers,
        }
    }
}

fn expand_env_var(s: &str) -> String {
    if s.starts_with("${") && s.ends_with('}') {
        let var_name = &s[2..s.len()-1];
        std::env::var(var_name).unwrap_or_else(|_| {
            eprintln!("Warning: env var {} not set", var_name);
            String::new()
        })
    } else {
        s.to_string()
    }
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("config file not found at '{0}': {1}")]
    FileNotFound(String, String),
    #[error("config parse error: {0}")]
    ParseError(String),
    #[error("unknown provider: {0}")]
    UnknownProvider(String),
    #[error("circular fallback chain starting at '{0}'")]
    CircularFallback(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_mock_config_is_valid() {
        let config = ForgeConfig::default_mock_config();
        assert_eq!(config.llm.default, "mock");
        assert!(config.providers.contains_key("mock"));
        assert!(config.validate().is_ok());
    }

    #[test]
    fn unknown_default_provider_rejected() {
        let config = ForgeConfig {
            llm: LLMConfig {
                default: "nonexistent".to_string(),
                routing: None,
                budget:  None,
            },
            providers: HashMap::new(),
        };
        assert!(matches!(config.validate(), Err(ConfigError::UnknownProvider(_))));
    }

    #[test]
    fn circular_fallback_detected() {
        let mut providers = HashMap::new();
        providers.insert("a".to_string(), ProviderConfig {
            type_: "mock".to_string(), model: None, api_key: None,
            base_url: None, fallback: Some("b".to_string()),
            capabilities: None, headers: None, timeout_secs: None,
        });
        providers.insert("b".to_string(), ProviderConfig {
            type_: "mock".to_string(), model: None, api_key: None,
            base_url: None, fallback: Some("a".to_string()),
            capabilities: None, headers: None, timeout_secs: None,
        });
        let config = ForgeConfig {
            llm: LLMConfig {
                default: "a".to_string(),
                routing: None,
                budget:  None,
            },
            providers,
        };
        assert!(matches!(config.validate(), Err(ConfigError::CircularFallback(_))));
    }

    #[test]
    fn expand_env_var_resolves() {
        std::env::set_var("FORGE_TEST_KEY_8", "secret123");
        assert_eq!(expand_env_var("${FORGE_TEST_KEY_8}"), "secret123");
        std::env::remove_var("FORGE_TEST_KEY_8");
    }

    #[test]
    fn expand_env_var_passthrough() {
        assert_eq!(expand_env_var("plain-string"), "plain-string");
    }

    #[test]
    fn parse_toml_config() {
        let toml_str = r#"
[llm]
default = "mock"

[providers.mock]
type = "mock"
model = "mock-model"
"#;
        let config: ForgeConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.llm.default, "mock");
        assert!(config.providers.contains_key("mock"));
        assert!(config.validate().is_ok());
    }

    #[test]
    fn parse_toml_with_budget() {
        let toml_str = r#"
[llm]
default = "mock"

[llm.budget]
max_cost_usd = 5.0
alert_at_pct = 80

[providers.mock]
type = "mock"
"#;
        let config: ForgeConfig = toml::from_str(toml_str).unwrap();
        let budget = config.llm.budget.unwrap();
        assert_eq!(budget.max_cost_usd, Some(5.0));
        assert_eq!(budget.alert_at_pct, Some(80));
    }
}
