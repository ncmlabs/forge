// FORGE HTTP server runtime
// Dispatches HTTP requests to endpoint declarations. See issue #43.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Instant;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::ServeDir;

use crate::config::ServerConfig;
use crate::runtime::confidence::{ConfidentValue, Value};
use crate::runtime::executor::{EndpointResult, TaskExecutor};

// ── Server ───────────────────────────────────────────────────────────────────

pub struct ForgeServer {
    executor: TaskExecutor,
    host: String,
    port: u16,
    cors_origins: Vec<String>,
    static_root: Option<String>,
    static_prefix: Option<String>,
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
            executor,
            host,
            port,
            cors_origins,
            static_root,
            static_prefix,
        }
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

    /// Build the axum router from registered endpoints.
    fn build_router(&self) -> Router {
        let mut router = Router::new();

        for name in self.executor.endpoints().keys() {
            let get_path = format!("/{name}");
            let post_path = get_path.clone();
            let name_get = name.clone();
            let name_post = name.clone();

            router = router
                .route(
                    &get_path,
                    get(
                        move |state: State<TaskExecutor>,
                              query: Query<HashMap<String, String>>,
                              headers: HeaderMap| {
                            handle_get(state, Path(name_get), query, headers)
                        },
                    ),
                )
                .route(
                    &post_path,
                    post(
                        move |state: State<TaskExecutor>,
                              headers: HeaderMap,
                              body: axum::Json<serde_json::Value>| {
                            handle_post(state, Path(name_post), headers, body)
                        },
                    ),
                );
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

        router.layer(cors).with_state(self.executor.clone())
    }

    /// Print startup banner listing registered endpoints.
    fn print_banner(&self) {
        println!("Listening on http://{}:{}", self.host, self.port);
        if let (Some(ref root), Some(ref prefix)) = (&self.static_root, &self.static_prefix) {
            println!("  Static files: {} -> {}", root, prefix);
        }
        let endpoints = self.executor.endpoints();
        if endpoints.is_empty() {
            println!("  (no endpoints registered)");
        } else {
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

// ── Handlers ─────────────────────────────────────────────────────────────────

async fn handle_get(
    State(executor): State<TaskExecutor>,
    Path(endpoint_name): Path<String>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let request = build_request_record("GET", &endpoint_name, &params, &headers, "");
    let args = params_to_args(params);
    dispatch_endpoint(executor, &endpoint_name, args, request).await
}

async fn handle_post(
    State(executor): State<TaskExecutor>,
    Path(endpoint_name): Path<String>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> Response {
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
    dispatch_endpoint(executor, &endpoint_name, args, request).await
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
            let response = endpoint_result_to_response(result);
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

fn endpoint_result_to_response(result: EndpointResult) -> Response {
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
        Value::Html(s) => ("text/html", s.clone()),
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
