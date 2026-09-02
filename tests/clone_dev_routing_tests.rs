// Integration test for T8.6 (#361) — multi-provider phase routing.
//
// Drives a small `.forge` program through the runtime with three named
// MockProvider instances (`mock-anthropic`, `mock-openai`, `mock-ollama`)
// and the new phase-keyed routing chain wired into ProviderRegistry. The
// program calls `reason "..." for plan` and `reason "..." for ops_investigate`,
// each of which must dispatch to a distinct provider per the routing
// table. We then force the primary provider to fail and assert the
// chain falls through to the configured fallback.
//
// This is the DoD test for #361: "one end-to-end task uses at least
// two different providers across its phases; both provider calls
// visible in traces."

use std::collections::HashMap;
use std::sync::Arc;

use forge::llm::providers::mock::MockProvider;
use forge::llm::registry::ProviderRegistry;
use forge::llm::{
    BoxedProvider, CompletionRequest, CompletionResponse, LLMProvider, ProviderCapabilities,
    ProviderError,
};
use forge::runtime::confidence::{ConfidentValue, Value};
use forge::runtime::executor::TaskExecutor;

const PROGRAM_SRC: &str = r#"#! boundary: server

task plan_step
  needs issue: Text
  gives Text
  do
    result = reason "Draft a plan for {issue}" for plan
    give "{result}"

task ops_step
  needs query: Text
  gives Text
  do
    result = reason "Investigate ops query: {query}" for ops_investigate
    give "{result}"

# Drives both phases in one endpoint call so a single test invocation
# produces traces for two distinct providers.
endpoint run_pipeline(issue: Text, query: Text) -> Text
  plan = plan_step(issue)
  ops = ops_step(query)
  give "{plan}|{ops}"
"#;

/// FailingProvider — used to validate fallback chain traversal. The
/// failure is deterministic per call so the test's chain step counts
/// stay stable.
struct FailingProvider {
    name: String,
    caps: ProviderCapabilities,
}

#[async_trait::async_trait]
impl LLMProvider for FailingProvider {
    fn name(&self) -> &str {
        &self.name
    }
    fn capabilities(&self) -> &ProviderCapabilities {
        &self.caps
    }
    async fn complete(&self, _req: CompletionRequest) -> Result<CompletionResponse, ProviderError> {
        Err(ProviderError::Unavailable {
            provider: self.name.clone(),
            reason: "deterministic failure for chain test".to_string(),
        })
    }
}

fn build_registry_two_phases() -> Arc<ProviderRegistry> {
    let anthropic = MockProvider::new("mock-anthropic").with_default("[anthropic plan response]");
    let openai = MockProvider::new("mock-openai").with_default("[openai impl response]");
    let ollama = MockProvider::new("mock-ollama").with_default("[ollama ops response]");

    let mut registry = ProviderRegistry::new("mock-anthropic");
    registry.register("mock-anthropic", Arc::new(anthropic));
    registry.register("mock-openai", Arc::new(openai));
    registry.register("mock-ollama", Arc::new(ollama));
    registry.set_phase_chain("plan", vec!["mock-anthropic".into()]);
    registry.set_phase_chain("implement", vec!["mock-openai".into()]);
    registry.set_phase_chain("ops_investigate", vec!["mock-ollama".into()]);

    Arc::new(registry)
}

fn build_registry_with_fallback() -> Arc<ProviderRegistry> {
    // Primary fails → fallback succeeds. The chain is `plan` →
    // [failing-primary, mock-anthropic]. Cost (and trace `provider`)
    // must reflect the responder, not the chain head.
    let primary: BoxedProvider = Arc::new(FailingProvider {
        name: "primary".into(),
        caps: ProviderCapabilities::default(),
    });
    let fallback = MockProvider::new("mock-anthropic").with_default("[fallback served]");
    let ollama = MockProvider::new("mock-ollama").with_default("[ollama ops response]");

    let mut registry = ProviderRegistry::new("mock-anthropic");
    registry.register("primary", primary);
    registry.register("mock-anthropic", Arc::new(fallback));
    registry.register("mock-ollama", Arc::new(ollama));
    registry.set_phase_chain("plan", vec!["primary".into(), "mock-anthropic".into()]);
    registry.set_phase_chain("ops_investigate", vec!["mock-ollama".into()]);

    Arc::new(registry)
}

async fn run_pipeline(registry: Arc<ProviderRegistry>) -> (String, forge::tracer::Tracer) {
    let program = forge::parser::parse(PROGRAM_SRC).expect("parse pipeline source");
    let tracer = forge::tracer::Tracer::with_capture();
    let executor = TaskExecutor::new(program, registry, Some(tracer.clone()));

    let mut args: HashMap<String, ConfidentValue> = HashMap::new();
    args.insert(
        "issue".into(),
        ConfidentValue::deterministic(Value::Text("FORGE-361".into())),
    );
    args.insert(
        "query".into(),
        ConfidentValue::deterministic(Value::Text("disk usage".into())),
    );

    let result = executor
        .exec_endpoint("run_pipeline", args, None)
        .await
        .expect("endpoint dispatch");
    let combined = match result.value.value {
        Value::Text(t) => t,
        other => panic!("expected Text result, got {:?}", other),
    };
    (combined, tracer)
}

fn llm_response_events(tracer: &forge::tracer::Tracer) -> Vec<serde_json::Value> {
    tracer
        .captured_log()
        .into_iter()
        .filter(|(name, _)| name == "llm_response")
        .map(|(_, payload)| payload)
        .collect()
}

#[tokio::test]
async fn end_to_end_pipeline_dispatches_distinct_providers_per_phase() {
    // DoD: "one end-to-end task uses at least two different providers
    // across its phases; both provider calls visible in traces."
    let (combined, tracer) = run_pipeline(build_registry_two_phases()).await;

    // Both phases ran and contributed to the result.
    assert!(
        combined.contains("anthropic"),
        "anthropic plan response missing from combined: {combined}"
    );
    assert!(
        combined.contains("ollama"),
        "ollama ops response missing from combined: {combined}"
    );

    let events = llm_response_events(&tracer);
    assert_eq!(events.len(), 2, "expected two llm_response events");

    let mut by_phase: HashMap<String, String> = HashMap::new();
    for ev in &events {
        let phase = ev["phase"].as_str().expect("phase carried on event");
        let provider = ev["provider"].as_str().expect("provider carried on event");
        by_phase.insert(phase.to_string(), provider.to_string());
    }

    assert_eq!(by_phase.get("plan"), Some(&"mock-anthropic".to_string()));
    assert_eq!(
        by_phase.get("ops_investigate"),
        Some(&"mock-ollama".to_string())
    );
}

#[tokio::test]
async fn pipeline_phase_chain_falls_through_on_primary_failure() {
    // DoD: "if primary provider errors (timeout, 5xx), the `fallback`
    // list is tried in order; cost is charged to whichever provider
    // served the response."
    let (_, tracer) = run_pipeline(build_registry_with_fallback()).await;
    let events = llm_response_events(&tracer);

    let plan_event = events
        .iter()
        .find(|e| e["phase"].as_str() == Some("plan"))
        .expect("plan event present");

    // The trace records the *responder*, not the chain head — so
    // when the primary failed the event must show the fallback name.
    assert_eq!(
        plan_event["provider"].as_str(),
        Some("mock-anthropic"),
        "plan event should attribute cost to the fallback that served"
    );
}
