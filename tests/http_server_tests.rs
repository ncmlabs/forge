// FORGE HTTP server integration tests — issue #43

use std::sync::Arc;

use forge::config::ServerConfig;
use forge::llm::registry::ProviderRegistry;
use forge::runtime::executor::TaskExecutor;
use forge::runtime::http_server::ForgeServer;

// ── Helpers ──────────────────────────────────────────────────────

fn mock_registry() -> Arc<ProviderRegistry> {
    let config = forge::config::ForgeConfig::default_mock_config();
    Arc::new(ProviderRegistry::from_config(config).expect("mock registry should build"))
}

fn parse_and_build(source: &str) -> TaskExecutor {
    let program = forge::parser::parse(source).expect("parse failed");
    TaskExecutor::new(program, mock_registry(), None)
}

/// Spawn a server on a random port and return the base URL.
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

    // Give the server a moment to start
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    format!("http://127.0.0.1:{port}")
}

// ── Tests ────────────────────────────────────────────────────────

const HELLO_SERVER: &str = r#"#! boundary: server

task greet
  needs name: Text
  gives Text
  do
    give "Hello, {name}!"

endpoint hello(name: Text) -> Text
  result = greet(name)
  give result
"#;

#[tokio::test]
async fn get_endpoint_with_query_param() {
    let base = spawn_server(HELLO_SERVER).await;
    let resp = reqwest::get(format!("{base}/hello?name=world"))
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert_eq!(body, "Hello, world!");
}

#[tokio::test]
async fn post_endpoint_with_json_body() {
    let base = spawn_server(HELLO_SERVER).await;
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base}/hello"))
        .json(&serde_json::json!({"name": "world"}))
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert_eq!(body, "Hello, world!");
}

#[tokio::test]
async fn unknown_route_returns_404() {
    let base = spawn_server(HELLO_SERVER).await;
    let resp = reqwest::get(format!("{base}/nonexistent"))
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 404);
}

const NO_PARAMS_SERVER: &str = r#"#! boundary: server

endpoint ping() -> Text
  give "pong"
"#;

#[tokio::test]
async fn endpoint_with_no_params() {
    let base = spawn_server(NO_PARAMS_SERVER).await;
    let resp = reqwest::get(format!("{base}/ping"))
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert_eq!(body, "pong");
}

#[tokio::test]
async fn endpoint_map_populated() {
    let executor = parse_and_build(HELLO_SERVER);
    let endpoints = executor.endpoints();
    assert_eq!(endpoints.len(), 1);
    assert!(endpoints.contains_key("hello"));
}

#[tokio::test]
async fn server_config_defaults() {
    let config = ServerConfig {
        host: None,
        port: None,
        cors_origins: None,
    };
    assert_eq!(config.host_or_default(), "127.0.0.1");
    assert_eq!(config.port_or_default(), 3000);
}

#[tokio::test]
async fn server_config_custom() {
    let config = ServerConfig {
        host: Some("0.0.0.0".to_string()),
        port: Some(8080),
        cors_origins: Some(vec!["http://localhost:3000".to_string()]),
    };
    assert_eq!(config.host_or_default(), "0.0.0.0");
    assert_eq!(config.port_or_default(), 8080);
}

const MULTI_ENDPOINT_SERVER: &str = r#"#! boundary: server

endpoint greet(name: Text) -> Text
  give "Hi, {name}!"

endpoint health() -> Text
  give "ok"
"#;

#[tokio::test]
async fn multiple_endpoints_registered() {
    let base = spawn_server(MULTI_ENDPOINT_SERVER).await;

    let resp = reqwest::get(format!("{base}/greet?name=Alice"))
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.text().await.unwrap(), "Hi, Alice!");

    let resp = reqwest::get(format!("{base}/health"))
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.text().await.unwrap(), "ok");
}

// ── Issue #44: Request injection and response metadata ──────────

const REQUEST_ECHO_SERVER: &str = r#"#! boundary: server

endpoint echo() -> Text
  give request.method
"#;

#[tokio::test]
async fn request_method_injected_get() {
    let base = spawn_server(REQUEST_ECHO_SERVER).await;
    let resp = reqwest::get(format!("{base}/echo"))
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.text().await.unwrap(), "GET");
}

#[tokio::test]
async fn request_method_injected_post() {
    let base = spawn_server(REQUEST_ECHO_SERVER).await;
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base}/echo"))
        .json(&serde_json::json!({}))
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.text().await.unwrap(), "POST");
}

const REQUEST_PATH_SERVER: &str = r#"#! boundary: server

endpoint info() -> Text
  give request.path
"#;

#[tokio::test]
async fn request_path_injected() {
    let base = spawn_server(REQUEST_PATH_SERVER).await;
    let resp = reqwest::get(format!("{base}/info"))
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.text().await.unwrap(), "/info");
}

const REQUEST_BODY_SERVER: &str = r#"#! boundary: server

endpoint body_echo() -> Text
  give request.body
"#;

#[tokio::test]
async fn request_body_injected_post() {
    let base = spawn_server(REQUEST_BODY_SERVER).await;
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base}/body_echo"))
        .json(&serde_json::json!({"key": "value"}))
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(body.contains("key"));
    assert!(body.contains("value"));
}

const STATUS_SERVER: &str = r#"#! boundary: server

endpoint not_found() -> Text
  give "gone" with status: 404
"#;

#[tokio::test]
async fn give_with_status_code() {
    let base = spawn_server(STATUS_SERVER).await;
    let resp = reqwest::get(format!("{base}/not_found"))
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 404);
    assert_eq!(resp.text().await.unwrap(), "gone");
}

const CONTENT_TYPE_SERVER: &str = r#"#! boundary: server

endpoint page() -> Text
  give "<h1>Hi</h1>" with content_type: "text/html"
"#;

#[tokio::test]
async fn give_with_content_type() {
    let base = spawn_server(CONTENT_TYPE_SERVER).await;
    let resp = reqwest::get(format!("{base}/page"))
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 200);
    let ct = resp
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(ct, "text/html");
    assert_eq!(resp.text().await.unwrap(), "<h1>Hi</h1>");
}

const MULTI_META_SERVER: &str = r#"#! boundary: server

endpoint created() -> Text
  give "done" with status: 201, content_type: "application/json"
"#;

#[tokio::test]
async fn give_with_status_and_content_type() {
    let base = spawn_server(MULTI_META_SERVER).await;
    let resp = reqwest::get(format!("{base}/created"))
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 201);
    let ct = resp
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(ct, "application/json");
    assert_eq!(resp.text().await.unwrap(), "done");
}

const DEFAULT_STATUS_SERVER: &str = r#"#! boundary: server

endpoint ok_endpoint() -> Text
  give "all good"
"#;

#[tokio::test]
async fn default_status_is_200() {
    let base = spawn_server(DEFAULT_STATUS_SERVER).await;
    let resp = reqwest::get(format!("{base}/ok_endpoint"))
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 200);
}
