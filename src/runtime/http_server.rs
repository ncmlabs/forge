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
use crate::runtime::agent_lifecycle::AgentLifecycle;
use crate::runtime::confidence::{ConfidentValue, Value};
use crate::runtime::cost_aggregator::SharedCostAggregator;
use crate::runtime::event_bus::EventPayload;
use crate::runtime::event_bus::SharedEventBus;
use crate::runtime::executor::{EndpointResult, TaskExecutor};
use crate::runtime::instance_registry::SharedInstanceRegistry;
use crate::runtime::knowledge_store::SharedKnowledgeStore;
use crate::runtime::storage::SharedStorage;
use crate::runtime::system::{SharedSignalSenders, TopologySnapshot};
use crate::runtime::task_history_aggregator::SharedTaskHistoryAggregator;
use crate::runtime::warded::SharedWardenSnapshots;
use crate::runtime::webhook_driver::WebhookDriver;
use crate::runtime::webhook_rate_limiter::WebhookRateLimiter;

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
    /// Shared task history aggregator for the mastery tile (issue #304).
    pub task_history_aggregator: Option<SharedTaskHistoryAggregator>,
    /// Shared knowledge store handle for mastery-level readback (issue #304).
    pub mastery_knowledge_store: Option<SharedKnowledgeStore>,
    /// Signal senders for failure injection (issue #143).
    pub signal_senders: Option<SharedSignalSenders>,
    /// Static registry of declared `webhook TRIGGER` blocks — event sources
    /// for `POST /wake/{agent}/{trigger}` (issue #335).
    pub webhook_driver: Option<Arc<WebhookDriver>>,
    /// Storage handle for the wake-secrets table. Read-only from the HTTP
    /// handler; writes go through the `forge wake` CLI.
    pub wake_storage: Option<SharedStorage>,
    /// Shared agent lifecycle so webhook wakes rehydrate before bus publish.
    pub agent_lifecycle: Option<Arc<AgentLifecycle>>,
    /// Per-`(agent, trigger)` token-bucket limiter for webhook ingress.
    pub webhook_rate_limiter: Arc<WebhookRateLimiter>,
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
                task_history_aggregator: None,
                mastery_knowledge_store: None,
                signal_senders: None,
                webhook_driver: None,
                wake_storage: None,
                agent_lifecycle: None,
                webhook_rate_limiter: Arc::new(WebhookRateLimiter::default_for_webhooks()),
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

    /// Attach the task-history aggregator for the mastery tile (issue #304).
    pub fn with_task_history_aggregator(mut self, agg: SharedTaskHistoryAggregator) -> Self {
        self.state.task_history_aggregator = Some(agg);
        self
    }

    /// Attach a knowledge store handle so the mastery endpoint can read
    /// `mastery-{specialist}-{project}` entries (issue #304).
    pub fn with_mastery_knowledge_store(mut self, ks: SharedKnowledgeStore) -> Self {
        self.state.mastery_knowledge_store = Some(ks);
        self
    }

    /// Attach signal senders for failure injection (issue #143).
    pub fn with_signal_senders(mut self, senders: SharedSignalSenders) -> Self {
        self.state.signal_senders = Some(senders);
        self
    }

    /// Attach the webhook driver registry for `POST /wake/...` routing (issue #335).
    pub fn with_webhook_driver(mut self, driver: Arc<WebhookDriver>) -> Self {
        self.state.webhook_driver = Some(driver);
        self
    }

    /// Attach the storage handle used to look up per-`(agent, trigger)` HMAC
    /// secrets (issue #335). Separate from `inspect_storage` because this path
    /// is hot and must not block on reads serving introspection snapshots.
    pub fn with_wake_storage(mut self, storage: SharedStorage) -> Self {
        self.state.wake_storage = Some(storage);
        self
    }

    /// Attach the shared agent lifecycle so `mode: wake` webhooks can
    /// rehydrate the target specialist before the event bus publishes
    /// (issue #335, mirroring the correlate path from #350).
    pub fn with_agent_lifecycle(mut self, lc: Arc<AgentLifecycle>) -> Self {
        self.state.agent_lifecycle = Some(lc);
        self
    }

    /// Override the default rate limiter (issue #335). Primarily for tests;
    /// production uses `WebhookRateLimiter::default_for_webhooks()`.
    pub fn with_webhook_rate_limiter(mut self, rl: Arc<WebhookRateLimiter>) -> Self {
        self.state.webhook_rate_limiter = rl;
        self
    }

    /// Get a handle to the reload broadcast sender (for the watcher to signal reloads).
    pub fn reload_sender(&self) -> Option<broadcast::Sender<()>> {
        self.state.reload_tx.clone()
    }

    /// Build the axum router with dynamic endpoint dispatch. Public so
    /// integration tests can mount it against a random TCP port without
    /// going through `run()`'s signal-handling loop.
    pub fn build_router(&self) -> Router<()> {
        // Dynamic catch-all routing: endpoint lookup happens at request time,
        // so new/removed endpoints are visible immediately after hot-reload.
        // Webhook route: POST /webhook/:name with Content-Type validation and optional HMAC
        let mut router = Router::new()
            // Dedicated approval webhook: handles Slack interactive payloads
            // (form-encoded) and direct JSON.  Must be registered before the
            // generic /webhook/{endpoint} catch-all so axum matches it first.
            .route(
                "/webhook/approval",
                axum::routing::post(handle_approval_webhook),
            )
            .route("/webhook/{endpoint}", axum::routing::post(handle_webhook))
            // Wake-driver webhook (#335): HMAC-verified POST that routes into a
            // specific agent's declared `webhook TRIGGER` block. Registered
            // before the generic `/{endpoint}` catch-all below.
            .route(
                "/wake/{agent_name}/{trigger_name}",
                axum::routing::post(handle_wake_webhook),
            )
            .route("/api/{endpoint}", get(handle_get_api).post(handle_post_api))
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
            .route("/__forge/inspect/costs", get(handle_inspect_costs))
            .route("/__forge/inspect/mastery", get(handle_inspect_mastery))
            .route("/__forge/inspect/schedules", get(handle_inspect_schedules));

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
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    handle_get_endpoint(state, endpoint_name, params, headers).await
}

async fn handle_get_api(
    State(state): State<AppState>,
    Path(endpoint_name): Path<String>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    handle_get_endpoint(state, api_endpoint_name(&endpoint_name), params, headers).await
}

async fn handle_get_endpoint(
    state: AppState,
    endpoint_name: String,
    mut params: HashMap<String, String>,
    headers: HeaderMap,
) -> Response {
    let executor = state.executor.read().unwrap().clone();
    if !executor.endpoints().contains_key(&endpoint_name) {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    }
    // Fill in missing params with empty strings so endpoints don't crash on undefined vars
    let endpoint = executor.endpoints().get(&endpoint_name).cloned();
    if let Some(ep) = endpoint.as_ref() {
        for param in &ep.params {
            params.entry(param.node.name.clone()).or_default();
        }
    }
    let request = build_request_record("GET", &endpoint_name, &params, &headers, "");
    let args = string_params_to_args(params, endpoint.as_ref());
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
    handle_post_endpoint(state, endpoint_name, headers, body_bytes).await
}

async fn handle_post_api(
    State(state): State<AppState>,
    Path(endpoint_name): Path<String>,
    headers: HeaderMap,
    body_bytes: axum::body::Bytes,
) -> Response {
    handle_post_endpoint(
        state,
        api_endpoint_name(&endpoint_name),
        headers,
        body_bytes,
    )
    .await
}

async fn handle_post_endpoint(
    state: AppState,
    endpoint_name: String,
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

    let endpoint = executor.endpoints().get(&endpoint_name).cloned();
    let (params, args): (HashMap<String, String>, HashMap<String, ConfidentValue>) =
        if content_type.contains("application/json") {
            // JSON body
            match serde_json::from_slice::<serde_json::Value>(&body_bytes) {
                Ok(json) => match json.as_object() {
                    Some(map) => (json_object_to_string_params(map), json_object_to_args(map)),
                    None => (HashMap::new(), HashMap::new()),
                },
                Err(_) => (HashMap::new(), HashMap::new()),
            }
        } else if !raw_body.contains('=') {
            match endpoint.as_ref() {
                Some(ep) => match raw_body_to_single_param_args(ep, &raw_body) {
                    Some(args) => (single_param_string_params(ep, &raw_body), args),
                    None => (HashMap::new(), HashMap::new()),
                },
                None => (HashMap::new(), HashMap::new()),
            }
        } else {
            // Form-encoded body (application/x-www-form-urlencoded)
            let params: HashMap<String, String> = raw_body
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
                .collect();
            let args = string_params_to_args(params.clone(), endpoint.as_ref());
            (params, args)
        };

    let request = build_request_record("POST", &endpoint_name, &params, &headers, &raw_body);
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

/// GET /__forge/inspect/schedules — declared wake surface + live schedule state (issue #336).
///
/// Walks the program AST for declared `schedule`/`correlate` blocks, then
/// unions in the live `ScheduleState` rows from storage. Declared-but-not-yet-
/// fired schedules appear with `state: null`. Webhooks are listed at the top
/// level with a `signed` flag — secrets are never exposed.
async fn handle_inspect_schedules(State(state): State<AppState>) -> Response {
    use crate::ast::{Precision, ScheduleMode, TopLevel, WhenExpr};

    // Declarations are keyed by the AST agent name (e.g. `cadence_probe`), but
    // WakeService persists state under the system alias (e.g. `probe` from
    // `use probe: cadence_probe`). Build `alias -> agent_name` from any
    // system bindings so the two can be unioned cleanly — fall back to
    // alias = agent_name when no system block is declared.
    let mut alias_to_agent: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    let mut correlations_decl: Vec<serde_json::Value> = Vec::new();
    let mut declarations: std::collections::BTreeMap<(String, String), serde_json::Value> =
        std::collections::BTreeMap::new();
    {
        let executor = state.executor.read().unwrap();
        for item in &executor.program().items {
            if let TopLevel::System(sys) = &item.node {
                for binding in &sys.bindings {
                    alias_to_agent.insert(binding.node.alias.clone(), binding.node.target.clone());
                }
            }
        }
        // Declarations are indexed by alias if a binding exists, otherwise by
        // the agent name itself — `forge run` without a system block registers
        // schedules under the raw agent name.
        let agent_to_alias: std::collections::HashMap<&str, &str> = alias_to_agent
            .iter()
            .map(|(alias, agent)| (agent.as_str(), alias.as_str()))
            .collect();
        for item in &executor.program().items {
            if let TopLevel::Agent(agent) = &item.node {
                let agent_name_str = agent.name.node.as_str();
                let key_name = agent_to_alias
                    .get(agent_name_str)
                    .map(|s| (*s).to_string())
                    .unwrap_or_else(|| agent.name.node.clone());
                let agent_name = &key_name;
                for sched in &agent.schedules {
                    let f = &sched.node;
                    let when = f.when.as_ref().map(|w| match &w.node {
                        WhenExpr::DailyAt(t) => format!("daily {:02}:{:02}", t.hour, t.minute),
                        WhenExpr::Every(d) => format!("every {} {:?}", d.value, d.unit),
                        WhenExpr::Cron(s) => format!("cron \"{s}\""),
                    });
                    let mode = f.mode.as_ref().map(|m| match m.node {
                        ScheduleMode::Spawn => "spawn",
                        ScheduleMode::Wake => "wake",
                    });
                    let precision = f.precision.as_ref().map(|p| match p.node {
                        Precision::High => "high",
                    });
                    let emit = f.emit.as_ref().map(|e| e.node.clone());
                    declarations.insert(
                        (agent_name.clone(), f.name.node.clone()),
                        serde_json::json!({
                            "when": when,
                            "mode": mode,
                            "precision": precision,
                            "emit": emit,
                        }),
                    );
                }
                for corr in &agent.correlates {
                    let c = &corr.node;
                    let mode = c.mode.as_ref().map(|m| match m.node {
                        ScheduleMode::Spawn => "spawn",
                        ScheduleMode::Wake => "wake",
                    });
                    correlations_decl.push(serde_json::json!({
                        "agent": agent_name,
                        "event_type": c.event_type.node,
                        "field": c.field_name.node,
                        "mode": mode,
                        "emit": c.emit.as_ref().map(|e| e.node.clone()),
                    }));
                }
            }
        }
    }

    // Merge in live ScheduleState rows.
    let mut schedule_rows: Vec<serde_json::Value> = Vec::new();
    let mut seen: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
    if let Some(ref storage) = state.inspect_storage {
        match storage.list_all_schedules() {
            Ok(rows) => {
                for (agent, schedule, st) in rows {
                    let key = (agent.clone(), schedule.clone());
                    let declaration = declarations.get(&key).cloned();
                    seen.insert(key);
                    schedule_rows.push(serde_json::json!({
                        "agent": agent,
                        "schedule": schedule,
                        "declaration": declaration,
                        "next_run_at_ms": st.next_run_at_ms,
                        "last_run_at_ms": st.last_run_at_ms,
                        "last_status": format!("{:?}", st.last_status).to_lowercase(),
                        "consecutive_errors": st.consecutive_errors,
                        "claimed_by": st.claimed_by,
                        "claim_expires_at_ms": st.claim_expires_at_ms,
                    }));
                }
            }
            Err(e) => {
                return json_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    serde_json::json!({"error": format!("{}", e)}),
                );
            }
        }
    }
    // Append declared-but-never-fired schedules.
    for ((agent, schedule), declaration) in &declarations {
        if !seen.contains(&(agent.clone(), schedule.clone())) {
            schedule_rows.push(serde_json::json!({
                "agent": agent,
                "schedule": schedule,
                "declaration": declaration,
                "next_run_at_ms": null,
                "last_run_at_ms": null,
                "last_status": "not_registered",
                "consecutive_errors": 0,
                "claimed_by": null,
                "claim_expires_at_ms": null,
            }));
        }
    }

    // Correlation live counts.
    let correlations_live: Vec<serde_json::Value> = match state.inspect_storage.as_ref() {
        Some(storage) => match storage.list_all_correlations() {
            Ok(rows) => rows
                .into_iter()
                .map(|(agent, field, count)| {
                    serde_json::json!({
                        "agent": agent,
                        "field": field,
                        "value_count": count,
                    })
                })
                .collect(),
            Err(_) => Vec::new(),
        },
        None => Vec::new(),
    };

    // Webhook endpoints: every endpoint is reachable at `/webhook/{name}`; the
    // `signed` flag tells operators whether an HMAC secret is enforced.
    let webhooks: Vec<serde_json::Value> = {
        let executor = state.executor.read().unwrap();
        executor
            .endpoints()
            .keys()
            .map(|endpoint| {
                serde_json::json!({
                    "endpoint": endpoint,
                    "signed": state.webhook_secrets.contains_key(endpoint),
                })
            })
            .collect()
    };

    json_response(
        StatusCode::OK,
        serde_json::json!({
            "schedules": schedule_rows,
            "webhooks": webhooks,
            "correlations_declared": correlations_decl,
            "correlations_live": correlations_live,
        }),
    )
}

/// GET /__forge/inspect/mastery — per-(specialist, project) mastery progression
/// plus per-task `review_rounds` trend (issue #304). The response combines:
/// - knowledge-store entries under `mastery-{specialist}-{project}` (level
///   transitions, sparse — one entry per FSM level change)
/// - `TaskHistoryAggregator` snapshot (per-task `review_rounds` trend, dense)
async fn handle_inspect_mastery(State(state): State<AppState>) -> Response {
    let tasks_snapshot = match state.task_history_aggregator.as_ref() {
        Some(agg) => agg.read().await.snapshot(),
        None => serde_json::json!({
            "total_tasks": 0,
            "projects": [],
            "tasks_by_project": {},
            "uptime_secs": 0.0,
        }),
    };

    let (mastery_map, projects_from_knowledge) =
        collect_mastery_snapshots(state.mastery_knowledge_store.as_ref());

    // Merge projects discovered from both sources.
    let mut projects: std::collections::BTreeSet<String> = tasks_snapshot["projects"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    projects.extend(projects_from_knowledge);
    let projects: Vec<String> = projects.into_iter().collect();

    json_response(
        StatusCode::OK,
        serde_json::json!({
            "specialists": ["planner", "implementer", "tester", "reviewer", "release_manager"],
            "projects": projects,
            "mastery": mastery_map,
            "tasks": tasks_snapshot,
        }),
    )
}

/// Read `mastery-{specialist}-{project}` knowledge entries and assemble per-tuple
/// transition timelines. Returns the JSON map keyed `"{specialist}::{project}"`
/// plus the set of projects discovered along the way.
fn collect_mastery_snapshots(
    store: Option<&SharedKnowledgeStore>,
) -> (serde_json::Value, Vec<String>) {
    let mut out = serde_json::Map::new();
    let mut projects = Vec::new();
    let Some(store) = store else {
        return (serde_json::Value::Object(out), projects);
    };
    let Ok(guard) = store.lock() else {
        return (serde_json::Value::Object(out), projects);
    };
    let all = guard.export_entries();
    drop(guard);

    // Group entries by (specialist, project).
    let mut by_key: HashMap<(String, String), Vec<SwarmMasteryEntry>> = HashMap::new();
    for entry in all {
        let Some(category) = entry.category.as_deref() else {
            continue;
        };
        let Some(rest) = category.strip_prefix("mastery-") else {
            continue;
        };
        // Parse the compact snapshot lines that `swarm_mastery_tuple` writes.
        let Some(parsed) = parse_swarm_mastery_snapshot(&entry.content) else {
            continue;
        };
        // Prefer the parsed (specialist, project) — they're authoritative —
        // but fall back to the category if parsing a field is missing.
        let (specialist, project) = if let Some((s, p)) = split_specialist_project(rest) {
            (
                parsed.specialist.clone().unwrap_or_else(|| s.to_string()),
                parsed.project.clone().unwrap_or_else(|| p.to_string()),
            )
        } else {
            (
                parsed.specialist.clone().unwrap_or_default(),
                parsed.project.clone().unwrap_or_default(),
            )
        };
        if specialist.is_empty() || project.is_empty() {
            continue;
        }
        by_key
            .entry((specialist, project))
            .or_default()
            .push(SwarmMasteryEntry {
                at: entry.created_at,
                level: parsed.level,
                score: parsed.score,
                clean_count: parsed.clean_count,
                regress_count: parsed.regress_count,
                total: parsed.total,
                last_task: parsed.last_task,
            });
    }

    for ((specialist, project), mut entries) in by_key {
        entries.sort_by_key(|e| e.at);
        let latest = entries.last().cloned();
        let transitions: Vec<serde_json::Value> = entries
            .iter()
            .map(|e| {
                serde_json::json!({
                    "at": e.at.to_rfc3339(),
                    "level": e.level,
                    "score": e.score,
                    "clean_count": e.clean_count,
                    "regress_count": e.regress_count,
                    "total": e.total,
                    "last_task": e.last_task,
                })
            })
            .collect();
        let key = format!("{}::{}", specialist, project);
        out.insert(
            key,
            serde_json::json!({
                "specialist": specialist,
                "project": project,
                "current_level": latest.as_ref().map(|e| e.level.clone()).unwrap_or_else(|| "novice".to_string()),
                "current_score": latest.as_ref().map(|e| e.score).unwrap_or(0.0),
                "clean_count": latest.as_ref().map(|e| e.clean_count).unwrap_or(0),
                "regress_count": latest.as_ref().map(|e| e.regress_count).unwrap_or(0),
                "total": latest.as_ref().map(|e| e.total).unwrap_or(0),
                "transitions": transitions,
            }),
        );
        projects.push(project);
    }

    projects.sort();
    projects.dedup();
    (serde_json::Value::Object(out), projects)
}

#[derive(Clone)]
struct SwarmMasteryEntry {
    at: chrono::DateTime<chrono::Utc>,
    level: String,
    score: f64,
    clean_count: u64,
    regress_count: u64,
    total: u64,
    last_task: String,
}

#[derive(Default)]
struct ParsedSwarmMastery {
    specialist: Option<String>,
    project: Option<String>,
    level: String,
    score: f64,
    clean_count: u64,
    regress_count: u64,
    total: u64,
    last_task: String,
}

/// Parse the free-text snapshot written by `swarm_mastery_tuple.learn(...)`:
///
/// ```text
/// SWARM-MASTERY specialist:{s} project:{p} level:{l} score:{n}
/// clean:{c} regress:{r} total:{t}
/// last_task:{task_id}
/// ```
///
/// Returns `None` if the expected `SWARM-MASTERY` marker is absent — this
/// keeps us robust to unrelated entries that share the `mastery-*` category
/// namespace.
fn parse_swarm_mastery_snapshot(content: &str) -> Option<ParsedSwarmMastery> {
    if !content.contains("SWARM-MASTERY") {
        return None;
    }
    let mut parsed = ParsedSwarmMastery::default();
    for token in content.split_whitespace() {
        let Some((key, value)) = token.split_once(':') else {
            continue;
        };
        match key {
            "specialist" => parsed.specialist = Some(value.to_string()),
            "project" => parsed.project = Some(value.to_string()),
            "level" => parsed.level = value.to_string(),
            "score" => parsed.score = value.parse::<f64>().unwrap_or(0.0),
            "clean" => parsed.clean_count = value.parse::<u64>().unwrap_or(0),
            "regress" => parsed.regress_count = value.parse::<u64>().unwrap_or(0),
            "total" => parsed.total = value.parse::<u64>().unwrap_or(0),
            "last_task" => parsed.last_task = value.to_string(),
            _ => {}
        }
    }
    Some(parsed)
}

/// Split a category suffix like `planner-ncmlabs-forge-playground` into
/// (specialist, project). Specialists are a closed set, so we match the
/// prefix and treat the remainder as the project slug.
fn split_specialist_project(suffix: &str) -> Option<(&str, &str)> {
    const SPECIALISTS: &[&str] = &[
        "planner",
        "implementer",
        "tester",
        "reviewer",
        "release_manager",
    ];
    for s in SPECIALISTS {
        if let Some(rest) = suffix.strip_prefix(&format!("{}-", s)) {
            return Some((s, rest));
        }
    }
    None
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
    let body_bytes = body.len();

    // Compute signature outcome up front so the tracer event (issue #336) can
    // record whether HMAC validation passed — `None` means no secret was
    // configured, `Some(true/false)` means the check ran with this outcome.
    let signature_valid: Option<bool> = state.webhook_secrets.get(&endpoint_name).map(|secret| {
        let signature = headers
            .get("x-hub-signature-256")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        verify_hmac_signature(secret, &body, signature)
    });

    // Emit `webhook_received` regardless of downstream accept/reject — an
    // arrived-but-rejected webhook is still observable traffic.
    if let Some(tracer) = state.executor.read().unwrap().tracer() {
        tracer.webhook_received(&endpoint_name, signature_valid, body_bytes);
    }

    if signature_valid == Some(false) {
        return (StatusCode::UNAUTHORIZED, "invalid signature").into_response();
    }

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

    let (params, args): (HashMap<String, String>, HashMap<String, ConfidentValue>) =
        match json_value.as_object() {
            Some(map) => (json_object_to_string_params(map), json_object_to_args(map)),
            None => (HashMap::new(), HashMap::new()),
        };

    let request = build_request_record("POST", &endpoint_name, &params, &headers, &body_str);
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
    use hmac::{Hmac, KeyInit, Mac};
    use sha2::Sha256;

    let expected_hex = match signature.strip_prefix("sha256=") {
        Some(hex) => hex,
        None => return false,
    };

    let mut mac: Hmac<Sha256> = match Hmac::new_from_slice(secret.as_bytes()) {
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

fn api_endpoint_name(endpoint_name: &str) -> String {
    format!("api_{}", endpoint_name.replace('-', "_"))
}

/// Wake-webhook handler (issue #335). Flow:
///
/// 1. Per-`(agent, trigger)` rate-limit check → `429` on exceed.
/// 2. Read raw body as bytes.
/// 3. Look up the registered HMAC secret → `404` if absent.
/// 4. Verify `X-Hub-Signature-256` against the raw body → `401` on mismatch.
/// 5. Resolve the declared webhook block in the driver → `404` if absent.
/// 6. For `mode: wake`, await `AgentLifecycle::rehydrate_or_spawn(agent)`
///    *before* publishing so the rehydrated specialist is subscribed.
/// 7. Publish the declared event on the bus with body-derived fields.
/// 8. Return `202 Accepted`.
///
/// Tracer events (`webhook_received` etc.) are emitted via `events_tx`; they
/// intentionally never include secret material or signature bytes.
async fn handle_wake_webhook(
    State(state): State<AppState>,
    Path((agent_name, trigger_name)): Path<(String, String)>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    // 1. Rate limit.
    if !state.webhook_rate_limiter.check(&agent_name, &trigger_name) {
        emit_wake_event(
            &state,
            "webhook_rate_limited",
            serde_json::json!({
                "agent": agent_name,
                "trigger": trigger_name,
            }),
        );
        return (StatusCode::TOO_MANY_REQUESTS, "rate limit exceeded").into_response();
    }

    // 3. Secret lookup (before HMAC verify).
    let storage = match &state.wake_storage {
        Some(s) => s,
        None => {
            emit_wake_event(
                &state,
                "webhook_rejected_unknown",
                serde_json::json!({
                    "agent": agent_name,
                    "trigger": trigger_name,
                    "reason": "wake_storage_unconfigured",
                }),
            );
            return (StatusCode::NOT_FOUND, "wake webhooks not configured").into_response();
        }
    };
    let secret = match storage.lookup_wake_secret(&agent_name, &trigger_name) {
        Ok(Some(s)) => s,
        Ok(None) => {
            emit_wake_event(
                &state,
                "webhook_rejected_unknown",
                serde_json::json!({
                    "agent": agent_name,
                    "trigger": trigger_name,
                    "reason": "secret_missing",
                }),
            );
            return (StatusCode::NOT_FOUND, "unknown webhook").into_response();
        }
        Err(_) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, "storage error").into_response();
        }
    };

    // 4. HMAC verify against the raw body — do this before parsing.
    let signature = headers
        .get("x-hub-signature-256")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !verify_hmac_signature(&secret, &body, signature) {
        emit_wake_event(
            &state,
            "webhook_rejected_signature",
            serde_json::json!({
                "agent": agent_name,
                "trigger": trigger_name,
            }),
        );
        return (StatusCode::UNAUTHORIZED, "invalid signature").into_response();
    }

    // 5. Driver lookup — 404 if no declared webhook block for this pair.
    let driver = match &state.webhook_driver {
        Some(d) => d.clone(),
        None => {
            return (StatusCode::NOT_FOUND, "no webhooks declared").into_response();
        }
    };
    let registration = match driver.match_webhook(&agent_name, &trigger_name) {
        Some(r) => r.clone(),
        None => {
            emit_wake_event(
                &state,
                "webhook_rejected_unknown",
                serde_json::json!({
                    "agent": agent_name,
                    "trigger": trigger_name,
                    "reason": "unknown_pair",
                }),
            );
            return (StatusCode::NOT_FOUND, "unknown webhook").into_response();
        }
    };

    // GitHub sends a `ping` payload when a webhook is created. That body is
    // intentionally not shaped like the later `issues` payloads, so accept the
    // verified handshake without publishing it into the typed FORGE event.
    if headers
        .get("x-github-event")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|event| event.eq_ignore_ascii_case("ping"))
    {
        emit_wake_event(
            &state,
            "webhook_ping_ignored",
            serde_json::json!({
                "agent": agent_name,
                "trigger": trigger_name,
                "provider": "github",
                "bytes_len": body.len(),
            }),
        );
        return (StatusCode::ACCEPTED, "accepted").into_response();
    }

    // 6. If `mode: wake`, rehydrate the target agent before publishing so the
    // rehydrated specialist is subscribed when the event fans out.
    let mut rehydrated = false;
    if matches!(registration.mode, crate::ast::ScheduleMode::Wake) {
        if let Some(ref lc) = state.agent_lifecycle {
            match lc.rehydrate_or_spawn(&registration.agent).await {
                Ok(handle) => {
                    rehydrated = !handle.was_already_live;
                }
                Err(e) => {
                    emit_wake_event(
                        &state,
                        "webhook_rehydrate_failed",
                        serde_json::json!({
                            "agent": agent_name,
                            "trigger": trigger_name,
                            "reason": format!("{e:?}"),
                        }),
                    );
                    return (StatusCode::INTERNAL_SERVER_ERROR, "rehydrate failed").into_response();
                }
            }
        }
    }

    // 7. Build the event payload. If body is JSON object, expose its fields
    // as `ConfidentValue`s (same shape as the existing `/webhook/{endpoint}`
    // handler). Otherwise, expose the raw body as a `Text` under `body`.
    let payload_format;
    let fields: HashMap<String, ConfidentValue> = if let Ok(v) =
        serde_json::from_slice::<serde_json::Value>(&body)
    {
        match v.as_object() {
            Some(map) => {
                payload_format = "json";
                json_object_to_args(map)
            }
            None => {
                payload_format = "text";
                let mut m = HashMap::new();
                m.insert(
                    "body".to_string(),
                    ConfidentValue::deterministic(Value::Text(
                        String::from_utf8_lossy(&body).to_string(),
                    )),
                );
                m
            }
        }
    } else {
        payload_format = "text";
        let mut m = HashMap::new();
        m.insert(
            "body".to_string(),
            ConfidentValue::deterministic(Value::Text(String::from_utf8_lossy(&body).to_string())),
        );
        m
    };
    let event_payload = EventPayload {
        event_name: registration.emit_event.clone(),
        args: Vec::new(),
        source_agent: format!("webhook:{}", trigger_name),
        fields,
    };

    if let Some(ref bus) = state.event_bus {
        let guard = bus.read().await;
        guard.publish(&event_payload);
    }

    // 8. Trace the successful receipt.
    emit_wake_event(
        &state,
        "webhook_received",
        serde_json::json!({
            "agent": agent_name,
            "trigger": trigger_name,
            "mode": match registration.mode {
                crate::ast::ScheduleMode::Wake => "wake",
                crate::ast::ScheduleMode::Spawn => "spawn",
            },
            "emit": registration.emit_event,
            "bytes_len": body.len(),
            "payload_format": payload_format,
            "rehydrated": rehydrated,
        }),
    );

    (StatusCode::ACCEPTED, "accepted").into_response()
}

fn emit_wake_event(state: &AppState, event: &str, data: serde_json::Value) {
    let Some(ref tx) = state.events_tx else {
        return;
    };
    let ts_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let mut obj = match data {
        serde_json::Value::Object(m) => m,
        other => {
            let mut m = serde_json::Map::new();
            m.insert("value".to_string(), other);
            m
        }
    };
    obj.insert("ts_ms".to_string(), serde_json::json!(ts_ms));
    obj.insert("event".to_string(), serde_json::json!(event));
    let _ = tx.send(serde_json::Value::Object(obj).to_string());
}

/// Convert a map of string-valued parameters into `ConfidentValue` args, coercing
/// each value to the endpoint's declared parameter type when the endpoint is
/// known. Unknown params (not declared on the endpoint) and endpoints that
/// aren't resolved fall through as `Value::Text`.
///
/// This keeps form-encoded POST bodies and GET query strings symmetric with the
/// raw-body single-param path (see `raw_body_to_single_param_args`): typed
/// endpoints receive `Number` / `Bool` instead of `Text("98")`.
fn string_params_to_args(
    params: HashMap<String, String>,
    endpoint: Option<&crate::ast::EndpointDecl>,
) -> HashMap<String, ConfidentValue> {
    params
        .into_iter()
        .map(|(k, v)| {
            let type_name = endpoint.and_then(|ep| {
                ep.params
                    .iter()
                    .find(|p| p.node.name == k)
                    .map(|p| &p.node.type_name.node)
            });
            let value = match type_name {
                Some(ty) => coerce_to_param_type(&v, ty),
                None => Value::Text(v),
            };
            (k, ConfidentValue::deterministic(value))
        })
        .collect()
}

/// Coerce a raw string value to a FORGE `Value` matching the declared
/// parameter type. Numbers parse via `f64`, bools accept `true`/`1`. On parse
/// failure we fall back to `Value::Text` so the endpoint can still surface a
/// meaningful type error instead of a silent empty response. Shared between
/// `string_params_to_args` (form / query) and `raw_body_to_single_param_args`
/// (raw body).
fn coerce_to_param_type(raw: &str, type_name: &crate::ast::TypeName) -> Value {
    match type_name {
        crate::ast::TypeName::Number => raw
            .trim()
            .parse::<f64>()
            .map(Value::Number)
            .unwrap_or_else(|_| Value::Text(raw.to_string())),
        crate::ast::TypeName::Bool => {
            let normalized = raw.trim();
            Value::Bool(normalized == "true" || normalized == "1")
        }
        _ => Value::Text(raw.to_string()),
    }
}

fn json_object_to_string_params(
    map: &serde_json::Map<String, serde_json::Value>,
) -> HashMap<String, String> {
    map.iter()
        .map(|(k, v)| {
            let s = match v {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            (k.clone(), s)
        })
        .collect()
}

fn json_object_to_args(
    map: &serde_json::Map<String, serde_json::Value>,
) -> HashMap<String, ConfidentValue> {
    map.iter()
        .map(|(k, v)| (k.clone(), json_value_to_confident(v)))
        .collect()
}

fn json_value_to_confident(value: &serde_json::Value) -> ConfidentValue {
    let converted = match value {
        serde_json::Value::Null => Value::Unit,
        serde_json::Value::Bool(v) => Value::Bool(*v),
        serde_json::Value::Number(v) => Value::Number(v.as_f64().unwrap_or(0.0)),
        serde_json::Value::String(v) => Value::Text(v.clone()),
        serde_json::Value::Array(items) => Value::Array(
            items
                .iter()
                .map(json_value_to_confident)
                .collect::<Vec<ConfidentValue>>(),
        ),
        serde_json::Value::Object(fields) => Value::Record(json_object_to_args(fields)),
    };
    ConfidentValue::deterministic(converted)
}

fn single_param_string_params(
    endpoint: &crate::ast::EndpointDecl,
    raw_body: &str,
) -> HashMap<String, String> {
    endpoint
        .params
        .first()
        .map(|param| HashMap::from([(param.node.name.clone(), raw_body.to_string())]))
        .unwrap_or_default()
}

fn raw_body_to_single_param_args(
    endpoint: &crate::ast::EndpointDecl,
    raw_body: &str,
) -> Option<HashMap<String, ConfidentValue>> {
    if endpoint.params.len() != 1 {
        return None;
    }

    let param = endpoint.params.first()?;
    let value = coerce_to_param_type(raw_body, &param.node.type_name.node);

    Some(HashMap::from([(
        param.node.name.clone(),
        ConfidentValue::deterministic(value),
    )]))
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
            if let Some(inner) = custom_record_payload(fields) {
                return value_to_json(inner);
            }
            let map: serde_json::Map<String, serde_json::Value> = fields
                .iter()
                .map(|(k, v)| (k.clone(), value_to_json(v)))
                .collect();
            serde_json::Value::Object(map)
        }
    }
}

fn custom_record_payload(fields: &HashMap<String, ConfidentValue>) -> Option<&ConfidentValue> {
    if !matches!(fields.get("_type")?.value, Value::Text(_)) {
        return None;
    }
    match &fields.get("_value")?.value {
        Value::Record(_) => fields.get("_value"),
        _ => None,
    }
}

// ── Approval webhook handler ────────────────────────────────────────────────
// Dedicated endpoint for human-in-the-loop approval gates (issue #182).
// Accepts Slack interactive payloads (form-encoded) and direct JSON, then
// publishes an ApprovalResponse event to the bus.

/// Parsed Slack approval payload.
struct SlackApproval {
    request_id: String,
    approved: bool,
    comment: String,
    /// Per-interaction URL provided by Slack for updating the original message.
    response_url: Option<String>,
}

/// Parse a Slack interactive payload (application/x-www-form-urlencoded).
/// Slack wraps its JSON inside a form field named `payload`.
fn parse_slack_approval_payload(body: &[u8]) -> Result<SlackApproval, String> {
    let body_str = String::from_utf8(body.to_vec()).map_err(|_| "invalid UTF-8")?;
    // Find the payload= form field and URL-decode its value.
    let payload_json = body_str
        .split('&')
        .find_map(|pair| {
            let (key, val) = pair.split_once('=')?;
            if key == "payload" {
                urlencoding::decode(val).ok().map(|s| s.into_owned())
            } else {
                None
            }
        })
        .ok_or("missing payload field")?;
    let json: serde_json::Value =
        serde_json::from_str(&payload_json).map_err(|_| "invalid JSON in payload")?;
    // Extract action_id and value from actions[0].
    let action = json.pointer("/actions/0").ok_or("missing actions[0]")?;
    let value = action.get("value").and_then(|v| v.as_str()).unwrap_or("");
    // value format: "approved:{request_id}" or "rejected:{request_id}"
    let (decision, request_id) = value
        .split_once(':')
        .ok_or("button value must be decision:request_id")?;
    let approved = decision == "approved";
    // Build comment from Slack user info.
    let user_name = json
        .pointer("/user/name")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let user_id = json
        .pointer("/user/id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let comment = format!("{} ({})", user_name, user_id);
    let response_url = json
        .get("response_url")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    Ok(SlackApproval {
        request_id: request_id.to_string(),
        approved,
        comment,
        response_url,
    })
}

/// Parse a direct JSON approval payload (for testing and non-Slack sources).
fn parse_json_approval_payload(body: &[u8]) -> Result<(String, bool, String), String> {
    let json: serde_json::Value = serde_json::from_slice(body).map_err(|_| "invalid JSON")?;
    let request_id = json
        .get("request_id")
        .and_then(|v| v.as_str())
        .ok_or("missing request_id")?
        .to_string();
    let approved = json
        .get("approved")
        .and_then(|v| v.as_bool())
        .ok_or("missing approved")?;
    let comment = json
        .get("comment")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    Ok((request_id, approved, comment))
}

/// POST /webhook/approval — inject an ApprovalResponse event into the bus.
async fn handle_approval_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let content_type = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let is_slack = content_type.contains("x-www-form-urlencoded");

    let (request_id, approved, comment, response_url) = if is_slack {
        match parse_slack_approval_payload(&body) {
            Ok(s) => (s.request_id, s.approved, s.comment, s.response_url),
            Err(e) => return (StatusCode::BAD_REQUEST, e).into_response(),
        }
    } else if content_type.contains("json") {
        match parse_json_approval_payload(&body) {
            Ok((r, a, c)) => (r, a, c, None),
            Err(e) => return (StatusCode::BAD_REQUEST, e).into_response(),
        }
    } else {
        return (StatusCode::BAD_REQUEST, "unsupported Content-Type").into_response();
    };

    let Some(ref bus) = state.event_bus else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no event bus configured").into_response();
    };

    use crate::runtime::event_bus::EventPayload;
    let mut fields = HashMap::new();
    fields.insert(
        "request_id".to_string(),
        ConfidentValue::deterministic(Value::Text(request_id.clone())),
    );
    fields.insert(
        "approved".to_string(),
        ConfidentValue::deterministic(Value::Bool(approved)),
    );
    fields.insert(
        "comment".to_string(),
        ConfidentValue::deterministic(Value::Text(comment.clone())),
    );

    let payload = EventPayload {
        event_name: "ApprovalResponse".to_string(),
        args: vec![],
        source_agent: "webhook".to_string(),
        fields,
    };

    let bus_guard = bus.read().await;
    bus_guard.publish(&payload);

    // Update the Slack message via response_url (async, non-blocking).
    if let Some(url) = response_url {
        let (icon, verb) = if approved {
            ("\u{2705}", "Approved")
        } else {
            ("\u{274c}", "Rejected")
        };
        let update_body = serde_json::json!({
            "replace_original": true,
            "text": format!("{icon} {verb} by {comment}"),
            "blocks": [{
                "type": "section",
                "text": {
                    "type": "mrkdwn",
                    "text": format!("{icon} *{verb}* by {comment}\nRequest: `{request_id}`")
                }
            }]
        });
        tokio::spawn(async move {
            let _ = reqwest::Client::new()
                .post(&url)
                .json(&update_body)
                .send()
                .await;
        });
    }

    // Return confirmation (empty for Slack, JSON for direct callers).
    if is_slack {
        StatusCode::OK.into_response()
    } else {
        let json = serde_json::json!({
            "status": if approved { "approved" } else { "rejected" },
            "request_id": request_id,
        });
        (
            StatusCode::OK,
            [("content-type", "application/json")],
            json.to_string(),
        )
            .into_response()
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
