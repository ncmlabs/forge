// Integration tests for FORGE HTTP client (issue #51)
// Spins up local mock servers to verify web.fetch, web.post, search,
// error handling, and timeout behavior end-to-end.

use std::collections::HashMap;

use std::sync::Arc;
use std::time::Duration;

use axum::extract::Query;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::Router;
use tokio::net::TcpListener;

use forge::config::WebConfig;
use forge::runtime::http_client::{search, ForgeHttpClient};

// ── Helpers ──────────────────────────────────────────────────────

/// Start a mock HTTP server on a random port, return its base URL.
async fn start_mock_server(app: Router) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{}", addr)
}

// ── web.fetch tests ──────────────────────────────────────────────

#[tokio::test]
async fn fetch_returns_body_as_text() {
    let app = Router::new().route("/hello", get(|| async { "Hello from mock server" }));
    let base = start_mock_server(app).await;
    let client = ForgeHttpClient::new(None);

    let result = client.fetch(&format!("{}/hello", base)).await;
    assert_eq!(result.unwrap(), "Hello from mock server");
}

#[tokio::test]
async fn fetch_returns_error_on_404() {
    let app = Router::new().route("/exists", get(|| async { "ok" }));
    let base = start_mock_server(app).await;
    let client = ForgeHttpClient::new(None);

    let url = format!("{}/missing", base);
    let result = client.fetch(&url).await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.contains("HTTP 404"), "expected 404 error, got: {}", err);
}

#[tokio::test]
async fn fetch_returns_error_on_500() {
    let app = Router::new().route(
        "/error",
        get(|| async { (StatusCode::INTERNAL_SERVER_ERROR, "server broke") }),
    );
    let base = start_mock_server(app).await;
    let client = ForgeHttpClient::new(None);

    let url = format!("{}/error", base);
    let result = client.fetch(&url).await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.contains("HTTP 500"), "expected 500 error, got: {}", err);
}

#[tokio::test]
async fn fetch_returns_connection_error_for_invalid_host() {
    let client = ForgeHttpClient::new(None);
    let result = client.fetch("http://127.0.0.1:1").await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.contains("connection failed"),
        "expected connection error, got: {}",
        err
    );
}

#[tokio::test]
async fn fetch_returns_timeout_error() {
    let app = Router::new().route(
        "/slow",
        get(|| async {
            tokio::time::sleep(Duration::from_secs(5)).await;
            "too late"
        }),
    );
    let base = start_mock_server(app).await;

    // 1-second timeout
    let config = WebConfig {
        timeout_secs: Some(1),
        max_redirects: None,
        search_provider: None,
        search_api_key: None,
        search_url: None,
    };
    let client = ForgeHttpClient::new(Some(&config));

    let url = format!("{}/slow", base);
    let result = client.fetch(&url).await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.contains("timeout"),
        "expected timeout error, got: {}",
        err
    );
}

// ── web.post tests ───────────────────────────────────────────────

#[tokio::test]
async fn post_sends_body_and_returns_response() {
    let app = Router::new().route(
        "/echo",
        post(|body: String| async move { format!("echoed: {}", body) }),
    );
    let base = start_mock_server(app).await;
    let client = ForgeHttpClient::new(None);

    let result = client.post(&format!("{}/echo", base), "hello world").await;
    assert_eq!(result.unwrap(), "echoed: hello world");
}

#[tokio::test]
async fn post_returns_error_on_4xx() {
    let app = Router::new().route(
        "/reject",
        post(|| async { (StatusCode::BAD_REQUEST, "bad request") }),
    );
    let base = start_mock_server(app).await;
    let client = ForgeHttpClient::new(None);

    let url = format!("{}/reject", base);
    let result = client.post(&url, "data").await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.contains("HTTP 400"), "expected 400 error, got: {}", err);
}

// ── search tests ─────────────────────────────────────────────────

#[tokio::test]
async fn search_parses_searxng_json_response() {
    let app = Router::new().route(
        "/search",
        get(|Query(params): Query<HashMap<String, String>>| async move {
            let query = params.get("q").cloned().unwrap_or_default();
            let json = serde_json::json!({
                "results": [
                    {
                        "title": format!("Result about {}", query),
                        "url": "https://example.com/1",
                        "content": format!("Snippet about {}", query)
                    },
                    {
                        "title": "Second result",
                        "url": "https://example.com/2",
                        "content": "Another snippet"
                    }
                ]
            });
            axum::Json(json)
        }),
    );
    let base = start_mock_server(app).await;

    let config = WebConfig {
        timeout_secs: None,
        max_redirects: None,
        search_provider: Some("searxng".to_string()),
        search_api_key: None,
        search_url: Some(base),
    };
    let client = ForgeHttpClient::new(Some(&config));

    let results = search(&client, "rust programming", Some(&config))
        .await
        .unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].title, "Result about rust programming");
    assert_eq!(results[0].url, "https://example.com/1");
    assert_eq!(results[0].snippet, "Snippet about rust programming");
    assert_eq!(results[1].title, "Second result");
}

#[tokio::test]
async fn search_returns_empty_list_for_no_results() {
    let app = Router::new().route(
        "/search",
        get(|| async { axum::Json(serde_json::json!({ "results": [] })) }),
    );
    let base = start_mock_server(app).await;

    let config = WebConfig {
        timeout_secs: None,
        max_redirects: None,
        search_provider: Some("searxng".to_string()),
        search_api_key: None,
        search_url: Some(base),
    };
    let client = ForgeHttpClient::new(Some(&config));

    let results = search(&client, "nothing", Some(&config)).await.unwrap();
    assert!(results.is_empty());
}

#[tokio::test]
async fn search_returns_error_for_unsupported_provider() {
    let config = WebConfig {
        timeout_secs: None,
        max_redirects: None,
        search_provider: Some("nonexistent".to_string()),
        search_api_key: None,
        search_url: None,
    };
    let client = ForgeHttpClient::new(Some(&config));

    let result = search(&client, "test", Some(&config)).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("unsupported search provider"));
}

#[tokio::test]
async fn search_returns_error_when_server_unavailable() {
    let config = WebConfig {
        timeout_secs: Some(1),
        max_redirects: None,
        search_provider: Some("searxng".to_string()),
        search_api_key: None,
        search_url: Some("http://127.0.0.1:1".to_string()),
    };
    let client = ForgeHttpClient::new(Some(&config));

    let result = search(&client, "test", Some(&config)).await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.contains("connection failed"),
        "expected connection error, got: {}",
        err
    );
}

// ── Config tests ─────────────────────────────────────────────────

#[tokio::test]
async fn config_defaults_are_applied() {
    let config = WebConfig {
        timeout_secs: None,
        max_redirects: None,
        search_provider: None,
        search_api_key: None,
        search_url: None,
    };
    assert_eq!(config.timeout_or_default(), 30);
    assert_eq!(config.max_redirects_or_default(), 10);
}

#[tokio::test]
async fn config_custom_values_are_used() {
    let config = WebConfig {
        timeout_secs: Some(60),
        max_redirects: Some(3),
        search_provider: Some("searxng".to_string()),
        search_api_key: Some("key123".to_string()),
        search_url: Some("https://search.example.com".to_string()),
    };
    assert_eq!(config.timeout_or_default(), 60);
    assert_eq!(config.max_redirects_or_default(), 3);
}

#[test]
fn config_toml_with_web_section_parses() {
    let toml_str = r#"
[llm]
default = "mock"

[providers.mock]
type = "mock"

[web]
timeout_secs = 15
max_redirects = 3
search_provider = "searxng"
search_url = "http://localhost:9090"
"#;
    let config: forge::config::ForgeConfig = toml::from_str(toml_str).unwrap();
    let web = config.web.unwrap();
    assert_eq!(web.timeout_or_default(), 15);
    assert_eq!(web.max_redirects_or_default(), 3);
    assert_eq!(web.search_provider.unwrap(), "searxng");
    assert_eq!(web.search_url.unwrap(), "http://localhost:9090");
}

#[test]
fn config_toml_without_web_section_parses() {
    let toml_str = r#"
[llm]
default = "mock"

[providers.mock]
type = "mock"
"#;
    let config: forge::config::ForgeConfig = toml::from_str(toml_str).unwrap();
    assert!(config.web.is_none());
}

// ── Executor integration (web.fetch via .forge program) ──────────

use forge::llm::providers::mock::MockProvider;
use forge::llm::registry::ProviderRegistry;
use forge::runtime::executor::TaskExecutor;

fn mock_registry(mock: MockProvider) -> Arc<ProviderRegistry> {
    let mut reg = ProviderRegistry::new("mock");
    reg.register("mock", Arc::new(mock));
    Arc::new(reg)
}

#[tokio::test]
async fn executor_web_fetch_returns_text() {
    let app = Router::new().route("/data", get(|| async { "server response" }));
    let base = start_mock_server(app).await;

    let source = format!(
        r#"
#! boundary: server

fn main
  page = web.fetch("{}/data")
  say page
"#,
        base
    );
    let program = forge::parser::parse(&source).unwrap();
    let mock = MockProvider::new("mock").with_default("mock");
    let config = forge::config::ForgeConfig::default_mock_config();
    let executor = TaskExecutor::new(program, mock_registry(mock), None).with_config(config);
    let result = executor.run().await;
    assert!(
        result.is_ok(),
        "executor should succeed: {:?}",
        result.err()
    );
    let outputs = executor.outputs();
    assert_eq!(outputs, vec!["server response"]);
}

#[tokio::test]
async fn executor_web_post_returns_response() {
    let app = Router::new().route(
        "/submit",
        post(|body: String| async move { format!("got: {}", body) }),
    );
    let base = start_mock_server(app).await;

    let source = format!(
        r#"
#! boundary: server

fn main
  resp = web.post("{}/submit", "payload")
  say resp
"#,
        base
    );
    let program = forge::parser::parse(&source).unwrap();
    let mock = MockProvider::new("mock").with_default("mock");
    let config = forge::config::ForgeConfig::default_mock_config();
    let executor = TaskExecutor::new(program, mock_registry(mock), None).with_config(config);
    let result = executor.run().await;
    assert!(
        result.is_ok(),
        "executor should succeed: {:?}",
        result.err()
    );
    let outputs = executor.outputs();
    assert_eq!(outputs, vec!["got: payload"]);
}

#[tokio::test]
async fn executor_web_fetch_error_caught_by_try_or() {
    let source = r#"
#! boundary: server

fn main
  result = try web.fetch("http://127.0.0.1:1/bad") or "fallback"
  say result
"#;
    let program = forge::parser::parse(source).unwrap();
    let mock = MockProvider::new("mock").with_default("mock");
    let config = forge::config::ForgeConfig::default_mock_config();
    let executor = TaskExecutor::new(program, mock_registry(mock), None).with_config(config);
    let result = executor.run().await;
    assert!(
        result.is_ok(),
        "try/or should catch error: {:?}",
        result.err()
    );
    let outputs = executor.outputs();
    assert_eq!(outputs, vec!["fallback"]);
}

#[tokio::test]
async fn executor_search_returns_list_of_records() {
    let app = Router::new().route(
        "/search",
        get(|| async {
            axum::Json(serde_json::json!({
                "results": [
                    {"title": "Found it", "url": "https://found.com", "content": "A snippet"}
                ]
            }))
        }),
    );
    let base = start_mock_server(app).await;

    let source = r#"
#! boundary: server

fn main
  results = search "test query"
  first = results[0]
  say first.title
"#;
    let program = forge::parser::parse(source).unwrap();
    let mock = MockProvider::new("mock").with_default("mock");
    let mut config = forge::config::ForgeConfig::default_mock_config();
    config.web = Some(forge::config::WebConfig {
        timeout_secs: None,
        max_redirects: None,
        search_provider: Some("searxng".to_string()),
        search_api_key: None,
        search_url: Some(base),
    });
    let executor = TaskExecutor::new(program, mock_registry(mock), None).with_config(config);
    let result = executor.run().await;
    assert!(result.is_ok(), "search should succeed: {:?}", result.err());
    let outputs = executor.outputs();
    assert_eq!(outputs, vec!["Found it"]);
}
