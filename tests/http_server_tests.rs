// FORGE HTTP server integration tests — issue #43

use std::sync::Arc;

use forge::config::{ServerConfig, StaticConfig};
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

/// Spawn a server with custom config on a random port and return the base URL.
async fn spawn_server_with_config(source: &str, config: Option<&ServerConfig>) -> String {
    let executor = parse_and_build(source);
    let server = ForgeServer::new(executor, config);

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

/// Spawn a server with config attached to executor (for asset() tests).
async fn spawn_server_with_full_config(
    source: &str,
    forge_config: forge::config::ForgeConfig,
) -> String {
    let program = forge::parser::parse(source).expect("parse failed");
    let executor =
        TaskExecutor::new(program, mock_registry(), None).with_config(forge_config.clone());
    let server = ForgeServer::new(executor, forge_config.server.as_ref());

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

const API_TYPED_SERVER: &str = r#"#! boundary: server

type TypedPayload
  score: Number
  first: Text
  enabled: Bool

endpoint api_typed(score: Number, items: Text[], enabled: Bool) -> TypedPayload
  give TypedPayload(score: score + 1, first: items[0], enabled: enabled)
"#;

#[tokio::test]
async fn api_route_maps_to_prefixed_endpoint_and_preserves_json_types() {
    let base = spawn_server(API_TYPED_SERVER).await;
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base}/api/typed"))
        .json(&serde_json::json!({
            "score": 41,
            "items": ["alpha", "beta"],
            "enabled": true
        }))
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 200);
    let ct = resp
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(ct, "application/json");
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["score"], 42.0);
    assert_eq!(body["first"], "alpha");
    assert_eq!(body["enabled"], true);
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
        static_files: None,
        webhook_secrets: None,
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
        static_files: None,
        webhook_secrets: None,
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

// ── Issue #44: Html type and content-type inference ───────────

const HTML_ENDPOINT_SERVER: &str = r#"#! boundary: server

endpoint page() -> Html
  give "<h1>Hello</h1>"
"#;

#[tokio::test]
async fn html_return_type_infers_content_type() {
    let base = spawn_server(HTML_ENDPOINT_SERVER).await;
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
    let body = resp.text().await.unwrap();
    assert!(body.contains("<h1>Hello</h1>"));
}

const HTML_OVERRIDE_SERVER: &str = r#"#! boundary: server

endpoint page() -> Html
  give "<data/>" with content_type: "application/xml"
"#;

#[tokio::test]
async fn explicit_content_type_overrides_return_type() {
    let base = spawn_server(HTML_OVERRIDE_SERVER).await;
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
    assert_eq!(ct, "application/xml");
}

// ── Issue #44: Missing record fields return Unit ─────────────

const MISSING_QUERY_SERVER: &str = r#"#! boundary: server

endpoint check() -> Text
  val = request.query.nonexistent
  give val
"#;

#[tokio::test]
async fn missing_query_field_returns_unit() {
    let base = spawn_server(MISSING_QUERY_SERVER).await;
    let resp = reqwest::get(format!("{base}/check"))
        .await
        .expect("request failed");
    // Unit maps to 204 No Content
    assert_eq!(resp.status(), 204);
}

const MISSING_HEADER_SERVER: &str = r#"#! boundary: server

endpoint check() -> Text
  val = request.headers.x_nonexistent
  give val
"#;

#[tokio::test]
async fn missing_header_field_returns_unit() {
    let base = spawn_server(MISSING_HEADER_SERVER).await;
    let resp = reqwest::get(format!("{base}/check"))
        .await
        .expect("request failed");
    // Unit maps to 204 No Content
    assert_eq!(resp.status(), 204);
}

// ── Issue #45: Html auto-escaping and composition ──────────────

const HTML_ESCAPE_SERVER: &str = r#"#! boundary: server

endpoint safe(name: Text) -> Html
  give "<p>Hello, {name}</p>"
"#;

#[tokio::test]
async fn html_auto_escapes_interpolation() {
    let base = spawn_server(HTML_ESCAPE_SERVER).await;
    let resp = reqwest::get(format!("{base}/safe?name=<script>alert(1)</script>"))
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    // The interpolated value should be escaped
    assert!(body.contains("&lt;script&gt;"));
    assert!(!body.contains("<script>alert"));
}

const HTML_RAW_SERVER: &str = r#"#! boundary: server

pure nav
  gives Html
  do
    give "<nav>Home</nav>"

endpoint page() -> Html
  n = nav()
  give "<div>{!n}</div>"
"#;

#[tokio::test]
async fn html_raw_interp_bypasses_escaping() {
    let base = spawn_server(HTML_RAW_SERVER).await;
    let resp = reqwest::get(format!("{base}/page"))
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    // {!n} should insert raw HTML without escaping
    assert!(body.contains("<nav>Home</nav>"));
}

const HTML_LAYOUT_SERVER: &str = r#"#! boundary: server

endpoint doc() -> Html
  give html.layout("My Page", "<p>content</p>")
"#;

#[tokio::test]
async fn html_layout_produces_document() {
    let base = spawn_server(HTML_LAYOUT_SERVER).await;
    let resp = reqwest::get(format!("{base}/doc"))
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
    let body = resp.text().await.unwrap();
    assert!(body.starts_with("<!DOCTYPE html>"));
    assert!(body.contains("<title>My Page</title>"));
    assert!(body.contains("<p>content</p>"));
}

const HTML_COMPOSE_SERVER: &str = r#"#! boundary: server

pure header
  gives Html
  do
    give "<header>FORGE</header>"

pure footer
  gives Html
  do
    give "<footer>2026</footer>"

endpoint composed() -> Html
  h = header()
  f = footer()
  give "<div>{!h}<main>body</main>{!f}</div>"
"#;

#[tokio::test]
async fn html_composition_via_pure_functions() {
    let base = spawn_server(HTML_COMPOSE_SERVER).await;
    let resp = reqwest::get(format!("{base}/composed"))
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(body.contains("<header>FORGE</header>"));
    assert!(body.contains("<main>body</main>"));
    assert!(body.contains("<footer>2026</footer>"));
}

// ── Static file serving tests (issue #46) ───────────────────────

#[tokio::test]
async fn static_file_served_with_correct_mime() {
    let tmp = tempfile::tempdir().expect("tmpdir");
    let css_dir = tmp.path().join("css");
    std::fs::create_dir_all(&css_dir).unwrap();
    std::fs::write(css_dir.join("style.css"), "body { color: red }").unwrap();

    let config = ServerConfig {
        host: None,
        port: None,
        cors_origins: None,
        webhook_secrets: None,
        static_files: Some(StaticConfig {
            root: Some(tmp.path().to_str().unwrap().to_string()),
            prefix: Some("/static".to_string()),
        }),
    };

    let source = r#"#! boundary: server
endpoint ping()
  give "pong"
"#;
    let base = spawn_server_with_config(source, Some(&config)).await;
    let resp = reqwest::get(format!("{base}/static/css/style.css"))
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 200);
    let ct = resp
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(ct.contains("text/css"), "expected text/css, got {ct}");
    let body = resp.text().await.unwrap();
    assert_eq!(body, "body { color: red }");
}

#[tokio::test]
async fn static_file_404_for_missing() {
    let tmp = tempfile::tempdir().expect("tmpdir");

    let config = ServerConfig {
        host: None,
        port: None,
        cors_origins: None,
        webhook_secrets: None,
        static_files: Some(StaticConfig {
            root: Some(tmp.path().to_str().unwrap().to_string()),
            prefix: Some("/static".to_string()),
        }),
    };

    let source = r#"#! boundary: server
endpoint ping()
  give "pong"
"#;
    let base = spawn_server_with_config(source, Some(&config)).await;
    let resp = reqwest::get(format!("{base}/static/nope.js"))
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn static_does_not_shadow_dynamic_routes() {
    let tmp = tempfile::tempdir().expect("tmpdir");
    // Create a file that would match the endpoint name
    std::fs::write(tmp.path().join("ping"), "static content").unwrap();

    let config = ServerConfig {
        host: None,
        port: None,
        cors_origins: None,
        webhook_secrets: None,
        static_files: Some(StaticConfig {
            root: Some(tmp.path().to_str().unwrap().to_string()),
            prefix: Some("/static".to_string()),
        }),
    };

    let source = r#"#! boundary: server
endpoint ping()
  give "dynamic pong"
"#;
    let base = spawn_server_with_config(source, Some(&config)).await;
    // Dynamic route should win
    let resp = reqwest::get(format!("{base}/ping"))
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert_eq!(body, "dynamic pong");
}

#[tokio::test]
async fn asset_function_returns_prefixed_path() {
    let source = r#"#! boundary: server
endpoint link() -> Html
  url = asset("css/style.css")
  give "<link href='{url}'>"
"#;
    let mut config = forge::config::ForgeConfig::default_mock_config();
    config.server = Some(ServerConfig {
        host: None,
        port: None,
        cors_origins: None,
        webhook_secrets: None,
        static_files: Some(StaticConfig {
            root: Some("static".to_string()),
            prefix: Some("/static".to_string()),
        }),
    });

    let base = spawn_server_with_full_config(source, config).await;
    let resp = reqwest::get(format!("{base}/link"))
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(
        body.contains("/static/css/style.css"),
        "expected /static/css/style.css in: {body}"
    );
}

#[tokio::test]
async fn asset_function_custom_prefix() {
    let source = r#"#! boundary: server
endpoint img_url()
  url = asset("img/logo.png")
  give url
"#;
    let mut config = forge::config::ForgeConfig::default_mock_config();
    config.server = Some(ServerConfig {
        host: None,
        port: None,
        cors_origins: None,
        webhook_secrets: None,
        static_files: Some(StaticConfig {
            root: Some("public".to_string()),
            prefix: Some("/assets".to_string()),
        }),
    });

    let base = spawn_server_with_full_config(source, config).await;
    let resp = reqwest::get(format!("{base}/img_url"))
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert_eq!(body, "/assets/img/logo.png");
}

// ── Issue #52: Webhook support ────────────────────────────────────

const WEBHOOK_SERVER: &str = r#"#! boundary: server

endpoint hook(payload: Text) -> Text
  give "received: {payload}"
"#;

/// Spawn a server with event bus attached for webhook tests.
async fn spawn_webhook_server(source: &str) -> String {
    let program = forge::parser::parse(source).expect("parse failed");
    let executor = TaskExecutor::new(program, mock_registry(), None);
    let event_bus = forge::runtime::event_bus::EventBus::new_shared(None);
    let server = ForgeServer::new(executor, None).with_event_bus(event_bus);

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

/// Spawn a server with HMAC secrets for signature verification tests.
async fn spawn_webhook_server_with_secrets(
    source: &str,
    secrets: std::collections::HashMap<String, String>,
) -> String {
    let program = forge::parser::parse(source).expect("parse failed");
    let executor = TaskExecutor::new(program, mock_registry(), None);
    let event_bus = forge::runtime::event_bus::EventBus::new_shared(None);
    let server = ForgeServer::new(executor, None)
        .with_event_bus(event_bus)
        .with_webhook_secrets(secrets);

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

#[tokio::test]
async fn webhook_post_dispatches_to_endpoint() {
    let base = spawn_webhook_server(WEBHOOK_SERVER).await;
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base}/webhook/hook"))
        .header("content-type", "application/json")
        .body(r#"{"payload": "hello"}"#)
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert_eq!(body, "received: hello");
}

#[tokio::test]
async fn webhook_json_body_parsed_as_text_param() {
    let base = spawn_webhook_server(WEBHOOK_SERVER).await;
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base}/webhook/hook"))
        .header("content-type", "application/json")
        .body(r#"{"payload": "test data"}"#)
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(body.contains("test data"));
}

#[tokio::test]
async fn webhook_invalid_json_returns_400() {
    let base = spawn_webhook_server(WEBHOOK_SERVER).await;
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base}/webhook/hook"))
        .header("content-type", "application/json")
        .body("not json at all {{{")
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn webhook_wrong_content_type_returns_400() {
    let base = spawn_webhook_server(WEBHOOK_SERVER).await;
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base}/webhook/hook"))
        .header("content-type", "text/plain")
        .body(r#"{"payload": "hello"}"#)
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 400);
    let body = resp.text().await.unwrap();
    assert!(body.contains("application/json"));
}

#[tokio::test]
async fn webhook_nonexistent_endpoint_returns_404() {
    let base = spawn_webhook_server(WEBHOOK_SERVER).await;
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base}/webhook/nonexistent"))
        .header("content-type", "application/json")
        .body(r#"{}"#)
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn webhook_hmac_valid_signature_accepted() {
    use hmac::{Hmac, KeyInit, Mac};
    use sha2::Sha256;

    let secret = "test-secret-123";
    let mut secrets = std::collections::HashMap::new();
    secrets.insert("hook".to_string(), secret.to_string());

    let base = spawn_webhook_server_with_secrets(WEBHOOK_SERVER, secrets).await;

    let body = r#"{"payload": "signed"}"#;
    let mut mac: Hmac<Sha256> = Hmac::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(body.as_bytes());
    let signature = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base}/webhook/hook"))
        .header("content-type", "application/json")
        .header("x-hub-signature-256", &signature)
        .body(body)
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(body.contains("signed"));
}

#[tokio::test]
async fn webhook_hmac_invalid_signature_rejected() {
    let mut secrets = std::collections::HashMap::new();
    secrets.insert("hook".to_string(), "real-secret".to_string());

    let base = spawn_webhook_server_with_secrets(WEBHOOK_SERVER, secrets).await;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base}/webhook/hook"))
        .header("content-type", "application/json")
        .header("x-hub-signature-256", "sha256=deadbeef")
        .body(r#"{"payload": "hacked"}"#)
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn webhook_hmac_missing_signature_rejected() {
    let mut secrets = std::collections::HashMap::new();
    secrets.insert("hook".to_string(), "my-secret".to_string());

    let base = spawn_webhook_server_with_secrets(WEBHOOK_SERVER, secrets).await;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base}/webhook/hook"))
        .header("content-type", "application/json")
        .body(r#"{"payload": "unsigned"}"#)
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 401);
}

const WEBHOOK_EMIT_SERVER: &str = r#"#! boundary: server

event WebhookReceived
  data: Text

endpoint github_push(payload: Text) -> Text
  emit WebhookReceived(data: payload)
  give "ok"
"#;

#[tokio::test]
async fn webhook_endpoint_can_emit_events() {
    let base = spawn_webhook_server(WEBHOOK_EMIT_SERVER).await;
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base}/webhook/github_push"))
        .header("content-type", "application/json")
        .body(r#"{"payload": "push event data"}"#)
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.text().await.unwrap(), "ok");
}

const WEBHOOK_REQUEST_CONTEXT_SERVER: &str = r#"#! boundary: server

endpoint inspect() -> Text
  method = request.method
  body = request.body
  give "{method}: {body}"
"#;

#[tokio::test]
async fn webhook_request_context_available() {
    let base = spawn_webhook_server(WEBHOOK_REQUEST_CONTEXT_SERVER).await;
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base}/webhook/inspect"))
        .header("content-type", "application/json")
        .body(r#"{"key": "value"}"#)
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(body.starts_with("POST:"));
    assert!(body.contains("key"));
}

#[tokio::test]
async fn webhook_no_secret_configured_skips_verification() {
    // When no secret is configured for this endpoint, requests without signatures should work
    let base = spawn_webhook_server(WEBHOOK_SERVER).await;
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base}/webhook/hook"))
        .header("content-type", "application/json")
        .body(r#"{"payload": "no secret needed"}"#)
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 200);
}

// ── Multi-file serve support ──────────────────────────────────

const MULTI_CORE: &str = r#"
pure greet_msg
  needs name: Text
  gives Text
  do
    give "Hello, {name}!"

pure add_numbers
  needs a: Number, b: Number
  gives Number
  do
    give a + b
"#;

const MULTI_WEB: &str = r#"#! boundary: server

endpoint hello(name: Text) -> Text
  give greet_msg(name)

endpoint math(a: Text) -> Text
  give "computed"
"#;

/// Spawn a server from multiple source files merged together.
async fn spawn_multi_file_server(sources: &[&str]) -> String {
    let mut source_files = Vec::new();
    for (i, src) in sources.iter().enumerate() {
        let program = forge::parser::parse(src).expect("parse failed");
        source_files.push(forge::compose::SourceFile {
            path: format!("file{i}.forge"),
            source: src.to_string(),
            program,
        });
    }
    let composed = forge::compose::merge_programs(&source_files).expect("merge failed");
    let executor = TaskExecutor::new(composed.program, mock_registry(), None);
    let event_bus = forge::runtime::event_bus::EventBus::new_shared(None);
    let server = ForgeServer::new(executor, None).with_event_bus(event_bus);

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

#[tokio::test]
async fn multi_file_endpoint_calls_pure_from_other_file() {
    let base = spawn_multi_file_server(&[MULTI_WEB, MULTI_CORE]).await;
    let resp = reqwest::get(format!("{base}/hello?name=World"))
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.text().await.unwrap(), "Hello, World!");
}

#[tokio::test]
async fn multi_file_endpoints_registered_from_server_boundary() {
    let base = spawn_multi_file_server(&[MULTI_WEB, MULTI_CORE]).await;

    // Endpoint from web file works
    let resp = reqwest::get(format!("{base}/hello?name=Test"))
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 200);

    // Second endpoint works too
    let resp = reqwest::get(format!("{base}/math?a=5"))
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn multi_file_unknown_route_returns_404() {
    let base = spawn_multi_file_server(&[MULTI_WEB, MULTI_CORE]).await;
    let resp = reqwest::get(format!("{base}/nonexistent"))
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn multi_file_post_with_json_body() {
    let base = spawn_multi_file_server(&[MULTI_WEB, MULTI_CORE]).await;
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base}/hello"))
        .json(&serde_json::json!({"name": "PostUser"}))
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.text().await.unwrap(), "Hello, PostUser!");
}

#[tokio::test]
async fn multi_file_webhook_route_works() {
    let base = spawn_multi_file_server(&[MULTI_WEB, MULTI_CORE]).await;
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base}/webhook/hello"))
        .header("content-type", "application/json")
        .body(r#"{"name": "WebhookUser"}"#)
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.text().await.unwrap(), "Hello, WebhookUser!");
}

// ── Multi-file with events and emit from endpoints ────────────

const MULTI_EVENTS_CORE: &str = r#"
event DataReceived
  payload: Text

task process_data
  needs data: Text
  gives Text
  do
    give "processed: {data}"
"#;

const MULTI_EVENTS_WEB: &str = r#"#! boundary: server

endpoint ingest(data: Text) -> Text
  emit DataReceived(payload: data)
  result = process_data(data)
  give result
"#;

#[tokio::test]
async fn multi_file_emit_from_endpoint_with_event_from_other_file() {
    let base = spawn_multi_file_server(&[MULTI_EVENTS_WEB, MULTI_EVENTS_CORE]).await;
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base}/webhook/ingest"))
        .header("content-type", "application/json")
        .body(r#"{"data": "test payload"}"#)
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.text().await.unwrap(), "processed: test payload");
}

// ── Multi-file with Html rendering ────────────────────────────

const MULTI_HTML_CORE: &str = r#"
pure render_header
  gives Html
  do
    give "<header>FORGE</header>"
"#;

const MULTI_HTML_WEB: &str = r#"#! boundary: server

endpoint page() -> Html
  h = render_header()
  give html.layout("Test Page", h)
"#;

#[tokio::test]
async fn multi_file_html_rendering_across_files() {
    let base = spawn_multi_file_server(&[MULTI_HTML_WEB, MULTI_HTML_CORE]).await;
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
    let body = resp.text().await.unwrap();
    assert!(body.contains("<!DOCTYPE html>"));
    assert!(body.contains("<title>Test Page</title>"));
    assert!(body.contains("<header>FORGE</header>"));
}

// ── forge-sensei web endpoint tests ───────────────────────────

/// Load and merge the actual forge-sensei source files.
async fn spawn_sensei_web_server() -> String {
    let source_files = sensei_server_source_files();
    let composed =
        forge::compose::merge_programs(&source_files).expect("merge sensei files failed");
    let executor = TaskExecutor::new(composed.program, mock_registry(), None);
    let event_bus = forge::runtime::event_bus::EventBus::new_shared(None);
    let server = ForgeServer::new(executor, None).with_event_bus(event_bus);

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

fn sensei_server_source_files() -> Vec<forge::compose::SourceFile> {
    [
        ("types.forge", "workflows/forge-sensei/shared/types.forge"),
        ("events.forge", "workflows/forge-sensei/shared/events.forge"),
        ("states.forge", "workflows/forge-sensei/shared/states.forge"),
        ("tasks.forge", "workflows/forge-sensei/server/tasks.forge"),
        ("flows.forge", "workflows/forge-sensei/server/flows.forge"),
        ("agent.forge", "workflows/forge-sensei/server/agent.forge"),
        ("web.forge", "workflows/forge-sensei/server/web.forge"),
    ]
    .into_iter()
    .map(|(name, path)| {
        let source = std::fs::read_to_string(path).unwrap_or_else(|_| panic!("read {path}"));
        let program =
            forge::parser::parse(&source).unwrap_or_else(|_| panic!("parse {path} failed"));
        forge::compose::SourceFile {
            path: name.to_string(),
            source,
            program,
        }
    })
    .collect()
}

#[tokio::test]
async fn sensei_status_returns_html_page() {
    let base = spawn_sensei_web_server().await;
    let resp = reqwest::get(format!("{base}/status"))
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
    let body = resp.text().await.unwrap();
    assert!(body.contains("<title>forge-sensei | Status</title>"));
    assert!(body.contains("Sensei Status"));
    assert!(body.contains("novice")); // default level
}

#[tokio::test]
async fn sensei_status_has_navigation() {
    let base = spawn_sensei_web_server().await;
    let body = reqwest::get(format!("{base}/status"))
        .await
        .expect("request failed")
        .text()
        .await
        .unwrap();
    // Nav links present
    assert!(body.contains("href=\"/status\""));
    assert!(body.contains("href=\"/ask\""));
    assert!(body.contains("href=\"/review\""));
}

#[tokio::test]
async fn sensei_ask_form_returns_html_with_textarea() {
    let base = spawn_sensei_web_server().await;
    let resp = reqwest::get(format!("{base}/ask_form"))
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(body.contains("<title>forge-sensei | Ask</title>"));
    assert!(body.contains("<textarea"));
    assert!(body.contains("name=\"question\""));
    assert!(body.contains("action=\"/ask\""));
}

#[tokio::test]
async fn sensei_review_form_returns_html_with_textarea() {
    let base = spawn_sensei_web_server().await;
    let resp = reqwest::get(format!("{base}/review_form"))
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(body.contains("<title>forge-sensei | Review</title>"));
    assert!(body.contains("<textarea"));
    assert!(body.contains("name=\"code\""));
    assert!(body.contains("action=\"/review\""));
}

#[tokio::test]
async fn sensei_ask_endpoint_dispatches() {
    // The ask endpoint calls answer_query flow which uses classify + reason (LLM).
    // With mock provider, we get a runtime error (500) — but the endpoint IS reached.
    // A 404 would mean routing failed; 500 means it dispatched and hit the LLM path.
    let base = spawn_sensei_web_server().await;
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base}/ask"))
        .json(&serde_json::json!({"question": "what is a task?"}))
        .send()
        .await
        .expect("request failed");
    let status = resp.status().as_u16();
    let body = resp.text().await.unwrap();
    // Either 200 (mock returned something) or 500 (runtime error from LLM path)
    assert!(
        status == 200 || status == 500,
        "expected 200 or 500, got {status}: {body}"
    );
    // Must NOT be 404 (that would mean endpoint not found)
    assert_ne!(status, 404);
    if status == 200 {
        assert!(body.contains("Answer"));
    } else {
        assert!(body.contains("runtime error"));
    }
}

#[tokio::test]
async fn sensei_review_endpoint_dispatches() {
    // Same as ask — the review flow calls classify + reason, which hits the mock provider.
    let base = spawn_sensei_web_server().await;
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base}/review"))
        .json(&serde_json::json!({"code": "task hello\n  gives Text\n  do\n    give hi"}))
        .send()
        .await
        .expect("request failed");
    let status = resp.status().as_u16();
    let body = resp.text().await.unwrap();
    assert!(
        status == 200 || status == 500,
        "expected 200 or 500, got {status}: {body}"
    );
    assert_ne!(status, 404);
    if status == 200 {
        assert!(body.contains("Review Results"));
    } else {
        assert!(body.contains("runtime error"));
    }
}

#[tokio::test]
async fn sensei_webhook_ingest_accepts_json() {
    let base = spawn_sensei_web_server().await;
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base}/webhook/webhook_ingest"))
        .header("content-type", "application/json")
        .body(r#"{"category": "SYNTAX", "fact": "tasks use reason keyword"}"#)
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.text().await.unwrap(), "ok");
}

#[tokio::test]
async fn sensei_webhook_learn_accepts_json() {
    let base = spawn_sensei_web_server().await;
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base}/webhook/webhook_learn"))
        .header("content-type", "application/json")
        .body(r#"{"question": "how do flows work?", "resolution": "flows use stages with needs"}"#)
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.text().await.unwrap(), "ok");
}

#[tokio::test]
async fn sensei_json_status_endpoint_returns_api_payload() {
    let base = spawn_sensei_web_server().await;
    let resp = reqwest::get(format!("{base}/api/status"))
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 200);
    let ct = resp
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(ct, "application/json");
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["status"].as_str().unwrap().contains("forge-sensei"));
}

#[tokio::test]
async fn sensei_json_update_mastery_preserves_number_param() {
    let base = spawn_sensei_web_server().await;
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base}/api/update-mastery"))
        .json(&serde_json::json!({"score": 72}))
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["result"].as_str().unwrap().contains("journeyman"));
}

#[tokio::test]
async fn sensei_raw_body_update_mastery_preserves_number_param() {
    let base = spawn_sensei_web_server().await;
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base}/api/update-mastery"))
        .body("75")
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["result"].as_str().unwrap().contains("journeyman"));
}

#[tokio::test]
async fn sensei_nonexistent_endpoint_returns_404() {
    let base = spawn_sensei_web_server().await;
    let resp = reqwest::get(format!("{base}/nonexistent"))
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn sensei_all_endpoints_registered() {
    let source_files = sensei_server_source_files();
    let composed =
        forge::compose::merge_programs(&source_files).expect("merge sensei files failed");
    let executor = TaskExecutor::new(composed.program, mock_registry(), None);
    let endpoints = executor.endpoints();

    // HTML, webhook, and JSON API endpoints should be registered.
    assert!(endpoints.contains_key("status"), "missing status endpoint");
    assert!(
        endpoints.contains_key("ask_form"),
        "missing ask_form endpoint"
    );
    assert!(endpoints.contains_key("ask"), "missing ask endpoint");
    assert!(
        endpoints.contains_key("review_form"),
        "missing review_form endpoint"
    );
    assert!(endpoints.contains_key("review"), "missing review endpoint");
    assert!(
        endpoints.contains_key("webhook_ingest"),
        "missing webhook_ingest endpoint"
    );
    assert!(
        endpoints.contains_key("webhook_learn"),
        "missing webhook_learn endpoint"
    );
    assert!(
        endpoints.contains_key("api_status"),
        "missing api_status endpoint"
    );
    assert!(
        endpoints.contains_key("api_ask"),
        "missing api_ask endpoint"
    );
    assert!(
        endpoints.contains_key("api_review"),
        "missing api_review endpoint"
    );
    assert!(
        endpoints.contains_key("api_ingest"),
        "missing api_ingest endpoint"
    );
    assert!(
        endpoints.contains_key("api_ingest_fact"),
        "missing api_ingest_fact endpoint"
    );
    assert!(
        endpoints.contains_key("api_learn_from_session"),
        "missing api_learn_from_session endpoint"
    );
    assert!(
        endpoints.contains_key("api_assess_detailed"),
        "missing api_assess_detailed endpoint"
    );
    assert!(
        endpoints.contains_key("api_deep_dive"),
        "missing api_deep_dive endpoint"
    );
    assert!(
        endpoints.contains_key("api_update_mastery"),
        "missing api_update_mastery endpoint"
    );
    assert!(
        endpoints.contains_key("api_batch_assess"),
        "missing api_batch_assess endpoint"
    );
    assert!(
        endpoints.contains_key("api_self_assess"),
        "missing api_self_assess endpoint (#247)"
    );
    assert_eq!(endpoints.len(), 18);
}

// ── Approval webhook tests (issue #182) ──────────────────────────

/// Spawn a server whose event bus is returned so tests can subscribe.
async fn spawn_approval_server() -> (String, forge::runtime::event_bus::SharedEventBus) {
    let source = "#! boundary: server\n\nendpoint noop() -> Text\n  give \"ok\"\n";
    let program = forge::parser::parse(source).expect("parse failed");
    let executor = TaskExecutor::new(program, mock_registry(), None);
    let event_bus = forge::runtime::event_bus::EventBus::new_shared(None);
    let server = ForgeServer::new(executor, None).with_event_bus(event_bus.clone());

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
    (format!("http://127.0.0.1:{port}"), event_bus)
}

/// Spawn a server *without* an event bus.
async fn spawn_approval_server_no_bus() -> String {
    let source = "#! boundary: server\n\nendpoint noop() -> Text\n  give \"ok\"\n";
    let program = forge::parser::parse(source).expect("parse failed");
    let executor = TaskExecutor::new(program, mock_registry(), None);
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

#[tokio::test]
async fn approval_webhook_json_publishes_event() {
    let (base, bus) = spawn_approval_server().await;

    // Subscribe to ApprovalResponse before sending the webhook.
    let mut rx = {
        let mut guard = bus.write().await;
        guard.subscribe("ApprovalResponse", "test-agent", None)
    };

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base}/webhook/approval"))
        .header("content-type", "application/json")
        .body(r#"{"request_id":"req-001","approved":true,"comment":"lgtm"}"#)
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 200);

    let event = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
        .await
        .expect("timed out waiting for event")
        .expect("channel closed");

    assert_eq!(event.event_name, "ApprovalResponse");
    assert_eq!(event.source_agent, "webhook");
    let req_id = event.fields.get("request_id").expect("missing request_id");
    assert!(matches!(&req_id.value, forge::runtime::confidence::Value::Text(s) if s == "req-001"));
    let approved = event.fields.get("approved").expect("missing approved");
    assert!(matches!(&approved.value, forge::runtime::confidence::Value::Bool(true)));
    let comment = event.fields.get("comment").expect("missing comment");
    assert!(matches!(&comment.value, forge::runtime::confidence::Value::Text(s) if s == "lgtm"));
}

#[tokio::test]
async fn approval_webhook_form_encoded_publishes_event() {
    let (base, bus) = spawn_approval_server().await;

    let mut rx = {
        let mut guard = bus.write().await;
        guard.subscribe("ApprovalResponse", "test-agent", None)
    };

    // Simulate Slack interactive payload: form-encoded with a JSON `payload` field.
    let slack_json = r#"{"actions":[{"action_id":"approve","value":"approved:req-002"}],"user":{"id":"U123","name":"alice"}}"#;
    let form_body = format!("payload={}", urlencoding::encode(slack_json));

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base}/webhook/approval"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body(form_body)
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 200);

    let event = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
        .await
        .expect("timed out waiting for event")
        .expect("channel closed");

    assert_eq!(event.event_name, "ApprovalResponse");
    let req_id = event.fields.get("request_id").expect("missing request_id");
    assert!(matches!(&req_id.value, forge::runtime::confidence::Value::Text(s) if s == "req-002"));
    let approved = event.fields.get("approved").expect("missing approved");
    assert!(matches!(&approved.value, forge::runtime::confidence::Value::Bool(true)));
    let comment = event.fields.get("comment").expect("missing comment");
    // Comment is built from Slack user info.
    assert!(matches!(&comment.value, forge::runtime::confidence::Value::Text(s) if s == "alice (U123)"));
}

#[tokio::test]
async fn approval_webhook_no_event_bus_returns_503() {
    let base = spawn_approval_server_no_bus().await;
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base}/webhook/approval"))
        .header("content-type", "application/json")
        .body(r#"{"request_id":"req-003","approved":false,"comment":"nope"}"#)
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 503);
}

#[tokio::test]
async fn approval_webhook_invalid_payload_returns_400() {
    let (base, _bus) = spawn_approval_server().await;
    let client = reqwest::Client::new();

    // Invalid JSON body.
    let resp = client
        .post(format!("{base}/webhook/approval"))
        .header("content-type", "application/json")
        .body("not json")
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 400);

    // Unsupported content type.
    let resp = client
        .post(format!("{base}/webhook/approval"))
        .header("content-type", "text/plain")
        .body("hello")
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 400);
}
