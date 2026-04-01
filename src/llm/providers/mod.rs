use std::sync::Arc;
use crate::config::ProviderConfig;
use crate::llm::BoxedProvider;

pub mod anthropic;
pub mod openai_compat;
pub mod mock;

fn resolve_api_key(config: &ProviderConfig, provider_type: &str, provider_name: &str) -> Result<String, String> {
    // 1. Check config api_key (already env-expanded by config loading)
    if let Some(key) = config.api_key.as_deref() {
        if !key.is_empty() {
            return Ok(key.to_string());
        }
    }

    // 2. Well-known env var by provider type
    let env_var = match provider_type {
        "anthropic" => Some("ANTHROPIC_API_KEY"),
        "openai" => Some("OPENAI_API_KEY"),
        "groq" => Some("GROQ_API_KEY"),
        "together" => Some("TOGETHER_API_KEY"),
        "mistral" => Some("MISTRAL_API_KEY"),
        _ => None,
    };

    if let Some(var) = env_var {
        if let Ok(val) = std::env::var(var) {
            if !val.is_empty() {
                return Ok(val);
            }
        }
    }

    // 3. Generic fallback: {UPPERCASE_NAME}_API_KEY
    let generic_var = format!("{}_API_KEY", provider_name.to_uppercase());
    if let Ok(val) = std::env::var(&generic_var) {
        if !val.is_empty() {
            return Ok(val);
        }
    }

    let hint = env_var.unwrap_or(&generic_var);
    Err(format!(
        "{} provider '{}' requires an API key.\n  Set api_key in forge.config.toml or export {}",
        provider_type, provider_name, hint
    ))
}

pub fn build_provider(
    name: &str,
    config: &ProviderConfig,
) -> Result<BoxedProvider, String> {
    match config.type_.as_str() {
        "anthropic" => {
            let api_key = resolve_api_key(config, "anthropic", name)?;
            let model = config.model.as_deref()
                .unwrap_or("claude-haiku-4-5-20251001");
            Ok(Arc::new(anthropic::AnthropicProvider::new(
                name, &api_key, model, config
            )?))
        }

        "openai" => {
            let api_key = resolve_api_key(config, "openai", name)?;
            let model = config.model.as_deref()
                .unwrap_or("gpt-4o");
            let mut cfg = config.clone();
            cfg.base_url = Some("https://api.openai.com/v1".to_string());
            Ok(Arc::new(openai_compat::OpenAICompatProvider::new(
                name, &api_key, model, &cfg
            )?))
        }

        "openai-compat" | "ollama" | "vllm" | "lmstudio" => {
            let base_url = config.base_url.as_deref()
                .ok_or(format!("{} provider requires base_url", config.type_))?;
            let model = config.model.as_deref()
                .ok_or(format!("{} provider requires model", config.type_))?;
            let api_key = config.api_key.clone()
                .unwrap_or_else(|| "not-required".to_string());
            let _ = base_url; // used by OpenAICompatProvider via config
            Ok(Arc::new(openai_compat::OpenAICompatProvider::new(
                name, &api_key, model, config
            )?))
        }

        "mock" => Ok(Arc::new(mock::MockProvider::new(name))),

        other => Err(format!("unknown provider type '{}' for '{}'", other, name)),
    }
}
