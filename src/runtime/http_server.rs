// FORGE HTTP server runtime
// Dispatches HTTP requests to endpoint declarations. See issue #43.
// Hot-reload support via SwappableExecutor. See issue #47.

use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::{Arc, RwLock};
use std::time::Instant;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::ServeDir;

use crate::config::ServerConfig;
use crate::runtime::confidence::{ConfidentValue, Value};
use crate::runtime::executor::{EndpointResult, TaskExecutor};

// ── Swappable executor for hot-reload ───────────────────────────────────────

/// Shared executor state that can be hot-swapped by the file watcher.
/// Handlers clone the executor out under a brief read lock, then release it.
pub type SwappableExecutor = Arc<RwLock<TaskExecutor>>;

/// Combined server state shared with axum handlers.
#[derive(Clone)]
pub struct AppState {
    pub executor: SwappableExecutor,
    /// Broadcast channel for SSE reload notifications (only in watch mode).
    pub reload_tx: Option<broadcast::Sender<()>>,
}

impl AppState {
    fn is_watch_mode(&self) -> bool {
        self.reload_tx.is_some()
    }
}

// ── Server ───────────────────────────────────────────────────────────────────

pub struct ForgeServer {
    state: AppState,
    host: String,
    port: u16,
    cors_origins: Vec<String>,
    static_root: Option<String>,
    static_prefix: Option<String>,
    watch_mode: bool,
}

impl ForgeServer {
    pub fn new(executor: TaskExecutor, config: Option<&ServerConfig>) -> Self {
        let (host, port, cors_origins, static_root, static_prefix) = match config {
            Some(c) => (
                c.host_or_default().to_string(),
                c.port_or_default(),
                c.cors_origins.clone().unwrap_or_default(),
                c.static_files
                    .as_ref()
                    .map(|s| s.root_or_default().to_string()),
                c.static_files
                    .as_ref()
                    .map(|s| s.prefix_or_default().to_string()),
            ),
            None => ("127.0.0.1".to_string(), 3000, Vec::new(), None, None),
        };

        Self {
            state: AppState {
                executor: Arc::new(RwLock::new(executor)),
                reload_tx: None,
            },
            host,
            port,
            cors_origins,
            static_root,
            static_prefix,
            watch_mode: false,
        }
    }

    /// Enable watch mode: adds SSE reload endpoint and injects reload script in HTML responses.
    pub fn with_watch_mode(mut self, watch: bool) -> Self {
        self.watch_mode = watch;
        if watch {
            let (tx, _) = broadcast::channel(16);
            self.state.reload_tx = Some(tx);
        }
        self
    }

    /// Override host (CLI flag takes precedence over config).
    pub fn with_host(mut self, host: String) -> Self {
        self.host = host;
        self
    }

    /// Override port (CLI flag takes precedence over config).
    pub fn with_port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    /// Get a handle to the swappable executor for the file watcher.
    pub fn swappable_executor(&self) -> SwappableExecutor {
        self.state.executor.clone()
    }

    /// Get a handle to the reload broadcast sender (for the watcher to signal reloads).
    pub fn reload_sender(&self) -> Option<broadcast::Sender<()>> {
        self.state.reload_tx.clone()
    }

    /// Build the axum router with dynamic endpoint dispatch.
    fn build_router(&self) -> Router<()> {
        // Dynamic catch-all routing: endpoint lookup happens at request time,
        // so new/removed endpoints are visible immediately after hot-reload.
        let mut router = Router::new()
            .route("/{endpoint}", get(handle_get).post(handle_post));

        // SSE reload endpoint (watch mode only)
        if self.watch_mode {
            router = router.route("/__forge/reload", get(handle_sse_reload));
        }

        // CORS
        let cors = if self.cors_origins.is_empty() {
            CorsLayer::new().allow_origin(Any)
        } else {
            let origins: Vec<_> = self
                .cors_origins
                .iter()
                .filter_map(|o| o.parse().ok())
                .collect();
            CorsLayer::new().allow_origin(origins)
        };

        let router = if let Some(ref root) = self.static_root {
            let prefix = self.static_prefix.as_deref().unwrap_or("/static");
            router
                .nest_service(prefix, ServeDir::new(root))
                .fallback(fallback_handler)
        } else {
            router.fallback(fallback_handler)
        };

        router.layer(cors).with_state(self.state.clone())
    }

    /// Print startup banner listing registered endpoints.
    fn print_banner(&self) {
        let mode = if self.watch_mode { " (watch mode)" } else { "" };
        println!("Listening on http://{}:{}{}", self.host, self.port, mode);
        if let (Some(ref root), Some(ref prefix)) = (&self.static_root, &self.static_prefix) {
            println!("  Static files: {} -> {}", root, prefix);
        }
        if self.watch_mode {
            println!("  SSE reload:   /__forge/reload");
        }
        let executor = self.state.executor.read().unwrap();
        let endpoints = executor.endpoints();
        if endpoints.is_empty() {
            println!("  (no endpoints registered)");
        } else {
            print_endpoint_list(endpoints);
        }
    }

    /// Start the HTTP server with graceful shutdown on SIGINT.
    pub async fn run(self) -> anyhow::Result<()> {
        self.print_banner();

        let addr: SocketAddr = format!("{}:{}", self.host, self.port).parse()?;
        let router = self.build_router();

        let listener = tokio::net::TcpListener::bind(addr).await?;
        axum::serve(listener, router)
            .with_graceful_shutdown(shutdown_signal())
            .await?;

        println!("\nServer stopped.");
        Ok(())
    }

    /// Start on a provided listener (for testing with random ports).
    pub async fn run_on_listener(self, listener: tokio::net::TcpListener) -> anyhow::Result<()> {
        let router = self.build_router();
        axum::serve(listener, router)
            .with_graceful_shutdown(shutdown_signal())
            .await?;
        Ok(())
    }
}

/// Print endpoint list (reused by banner and reload logging).
pub fn print_endpoint_list(endpoints: &HashMap<String, crate::ast::EndpointDecl>) {
    for (name, ep) in endpoints {
        let params: Vec<String> = ep
            .params
            .iter()
            .map(|p| format!("{}=<{:?}>", p.node.name, p.node.type_name.node))
            .collect();
        let ret = ep
            .return_type
            .as_ref()
            .map(|t| {
                let types: Vec<String> = t
                    .node
                    .types
                    .iter()
                    .map(|tn| format!("{:?}", tn.node))
                    .collect();
                format!(" -> {}", types.join(", "))
            })
            .unwrap_or_default();
        println!("  GET  /{name}?{}{ret}", params.join("&"));
        println!("  POST /{name}{ret}");
    }
}

/// Dev-mode reload script injected before </body> in HTML responses.
const RELOAD_SCRIPT: &str =
    r#"<script>new EventSource("/__forge/reload").onmessage=()=>location.reload()</script>"#;

// ── Handlers ─────────────────────────────────────────────────────────────────

async fn handle_get(
    State(state): State<AppState>,
    Path(endpoint_name): Path<String>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let executor = state.executor.read().unwrap().clone();
    if !executor.endpoints().contains_key(&endpoint_name) {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    }
    let request = build_request_record("GET", &endpoint_name, &params, &headers, "");
    let args = params_to_args(params);
    dispatch_endpoint(executor, &endpoint_name, args, request, state.is_watch_mode()).await
}

async fn handle_post(
    State(state): State<AppState>,
    Path(endpoint_name): Path<String>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> Response {
    let executor = state.executor.read().unwrap().clone();
    if !executor.endpoints().contains_key(&endpoint_name) {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    }
    let raw_body = body.to_string();
    let params = match body.as_object() {
        Some(map) => map
            .iter()
            .map(|(k, v)| {
                let s = match v {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                (k.clone(), s)
            })
            .collect(),
        None => HashMap::new(),
    };
    let request = build_request_record("POST", &endpoint_name, &params, &headers, &raw_body);
    let args = params_to_args(params);
    dispatch_endpoint(executor, &endpoint_name, args, request, state.is_watch_mode()).await
}

/// SSE endpoint for browser auto-reload in watch mode.
async fn handle_sse_reload(
    State(state): State<AppState>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    let rx = state
        .reload_tx
        .as_ref()
        .expect("SSE reload route registered without watch mode")
        .subscribe();

    let stream = BroadcastStream::new(rx).filter_map(|result| match result {
        Ok(()) => Some(Ok(Event::default().data("reload"))),
        Err(_) => None, // lagged — skip
    });

    Sse::new(stream).keep_alive(KeepAlive::default())
}

fn params_to_args(params: HashMap<String, String>) -> HashMap<String, ConfidentValue> {
    params
        .into_iter()
        .map(|(k, v)| (k, ConfidentValue::deterministic(Value::Text(v))))
        .collect()
}

async fn dispatch_endpoint(
    executor: TaskExecutor,
    endpoint_name: &str,
    args: HashMap<String, ConfidentValue>,
    request: ConfidentValue,
    inject_reload: bool,
) -> Response {
    let start = Instant::now();

    // Trace the request
    if let Some(tracer) = executor.tracer() {
        tracer.http_request(endpoint_name, "HTTP", &format!("/{endpoint_name}"));
    }

    match executor
        .exec_endpoint(endpoint_name, args, Some(request))
        .await
    {
        Ok(result) => {
            let duration_ms = start.elapsed().as_millis() as u64;
            let response = endpoint_result_to_response(result, inject_reload);
            let status = response.status().as_u16();
            if let Some(tracer) = executor.tracer() {
                tracer.http_response(endpoint_name, status, duration_ms);
            }
            response
        }
        Err(e) => {
            let duration_ms = start.elapsed().as_millis() as u64;
            if let Some(tracer) = executor.tracer() {
                tracer.http_response(endpoint_name, 500, duration_ms);
            }
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("runtime error: {e}"),
            )
                .into_response()
        }
    }
}

fn endpoint_result_to_response(result: EndpointResult, inject_reload: bool) -> Response {
    let default_status = if matches!(result.value.value, Value::Unit) {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::OK
    };
    let status_code = result
        .status
        .and_then(|s| StatusCode::from_u16(s).ok())
        .unwrap_or(default_status);

    let (value_inferred_ct, body) = match &result.value.value {
        Value::Text(s) => ("text/plain", s.clone()),
        Value::Html(s) => {
            // In watch mode, inject the reload script before </body> or at the end
            let html = if inject_reload {
                inject_reload_script(s)
            } else {
                s.clone()
            };
            ("text/html", html)
        }
        Value::Number(n) => ("text/plain", n.to_string()),
        Value::Bool(b) => ("text/plain", b.to_string()),
        Value::Unit => return status_code.into_response(),
        Value::List(_) | Value::Record(_) | Value::Array(_) => {
            let json = value_to_json(&result.value);
            // Priority: explicit > return-type annotation > value-inferred
            let ct = result
                .content_type
                .as_deref()
                .or(result.default_content_type.as_deref())
                .unwrap_or("application/json");
            return (
                status_code,
                [(axum::http::header::CONTENT_TYPE, ct)],
                serde_json::to_string(&json).unwrap_or_default(),
            )
                .into_response();
        }
    };

    // Priority: explicit > return-type annotation > value-inferred
    let ct = result
        .content_type
        .as_deref()
        .or(result.default_content_type.as_deref())
        .unwrap_or(value_inferred_ct);
    (status_code, [(axum::http::header::CONTENT_TYPE, ct)], body).into_response()
}

/// Inject the reload script into an HTML body string.
fn inject_reload_script(html: &str) -> String {
    // Insert before </body> if present, otherwise append
    if let Some(pos) = html.to_lowercase().rfind("</body>") {
        let mut result = String::with_capacity(html.len() + RELOAD_SCRIPT.len());
        result.push_str(&html[..pos]);
        result.push_str(RELOAD_SCRIPT);
        result.push_str(&html[pos..]);
        result
    } else {
        let mut result = String::with_capacity(html.len() + RELOAD_SCRIPT.len());
        result.push_str(html);
        result.push_str(RELOAD_SCRIPT);
        result
    }
}

fn build_request_record(
    method: &str,
    endpoint_name: &str,
    query_params: &HashMap<String, String>,
    headers: &HeaderMap,
    body: &str,
) -> ConfidentValue {
    let det = |v: Value| ConfidentValue::deterministic(v);

    let query_record: HashMap<String, ConfidentValue> = query_params
        .iter()
        .map(|(k, v)| (k.clone(), det(Value::Text(v.clone()))))
        .collect();

    let header_record: HashMap<String, ConfidentValue> = headers
        .iter()
        .map(|(k, v)| {
            let key = k.as_str().replace('-', "_");
            let val = v.to_str().unwrap_or("").to_string();
            (key, det(Value::Text(val)))
        })
        .collect();

    let fields: HashMap<String, ConfidentValue> = [
        ("method".to_string(), det(Value::Text(method.to_string()))),
        (
            "path".to_string(),
            det(Value::Text(format!("/{endpoint_name}"))),
        ),
        ("query".to_string(), det(Value::Record(query_record))),
        ("headers".to_string(), det(Value::Record(header_record))),
        ("body".to_string(), det(Value::Text(body.to_string()))),
    ]
    .into_iter()
    .collect();

    ConfidentValue::deterministic(Value::Record(fields))
}

fn value_to_json(val: &ConfidentValue) -> serde_json::Value {
    match &val.value {
        Value::Text(s) | Value::Html(s) => serde_json::Value::String(s.clone()),
        Value::Number(n) => serde_json::json!(n),
        Value::Bool(b) => serde_json::Value::Bool(*b),
        Value::Unit => serde_json::Value::Null,
        Value::List(items) | Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(value_to_json).collect())
        }
        Value::Record(fields) => {
            let map: serde_json::Map<String, serde_json::Value> = fields
                .iter()
                .map(|(k, v)| (k.clone(), value_to_json(v)))
                .collect();
            serde_json::Value::Object(map)
        }
    }
}

async fn fallback_handler() -> Response {
    (StatusCode::NOT_FOUND, "not found").into_response()
}

async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("failed to listen for SIGINT");
}
