use forge::config::ForgeConfig;
use forge::llm::cost_tracker::{BudgetError, CostTracker};
use forge::llm::providers::mock::MockProvider;
use forge::llm::registry::ProviderRegistry;
use forge::llm::{CapabilityHint, CompletionRequest, CompletionResponse, LLMProvider};

// ── Mock provider tests ─────────────────────────────────────────────────────

fn mock() -> MockProvider {
    MockProvider::new("test")
        .with_response("classify", "support")
        .with_response("summarize", "brief summary of the document")
        .with_default("I don't know")
}

#[tokio::test]
async fn mock_pattern_matching() {
    let m = mock();
    let resp = m
        .complete(CompletionRequest::simple("please classify this text"))
        .await
        .unwrap();
    assert_eq!(resp.content, "support");
    assert_eq!(resp.provider_name, "test");
    assert_eq!(resp.cost_usd, 0.0);
}

#[tokio::test]
async fn mock_default_response() {
    let m = mock();
    let resp = m
        .complete(CompletionRequest::simple("something unrecognised"))
        .await
        .unwrap();
    assert_eq!(resp.content, "I don't know");
}

#[tokio::test]
async fn mock_health_check() {
    let m = mock();
    assert!(m.health_check().await.is_ok());
}

// ── Confidence estimate tests ───────────────────────────────────────────────

#[test]
fn estimate_confidence_no_hedging() {
    let resp = CompletionResponse {
        content: "The answer is 42.".to_string(),
        tool_calls: vec![],
        tokens_in: 10,
        tokens_out: 5,
        latency_ms: 50,
        model_used: "test".to_string(),
        provider_name: "test".to_string(),
        cost_usd: 0.0,
    };
    assert!((resp.estimate_confidence() - 0.85).abs() < 0.01);
}

#[test]
fn estimate_confidence_with_hedging() {
    let resp = CompletionResponse {
        content: "I think it might be 42, but I'm not sure.".to_string(),
        tool_calls: vec![],
        tokens_in: 10,
        tokens_out: 10,
        latency_ms: 50,
        model_used: "test".to_string(),
        provider_name: "test".to_string(),
        cost_usd: 0.0,
    };
    let conf = resp.estimate_confidence();
    assert!(conf < 0.85, "should be lower than baseline: {}", conf);
}

#[test]
fn estimate_confidence_floor() {
    let resp = CompletionResponse {
        content: "I'm not sure, I think it's possibly something, might be unclear, I don't know, it depends, I cannot say".to_string(),
        tool_calls: vec![],
        tokens_in: 10, tokens_out: 20, latency_ms: 50,
        model_used: "test".to_string(), provider_name: "test".to_string(),
        cost_usd: 0.0,
    };
    assert!((resp.estimate_confidence() - 0.3).abs() < 0.01);
}

// ── Registry tests ──────────────────────────────────────────────────────────

#[tokio::test]
async fn registry_from_mock_config() {
    let config = ForgeConfig::default_mock_config();
    let registry = ProviderRegistry::from_config(config).unwrap();
    let resp = registry
        .resolve_and_complete(CompletionRequest::simple("hello"), None)
        .await
        .unwrap();
    assert_eq!(resp.provider_name, "mock");
}

#[tokio::test]
async fn registry_explicit_pin() {
    let config = ForgeConfig::default_mock_config();
    let registry = ProviderRegistry::from_config(config).unwrap();
    let hint = CapabilityHint {
        provider_name: Some("mock".to_string()),
        ..Default::default()
    };
    let resp = registry
        .resolve_and_complete(CompletionRequest::simple("test"), Some(&hint))
        .await
        .unwrap();
    assert_eq!(resp.provider_name, "mock");
}

// ── Cost tracker tests ──────────────────────────────────────────────────────

#[test]
fn budget_exceeded() {
    let tracker = CostTracker::new(Some(0.001), 80);
    let resp = CompletionResponse {
        cost_usd: 0.002,
        content: "hello".to_string(),
        tool_calls: vec![],
        tokens_in: 100,
        tokens_out: 50,
        latency_ms: 1,
        model_used: "mock".to_string(),
        provider_name: "mock".to_string(),
    };
    assert!(matches!(
        tracker.record(&resp),
        Err(BudgetError::Exceeded { .. })
    ));
}

// ── Config tests ────────────────────────────────────────────────────────────

#[test]
fn config_round_trip() {
    let toml_str = r#"
[llm]
default = "mock"

[llm.budget]
max_cost_usd = 1.0
alert_at_pct = 90

[providers.mock]
type = "mock"
model = "mock-model"
"#;
    let config: ForgeConfig = toml::from_str(toml_str).unwrap();
    assert_eq!(config.llm.default, "mock");
    assert_eq!(config.llm.budget.as_ref().unwrap().max_cost_usd, Some(1.0));
}

// ── Request builder tests ───────────────────────────────────────────────────

#[test]
fn completion_request_builder() {
    let req = CompletionRequest::simple("hello")
        .with_system("you are helpful")
        .with_temperature(0.5)
        .with_max_tokens(2048);
    assert_eq!(req.prompt, "hello");
    assert_eq!(req.system.as_deref(), Some("you are helpful"));
    assert_eq!(req.temperature, 0.5);
    assert_eq!(req.max_tokens, 2048);
}
