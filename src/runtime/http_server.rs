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
use crate::runtime::cost_aggregator::SharedCostAggregator;
use crate::runtime::event_bus::SharedEventBus;
use crate::runtime::executor::{EndpointResult, TaskExecutor};
use crate::runtime::instance_registry::SharedInstanceRegistry;
use crate::runtime::storage::SharedStorage;
use crate::runtime::system::{SharedSignalSenders, TopologySnapshot};
use crate::runtime::warded::SharedWardenSnapshots;

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
    /// Broadcast channel for live trace events (SSE).
    pub events_tx: Option<broadcast::Sender<String>>,
    /// Shared event bus for webhook → agent event delivery.
    pub event_bus: Option<SharedEventBus>,
    /// HMAC secrets keyed by webhook endpoint name.
    pub webhook_secrets: HashMap<String, String>,
    /// Shared instance registry for agent introspection (issue #139).
    pub instance_registry: Option<SharedInstanceRegistry>,
    /// Shared storage handle for storage introspection (issue #139).
    pub inspect_storage: Option<SharedStorage>,
    /// Shared warden snapshots for warden introspection (issue #139).
    pub warden_snapshots: Option<SharedWardenSnapshots>,
    /// Static topology snapshot for topology introspection (issue #139).
    pub topology: Option<TopologySnapshot>,
    /// Shared cost aggregator for token economy visibility (issue #142).
    pub cost_aggregator: Option<SharedCostAggregator>,
    /// Signal senders for failure injection (issue #143).
    pub signal_senders: Option<SharedSignalSenders>,
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
    webhook_secrets: HashMap<String, String>,
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
                events_tx: None,
                event_bus: None,
                webhook_secrets: HashMap::new(),
                instance_registry: None,
                inspect_storage: None,
                warden_snapshots: None,
                topology: None,
                cost_aggregator: None,
                signal_senders: None,
            },
            host,
            port,
            cors_origins,
            static_root,
            static_prefix,
            watch_mode: false,
            webhook_secrets: HashMap::new(),
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

    /// Attach an event bus for webhook → agent event delivery.
    pub fn with_event_bus(mut self, bus: SharedEventBus) -> Self {
        // Wire the event bus into the executor so `emit` works in endpoint handlers.
        // We clone the executor, attach the bus, and replace it.
        {
            let mut guard = self.state.executor.write().unwrap();
            let executor = guard.clone().with_event_bus(bus.clone());
            *guard = executor;
        }
        self.state.event_bus = Some(bus);
        self
    }

    /// Set HMAC secrets for webhook signature verification.
    /// Keys are endpoint names, values are secret strings.
    pub fn with_webhook_secrets(mut self, secrets: HashMap<String, String>) -> Self {
        self.state.webhook_secrets = secrets.clone();
        self.webhook_secrets = secrets;
        self
    }

    /// Get a handle to the swappable executor for the file watcher.
    pub fn swappable_executor(&self) -> SwappableExecutor {
        self.state.executor.clone()
    }

    /// Attach a live trace event broadcast channel for the SSE endpoint.
    pub fn with_events_tx(mut self, tx: broadcast::Sender<String>) -> Self {
        self.state.events_tx = Some(tx);
        self
    }

    /// Attach an instance registry for agent introspection.
    pub fn with_instance_registry(mut self, registry: SharedInstanceRegistry) -> Self {
        self.state.instance_registry = Some(registry);
        self
    }

    /// Attach storage handle for storage introspection.
    pub fn with_inspect_storage(mut self, storage: SharedStorage) -> Self {
        self.state.inspect_storage = Some(storage);
        self
    }

    /// Attach warden snapshots for warden introspection.
    pub fn with_warden_snapshots(mut self, snaps: SharedWardenSnapshots) -> Self {
        self.state.warden_snapshots = Some(snaps);
        self
    }

    /// Attach a static topology snapshot for topology introspection.
    pub fn with_topology(mut self, topo: TopologySnapshot) -> Self {
        self.state.topology = Some(topo);
        self
    }

    /// Attach a cost aggregator for token economy visibility (issue #142).
    pub fn with_cost_aggregator(mut self, agg: SharedCostAggregator) -> Self {
        self.state.cost_aggregator = Some(agg);
        self
    }

    /// Attach signal senders for failure injection (issue #143).
    pub fn with_signal_senders(mut self, senders: SharedSignalSenders) -> Self {
        self.state.signal_senders = Some(senders);
        self
    }

    /// Get a handle to the reload broadcast sender (for the watcher to signal reloads).
    pub fn reload_sender(&self) -> Option<broadcast::Sender<()>> {
        self.state.reload_tx.clone()
    }

    /// Build the axum router with dynamic endpoint dispatch.
    fn build_router(&self) -> Router<()> {
        // Dynamic catch-all routing: endpoint lookup happens at request time,
        // so new/removed endpoints are visible immediately after hot-reload.
        // Webhook route: POST /webhook/:name with Content-Type validation and optional HMAC
        let mut router = Router::new()
            .route("/webhook/{endpoint}", axum::routing::post(handle_webhook))
            .route("/{endpoint}", get(handle_get).post(handle_post))
            .route(
                "/",
                get(|| async { axum::response::Redirect::temporary("/home") }),
            );

        // SSE reload endpoint (watch mode only)
        if self.watch_mode {
            router = router.route("/__forge/reload", get(handle_sse_reload));
        }

        // SSE live trace endpoint (always in serve mode when events channel is wired)
        if self.state.events_tx.is_some() {
            router = router.route("/__forge/events", get(handle_sse_events));
        }

        // Introspection endpoints (issue #139) — always registered, return empty/404 gracefully
        router = router
            .route("/__forge/inspect/agents", get(handle_inspect_agents))
            .route("/__forge/inspect/agents/{id}", get(handle_inspect_agent))
            .route("/__forge/inspect/topology", get(handle_inspect_topology))
            .route("/__forge/inspect/wardens", get(handle_inspect_wardens))
            .route("/__forge/inspect/storage", get(handle_inspect_storage))
            .route("/__forge/inspect/costs", get(handle_inspect_costs));

        // Failure injection endpoint (issue #143)
        router = router.route(
            "/__forge/inject/{failure_type}",
            axum::routing::post(handle_inject),
        );

        // CORS
        let cors = if self.cors_origins.is_empty() {
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any)
        } else {
            let origins: Vec<_> = self
                .cors_origins
                .iter()
                .filter_map(|o| o.parse().ok())
                .collect();
            CorsLayer::new()
                .allow_origin(origins)
                .allow_methods(Any)
                .allow_headers(Any)
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
        if self.state.events_tx.is_some() {
            println!("  SSE trace:    /__forge/events");
        }
        if self.state.signal_senders.is_some() {
            println!("  Inject:       /__forge/inject/:type");
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
        println!("  POST /webhook/{name}{ret}");
    }
}

/// Dev-mode reload script injected before </body> in HTML responses.
const RELOAD_SCRIPT: &str =
    r#"<script>new EventSource("/__forge/reload").onmessage=()=>location.reload()</script>"#;

// ── Handlers ─────────────────────────────────────────────────────────────────

async fn handle_get(
    State(state): State<AppState>,
    Path(endpoint_name): Path<String>,
    Query(mut params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let executor = state.executor.read().unwrap().clone();
    if !executor.endpoints().contains_key(&endpoint_name) {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    }
    // Fill in missing params with empty strings so endpoints don't crash on undefined vars
    if let Some(ep) = executor.endpoints().get(&endpoint_name) {
        for param in &ep.params {
            params
                .entry(param.node.name.clone())
                .or_insert_with(String::new);
        }
    }
    let request = build_request_record("GET", &endpoint_name, &params, &headers, "");
    let args = params_to_args(params);
    dispatch_endpoint(
        executor,
        &endpoint_name,
        args,
        request,
        state.is_watch_mode(),
    )
    .await
}

async fn handle_post(
    State(state): State<AppState>,
    Path(endpoint_name): Path<String>,
    headers: HeaderMap,
    body_bytes: axum::body::Bytes,
) -> Response {
    let executor = state.executor.read().unwrap().clone();
    if !executor.endpoints().contains_key(&endpoint_name) {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    }
    let raw_body = String::from_utf8_lossy(&body_bytes).to_string();
    let content_type = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let params: HashMap<String, String> = if content_type.contains("application/json") {
        // JSON body
        match serde_json::from_slice::<serde_json::Value>(&body_bytes) {
            Ok(json) => match json.as_object() {
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
            },
            Err(_) => HashMap::new(),
        }
    } else {
        // Form-encoded body (application/x-www-form-urlencoded)
        raw_body
            .split('&')
            .filter_map(|pair| {
                let mut parts = pair.splitn(2, '=');
                let key = parts.next()?;
                let value = parts.next().unwrap_or("");
                if key.is_empty() {
                    return None;
                }
                // Decode percent-encoding (+ as space)
                let decode = |s: &str| {
                    let replaced = s.replace('+', " ");
                    urlencoding::decode(&replaced)
                        .map(|c| c.into_owned())
                        .unwrap_or(replaced)
                };
                Some((decode(key), decode(value)))
            })
            .collect()
    };

    let request = build_request_record("POST", &endpoint_name, &params, &headers, &raw_body);
    let args = params_to_args(params);
    dispatch_endpoint(
        executor,
        &endpoint_name,
        args,
        request,
        state.is_watch_mode(),
    )
    .await
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

/// SSE endpoint for live trace event streaming.
async fn handle_sse_events(
    State(state): State<AppState>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    let rx = state
        .events_tx
        .as_ref()
        .expect("SSE events route registered without events channel")
        .subscribe();

    let stream = BroadcastStream::new(rx).filter_map(|result| match result {
        Ok(json) => Some(Ok(Event::default().data(json))),
        Err(_) => None, // lagged — skip
    });

    Sse::new(stream).keep_alive(KeepAlive::default())
}

// ── Introspection handlers (issue #139) ────────────────────────────────────

/// GET /__forge/inspect/agents — list all running agent instances.
async fn handle_inspect_agents(State(state): State<AppState>) -> Response {
    let Some(ref registry) = state.instance_registry else {
        return json_response(StatusCode::OK, serde_json::json!([]));
    };
    let guard = registry.read().await;
    let agents: Vec<serde_json::Value> = guard
        .find_all()
        .iter()
        .map(|i| i.to_json_summary())
        .collect();
    json_response(StatusCode::OK, serde_json::json!(agents))
}

/// GET /__forge/inspect/agents/:id — deep inspection of a single agent.
async fn handle_inspect_agent(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let Some(ref registry) = state.instance_registry else {
        return json_response(
            StatusCode::NOT_FOUND,
            serde_json::json!({"error": "no instance registry"}),
        );
    };
    let uuid = match uuid::Uuid::parse_str(&id) {
        Ok(u) => u,
        Err(_) => {
            return json_response(
                StatusCode::BAD_REQUEST,
                serde_json::json!({"error": "invalid UUID"}),
            );
        }
    };
    let guard = registry.read().await;
    match guard.get(&uuid) {
        Some(info) => json_response(StatusCode::OK, info.to_json_deep()),
        None => json_response(
            StatusCode::NOT_FOUND,
            serde_json::json!({"error": "agent not found"}),
        ),
    }
}

/// GET /__forge/inspect/topology — system graph with agents, wiring, subscriptions.
async fn handle_inspect_topology(State(state): State<AppState>) -> Response {
    let mut obj = serde_json::json!({});

    // Static topology from system declaration
    if let Some(ref topo) = state.topology {
        obj["system_name"] = serde_json::json!(topo.system_name);
        obj["bindings"] = serde_json::json!(topo.bindings);
        obj["wiring"] = serde_json::json!(topo.wiring);
    }

    // Dynamic subscription info from event bus
    if let Some(ref bus) = state.event_bus {
        let guard = bus.read().await;
        let subs: Vec<serde_json::Value> = guard
            .subscription_info()
            .iter()
            .map(|s| {
                serde_json::json!({
                    "event": s.event_name,
                    "agent": s.agent_id,
                    "has_filter": s.has_filter,
                })
            })
            .collect();
        let routes = guard.route_info();
        obj["subscribers"] = serde_json::json!(subs);
        obj["routes"] = serde_json::json!(routes);
    }

    json_response(StatusCode::OK, obj)
}

/// GET /__forge/inspect/wardens — all wardens with health data.
async fn handle_inspect_wardens(State(state): State<AppState>) -> Response {
    let Some(ref snaps) = state.warden_snapshots else {
        return json_response(StatusCode::OK, serde_json::json!([]));
    };
    let guard = snaps.read().await;
    json_response(StatusCode::OK, serde_json::json!(*guard))
}

/// GET /__forge/inspect/storage — list storage keys with sizes.
async fn handle_inspect_storage(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let Some(ref storage) = state.inspect_storage else {
        return json_response(StatusCode::OK, serde_json::json!([]));
    };
    let prefix = params.get("prefix").map(|s| s.as_str()).unwrap_or("");
    match storage.list_with_sizes(prefix) {
        Ok(entries) => {
            let items: Vec<serde_json::Value> = entries
                .iter()
                .map(|(key, size)| serde_json::json!({"key": key, "size_bytes": size}))
                .collect();
            json_response(StatusCode::OK, serde_json::json!(items))
        }
        Err(e) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            serde_json::json!({"error": format!("{}", e)}),
        ),
    }
}

/// GET /__forge/inspect/costs — aggregated token/cost/confidence metrics (issue #142).
async fn handle_inspect_costs(State(state): State<AppState>) -> Response {
    let Some(ref agg) = state.cost_aggregator else {
        return json_response(
            StatusCode::OK,
            serde_json::json!({"totals": {"calls": 0, "tokens_in": 0, "tokens_out": 0, "cost_usd": 0.0}, "by_operation": {}, "by_provider_model": {}, "by_agent": {}, "confidence_histogram": [0,0,0,0,0,0,0,0,0,0], "uptime_secs": 0.0, "tokens_per_sec": 0.0}),
        );
    };
    let snapshot = agg.read().await.snapshot();
    json_response(StatusCode::OK, snapshot)
}

/// POST /__forge/inject/:failure_type — inject a failure signal into the warden (issue #143).
async fn handle_inject(
    State(state): State<AppState>,
    Path(failure_type): Path<String>,
    body: axum::body::Bytes,
) -> Response {
    // Parse failure type from path
    let signal_kind = match failure_type.as_str() {
        "stuck" | "crash" | "timeout" | "hallucination" | "budget" => failure_type.as_str(),
        other => {
            return json_response(
                StatusCode::BAD_REQUEST,
                serde_json::json!({"error": format!("unknown failure type: '{}'", other), "valid": ["stuck", "crash", "timeout", "hallucination", "budget"]}),
            );
        }
    };

    // Parse JSON body for agent name
    let body_json: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => {
            return json_response(
                StatusCode::BAD_REQUEST,
                serde_json::json!({"error": "invalid JSON body, expected {\"agent\": \"<name>\"}"}),
            );
        }
    };
    let agent_name = match body_json.get("agent").and_then(|v| v.as_str()) {
        Some(name) => name.to_string(),
        None => {
            return json_response(
                StatusCode::BAD_REQUEST,
                serde_json::json!({"error": "missing 'agent' field in request body"}),
            );
        }
    };

    // Get signal senders
    let Some(ref senders) = state.signal_senders else {
        return json_response(
            StatusCode::SERVICE_UNAVAILABLE,
            serde_json::json!({"error": "no system runtime active — failure injection requires a running system with wardens"}),
        );
    };

    // Find sender for the requested agent
    let guard = senders.read().await;
    let Some(tx) = guard.get(&agent_name) else {
        let available: Vec<&String> = guard.keys().collect();
        return json_response(
            StatusCode::NOT_FOUND,
            serde_json::json!({"error": format!("agent '{}' not found in any warden", agent_name), "available_agents": available}),
        );
    };

    // Build the signal
    use crate::runtime::agent::AgentSignal;
    let signal = match signal_kind {
        "stuck" => AgentSignal::Stuck {
            agent_name: agent_name.clone(),
        },
        "crash" => AgentSignal::Crash {
            agent_name: agent_name.clone(),
        },
        "timeout" => AgentSignal::Timeout {
            agent_name: agent_name.clone(),
        },
        "hallucination" => AgentSignal::Hallucination {
            agent_name: agent_name.clone(),
            detail: "injected via /__forge/inject".to_string(),
        },
        "budget" => AgentSignal::BudgetExceeded {
            agent_name: agent_name.clone(),
            detail: "injected via /__forge/inject".to_string(),
        },
        _ => unreachable!(),
    };

    // Send signal (non-blocking)
    match tx.try_send(signal) {
        Ok(()) => json_response(
            StatusCode::OK,
            serde_json::json!({"injected": signal_kind, "agent": agent_name}),
        ),
        Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => json_response(
            StatusCode::SERVICE_UNAVAILABLE,
            serde_json::json!({"error": "signal channel full — warden may be overloaded"}),
        ),
        Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => json_response(
            StatusCode::SERVICE_UNAVAILABLE,
            serde_json::json!({"error": "warden runtime has shut down"}),
        ),
    }
}

/// Helper: build a JSON response with the given status code.
fn json_response(status: StatusCode, body: serde_json::Value) -> Response {
    (
        status,
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        body.to_string(),
    )
        .into_response()
}

/// Webhook handler: POST /webhook/:name
/// Validates Content-Type is application/json, optionally verifies HMAC signature,
/// then dispatches to the endpoint handler with full request context.
async fn handle_webhook(
    State(state): State<AppState>,
    Path(endpoint_name): Path<String>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    // Content-Type must be application/json
    let content_type = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !content_type.starts_with("application/json") {
        return (
            StatusCode::BAD_REQUEST,
            "Content-Type must be application/json",
        )
            .into_response();
    }

    // Optional HMAC signature verification
    if let Some(secret) = state.webhook_secrets.get(&endpoint_name) {
        let signature = headers
            .get("x-hub-signature-256")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if !verify_hmac_signature(secret, &body, signature) {
            return (StatusCode::UNAUTHORIZED, "invalid signature").into_response();
        }
    }

    // Parse JSON body
    let body_str = match String::from_utf8(body.to_vec()) {
        Ok(s) => s,
        Err(_) => {
            return (StatusCode::BAD_REQUEST, "invalid UTF-8 body").into_response();
        }
    };
    let json_value: serde_json::Value = match serde_json::from_str(&body_str) {
        Ok(v) => v,
        Err(_) => {
            return (StatusCode::BAD_REQUEST, "invalid JSON body").into_response();
        }
    };

    // Clone executor with event bus attached
    let executor = {
        let ex = state.executor.read().unwrap().clone();
        if let Some(ref bus) = state.event_bus {
            if ex.event_bus().is_none() {
                ex.with_event_bus(bus.clone())
            } else {
                ex
            }
        } else {
            ex
        }
    };

    if !executor.endpoints().contains_key(&endpoint_name) {
        return (StatusCode::NOT_FOUND, "webhook endpoint not found").into_response();
    }

    let params: HashMap<String, String> = match json_value.as_object() {
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

    let request = build_request_record("POST", &endpoint_name, &params, &headers, &body_str);
    let args = params_to_args(params);
    dispatch_endpoint(
        executor,
        &endpoint_name,
        args,
        request,
        state.is_watch_mode(),
    )
    .await
}

/// Verify GitHub-style HMAC-SHA256 signature.
/// Signature format: "sha256=<hex_digest>"
fn verify_hmac_signature(secret: &str, body: &[u8], signature: &str) -> bool {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    let expected_hex = match signature.strip_prefix("sha256=") {
        Some(hex) => hex,
        None => return false,
    };

    let mut mac = match Hmac::<Sha256>::new_from_slice(secret.as_bytes()) {
        Ok(m) => m,
        Err(_) => return false,
    };
    mac.update(body);
    let result = mac.finalize().into_bytes();
    let computed_hex = hex::encode(result);

    // Constant-time comparison to prevent timing attacks
    use subtle::ConstantTimeEq;
    computed_hex
        .as_bytes()
        .ct_eq(expected_hex.as_bytes())
        .into()
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
