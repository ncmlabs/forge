use crate::config::{EmbeddingsConfig, ProviderConfig};
use crate::llm::{BoxedEmbeddingProvider, BoxedProvider};
use std::sync::Arc;

pub mod anthropic;
pub mod mock;
pub mod openai_compat;

fn resolve_api_key(
    config: &ProviderConfig,
    provider_type: &str,
    provider_name: &str,
) -> Result<String, String> {
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

pub fn build_provider(name: &str, config: &ProviderConfig) -> Result<BoxedProvider, String> {
    match config.type_.as_str() {
        "anthropic" => {
            let api_key = resolve_api_key(config, "anthropic", name)?;
            let model = config
                .model
                .as_deref()
                .unwrap_or("claude-haiku-4-5-20251001");
            Ok(Arc::new(anthropic::AnthropicProvider::new(
                name, &api_key, model, config,
            )?))
        }

        "openai" => {
            let api_key = resolve_api_key(config, "openai", name)?;
            let model = config.model.as_deref().unwrap_or("gpt-4o");
            let mut cfg = config.clone();
            cfg.base_url = Some("https://api.openai.com/v1".to_string());
            Ok(Arc::new(openai_compat::OpenAICompatProvider::new(
                name, &api_key, model, &cfg,
            )?))
        }

        "openai-compat" | "ollama" | "vllm" | "lmstudio" => {
            let base_url = config
                .base_url
                .as_deref()
                .ok_or(format!("{} provider requires base_url", config.type_))?;
            let model = config
                .model
                .as_deref()
                .ok_or(format!("{} provider requires model", config.type_))?;
            let api_key = config
                .api_key
                .clone()
                .unwrap_or_else(|| "not-required".to_string());
            let _ = base_url; // used by OpenAICompatProvider via config
            Ok(Arc::new(openai_compat::OpenAICompatProvider::new(
                name, &api_key, model, config,
            )?))
        }

        "mock" => Ok(Arc::new(mock::MockProvider::new(name))),

        other => Err(format!("unknown provider type '{}' for '{}'", other, name)),
    }
}

/// Build an embedding provider from config.
/// The `embed_config` references a provider from [providers.*] for connection details.
pub fn build_embedding_provider(
    provider_config: &ProviderConfig,
    embed_config: &EmbeddingsConfig,
) -> Result<BoxedEmbeddingProvider, String> {
    let provider_name = &embed_config.provider;
    match provider_config.type_.as_str() {
        "anthropic" => Err(
            "Anthropic does not support embeddings. Use an OpenAI-compatible provider (e.g., Ollama with nomic-embed-text).".to_string()
        ),

        "openai" | "openai-compat" | "ollama" | "vllm" | "lmstudio" => {
            let base_url = provider_config
                .base_url
                .as_deref()
                .unwrap_or("https://api.openai.com/v1");
            let model = embed_config
                .model
                .as_deref()
                .or(provider_config.model.as_deref())
                .ok_or_else(|| format!(
                    "embedding provider '{}' requires a model (set [embeddings].model or [providers.{}].model)",
                    provider_name, provider_name
                ))?;
            let api_key = provider_config
                .api_key
                .as_deref()
                .unwrap_or("not-required");
            let dimensions = embed_config.dimensions.unwrap_or(768);
            let cost = embed_config.cost_per_1k_tokens.unwrap_or(0.0);
            let timeout = provider_config.timeout_secs.unwrap_or(30);

            Ok(Arc::new(openai_compat::OpenAICompatEmbeddingProvider::new(
                provider_name, api_key, model, base_url, dimensions, cost, timeout,
            )))
        }

        "mock" => {
            let dimensions = embed_config.dimensions.unwrap_or(64);
            Ok(Arc::new(mock::MockEmbeddingProvider::new(provider_name, dimensions)))
        }

        other => Err(format!(
            "provider type '{}' does not support embeddings", other
        )),
    }
}
