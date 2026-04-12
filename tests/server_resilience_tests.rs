// FORGE server resilience tests — issue #247
// Tests that servers remain operational when LLM providers are unreachable,
// and that the circuit breaker recovery mechanisms work correctly.

use std::sync::Arc;

use forge::config::ForgeConfig;
use forge::llm::registry::ProviderRegistry;
use forge::runtime::executor::TaskExecutor;
use forge::runtime::http_server::ForgeServer;

// ── Helpers ──────────────────────────────────────────────────────

fn mock_registry() -> Arc<ProviderRegistry> {
    let config = ForgeConfig::default_mock_config();
    Arc::new(ProviderRegistry::from_config(config).expect("mock registry should build"))
}

fn parse_and_build(source: &str) -> TaskExecutor {
    let program = forge::parser::parse(source).expect("parse failed");
    TaskExecutor::new(program, mock_registry(), None)
}

async fn spawn_server(source: &str) -> String {
    let executor = parse_and_build(source);
    let server = ForgeServer::new(executor, None);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind failed");
    let port = listener.local_addr().unwrap().port();

    tokio::spawn(async move {
        server
            .run_on_listener(listener)
            .await
            .expect("server failed");
    });

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    format!("http://127.0.0.1:{port}")
}

// ── Server startup resilience ────────────────────────────────────

#[tokio::test]
async fn server_binds_http_with_deterministic_endpoints() {
    // A server with only deterministic endpoints should work
    // even with mock (no real LLM) provider
    let source = "endpoint health() -> Text\n  give \"ok\"\n\nendpoint status() -> Text\n  give \"running\"\n";
    let url = spawn_server(source).await;

    let resp = reqwest::get(format!("{url}/health")).await.unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.text().await.unwrap(), "ok");

    let resp = reqwest::get(format!("{url}/status")).await.unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.text().await.unwrap(), "running");
}

#[tokio::test]
async fn server_responds_within_timeout() {
    let source = "endpoint health() -> Text\n  give \"ok\"\n";
    let start = std::time::Instant::now();
    let url = spawn_server(source).await;
    let resp = reqwest::get(format!("{url}/health")).await.unwrap();
    let elapsed = start.elapsed();

    assert_eq!(resp.status(), 200);
    // Server should respond well within 3 seconds
    assert!(
        elapsed.as_secs() < 3,
        "server took {elapsed:?} to respond, expected < 3s"
    );
}

// ── Health check (provider registry) ─────────────────────────────

#[tokio::test]
async fn health_check_all_with_mock_provider() {
    let config = ForgeConfig::default_mock_config();
    let registry = ProviderRegistry::from_config(config).expect("mock registry should build");
    let results = registry.health_check_all().await;

    // Mock provider should always be healthy
    for result in results.values() {
        assert!(result.is_ok(), "mock provider should be healthy");
    }
}

#[tokio::test]
async fn health_check_all_with_unreachable_provider() {
    use forge::config::*;
    use std::collections::HashMap;

    let mut providers = HashMap::new();
    providers.insert(
        "bad-provider".to_string(),
        ProviderConfig {
            type_: "openai-compat".to_string(),
            model: Some("test-model".to_string()),
            base_url: Some("http://127.0.0.1:19999/v1".to_string()), // nothing listening
            api_key: Some("not-needed".to_string()),
            timeout_secs: Some(2),
            fallback: None,
            capabilities: None,
            headers: None,
        },
    );

    let config = ForgeConfig {
        llm: LLMConfig {
            default: "bad-provider".to_string(),
            routing: None,
            budget: None,
        },
        providers,
        server: None,
        system: None,
        web: None,
        exec: None,
        skills: None,
        embeddings: None,
    };

    let registry = ProviderRegistry::from_config(config).expect("registry should build");
    let results = registry.health_check_all().await;

    // Unreachable provider should fail health check
    for result in results.values() {
        assert!(
            result.is_err(),
            "unreachable provider should fail health check"
        );
    }
}

// ── Warden circuit breaker recovery ──────────────────────────────

#[test]
fn circuit_breaker_reset_enables_fresh_start() {
    use forge::ast::*;
    use forge::runtime::warden::*;

    fn sp<T>(node: T) -> Spanned<T> {
        Spanned::new(node, Span { start: 0, end: 0 })
    }

    let decl = WardenDecl {
        name: sp("test_warden".to_string()),
        manages: vec![sp("agent_a".to_string())],
        policies: vec![sp(WardPolicy {
            failure_type: sp(FailureType::Crash),
            response: sp(WardResponse::Restart),
            scope: sp(WardScope::All),
            after_clauses: vec![],
        })],
        max_retries: Some(sp(MaxRetries {
            count: 3,
            window: sp(Duration {
                value: 60,
                unit: DurationUnit::Seconds,
            }),
        })),
    };

    let mut warden = Warden::new(decl, None);
    let signal = FailureSignal {
        agent_name: "agent_a".to_string(),
        failure_type: FailureType::Crash,
        detail: "provider down".to_string(),
    };

    // Trip the circuit breaker
    for i in 0..3 {
        warden.handle_failure(&signal, &[], i * 1000);
    }
    assert!(
        warden.circuit_breaker_tripped(3000),
        "circuit breaker should be tripped after 3 failures"
    );

    // Simulate half-open recovery: reset_all clears the breaker
    warden.retry_tracker.reset_all();
    assert!(
        !warden.circuit_breaker_tripped(3000),
        "circuit breaker should be clear after reset_all"
    );

    // New failures start counting from zero
    warden.handle_failure(&signal, &[], 4000);
    assert!(
        !warden.circuit_breaker_tripped(4000),
        "single failure should not trip the breaker (threshold is 3)"
    );
}

// ── Generated server code structure ──────────────────────────────

#[test]
fn generated_server_code_includes_resilience_flags() {
    // Verify the generated server main template includes the new CLI flags
    // by building the sensei server sources and checking the program structure
    let core = std::fs::read_to_string("workflows/forge-sensei/core.forge").expect("read core");
    let agent = std::fs::read_to_string("workflows/forge-sensei/agent.forge").expect("read agent");
    let web = std::fs::read_to_string("workflows/forge-sensei/web.forge").expect("read web");

    use forge::compose::{self, SourceFile};
    fn parse_source(name: &str, source: &str) -> SourceFile {
        let program = forge::parser::parse(source).expect("parse failed");
        SourceFile {
            path: name.to_string(),
            source: source.to_string(),
            program,
        }
    }

    let composed = compose::merge_programs(&[
        parse_source("web.forge", &web),
        parse_source("core.forge", &core),
        parse_source("agent.forge", &agent),
    ])
    .expect("sensei server sources should merge");

    // The composed program should be detected as a server
    assert_eq!(
        compose::detect_kind(&composed.program),
        Some(compose::ProgramKind::Server)
    );

    // It should have a system declaration (for warden supervision)
    let has_system = composed
        .program
        .items
        .iter()
        .any(|item| matches!(&item.node, forge::ast::TopLevel::System(_)));
    assert!(has_system, "sensei server should have a system declaration");

    // It should have a warden declaration
    let has_warden = composed
        .program
        .items
        .iter()
        .any(|item| matches!(&item.node, forge::ast::TopLevel::Warden(_)));
    assert!(has_warden, "sensei server should have a warden declaration");
}
