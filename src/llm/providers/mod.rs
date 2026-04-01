use std::sync::Arc;
use crate::config::ProviderConfig;
use crate::llm::BoxedProvider;

pub mod anthropic;
pub mod openai_compat;
pub mod mock;

pub fn build_provider(
    name: &str,
    config: &ProviderConfig,
) -> Result<BoxedProvider, String> {
    match config.type_.as_str() {
        "anthropic" => {
            let api_key = config.api_key.as_deref()
                .ok_or("anthropic provider requires api_key")?;
            let model = config.model.as_deref()
                .unwrap_or("claude-haiku-4-5-20251001");
            Ok(Arc::new(anthropic::AnthropicProvider::new(
                name, api_key, model, config
            )?))
        }

        "openai" => {
            let api_key = config.api_key.as_deref()
                .ok_or("openai provider requires api_key")?;
            let model = config.model.as_deref()
                .unwrap_or("gpt-4o");
            let mut cfg = config.clone();
            cfg.base_url = Some("https://api.openai.com/v1".to_string());
            Ok(Arc::new(openai_compat::OpenAICompatProvider::new(
                name, api_key, model, &cfg
            )?))
        }

        "openai-compat" | "ollama" | "vllm" | "lmstudio" => {
            let base_url = config.base_url.as_deref()
                .ok_or(format!("{} provider requires base_url", config.type_))?;
            let model = config.model.as_deref()
                .ok_or(format!("{} provider requires model", config.type_))?;
            let api_key = config.api_key.as_deref().unwrap_or("not-required");
            let _ = base_url; // used by OpenAICompatProvider via config
            Ok(Arc::new(openai_compat::OpenAICompatProvider::new(
                name, api_key, model, config
            )?))
        }

        "mock" => Ok(Arc::new(mock::MockProvider::new(name))),

        other => Err(format!("unknown provider type '{}' for '{}'", other, name)),
    }
}
