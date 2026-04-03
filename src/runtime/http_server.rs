// FORGE HTTP server runtime
// Dispatches HTTP requests to endpoint declarations. See issue #43.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Instant;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use tower_http::cors::{Any, CorsLayer};

use crate::config::ServerConfig;
use crate::runtime::confidence::{ConfidentValue, Value};
use crate::runtime::executor::TaskExecutor;

// ── Server ───────────────────────────────────────────────────────────────────

pub struct ForgeServer {
    executor: TaskExecutor,
    host: String,
    port: u16,
    cors_origins: Vec<String>,
}

impl ForgeServer {
    pub fn new(executor: TaskExecutor, config: Option<&ServerConfig>) -> Self {
        let (host, port, cors_origins) = match config {
            Some(c) => (
                c.host_or_default().to_string(),
                c.port_or_default(),
                c.cors_origins.clone().unwrap_or_default(),
            ),
            None => ("127.0.0.1".to_string(), 3000, Vec::new()),
        };

        Self {
            executor,
            host,
            port,
            cors_origins,
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
                        move |state: State<TaskExecutor>, query: Query<HashMap<String, String>>| {
                            handle_get(state, Path(name_get), query)
                        },
                    ),
                )
                .route(
                    &post_path,
                    post(
                        move |state: State<TaskExecutor>, body: axum::Json<serde_json::Value>| {
                            handle_post(state, Path(name_post), body)
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

        router
            .fallback(fallback_handler)
            .layer(cors)
            .with_state(self.executor.clone())
    }

    /// Print startup banner listing registered endpoints.
    fn print_banner(&self) {
        println!("Listening on http://{}:{}", self.host, self.port);
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
) -> Response {
    let args = params_to_args(params);
    dispatch_endpoint(executor, &endpoint_name, args).await
}

async fn handle_post(
    State(executor): State<TaskExecutor>,
    Path(endpoint_name): Path<String>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> Response {
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
    let args = params_to_args(params);
    dispatch_endpoint(executor, &endpoint_name, args).await
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
) -> Response {
    let start = Instant::now();

    // Trace the request
    if let Some(tracer) = executor.tracer() {
        tracer.http_request(endpoint_name, "HTTP", &format!("/{endpoint_name}"));
    }

    match executor.exec_endpoint(endpoint_name, args).await {
        Ok(val) => {
            let duration_ms = start.elapsed().as_millis() as u64;
            let response = value_to_response(val);
            let status = if response.status().is_success() {
                response.status().as_u16()
            } else {
                response.status().as_u16()
            };
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

fn value_to_response(val: ConfidentValue) -> Response {
    match &val.value {
        Value::Text(s) => (StatusCode::OK, s.clone()).into_response(),
        Value::Number(n) => (StatusCode::OK, n.to_string()).into_response(),
        Value::Bool(b) => (StatusCode::OK, b.to_string()).into_response(),
        Value::Unit => StatusCode::NO_CONTENT.into_response(),
        Value::List(_) | Value::Record(_) | Value::Array(_) => {
            let json = value_to_json(&val);
            axum::Json(json).into_response()
        }
    }
}

fn value_to_json(val: &ConfidentValue) -> serde_json::Value {
    match &val.value {
        Value::Text(s) => serde_json::Value::String(s.clone()),
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
