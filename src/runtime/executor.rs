// FORGE task executor
// Walks the AST, evaluates expressions, dispatches on confidence predicates,
// calls LLM providers. See issue #9.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::ast::*;
use crate::llm::registry::ProviderRegistry;
use crate::llm::CompletionRequest;
use crate::runtime::agent::{AgentContext, EmittedEvent};
use crate::runtime::command_manager::SharedCommandManager;
use crate::runtime::confidence::{ConfidentValue, Value};
use crate::runtime::instance_registry::{InstanceInfo, InstanceStatus};
use crate::runtime::session_manager::{
    SessionConfig, SessionEvent, SessionListener, SharedSessionManager,
};
use crate::runtime::timer_engine::TimerEngine;
use crate::runtime::verification::{RiskClass, VerificationResult, VerificationStatus};
use crate::tracer::{LLMResponseInfo, Tracer};
// ── Errors ────────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error("undefined variable: {name}")]
    UndefinedVariable { name: String },

    #[error("type error: expected {expected}, got {got}")]
    TypeError { expected: String, got: String },

    #[error("no fn main entry point — add a fn main block")]
    NoMainFunction,

    #[error("not callable: {name}")]
    NotCallable { name: String },

    #[error("division by zero")]
    DivisionByZero,

    #[error("index out of bounds: {index} (length {len})")]
    IndexOutOfBounds { index: usize, len: usize },

    #[error("LLM error: {0}")]
    LLMError(String),

    #[error("not yet implemented: {0}")]
    Unsupported(String),

    #[error("flow error: {0}")]
    FlowError(String),

    #[error("pool error: {0}")]
    PoolError(String),

    // Control flow — not a real error, used to propagate `give` values
    #[error("give signal (internal)")]
    GiveSignal(ConfidentValue, Option<u16>, Option<String>),

    // Control flow — not a real error, used to signal graceful agent retirement
    #[error("retire signal (internal)")]
    RetireSignal,

    // Control flow — raised by the `exit(code)` capability in one-shot CLI clients
    // to propagate a process exit code. See issue #258 (part of epic #249).
    // The generated CLI dispatch (src/build.rs) catches this and calls
    // std::process::exit(code) without printing — the handler has already
    // emitted any user-facing message via `say`.
    #[error("")]
    Exit(i32),
}

/// Result from executing an endpoint, including optional response metadata.
#[derive(Debug, Clone)]
pub struct EndpointResult {
    pub value: ConfidentValue,
    pub status: Option<u16>,
    pub content_type: Option<String>,
    /// Content-type inferred from the endpoint's return type annotation.
    pub default_content_type: Option<String>,
}

// ── Environment (scope stack) ─────────────────────────────────────────────────

#[derive(Clone)]
pub(crate) struct Env {
    scopes: Vec<HashMap<String, ConfidentValue>>,
}

impl Env {
    pub(crate) fn new() -> Self {
        Self {
            scopes: vec![HashMap::new()],
        }
    }

    pub(crate) fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    pub(crate) fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    pub(crate) fn bind(&mut self, name: &str, value: ConfidentValue) {
        // If the variable already exists in an outer scope, update it there
        // (reassignment semantics). Otherwise, create in the current scope.
        for scope in self.scopes.iter_mut().rev() {
            if scope.contains_key(name) {
                scope.insert(name.to_string(), value);
                return;
            }
        }
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.to_string(), value);
        }
    }

    pub(crate) fn lookup(&self, name: &str) -> Result<&ConfidentValue, RuntimeError> {
        for scope in self.scopes.iter().rev() {
            if let Some(val) = scope.get(name) {
                return Ok(val);
            }
        }
        Err(RuntimeError::UndefinedVariable {
            name: name.to_string(),
        })
    }

    fn top_scope_bindings(&self) -> HashMap<String, ConfidentValue> {
        self.scopes.last().cloned().unwrap_or_default()
    }
}

// ── Task Executor ─────────────────────────────────────────────────────────────

pub struct TaskExecutor {
    program: Program,
    providers: Arc<ProviderRegistry>,
    tracer: Option<Tracer>,
    task_map: HashMap<String, TaskDecl>,
    pure_map: HashMap<String, PureDecl>,
    flow_map: HashMap<String, FlowDecl>,
    pool_map: HashMap<String, PoolDecl>,
    endpoint_map: HashMap<String, EndpointDecl>,
    output: Arc<Mutex<Vec<String>>>,
    agent_context: Option<Arc<Mutex<AgentContext>>>,
    timer_engine: Option<Arc<Mutex<TimerEngine>>>,
    storage: Option<crate::runtime::storage::SharedStorage>,
    instance_registry: Option<crate::runtime::instance_registry::SharedInstanceRegistry>,
    event_bus: Option<crate::runtime::event_bus::SharedEventBus>,
    agent_name: Option<String>,
    memory_persistent: bool,
    config: Option<crate::config::ForgeConfig>,
    /// When true, template strings produce `Value::Html` and auto-escape interpolated values.
    html_context: AtomicBool,
    /// Standalone knowledge store for endpoint/webhook context (no agent required).
    knowledge_store: Option<Arc<Mutex<crate::runtime::knowledge_store::KnowledgeStore>>>,
    /// Skill executor for LLM-mediated skill bridge (issue #40).
    skill_executor: Option<Arc<crate::runtime::skill_executor::SkillExecutor>>,
    /// Embedding provider for data.embed/data.search (issue #50).
    embedding_provider: Option<crate::llm::BoxedEmbeddingProvider>,
    /// Vector index for semantic search (issue #50).
    vector_index: Option<crate::runtime::vector_index::SharedVectorIndex>,
    /// Command manager for background process lifecycle (issue #162).
    command_manager: Option<SharedCommandManager>,
    /// Session manager for long-running external agent sessions (issue #190).
    session_manager: Option<SharedSessionManager>,
    /// Working directory override for sandbox isolation (issue #194).
    working_dir: Option<std::path::PathBuf>,
}

impl Clone for TaskExecutor {
    fn clone(&self) -> Self {
        Self {
            program: self.program.clone(),
            providers: self.providers.clone(),
            tracer: self.tracer.clone(),
            task_map: self.task_map.clone(),
            pure_map: self.pure_map.clone(),
            flow_map: self.flow_map.clone(),
            pool_map: self.pool_map.clone(),
            endpoint_map: self.endpoint_map.clone(),
            output: self.output.clone(),
            agent_context: self.agent_context.clone(),
            timer_engine: self.timer_engine.clone(),
            storage: self.storage.clone(),
            instance_registry: self.instance_registry.clone(),
            event_bus: self.event_bus.clone(),
            agent_name: self.agent_name.clone(),
            memory_persistent: self.memory_persistent,
            config: self.config.clone(),
            html_context: AtomicBool::new(self.html_context.load(Ordering::Relaxed)),
            knowledge_store: self.knowledge_store.clone(),
            skill_executor: self.skill_executor.clone(),
            embedding_provider: self.embedding_provider.clone(),
            vector_index: self.vector_index.clone(),
            command_manager: self.command_manager.clone(),
            session_manager: self.session_manager.clone(),
            working_dir: self.working_dir.clone(),
        }
    }
}

impl TaskExecutor {
    pub fn new(program: Program, providers: Arc<ProviderRegistry>, tracer: Option<Tracer>) -> Self {
        let mut task_map = HashMap::new();
        let mut pure_map = HashMap::new();
        let mut flow_map = HashMap::new();
        let mut pool_map = HashMap::new();
        let mut endpoint_map = HashMap::new();

        for item in &program.items {
            match &item.node {
                TopLevel::Task(t) => {
                    task_map.insert(t.name.node.clone(), t.clone());
                }
                TopLevel::Pure(p) => {
                    pure_map.insert(p.name.node.clone(), p.clone());
                }
                TopLevel::Flow(f) => {
                    flow_map.insert(f.name.node.clone(), f.clone());
                }
                TopLevel::Pool(pl) => {
                    pool_map.insert(pl.name.node.clone(), pl.clone());
                }
                TopLevel::Endpoint(e) => {
                    endpoint_map.insert(e.name.node.clone(), e.clone());
                }
                _ => {}
            }
        }

        Self {
            program,
            providers,
            tracer: tracer.clone(),
            task_map,
            pure_map,
            flow_map,
            pool_map,
            endpoint_map,
            output: Arc::new(Mutex::new(Vec::new())),
            agent_context: None,
            timer_engine: None,
            storage: None,
            instance_registry: None,
            event_bus: None,
            agent_name: None,
            memory_persistent: false,
            config: None,
            html_context: AtomicBool::new(false),
            knowledge_store: None,
            skill_executor: None,
            embedding_provider: None,
            vector_index: None,
            command_manager: None,
            session_manager: Some(
                crate::runtime::session_manager::new_shared_default_session_manager(tracer.clone()),
            ),
            working_dir: None,
        }
    }

    /// Set working directory for sandbox isolation (issue #194).
    pub fn with_working_dir(mut self, dir: std::path::PathBuf) -> Self {
        self.working_dir = Some(dir);
        self
    }

    /// Attach a ForgeConfig for system runtime configuration.
    pub fn with_config(mut self, config: crate::config::ForgeConfig) -> Self {
        self.config = Some(config);
        self
    }

    /// Attach a skill executor for LLM-mediated skill bridge (issue #40).
    pub fn with_skill_executor(
        mut self,
        executor: Arc<crate::runtime::skill_executor::SkillExecutor>,
    ) -> Self {
        self.skill_executor = Some(executor);
        self
    }

    /// Attach embedding provider and vector index for data.embed/data.search (issue #50).
    pub fn with_embeddings(
        mut self,
        provider: crate::llm::BoxedEmbeddingProvider,
        index: crate::runtime::vector_index::SharedVectorIndex,
    ) -> Self {
        self.embedding_provider = Some(provider);
        self.vector_index = Some(index);
        self
    }

    /// Attach an agent context for agent statement execution.
    pub fn with_agent_context(mut self, ctx: Arc<Mutex<AgentContext>>) -> Self {
        self.agent_context = Some(ctx);
        self
    }

    /// Attach an async timer engine for timer operations (issue #20).
    pub fn with_timer_engine(mut self, engine: Arc<Mutex<TimerEngine>>) -> Self {
        self.timer_engine = Some(engine);
        self
    }

    /// Attach the instance registry for runtime agent discovery (issue #82).
    pub fn with_instance_registry(
        mut self,
        registry: crate::runtime::instance_registry::SharedInstanceRegistry,
    ) -> Self {
        self.instance_registry = Some(registry);
        self
    }

    /// Attach the event bus for inter-agent communication (issue #83).
    pub fn with_event_bus(mut self, bus: crate::runtime::event_bus::SharedEventBus) -> Self {
        if let Some(ref mgr) = self.session_manager {
            mgr.set_event_bus(bus.clone());
        }
        self.event_bus = Some(bus);
        self
    }

    /// Attach storage for `data.store`/`data.get`/`data.list`/`data.delete` capabilities.
    pub fn with_storage(mut self, storage: crate::runtime::storage::SharedStorage) -> Self {
        self.storage = Some(storage);
        self
    }

    /// Get a clone of the storage handle (used to preserve storage across hot-reloads).
    pub fn storage_handle(&self) -> Option<crate::runtime::storage::SharedStorage> {
        self.storage.clone()
    }

    /// Attach a command manager for background process lifecycle (issue #162).
    pub fn with_command_manager(mut self, mgr: SharedCommandManager) -> Self {
        self.command_manager = Some(mgr);
        self
    }

    /// Get a reference to the command manager.
    pub fn command_manager(&self) -> Option<&SharedCommandManager> {
        self.command_manager.as_ref()
    }

    /// Attach a session manager for long-running external agent sessions (issue #190).
    pub fn with_session_manager(mut self, mgr: SharedSessionManager) -> Self {
        if let Some(ref bus) = self.event_bus {
            mgr.set_event_bus(bus.clone());
        }
        self.session_manager = Some(mgr);
        self
    }

    /// Get a reference to the session manager.
    pub fn session_manager(&self) -> Option<&SharedSessionManager> {
        self.session_manager.as_ref()
    }

    /// Configure persistent memory storage (issue #57).
    pub fn with_persistent_memory(
        mut self,
        storage: crate::runtime::storage::SharedStorage,
        agent_name: String,
    ) -> Self {
        self.storage = Some(storage);
        self.agent_name = Some(agent_name);
        self.memory_persistent = true;
        self
    }

    /// Get a reference to the agent context, if set.
    pub fn agent_context(&self) -> Option<&Arc<Mutex<AgentContext>>> {
        self.agent_context.as_ref()
    }

    /// Get collected `say` output (for testing)
    pub fn outputs(&self) -> Vec<String> {
        self.output.lock().unwrap().clone()
    }

    /// Get the tracer, if enabled.
    pub fn tracer(&self) -> Option<&Tracer> {
        self.tracer.as_ref()
    }

    /// Get registered endpoints (for HTTP server).
    pub fn endpoints(&self) -> &HashMap<String, EndpointDecl> {
        &self.endpoint_map
    }

    /// Get the parsed program backing this executor.
    pub fn program(&self) -> &Program {
        &self.program
    }

    /// Get the event bus reference (for webhook wiring).
    pub fn event_bus(&self) -> Option<&crate::runtime::event_bus::SharedEventBus> {
        self.event_bus.as_ref()
    }

    /// Attach a standalone knowledge store for endpoint/webhook context.
    /// Allows `recall` and `learn` to work outside agent handlers.
    pub fn with_knowledge_store(
        mut self,
        ks: crate::runtime::knowledge_store::KnowledgeStore,
    ) -> Self {
        self.knowledge_store = Some(Arc::new(Mutex::new(ks)));
        self
    }

    /// Attach a pre-existing shared knowledge store (already wrapped in Arc<Mutex>).
    /// Used to share the same KS between the executor and agent context (#309).
    pub fn with_shared_knowledge_store_arc(
        mut self,
        ks: crate::runtime::knowledge_store::SharedKnowledgeStore,
    ) -> Self {
        self.knowledge_store = Some(ks);
        self
    }

    /// Get a clone of the shared knowledge store handle, if any.
    pub fn knowledge_store_handle(
        &self,
    ) -> Option<crate::runtime::knowledge_store::SharedKnowledgeStore> {
        self.knowledge_store.clone()
    }

    async fn emit_named_args(
        &self,
        event_name: &str,
        args: &[Spanned<CallArg>],
        env: &mut Env,
    ) -> Result<(), RuntimeError> {
        let mut arg_vals = Vec::new();
        let mut fields = std::collections::HashMap::new();
        for arg in args {
            let val = self.eval_expr(&arg.node.value, env).await?;
            if let Some(ref label) = arg.node.label {
                fields.insert(label.node.clone(), val.clone());
            }
            arg_vals.push(val);
        }
        self.emit_precomputed(event_name, arg_vals, fields).await
    }

    async fn emit_precomputed(
        &self,
        event_name: &str,
        arg_vals: Vec<ConfidentValue>,
        fields: HashMap<String, ConfidentValue>,
    ) -> Result<(), RuntimeError> {
        if let Some(ref ctx_arc) = self.agent_context {
            ctx_arc
                .lock()
                .unwrap()
                .event_sink
                .emitted
                .push(EmittedEvent {
                    name: event_name.to_string(),
                    args: arg_vals,
                    fields,
                });
        } else if let Some(ref bus) = self.event_bus {
            let source = self
                .agent_name
                .clone()
                .unwrap_or_else(|| "endpoint".to_string());
            let payload = crate::runtime::event_bus::EventPayload {
                event_name: event_name.to_string(),
                args: arg_vals,
                source_agent: source.clone(),
                fields,
            };
            let delivered = bus.read().await.publish(&payload);
            if let Some(ref t) = self.tracer {
                t.event_emit(&source, event_name, delivered);
            }
        } else {
            return Err(RuntimeError::Unsupported("emit outside agent".into()));
        }
        Ok(())
    }

    async fn emit_session_hook(
        &self,
        hook: &SessionHook,
        payload: ConfidentValue,
        env: &Env,
    ) -> Result<(), RuntimeError> {
        let mut hook_env = env.clone();
        hook_env.push_scope();
        hook_env.bind("it", payload);
        self.emit_named_args(&hook.event.node, &hook.args, &mut hook_env)
            .await
    }

    fn type_name_to_string(type_name: &TypeName) -> String {
        match type_name {
            TypeName::Text => "Text".to_string(),
            TypeName::Number => "Number".to_string(),
            TypeName::Bool => "Bool".to_string(),
            TypeName::Results => "Results".to_string(),
            TypeName::Report => "Report".to_string(),
            TypeName::Intent => "Intent".to_string(),
            TypeName::Summary => "Summary".to_string(),
            TypeName::Failure => "Failure".to_string(),
            TypeName::Classification => "Classification".to_string(),
            TypeName::Conversation => "Conversation".to_string(),
            TypeName::Profile => "Profile".to_string(),
            TypeName::SearchResults => "SearchResults".to_string(),
            TypeName::Request => "Request".to_string(),
            TypeName::Response => "Response".to_string(),
            TypeName::Headers => "Headers".to_string(),
            TypeName::Html => "Html".to_string(),
            TypeName::AgentResult => "AgentResult".to_string(),
            TypeName::Custom(name) => name.clone(),
            TypeName::Array(inner, Some(size)) => {
                format!("{}[{}]", Self::type_name_to_string(inner), size)
            }
            TypeName::Array(inner, None) => format!("{}[]", Self::type_name_to_string(inner)),
        }
    }

    async fn eval_session_expr(
        &self,
        session: &SessionExpr,
        env: &mut Env,
    ) -> Result<ConfidentValue, RuntimeError> {
        let Some(manager) = self.session_manager.as_ref() else {
            return Err(RuntimeError::Unsupported(
                "session requires a session manager".to_string(),
            ));
        };

        let name = format!("{}", self.eval_expr(&session.name, env).await?.value);
        let agent = match &session.agent {
            Some(agent_expr) => format!("{}", self.eval_expr(agent_expr, env).await?.value),
            None => "default".to_string(),
        };
        let prompt = match &session.prompt {
            Some(prompt_expr) => Some(format!("{}", self.eval_expr(prompt_expr, env).await?.value)),
            None => None,
        };
        let tools = match &session.tools {
            Some(tools_expr) => match self.eval_expr(tools_expr, env).await?.value {
                Value::Array(items) | Value::List(items) => items
                    .into_iter()
                    .map(|item| format!("{}", item.value))
                    .collect(),
                other => {
                    return Err(RuntimeError::TypeError {
                        expected: "Array".to_string(),
                        got: format!("{}", other),
                    })
                }
            },
            None => Vec::new(),
        };
        let budget_usd = match &session.budget {
            Some(budget_expr) => {
                let budget = self.eval_expr(budget_expr, env).await?;
                match budget.value {
                    Value::Number(n) => Some(n as f32),
                    other => {
                        return Err(RuntimeError::TypeError {
                            expected: "Number".to_string(),
                            got: format!("{}", other),
                        })
                    }
                }
            }
            None => None,
        };

        let mut config = SessionConfig::new(name, agent);
        config.prompt = prompt;
        config.tools = tools;
        config.timeout_secs = session
            .timeout
            .as_ref()
            .map(|dur| dur.node.to_std().as_secs());
        config.budget_usd = budget_usd;
        config.gives = session
            .gives
            .as_ref()
            .map(|gives| Self::type_name_to_string(&gives.node));

        // Evaluate isolate modifier — create worktree for session (issue #194)
        let mut worktree_branch: Option<String> = None;
        if let Some(ref iso) = session.isolate {
            let branch_val = self.eval_expr(&iso.branch, env).await?;
            let branch = format!("{}", branch_val.value);
            let dir = crate::runtime::sandbox::create_worktree(&branch)
                .map_err(|e| RuntimeError::FlowError(format!("session isolate: {}", e)))?;
            config.working_dir = Some(dir.to_string_lossy().to_string());
            worktree_branch = Some(branch);
        }

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<SessionEvent>();
        let listener: SessionListener = Arc::new(move |event| {
            let _ = tx.send(event);
        });

        let run_fut = manager.run_to_completion(config, vec![listener]);
        tokio::pin!(run_fut);

        let result = loop {
            tokio::select! {
                state = &mut run_fut => {
                    let state = state.map_err(RuntimeError::FlowError)?;
                    let output = state.output.unwrap_or_else(ConfidentValue::default_agent_result);
                    while let Ok(event) = rx.try_recv() {
                        if let SessionEvent::Progress { payload, .. } = event {
                            if let Some(ref hook) = session.on_progress {
                                self.emit_session_hook(&hook.node, payload, env).await?;
                            }
                        }
                    }
                    if let Some(ref hook) = session.on_complete {
                        self.emit_session_hook(&hook.node, output.clone(), env).await?;
                    }
                    break if matches!(session.gives.as_ref().map(|g| &g.node), Some(TypeName::AgentResult)) {
                        self.coerce_session_result_to_agent_result(output, state.cost_usd)
                    } else {
                        self.coerce_session_result_to_text(output)
                    };
                }
                event = rx.recv() => {
                    let Some(event) = event else {
                        continue;
                    };
                    if let SessionEvent::Progress { payload, .. } = event {
                        if let Some(ref hook) = session.on_progress {
                            self.emit_session_hook(&hook.node, payload, env).await?;
                        }
                    }
                }
            }
        };

        // Cleanup worktree after session completes (issue #194)
        if let Some(branch) = worktree_branch {
            let _ = crate::runtime::sandbox::remove_worktree(&branch);
        }

        Ok(result)
    }

    /// Check if a session result passes the verification gate for the given risk level.
    /// Returns Ok(()) if the action is allowed, or an error describing why it's blocked.
    /// Backward-compatible: allows everything when no verification engine is configured
    /// or when verification hasn't run yet (Pending status). Issue #205.
    pub fn check_verification_gate(
        result: &ConfidentValue,
        max_risk: RiskClass,
    ) -> Result<(), RuntimeError> {
        let vr = match Self::extract_verification_from_result(result) {
            Some(vr) => vr,
            None => return Ok(()), // No verification metadata — allow (backward compat)
        };

        match vr.status {
            VerificationStatus::Pending => Ok(()), // Not yet checked — allow
            VerificationStatus::Verified => {
                if vr.is_actionable(max_risk) {
                    Ok(())
                } else {
                    Err(RuntimeError::FlowError(format!(
                        "verification gate: result verified but risk class {:?} exceeds allowed {:?}",
                        vr.risk_class, max_risk
                    )))
                }
            }
            VerificationStatus::Contradicted => Err(RuntimeError::FlowError(format!(
                "verification gate: {} contradiction(s) detected — blocking {:?} action",
                vr.contradictions.len(),
                max_risk
            ))),
            VerificationStatus::Insufficient => {
                if max_risk >= RiskClass::ExternalSideEffect {
                    Err(RuntimeError::FlowError(
                        "verification gate: insufficient evidence for external side-effect action"
                            .to_string(),
                    ))
                } else {
                    Ok(()) // Allow lower-risk actions with insufficient evidence
                }
            }
            VerificationStatus::Error => Err(RuntimeError::FlowError(
                "verification gate: verification engine error — cannot authorize action"
                    .to_string(),
            )),
        }
    }

    /// Extract a VerificationResult from an AgentResult's metadata.verification field.
    fn extract_verification_from_result(result: &ConfidentValue) -> Option<VerificationResult> {
        let fields = match &result.value {
            Value::Record(f) => f,
            _ => return None,
        };
        let meta = match fields.get("metadata") {
            Some(cv) => match &cv.value {
                Value::Record(m) => m,
                _ => return None,
            },
            None => return None,
        };
        meta.get("verification")
            .and_then(|cv| VerificationResult::from_value(&cv.value))
    }

    fn coerce_session_result_to_text(&self, result: ConfidentValue) -> ConfidentValue {
        match result.value {
            Value::Text(_) | Value::Html(_) => result,
            other => {
                ConfidentValue::from_skill(Value::Text(format!("{}", other)), result.confidence)
            }
        }
    }

    fn coerce_session_result_to_agent_result(
        &self,
        result: ConfidentValue,
        total_cost_usd: f32,
    ) -> ConfidentValue {
        match result.value {
            Value::Record(mut fields) => {
                if fields.contains_key("cost_usd") {
                    fields.insert(
                        "cost_usd".to_string(),
                        ConfidentValue::deterministic(Value::Number(total_cost_usd as f64)),
                    );
                    if !fields.contains_key("confidence") {
                        fields.insert(
                            "confidence".to_string(),
                            ConfidentValue::deterministic(Value::Number(result.confidence as f64)),
                        );
                    }
                    ConfidentValue::from_agent_result(fields)
                } else {
                    let record_text = format!("{}", Value::Record(fields));
                    let mut defaults = ConfidentValue::default_agent_result_fields();
                    defaults.insert(
                        "plan".to_string(),
                        ConfidentValue::from_skill(
                            Value::Text(record_text.clone()),
                            result.confidence,
                        ),
                    );
                    defaults.insert(
                        "patch_summary".to_string(),
                        ConfidentValue::from_skill(Value::Text(record_text), result.confidence),
                    );
                    defaults.insert(
                        "cost_usd".to_string(),
                        ConfidentValue::deterministic(Value::Number(total_cost_usd as f64)),
                    );
                    defaults.insert(
                        "confidence".to_string(),
                        ConfidentValue::deterministic(Value::Number(result.confidence as f64)),
                    );
                    ConfidentValue::from_agent_result(defaults)
                }
            }
            other => {
                let text = format!("{}", other);
                let mut fields = ConfidentValue::default_agent_result_fields();
                fields.insert(
                    "plan".to_string(),
                    ConfidentValue::from_skill(Value::Text(text.clone()), result.confidence),
                );
                fields.insert(
                    "patch_summary".to_string(),
                    ConfidentValue::from_skill(Value::Text(text), result.confidence),
                );
                fields.insert(
                    "cost_usd".to_string(),
                    ConfidentValue::deterministic(Value::Number(total_cost_usd as f64)),
                );
                fields.insert(
                    "confidence".to_string(),
                    ConfidentValue::deterministic(Value::Number(result.confidence as f64)),
                );
                ConfidentValue::from_agent_result(fields)
            }
        }
    }

    /// Check if an OutputType includes Html.
    fn returns_html_output(gives: &Option<Spanned<OutputType>>) -> bool {
        gives.as_ref().is_some_and(|ot| {
            ot.node
                .types
                .iter()
                .any(|t| matches!(&t.node, TypeName::Html))
        })
    }

    /// Execute an endpoint body with the given arguments and optional request context.
    pub async fn exec_endpoint(
        &self,
        name: &str,
        args: HashMap<String, ConfidentValue>,
        request: Option<ConfidentValue>,
    ) -> Result<EndpointResult, RuntimeError> {
        let endpoint = self
            .endpoint_map
            .get(name)
            .ok_or_else(|| RuntimeError::NotCallable {
                name: name.to_string(),
            })?;

        // Derive default content-type from return type annotation
        let default_ct = endpoint.return_type.as_ref().and_then(|ot| {
            ot.node.types.first().and_then(|tn| match &tn.node {
                crate::ast::TypeName::Html => Some("text/html".to_string()),
                crate::ast::TypeName::Text => Some("text/plain".to_string()),
                _ => None,
            })
        });

        let mut env = Env::new();
        if let Some(req) = request {
            env.bind("request", req);
        }
        for (k, v) in args {
            env.bind(&k, v);
        }

        // Set html_context if this endpoint returns Html
        let prev_html = self.html_context.load(Ordering::Relaxed);
        if Self::returns_html_output(&endpoint.return_type) {
            self.html_context.store(true, Ordering::Relaxed);
        }

        let result = match self.exec_stmts(&endpoint.body, &mut env).await {
            Ok(val) => Ok(EndpointResult {
                value: val,
                status: None,
                content_type: None,
                default_content_type: default_ct,
            }),
            Err(RuntimeError::GiveSignal(val, status, content_type)) => Ok(EndpointResult {
                value: val,
                status,
                content_type,
                default_content_type: default_ct,
            }),
            Err(e) => Err(e),
        };

        self.html_context.store(prev_html, Ordering::Relaxed);
        result
    }

    /// Run the program starting from `fn main`, or from a `system` declaration
    /// if no `fn main` is present.
    pub async fn run(&self) -> Result<ConfidentValue, RuntimeError> {
        // Try fn main first
        let main_decl = self.program.items.iter().find_map(|item| match &item.node {
            TopLevel::FnMain(m) => Some(m),
            _ => None,
        });

        if let Some(main_decl) = main_decl {
            let mut env = Env::new();
            return match self.exec_stmts(&main_decl.body, &mut env).await {
                Ok(val) => Ok(val),
                Err(RuntimeError::GiveSignal(val, ..)) => Ok(val),
                Err(RuntimeError::RetireSignal) => Ok(ConfidentValue::deterministic(Value::Unit)),
                Err(e) => Err(e),
            };
        }

        // Fall back to system declaration
        let system_decl = self.program.items.iter().find_map(|item| match &item.node {
            TopLevel::System(s) => Some(s),
            _ => None,
        });

        if let Some(system_decl) = system_decl {
            let system_config = self.config.as_ref().and_then(|c| c.system.as_ref());
            let system_runtime = crate::runtime::system::SystemRuntime::new(
                system_decl,
                &self.program,
                self.providers.clone(),
                self.tracer.clone(),
                system_config,
            )?;
            system_runtime.start().await?;
            return Ok(ConfidentValue::deterministic(Value::Unit));
        }

        Err(RuntimeError::NoMainFunction)
    }

    /// Extract a static topology snapshot from the compiled system declaration
    /// without starting the system runtime. Returns `None` if no system is declared.
    pub fn extract_topology(&self) -> Option<crate::runtime::system::TopologySnapshot> {
        self.build_system_runtime()
            .ok()
            .flatten()
            .map(|rt| rt.topology_snapshot())
    }

    /// Build a SystemRuntime from the program's system declaration without
    /// starting it. Returns `None` if no system is declared.
    pub fn build_system_runtime(
        &self,
    ) -> Result<Option<crate::runtime::system::SystemRuntime>, RuntimeError> {
        let system_decl = self.program.items.iter().find_map(|item| match &item.node {
            TopLevel::System(s) => Some(s),
            _ => None,
        });
        match system_decl {
            Some(decl) => {
                let system_config = self.config.as_ref().and_then(|c| c.system.as_ref());
                let mut runtime = crate::runtime::system::SystemRuntime::new_with_skills(
                    decl,
                    &self.program,
                    self.providers.clone(),
                    self.tracer.clone(),
                    system_config,
                    self.skill_executor.clone(),
                )?;
                // Share the executor's knowledge store with the system runtime so
                // agents use the same instance as endpoint recall (#309).
                if let Some(ref ks) = self.knowledge_store {
                    runtime = runtime.with_shared_knowledge_store(ks.clone());
                }
                Ok(Some(runtime))
            }
            None => Ok(None),
        }
    }

    // ── Statement execution ───────────────────────────────────────────────────

    /// Execute a list of statements. `give` propagates as `GiveSignal` error
    /// so it can cross if/else/match/for boundaries. Callers at function
    /// boundaries (call_task, call_pure, call_flow, run) must catch it.
    pub(crate) fn exec_stmts<'a>(
        &'a self,
        stmts: &'a [Spanned<Stmt>],
        env: &'a mut Env,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<ConfidentValue, RuntimeError>> + Send + 'a>,
    > {
        Box::pin(async move {
            for stmt in stmts {
                self.exec_stmt(stmt, env).await?;
            }
            Ok(ConfidentValue::deterministic(Value::Unit))
        })
    }

    fn exec_stmt<'a>(
        &'a self,
        stmt: &'a Spanned<Stmt>,
        env: &'a mut Env,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), RuntimeError>> + Send + 'a>>
    {
        Box::pin(async move {
            match &stmt.node {
                Stmt::Bind(name, expr) => {
                    let val = self.eval_expr(expr, env).await?;
                    env.bind(&name.node, val);
                }

                Stmt::Say(expr) => {
                    let val = self.eval_expr(expr, env).await?;
                    let text = format!("{}", val.value);
                    if let Some(ref tracer) = self.tracer {
                        tracer.say(&text);
                    }
                    println!("{}", text);
                    self.output.lock().unwrap().push(text);
                }

                Stmt::Give(expr, metas) => {
                    let val = self.eval_expr(expr, env).await?;
                    let mut status: Option<u16> = None;
                    let mut content_type: Option<String> = None;
                    for meta in metas {
                        let meta_val = self.eval_expr(&meta.node.value, env).await?;
                        match meta.node.key.node.as_str() {
                            "status" => {
                                if let Value::Number(n) = &meta_val.value {
                                    status = Some(*n as u16);
                                }
                            }
                            "content_type" => {
                                if let Value::Text(s) | Value::Html(s) = &meta_val.value {
                                    content_type = Some(s.clone());
                                }
                            }
                            _ => {} // ignore unknown keys
                        }
                    }
                    return Err(RuntimeError::GiveSignal(val, status, content_type));
                }

                Stmt::ExprStmt(expr) => {
                    self.eval_expr(expr, env).await?;
                }

                // ── Control flow ──────────────────────────────────────────────
                Stmt::When(when_block) => {
                    for clause in &when_block.clauses {
                        let pred = &clause.node.predicate.node;
                        let subject_val = env.lookup(&pred.subject.node)?;
                        let matches = match &pred.level.node {
                            ConfLevel::Sure(None) => subject_val.sure(),
                            ConfLevel::Sure(Some(t)) => subject_val.sure_above(*t as f32),
                            ConfLevel::Unsure => subject_val.unsure(),
                            ConfLevel::Unreliable => subject_val.unreliable(),
                            ConfLevel::Conflicted => subject_val.conflicted(),
                        };

                        if let Some(ref tracer) = self.tracer {
                            tracer.when_dispatch(
                                &pred.subject.node,
                                &format!("{:?}", pred.level.node),
                                matches,
                            );
                        }

                        if matches {
                            self.exec_stmt(&clause.node.body, env).await?;
                            return Ok(());
                        }
                    }
                    if let Some(else_clause) = &when_block.else_body {
                        if let Some(ref tracer) = self.tracer {
                            tracer.when_dispatch("_", "else", true);
                        }
                        self.exec_stmt(&else_clause.node.body, env).await?;
                    }
                }

                Stmt::IfElse(block) => {
                    let cond = self.eval_expr(&block.condition, env).await?;
                    if truthy(&cond) {
                        self.exec_stmts(&block.then_body, env).await?;
                    } else {
                        let mut handled = false;
                        for (cond_expr, body) in &block.else_ifs {
                            let c = self.eval_expr(cond_expr, env).await?;
                            if truthy(&c) {
                                self.exec_stmts(body, env).await?;
                                handled = true;
                                break;
                            }
                        }
                        if !handled {
                            if let Some(body) = &block.else_body {
                                self.exec_stmts(body, env).await?;
                            }
                        }
                    }
                }

                Stmt::Match(block) => {
                    let subject = self.eval_expr(&block.subject, env).await?;
                    for arm in &block.arms {
                        if let Some(bindings) = match_pattern(&arm.node.pattern, &subject) {
                            env.push_scope();
                            for (name, val) in bindings {
                                env.bind(&name, val);
                            }
                            self.exec_stmt(&arm.node.body, env).await?;
                            env.pop_scope();
                            return Ok(());
                        }
                    }
                }

                Stmt::For(for_loop) => {
                    let iterable = self.eval_expr(&for_loop.iterable, env).await?;
                    let items = match &iterable.value {
                        Value::Array(v) | Value::List(v) => v.clone(),
                        other => {
                            return Err(RuntimeError::TypeError {
                                expected: "Array or List".to_string(),
                                got: format!("{}", other),
                            })
                        }
                    };
                    for item in items {
                        env.push_scope();
                        env.bind(&for_loop.binding.node, item);
                        match self.exec_stmts(&for_loop.body, env).await {
                            Ok(_) => {
                                env.pop_scope();
                            }
                            Err(e) => {
                                env.pop_scope();
                                return Err(e);
                            }
                        }
                    }
                }

                // ── Agent features (issue #11) ─────────────────────────────
                Stmt::Emit(name, args) => {
                    self.emit_named_args(&name.node, args, env).await?;
                }
                Stmt::TransitionTo(state) => {
                    if let Some(ref ctx_arc) = self.agent_context {
                        let mut ctx = ctx_arc.lock().unwrap();
                        if let Some(ref mut sm) = ctx.state_machine {
                            sm.transition(&state.node)
                                .map_err(|e| RuntimeError::FlowError(e.to_string()))?;
                        } else {
                            return Err(RuntimeError::Unsupported(
                                "transition without lifecycle".into(),
                            ));
                        }
                    } else {
                        return Err(RuntimeError::Unsupported("transition outside agent".into()));
                    }
                }
                Stmt::StartTimer { name, context } => {
                    let ctx_val = match context {
                        Some(expr) => Some(self.eval_expr(expr, env).await?),
                        None => None,
                    };
                    if let Some(ref ctx_arc) = self.agent_context {
                        ctx_arc.lock().unwrap().timer_manager.start(&name.node)?;
                    } else {
                        return Err(RuntimeError::Unsupported(
                            "start timer outside agent".into(),
                        ));
                    }
                    if let Some(ref engine) = self.timer_engine {
                        engine
                            .lock()
                            .unwrap()
                            .start(&name.node, ctx_val)
                            .map_err(|e| RuntimeError::Unsupported(e.to_string()))?;
                    }
                }
                Stmt::CancelTimer { name, context } => {
                    let ctx_val = match context {
                        Some(expr) => Some(self.eval_expr(expr, env).await?),
                        None => None,
                    };
                    if let Some(ref ctx_arc) = self.agent_context {
                        ctx_arc.lock().unwrap().timer_manager.cancel(&name.node)?;
                    } else {
                        return Err(RuntimeError::Unsupported(
                            "cancel timer outside agent".into(),
                        ));
                    }
                    if let Some(ref engine) = self.timer_engine {
                        engine
                            .lock()
                            .unwrap()
                            .cancel(&name.node, &ctx_val)
                            .map_err(|e| RuntimeError::Unsupported(e.to_string()))?;
                    }
                }
                Stmt::ResetTimer(name) => {
                    if let Some(ref ctx_arc) = self.agent_context {
                        ctx_arc.lock().unwrap().timer_manager.reset(&name.node)?;
                    } else {
                        return Err(RuntimeError::Unsupported(
                            "reset timer outside agent".into(),
                        ));
                    }
                    if let Some(ref engine) = self.timer_engine {
                        engine
                            .lock()
                            .unwrap()
                            .reset(&name.node, None)
                            .map_err(|e| RuntimeError::Unsupported(e.to_string()))?;
                    }
                }
                Stmt::Forward(expr, target) => {
                    if let Some(ref ctx_arc) = self.agent_context {
                        let val = self.eval_expr(expr, env).await?;
                        let tgt = self.eval_expr(target, env).await?;
                        ctx_arc.lock().unwrap().event_sink.forwards.push((val, tgt));
                    } else {
                        return Err(RuntimeError::Unsupported("forward outside agent".into()));
                    }
                }
                Stmt::MemoryUpdate(field, idx, expr) => {
                    if let Some(ref ctx_arc) = self.agent_context {
                        let val = self.eval_expr(expr, env).await?;
                        if let Some(idx_expr) = idx {
                            // Array index update: memory.field[idx] = val
                            let idx_val = self.eval_expr(idx_expr, env).await?;
                            let i = match &idx_val.value {
                                Value::Number(n) => *n as usize,
                                _ => {
                                    return Err(RuntimeError::TypeError {
                                        expected: "Number".into(),
                                        got: format!("{}", idx_val.value),
                                    })
                                }
                            };
                            let mut ctx = ctx_arc.lock().unwrap();
                            if let Some(arr_val) = ctx.memory.get(&field.node).cloned() {
                                match arr_val.value {
                                    Value::Array(mut items) => {
                                        if i >= items.len() {
                                            return Err(RuntimeError::IndexOutOfBounds {
                                                index: i,
                                                len: items.len(),
                                            });
                                        }
                                        items[i] = val;
                                        ctx.memory.set(
                                            &field.node,
                                            ConfidentValue::deterministic(Value::Array(items)),
                                        );
                                    }
                                    _ => {
                                        return Err(RuntimeError::TypeError {
                                            expected: "Array".into(),
                                            got: format!("{}", arr_val.value),
                                        })
                                    }
                                }
                            }
                        } else {
                            ctx_arc.lock().unwrap().memory.set(&field.node, val);
                        }
                        // Re-bind memory in env so subsequent reads see the update
                        let ctx = ctx_arc.lock().unwrap();
                        env.bind(
                            "memory",
                            ConfidentValue::deterministic(ctx.memory.to_record()),
                        );
                        // Write-through for persistent memory (issue #57)
                        if self.memory_persistent {
                            if let (Some(ref storage), Some(ref name)) =
                                (&self.storage, &self.agent_name)
                            {
                                let key = format!("agent:{}:memory", name);
                                if let Ok(json) = ctx.memory.to_json() {
                                    let _ = storage.store(&key, &json);
                                }
                            }
                        }
                    } else {
                        return Err(RuntimeError::Unsupported(
                            "memory update outside agent".into(),
                        ));
                    }
                }
                Stmt::Escalate(target) => {
                    if let Some(ref ctx_arc) = self.agent_context {
                        ctx_arc
                            .lock()
                            .unwrap()
                            .event_sink
                            .escalations
                            .push(target.node.clone());
                    } else {
                        return Err(RuntimeError::Unsupported("escalate outside agent".into()));
                    }
                }
                Stmt::Learn(source, category_expr) => {
                    if let Some(ref ctx_arc) = self.agent_context {
                        // Get shared knowledge store handle (clone Arc, release ctx lock)
                        let ks_arc = {
                            let ctx = ctx_arc.lock().unwrap();
                            ctx.knowledge_store.clone()
                        };
                        let ks_arc = ks_arc.ok_or_else(|| {
                            RuntimeError::Unsupported(
                                "learn requires agent with knowledge store".into(),
                            )
                        })?;

                        // Evaluate optional category expression
                        let category = if let Some(cat_expr) = category_expr {
                            let val = self.eval_expr(cat_expr, env).await?;
                            Some(format!("{}", val.value))
                        } else {
                            None
                        };

                        match &source.node {
                            LearnSource::Direct(expr) => {
                                let val = self.eval_expr(expr, env).await?;
                                let text = format!("{}", val.value);
                                let mut ks = ks_arc.lock().unwrap();
                                if let Some(ref cat) = category {
                                    ks.learn_direct_categorized(&text, cat);
                                } else {
                                    ks.learn_direct(&text);
                                }
                            }
                            LearnSource::FromInteraction(args) => {
                                let mut arg_vals = Vec::new();
                                for arg in args {
                                    let val = self.eval_expr(&arg.node.value, env).await?;
                                    arg_vals.push(val);
                                }
                                let question = arg_vals
                                    .first()
                                    .map(|v| format!("{}", v.value))
                                    .unwrap_or_default();
                                let answer = arg_vals
                                    .get(1)
                                    .map(|v| format!("{}", v.value))
                                    .unwrap_or_default();
                                let confidence = arg_vals
                                    .get(2)
                                    .and_then(|v| match &v.value {
                                        Value::Number(n) => Some(*n as f32),
                                        _ => None,
                                    })
                                    .unwrap_or(0.5);
                                let mut ks = ks_arc.lock().unwrap();
                                if let Some(ref cat) = category {
                                    ks.learn_from_interaction_categorized(
                                        &question, &answer, confidence, cat,
                                    );
                                } else {
                                    ks.learn_from_interaction(&question, &answer, confidence);
                                }
                            }
                            LearnSource::FromDocument(expr) => {
                                let val = self.eval_expr(expr, env).await?;
                                let path = format!("{}", val.value);
                                let mut ks = ks_arc.lock().unwrap();
                                if let Some(ref cat) = category {
                                    ks.learn_from_document_categorized(&path, cat)
                                        .map_err(RuntimeError::Unsupported)?;
                                } else {
                                    ks.learn_from_document(&path)
                                        .map_err(RuntimeError::Unsupported)?;
                                }
                            }
                        }
                    } else {
                        return Err(RuntimeError::Unsupported("learn outside agent".into()));
                    }
                }

                Stmt::Spawn(spawn) => {
                    // 1. Find agent declaration by template name
                    let template_name = &spawn.template.node;
                    let mut agent_decl: Option<AgentDecl> = None;
                    let mut states_decl: Option<StatesDecl> = None;

                    for item in &self.program.items {
                        match &item.node {
                            TopLevel::Agent(a) if a.name.node == *template_name => {
                                agent_decl = Some(a.as_ref().clone());
                            }
                            TopLevel::States(s) if states_decl.is_none() => {
                                // Collect states decls for lifecycle lookup
                                if let Some(ref ad) = agent_decl {
                                    if ad.lifecycle.as_ref().map(|l| &l.node) == Some(&s.name.node)
                                    {
                                        states_decl = Some(s.clone());
                                    }
                                }
                            }
                            _ => {}
                        }
                    }

                    let agent_decl = agent_decl.ok_or_else(|| {
                        RuntimeError::Unsupported(format!(
                            "spawn: no agent declaration found for '{}'",
                            template_name
                        ))
                    })?;

                    // Re-scan for states if we found the agent after states were processed
                    if states_decl.is_none() {
                        if let Some(ref lc) = agent_decl.lifecycle {
                            for item in &self.program.items {
                                if let TopLevel::States(s) = &item.node {
                                    if s.name.node == lc.node {
                                        states_decl = Some(s.clone());
                                        break;
                                    }
                                }
                            }
                        }
                    }

                    // 2. Evaluate alias (for registry name)
                    let instance_name = if let Some(ref alias_expr) = spawn.alias {
                        let val = self.eval_expr(alias_expr, env).await?;
                        format!("{}", val.value)
                    } else {
                        template_name.clone()
                    };

                    // 3. Process spawn options
                    let mut knowledge_category: Option<String> = None;
                    let mut confidence_cap: Option<f32> = None;
                    let mut memory_inits: Vec<(String, ConfidentValue)> = Vec::new();
                    let mut isolate_branch: Option<String> = None;
                    let mut isolate_dir: Option<std::path::PathBuf> = None;

                    for opt in &spawn.options {
                        match &opt.node {
                            SpawnOption::KnowledgeFilter(cat) => {
                                knowledge_category = Some(cat.node.clone());
                            }
                            SpawnOption::ConfidenceCap(expr) => {
                                let val = self.eval_expr(expr, env).await?;
                                match &val.value {
                                    Value::Number(n) => {
                                        confidence_cap = Some(*n as f32);
                                    }
                                    _ => {
                                        return Err(RuntimeError::TypeError {
                                            expected: "Number".to_string(),
                                            got: format!("{:?}", val.value),
                                        });
                                    }
                                }
                            }
                            SpawnOption::MemoryInit(field, expr) => {
                                let val = self.eval_expr(expr, env).await?;
                                memory_inits.push((field.node.clone(), val));
                            }
                            SpawnOption::Isolate(iso) => {
                                let branch_val = self.eval_expr(&iso.branch, env).await?;
                                let branch = format!("{}", branch_val.value);
                                let dir = crate::runtime::sandbox::create_worktree(&branch)
                                    .map_err(|e| {
                                        RuntimeError::FlowError(format!("isolate: {}", e))
                                    })?;
                                isolate_branch = Some(branch);
                                isolate_dir = Some(dir);
                            }
                        }
                    }

                    // 4. Create the child agent process (child gets its own KS, not shared)
                    let mut child_process = crate::runtime::agent::AgentProcess::new(
                        agent_decl,
                        states_decl.as_ref(),
                        self.providers.clone(),
                        self.tracer.clone(),
                        self.program.clone(),
                        self.storage.clone(),
                        self.instance_registry.clone(),
                        None,
                    );

                    // 5. Transfer knowledge from parent to child
                    if let Some(ref cat) = knowledge_category {
                        // Get filtered entries from parent's knowledge store
                        let mut transferred_entries = Vec::new();
                        if let Some(ref ctx_arc) = self.agent_context {
                            let ks_arc = ctx_arc.lock().unwrap().knowledge_store.clone();
                            if let Some(ref ks_arc) = ks_arc {
                                let ks = ks_arc.lock().unwrap();
                                transferred_entries = ks.export_by_category(cat);
                            }
                        }

                        // Apply confidence cap (Principle I — Honesty)
                        let cap = confidence_cap.unwrap_or(1.0);
                        for entry in &mut transferred_entries {
                            entry.confidence = entry.confidence.min(cap);
                            entry.source =
                                crate::runtime::knowledge_store::KnowledgeSource::AgentTransfer {
                                    source_agent: self
                                        .agent_name
                                        .clone()
                                        .unwrap_or_else(|| "unknown".to_string()),
                                };
                        }

                        // Merge into child's knowledge store
                        if !transferred_entries.is_empty() {
                            let child_ctx = child_process.context();
                            let child_ks = child_ctx.lock().unwrap().knowledge_store.clone();
                            if let Some(ref ks_arc) = child_ks {
                                let mut ks = ks_arc.lock().unwrap();
                                ks.merge_imported(transferred_entries);
                            }
                        }
                    }

                    // 6. Set initial memory values
                    for (field, val) in memory_inits {
                        let child_ctx = child_process.context();
                        let mut ctx = child_ctx.lock().unwrap();
                        ctx.memory.set(&field, val);
                    }

                    // 7. Apply sandbox isolation (issue #194)
                    if let Some(dir) = isolate_dir {
                        child_process = child_process.with_working_dir(dir);
                    }
                    if let Some(ref branch) = isolate_branch {
                        child_process = child_process.with_worktree_branch(branch.clone());
                    }

                    // 8. Wire event bus
                    if let Some(ref bus) = self.event_bus {
                        child_process = child_process.with_event_bus(bus.clone()).await;
                    }

                    // 9. Register in instance registry
                    let alias = if instance_name != *template_name {
                        Some(instance_name.as_str())
                    } else {
                        None
                    };
                    let instance_id = if let Some(ref ir) = self.instance_registry {
                        let id = ir.write().await.register_with_worktree(
                            template_name,
                            alias,
                            isolate_branch.clone(),
                        );
                        Some(id)
                    } else {
                        None
                    };

                    // 9. Spawn child process.
                    //
                    // When called from inside an agent handler (self.agent_context.is_some()),
                    // the child runs as a background tokio task — the parent agent stays alive
                    // and continues handling its own events.
                    //
                    // When called from `fn main` (self.agent_context.is_none()), there is no
                    // long-lived parent. Fire-and-forget would tear the child down as soon as
                    // fn main returns. So in that case we run the child inline and await it,
                    // letting fn main act as a supervisor that waits for spawned agents to
                    // retire (issue #273 — composition of fn main + spawn + on start).
                    if self.agent_context.is_some() {
                        tokio::spawn(async move {
                            let _ = child_process.run().await;
                        });
                    } else {
                        // Run inline so fn main waits for the child to retire.
                        // Errors are surfaced (unlike background spawn which discards them).
                        child_process.run().await?;
                    }

                    // 10. Bind instance UUID to variable if requested
                    if let Some(ref binding) = spawn.binding {
                        let uuid_str = instance_id
                            .map(|id| id.to_string())
                            .unwrap_or_else(|| "no-registry".to_string());
                        env.bind(
                            &binding.node,
                            ConfidentValue::deterministic(Value::Text(uuid_str)),
                        );
                    }
                }

                Stmt::Retire(retire) => {
                    // 1. Resolve target
                    let target_alias = if let Some(ref target_expr) = retire.target {
                        let val = self.eval_expr(target_expr, env).await?;
                        Some(format!("{}", val.value))
                    } else {
                        None
                    };

                    // 2. Export knowledge if requested (Principle VIII — Accountability)
                    if let Some(ref export_path_expr) = retire.knowledge_export {
                        let path_val = self.eval_expr(export_path_expr, env).await?;
                        let export_path = format!("{}", path_val.value);

                        if let Some(ref ctx_arc) = self.agent_context {
                            let ks_arc = ctx_arc.lock().unwrap().knowledge_store.clone();
                            if let Some(ref ks_arc) = ks_arc {
                                let entries = ks_arc.lock().unwrap().export_entries();
                                let agent_name = self
                                    .agent_name
                                    .clone()
                                    .unwrap_or_else(|| "unknown".to_string());

                                let schema = crate::portability::AgentSchema {
                                    fields: Vec::new(),
                                    knowledge_config: None,
                                };

                                let pkg = crate::portability::build_package(
                                    &agent_name,
                                    &agent_name,
                                    None,
                                    schema,
                                    entries,
                                );

                                let json = serde_json::to_string_pretty(&pkg).map_err(|e| {
                                    RuntimeError::FlowError(format!(
                                        "retire: failed to serialize knowledge: {}",
                                        e
                                    ))
                                })?;

                                std::fs::write(&export_path, json).map_err(|e| {
                                    RuntimeError::FlowError(format!(
                                        "retire: failed to write {}: {}",
                                        export_path, e
                                    ))
                                })?;
                            }
                        }
                    }

                    // 3. Cleanup worktree + unregister from instance registry
                    if let Some(ref ir) = self.instance_registry {
                        let mut registry = ir.write().await;
                        if let Some(ref alias) = target_alias {
                            // Retire a specific instance by alias
                            if let Some(info) = registry.find_by_alias(alias) {
                                // Clean up worktree before unregister (issue #194)
                                if let Some(ref branch) = info.worktree_branch {
                                    let _ = crate::runtime::sandbox::remove_worktree(branch);
                                }
                                registry.unregister(&info.instance_id);
                            }
                        }
                        // If no target, self-retirement — unregister happens via
                        // the RetireSignal propagation in the agent event loop.
                        // Worktree cleanup for self-retire happens in AgentProcess::run() exit.
                    }

                    // 4. Signal termination
                    return Err(RuntimeError::RetireSignal);
                }
            }
            Ok(())
        })
    }

    // ── Expression evaluation ─────────────────────────────────────────────────

    pub(crate) fn eval_expr<'a>(
        &'a self,
        expr: &'a Spanned<Expr>,
        env: &'a mut Env,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<ConfidentValue, RuntimeError>> + Send + 'a>,
    > {
        Box::pin(async move {
            match &expr.node {
                Expr::NumberLit(n) => Ok(ConfidentValue::deterministic(Value::Number(*n))),

                Expr::BoolLit(b) => Ok(ConfidentValue::deterministic(Value::Bool(*b))),

                Expr::Ident(name) => {
                    match env.lookup(name) {
                        Ok(val) => Ok(val.clone()),
                        Err(_) if name.starts_with(|c: char| c.is_uppercase()) => {
                            // Uppercase unbound identifiers are type variants (e.g., Continue, Draw)
                            Ok(ConfidentValue::deterministic(Value::Text(name.clone())))
                        }
                        Err(e) => Err(e),
                    }
                }

                Expr::Template(parts) => {
                    let is_html = self.html_context.load(Ordering::Relaxed);
                    let mut result = String::new();
                    let mut min_conf: f32 = 1.0;
                    for part in parts {
                        match &part.node {
                            TemplatePart::Text(s) => result.push_str(s),
                            TemplatePart::Interp(inner_expr) => {
                                let val = self.eval_expr(inner_expr, env).await?;
                                min_conf = min_conf.min(val.confidence);
                                if is_html {
                                    // In Html context: Html values pass through raw,
                                    // everything else gets escaped for XSS prevention.
                                    match &val.value {
                                        Value::Html(s) => result.push_str(s),
                                        other => {
                                            let escaped = crate::runtime::html::html_escape(
                                                &format!("{}", other),
                                            );
                                            result.push_str(&escaped);
                                        }
                                    }
                                } else {
                                    result.push_str(&format!("{}", val.value));
                                }
                            }
                            TemplatePart::RawInterp(inner_expr) => {
                                // {!expr} — always raw, no escaping even in Html context
                                let val = self.eval_expr(inner_expr, env).await?;
                                min_conf = min_conf.min(val.confidence);
                                result.push_str(&format!("{}", val.value));
                            }
                        }
                    }
                    let value = if is_html {
                        Value::Html(result)
                    } else {
                        Value::Text(result)
                    };
                    if min_conf >= 1.0 {
                        Ok(ConfidentValue::deterministic(value))
                    } else {
                        Ok(ConfidentValue::derived(value, min_conf))
                    }
                }

                Expr::ArrayLit(elems) => {
                    let mut items = Vec::new();
                    let mut min_conf: f32 = 1.0;
                    for elem in elems {
                        let val = self.eval_expr(elem, env).await?;
                        min_conf = min_conf.min(val.confidence);
                        items.push(val);
                    }
                    if min_conf >= 1.0 {
                        Ok(ConfidentValue::deterministic(Value::Array(items)))
                    } else {
                        Ok(ConfidentValue::derived(Value::Array(items), min_conf))
                    }
                }

                Expr::Paren(inner) => self.eval_expr(inner, env).await,

                Expr::BinOp(left, op, right) => {
                    let lval = self.eval_expr(left, env).await?;
                    let rval = self.eval_expr(right, env).await?;
                    let min_conf = lval.confidence.min(rval.confidence);
                    let result = eval_binop(&lval.value, &op.node, &rval.value)?;
                    if min_conf >= 1.0 {
                        Ok(ConfidentValue::deterministic(result))
                    } else {
                        Ok(ConfidentValue::derived(result, min_conf))
                    }
                }

                Expr::UnaryOp(op, inner) => {
                    let val = self.eval_expr(inner, env).await?;
                    let result = match (&op.node, &val.value) {
                        (UnaryOp::Neg, Value::Number(n)) => Value::Number(-n),
                        (UnaryOp::Not, Value::Bool(b)) => Value::Bool(!b),
                        (UnaryOp::Not, other) => Value::Bool(!truthy_val(other)),
                        (UnaryOp::Neg, other) => {
                            return Err(RuntimeError::TypeError {
                                expected: "Number".to_string(),
                                got: format!("{}", other),
                            })
                        }
                    };
                    if val.confidence >= 1.0 {
                        Ok(ConfidentValue::deterministic(result))
                    } else {
                        Ok(ConfidentValue::derived(result, val.confidence))
                    }
                }

                // ── Access expressions ────────────────────────────────────────
                Expr::FieldAccess(obj_expr, field) => {
                    let obj = self.eval_expr(obj_expr, env).await?;
                    match (&obj.value, field.node.as_str()) {
                        // Array/List property accessors (no-arg methods as properties)
                        (Value::Array(v) | Value::List(v), "first") => Ok(v
                            .first()
                            .cloned()
                            .unwrap_or(ConfidentValue::deterministic(Value::Unit))),
                        (Value::Array(v) | Value::List(v), "all_same") => {
                            let same = if v.is_empty() {
                                true
                            } else {
                                let first = &v[0].value;
                                v.iter().all(|item| values_equal(&item.value, first))
                            };
                            Ok(ConfidentValue::deterministic(Value::Bool(same)))
                        }
                        (Value::Array(v) | Value::List(v), "len" | "length" | "count") => {
                            Ok(ConfidentValue::deterministic(Value::Number(v.len() as f64)))
                        }
                        (Value::Text(s) | Value::Html(s), "len" | "length") => {
                            Ok(ConfidentValue::deterministic(Value::Number(s.len() as f64)))
                        }
                        // Record field access — transparently unwrap _type/_value wrapper
                        (Value::Record(fields), _) => {
                            // Direct field access
                            if let Some(val) = fields.get(&field.node) {
                                return Ok(val.clone());
                            }
                            // Check for wrapped custom type: { _type: "Name", _value: { fields... } }
                            if let Some(inner) = fields.get("_value") {
                                if let Value::Record(inner_fields) = &inner.value {
                                    if let Some(val) = inner_fields.get(&field.node) {
                                        return Ok(val.clone());
                                    }
                                }
                            }
                            Ok(ConfidentValue::deterministic(Value::Unit))
                        }
                        _ => Err(RuntimeError::TypeError {
                            expected: "Record".to_string(),
                            got: format!("{}", obj.value),
                        }),
                    }
                }

                Expr::Index(obj_expr, idx_expr) => {
                    let obj = self.eval_expr(obj_expr, env).await?;
                    let idx = self.eval_expr(idx_expr, env).await?;
                    match (&obj.value, &idx.value) {
                        (Value::Array(items) | Value::List(items), Value::Number(n)) => {
                            let i = *n as usize;
                            items.get(i).cloned().ok_or(RuntimeError::IndexOutOfBounds {
                                index: i,
                                len: items.len(),
                            })
                        }
                        (Value::Record(fields), Value::Text(key)) => {
                            Ok(fields.get(key).cloned().unwrap_or_else(|| {
                                ConfidentValue::deterministic(Value::Text(String::new()))
                            }))
                        }
                        _ => Err(RuntimeError::TypeError {
                            expected: "Array[Number] or Record[Text]".to_string(),
                            got: format!("{}[{}]", obj.value, idx.value),
                        }),
                    }
                }

                Expr::MethodCall(obj_expr, method, args) => {
                    // html.layout() / html.escape() — built-in HTML capabilities
                    if let Expr::Ident(ref ns) = obj_expr.node {
                        if ns == "html" {
                            let mut arg_vals = Vec::new();
                            for arg in args {
                                arg_vals.push(self.eval_expr(&arg.node.value, env).await?);
                            }
                            match method.node.as_str() {
                                "layout" => {
                                    let title = arg_vals
                                        .first()
                                        .map(|v| format!("{}", v.value))
                                        .unwrap_or_default();
                                    let body = arg_vals
                                        .get(1)
                                        .map(|v| format!("{}", v.value))
                                        .unwrap_or_default();
                                    let html = crate::runtime::html::html_layout(&title, &body);
                                    return Ok(ConfidentValue::deterministic(Value::Html(html)));
                                }
                                "escape" => {
                                    let text = arg_vals
                                        .first()
                                        .map(|v| format!("{}", v.value))
                                        .unwrap_or_default();
                                    let escaped = crate::runtime::html::html_escape(&text);
                                    return Ok(ConfidentValue::deterministic(Value::Text(escaped)));
                                }
                                other => {
                                    return Err(RuntimeError::Unsupported(format!(
                                        "html.{}()",
                                        other
                                    )));
                                }
                            }
                        }
                    }

                    // markdown.render() — built-in Markdown capability
                    if let Expr::Ident(ref ns) = obj_expr.node {
                        if ns == "markdown" {
                            let mut arg_vals = Vec::new();
                            for arg in args {
                                arg_vals.push(self.eval_expr(&arg.node.value, env).await?);
                            }
                            match method.node.as_str() {
                                "render" => {
                                    let content = arg_vals
                                        .first()
                                        .map(|v| format!("{}", v.value))
                                        .unwrap_or_default();
                                    let html = crate::runtime::markdown::render_markdown(&content);
                                    return Ok(ConfidentValue::deterministic(Value::Html(html)));
                                }
                                other => {
                                    return Err(RuntimeError::Unsupported(format!(
                                        "markdown.{}()",
                                        other
                                    )));
                                }
                            }
                        }
                    }

                    // env.get(name, default) — read an environment variable with fallback.
                    // See issue #251 (part of epic #249).
                    if let Expr::Ident(ref ns) = obj_expr.node {
                        if ns == "env" && method.node == "get" {
                            let mut arg_vals = Vec::new();
                            for arg in args {
                                arg_vals.push(self.eval_expr(&arg.node.value, env).await?);
                            }
                            let name = arg_vals
                                .first()
                                .map(|v| format!("{}", v.value))
                                .unwrap_or_default();
                            let default = arg_vals
                                .get(1)
                                .map(|v| format!("{}", v.value))
                                .unwrap_or_default();
                            let value = std::env::var(&name).unwrap_or_else(|_| default.clone());
                            return Ok(ConfidentValue::deterministic(Value::Text(value)));
                        }
                    }

                    // text.to_number(s) — parse a text string to a number.
                    // Returns 0.0 if the text cannot be parsed.
                    if let Expr::Ident(ref ns) = obj_expr.node {
                        if ns == "text" && method.node == "to_number" {
                            let mut arg_vals = Vec::new();
                            for arg in args {
                                arg_vals.push(self.eval_expr(&arg.node.value, env).await?);
                            }
                            let s = arg_vals
                                .first()
                                .map(|v| format!("{}", v.value))
                                .unwrap_or_default();
                            let n: f64 = s.trim().parse().unwrap_or(0.0);
                            return Ok(ConfidentValue::deterministic(Value::Number(n)));
                        }
                    }

                    // proc.exit(code) — signal process exit. See issue #258.
                    // Raises RuntimeError::Exit, which the generated CLI dispatch
                    // (src/build.rs) translates to std::process::exit(code). Non-CLI
                    // contexts propagate the error like any other uncaught signal.
                    if let Expr::Ident(ref ns) = obj_expr.node {
                        if ns == "proc" && method.node == "exit" {
                            let mut arg_vals = Vec::new();
                            for arg in args {
                                arg_vals.push(self.eval_expr(&arg.node.value, env).await?);
                            }
                            let code = match arg_vals.first().map(|v| &v.value) {
                                Some(Value::Number(n)) => {
                                    // Clamp to conventional Unix exit-code range.
                                    (n.trunc() as i64).clamp(0, 255) as i32
                                }
                                _ => 1,
                            };
                            return Err(RuntimeError::Exit(code));
                        }
                    }

                    // web.fetch() / web.post() — built-in HTTP client capabilities
                    if let Expr::Ident(ref ns) = obj_expr.node {
                        if ns == "web" {
                            let mut arg_vals = Vec::new();
                            for arg in args {
                                arg_vals.push(self.eval_expr(&arg.node.value, env).await?);
                            }
                            let web_config = self.config.as_ref().and_then(|c| c.web.as_ref());
                            let client =
                                crate::runtime::http_client::ForgeHttpClient::new(web_config);
                            match method.node.as_str() {
                                "fetch" => {
                                    let url = arg_vals
                                        .first()
                                        .map(|v| format!("{}", v.value))
                                        .unwrap_or_default();
                                    match client.fetch(&url).await {
                                        Ok(body) => {
                                            return Ok(ConfidentValue::deterministic(Value::Text(
                                                body,
                                            )));
                                        }
                                        Err(e) => {
                                            return Err(RuntimeError::FlowError(e));
                                        }
                                    }
                                }
                                "post" => {
                                    let url = arg_vals
                                        .first()
                                        .map(|v| format!("{}", v.value))
                                        .unwrap_or_default();
                                    let body = arg_vals
                                        .get(1)
                                        .map(|v| format!("{}", v.value))
                                        .unwrap_or_default();
                                    match client.post(&url, &body).await {
                                        Ok(resp) => {
                                            return Ok(ConfidentValue::deterministic(Value::Text(
                                                resp,
                                            )));
                                        }
                                        Err(e) => {
                                            return Err(RuntimeError::FlowError(e));
                                        }
                                    }
                                }
                                other => {
                                    return Err(RuntimeError::Unsupported(format!(
                                        "web.{}()",
                                        other
                                    )));
                                }
                            }
                        }
                    }

                    // data.store() / data.get() / data.list() / data.delete() — KV persistence
                    if let Expr::Ident(ref ns) = obj_expr.node {
                        if ns == "data" {
                            let mut arg_vals = Vec::new();
                            for arg in args {
                                arg_vals.push(self.eval_expr(&arg.node.value, env).await?);
                            }
                            // When no storage is attached (e.g., lightweight server
                            // setups or tests without an attached redb), data.* degrades
                            // gracefully: get/list return empty, store/delete are no-ops.
                            // This lets endpoints like /api/status that fall back on
                            // `if raw == ""` keep working in any runtime configuration.
                            let storage = self.storage.as_ref();
                            match method.node.as_str() {
                                "store" => {
                                    if let Some(storage) = storage {
                                        let key = arg_vals
                                            .first()
                                            .map(|v| format!("{}", v.value))
                                            .unwrap_or_default();
                                        let value = arg_vals
                                            .get(1)
                                            .map(|v| format!("{}", v.value))
                                            .unwrap_or_default();
                                        storage.store(&key, &value).map_err(|e| {
                                            RuntimeError::FlowError(format!("data.store: {e}"))
                                        })?;
                                    }
                                    return Ok(ConfidentValue::deterministic(Value::Unit));
                                }
                                "get" => {
                                    let key = arg_vals
                                        .first()
                                        .map(|v| format!("{}", v.value))
                                        .unwrap_or_default();
                                    match storage {
                                        Some(storage) => match storage.get(&key) {
                                            Ok(Some(val)) => {
                                                return Ok(ConfidentValue::deterministic(
                                                    Value::Text(val),
                                                ));
                                            }
                                            // Missing key → empty string so callers can
                                            // use `if raw == ""` to detect absence.
                                            Ok(None) => {
                                                return Ok(ConfidentValue::deterministic(
                                                    Value::Text(String::new()),
                                                ));
                                            }
                                            Err(e) => {
                                                return Err(RuntimeError::FlowError(format!(
                                                    "data.get: {e}"
                                                )));
                                            }
                                        },
                                        // No storage attached → same semantics as missing key.
                                        None => {
                                            return Ok(ConfidentValue::deterministic(Value::Text(
                                                String::new(),
                                            )));
                                        }
                                    }
                                }
                                "list" => {
                                    let prefix = arg_vals
                                        .first()
                                        .map(|v| format!("{}", v.value))
                                        .unwrap_or_default();
                                    match storage {
                                        Some(storage) => match storage.list(&prefix) {
                                            Ok(keys) => {
                                                let items = keys
                                                    .into_iter()
                                                    .map(|k| {
                                                        ConfidentValue::deterministic(Value::Text(
                                                            k,
                                                        ))
                                                    })
                                                    .collect();
                                                return Ok(ConfidentValue::deterministic(
                                                    Value::Array(items),
                                                ));
                                            }
                                            Err(e) => {
                                                return Err(RuntimeError::FlowError(format!(
                                                    "data.list: {e}"
                                                )));
                                            }
                                        },
                                        None => {
                                            return Ok(ConfidentValue::deterministic(
                                                Value::Array(Vec::new()),
                                            ));
                                        }
                                    }
                                }
                                "delete" => {
                                    if let Some(storage) = storage {
                                        let key = arg_vals
                                            .first()
                                            .map(|v| format!("{}", v.value))
                                            .unwrap_or_default();
                                        storage.delete(&key).map_err(|e| {
                                            RuntimeError::FlowError(format!("data.delete: {e}"))
                                        })?;
                                    }
                                    return Ok(ConfidentValue::deterministic(Value::Unit));
                                }
                                "embed" => {
                                    let content = arg_vals
                                        .first()
                                        .map(|v| format!("{}", v.value))
                                        .unwrap_or_default();
                                    let provider = self.embedding_provider.as_ref().ok_or_else(|| {
                                        RuntimeError::Unsupported(
                                            "data.embed requires [embeddings] configuration in forge.config.toml".to_string(),
                                        )
                                    })?;
                                    let vi = self.vector_index.as_ref().ok_or_else(|| {
                                        RuntimeError::Unsupported(
                                            "data.embed requires [embeddings] configuration"
                                                .to_string(),
                                        )
                                    })?;

                                    let req = crate::llm::EmbeddingRequest {
                                        texts: vec![content.clone()],
                                        model: None,
                                    };
                                    let resp = provider.embed(req).await.map_err(|e| {
                                        RuntimeError::FlowError(format!("data.embed: {e}"))
                                    })?;
                                    let embedding =
                                        resp.embeddings.into_iter().next().ok_or_else(|| {
                                            RuntimeError::FlowError(
                                                "data.embed: no embedding returned".to_string(),
                                            )
                                        })?;

                                    // Generate a unique ID
                                    let id = format!("emb_{:x}", {
                                        use std::collections::hash_map::DefaultHasher;
                                        use std::hash::{Hash, Hasher};
                                        let mut h = DefaultHasher::new();
                                        content.hash(&mut h);
                                        std::time::SystemTime::now()
                                            .duration_since(std::time::UNIX_EPOCH)
                                            .unwrap_or_default()
                                            .as_nanos()
                                            .hash(&mut h);
                                        h.finish()
                                    });

                                    vi.lock()
                                        .await
                                        .insert(
                                            &id,
                                            &content,
                                            embedding,
                                            std::collections::HashMap::new(),
                                        )
                                        .map_err(|e| {
                                            RuntimeError::FlowError(format!("data.embed: {e}"))
                                        })?;

                                    // Emit trace event for cost tracking
                                    if let Some(ref tracer) = self.tracer {
                                        tracer.llm_response(&LLMResponseInfo {
                                            operation: "embed",
                                            provider: provider.name(),
                                            model: &resp.model_used,
                                            tokens_in: resp.tokens_used,
                                            tokens_out: 0,
                                            cost_usd: resp.cost_usd,
                                            confidence: 1.0,
                                            agent_name: self.agent_name.as_deref(),
                                        });
                                    }

                                    return Ok(ConfidentValue::deterministic(Value::Text(id)));
                                }
                                "search" => {
                                    let query = arg_vals
                                        .first()
                                        .map(|v| format!("{}", v.value))
                                        .unwrap_or_default();
                                    let top_k = arg_vals
                                        .get(1)
                                        .and_then(|v| match &v.value {
                                            Value::Number(n) => Some(*n as usize),
                                            _ => None,
                                        })
                                        .unwrap_or(5);

                                    let provider = self.embedding_provider.as_ref().ok_or_else(|| {
                                        RuntimeError::Unsupported(
                                            "data.search requires [embeddings] configuration in forge.config.toml".to_string(),
                                        )
                                    })?;
                                    let vi = self.vector_index.as_ref().ok_or_else(|| {
                                        RuntimeError::Unsupported(
                                            "data.search requires [embeddings] configuration"
                                                .to_string(),
                                        )
                                    })?;

                                    // Embed the query
                                    let req = crate::llm::EmbeddingRequest {
                                        texts: vec![query.clone()],
                                        model: None,
                                    };
                                    let resp = provider.embed(req).await.map_err(|e| {
                                        RuntimeError::FlowError(format!("data.search: {e}"))
                                    })?;
                                    let query_embedding =
                                        resp.embeddings.into_iter().next().ok_or_else(|| {
                                            RuntimeError::FlowError(
                                                "data.search: no embedding returned".to_string(),
                                            )
                                        })?;

                                    let results = vi.lock().await.search(&query_embedding, top_k);
                                    let best_score =
                                        results.first().map(|r| r.score).unwrap_or(0.0);

                                    let items: Vec<ConfidentValue> = results
                                        .into_iter()
                                        .map(|r| {
                                            let mut fields = HashMap::new();
                                            fields.insert(
                                                "id".into(),
                                                ConfidentValue::deterministic(Value::Text(r.id)),
                                            );
                                            fields.insert(
                                                "content".into(),
                                                ConfidentValue::deterministic(Value::Text(
                                                    r.content,
                                                )),
                                            );
                                            fields.insert(
                                                "score".into(),
                                                ConfidentValue::deterministic(Value::Number(
                                                    r.score as f64,
                                                )),
                                            );
                                            ConfidentValue::deterministic(Value::Record(fields))
                                        })
                                        .collect();

                                    // Emit trace event for cost tracking
                                    if let Some(ref tracer) = self.tracer {
                                        tracer.llm_response(&LLMResponseInfo {
                                            operation: "search",
                                            provider: provider.name(),
                                            model: &resp.model_used,
                                            tokens_in: resp.tokens_used,
                                            tokens_out: 0,
                                            cost_usd: resp.cost_usd,
                                            confidence: best_score,
                                            agent_name: self.agent_name.as_deref(),
                                        });
                                    }

                                    // Confidence derived from best cosine similarity score
                                    return Ok(ConfidentValue::derived(
                                        Value::List(items),
                                        best_score,
                                    ));
                                }
                                other => {
                                    return Err(RuntimeError::Unsupported(format!(
                                        "data.{}()",
                                        other
                                    )));
                                }
                            }
                        }
                    }

                    // skill.namespace.method() dispatch — issue #40
                    if let Expr::FieldAccess(inner, namespace) = &obj_expr.node {
                        if let Expr::Ident(ref prefix) = inner.node {
                            if prefix == "skill" {
                                if let Some(ref skill_executor) = self.skill_executor {
                                    let skill_name = namespace.node.clone();
                                    let method_name = method.node.clone();
                                    let mut arg_map = std::collections::HashMap::new();
                                    for arg in args {
                                        let val = self.eval_expr(&arg.node.value, env).await?;
                                        let key = arg
                                            .node
                                            .label
                                            .as_ref()
                                            .map(|l| l.node.clone())
                                            .unwrap_or_else(|| format!("_{}", arg_map.len()));
                                        arg_map.insert(key, val);
                                    }
                                    return skill_executor
                                        .execute(&skill_name, &method_name, &arg_map)
                                        .await
                                        .map_err(|e| {
                                            RuntimeError::FlowError(format!(
                                                "skill.{}.{}: {}",
                                                skill_name, method_name, e
                                            ))
                                        });
                                } else {
                                    return Err(RuntimeError::Unsupported(
                                        "skill calls require a skill executor — configure [skills] in forge.config.toml".to_string(),
                                    ));
                                }
                            }
                        }
                    }

                    // Pool .send() interception — pools are declarations, not runtime values
                    if method.node == "send" {
                        if let Expr::Ident(ref name) = obj_expr.node {
                            if let Some(pool_decl) = self.pool_map.get(name).cloned() {
                                let mut arg_vals = Vec::new();
                                for arg in args {
                                    arg_vals.push(self.eval_expr(&arg.node.value, env).await?);
                                }
                                let event = arg_vals
                                    .first()
                                    .map(|v| format!("{}", v.value))
                                    .unwrap_or_else(|| "default".to_string());
                                let payload: Vec<ConfidentValue> =
                                    arg_vals.into_iter().skip(1).collect();

                                let pool = crate::runtime::pool::PoolExecutor::new(
                                    pool_decl,
                                    &self.program,
                                    self.providers.clone(),
                                    self.tracer.clone(),
                                )?;
                                return pool.send(&event, payload).await;
                            }
                        }
                    }

                    let obj = self.eval_expr(obj_expr, env).await?;
                    match method.node.as_str() {
                        "len" | "count" => {
                            let len = match &obj.value {
                                Value::Array(v) | Value::List(v) => v.len(),
                                Value::Text(s) | Value::Html(s) => s.len(),
                                _ => {
                                    return Err(RuntimeError::TypeError {
                                        expected: "Array, List, or Text".to_string(),
                                        got: format!("{}", obj.value),
                                    })
                                }
                            };
                            Ok(ConfidentValue::deterministic(Value::Number(len as f64)))
                        }
                        "contains" | "any" => {
                            if args.is_empty() {
                                return Err(RuntimeError::TypeError {
                                    expected: "1 argument".to_string(),
                                    got: "0 arguments".to_string(),
                                });
                            }
                            let needle = self.eval_expr(&args[0].node.value, env).await?;
                            let found = match &obj.value {
                                Value::Array(v) | Value::List(v) => v
                                    .iter()
                                    .any(|item| values_equal(&item.value, &needle.value)),
                                Value::Text(s) | Value::Html(s) => {
                                    if let Value::Text(needle_s) | Value::Html(needle_s) =
                                        &needle.value
                                    {
                                        s.contains(needle_s.as_str())
                                    } else {
                                        false
                                    }
                                }
                                _ => false,
                            };
                            Ok(ConfidentValue::deterministic(Value::Bool(found)))
                        }
                        "all_same" => {
                            let same = match &obj.value {
                                Value::Array(v) | Value::List(v) => {
                                    if v.is_empty() {
                                        true
                                    } else {
                                        let first = &v[0].value;
                                        v.iter().all(|item| values_equal(&item.value, first))
                                    }
                                }
                                _ => false,
                            };
                            Ok(ConfidentValue::deterministic(Value::Bool(same)))
                        }
                        "first" => match &obj.value {
                            Value::Array(v) | Value::List(v) => Ok(v
                                .first()
                                .cloned()
                                .unwrap_or(ConfidentValue::deterministic(Value::Unit))),
                            _ => Err(RuntimeError::TypeError {
                                expected: "Array or List".to_string(),
                                got: format!("{}", obj.value),
                            }),
                        },
                        "none" => {
                            if args.is_empty() {
                                return Err(RuntimeError::TypeError {
                                    expected: "1 argument".to_string(),
                                    got: "0 arguments".to_string(),
                                });
                            }
                            let needle = self.eval_expr(&args[0].node.value, env).await?;
                            let none = match &obj.value {
                                Value::Array(v) | Value::List(v) => !v
                                    .iter()
                                    .any(|item| values_equal(&item.value, &needle.value)),
                                _ => true,
                            };
                            Ok(ConfidentValue::deterministic(Value::Bool(none)))
                        }
                        "lower" => {
                            let (text, wrap) = match &obj.value {
                                Value::Text(s) => (s.to_lowercase(), false),
                                Value::Html(s) => (s.to_lowercase(), true),
                                _ => {
                                    return Err(RuntimeError::TypeError {
                                        expected: "Text".to_string(),
                                        got: format!("{}", obj.value),
                                    })
                                }
                            };
                            let val = if wrap {
                                Value::Html(text)
                            } else {
                                Value::Text(text)
                            };
                            Ok(ConfidentValue::derived(val, obj.confidence))
                        }
                        "upper" => {
                            let (text, wrap) = match &obj.value {
                                Value::Text(s) => (s.to_uppercase(), false),
                                Value::Html(s) => (s.to_uppercase(), true),
                                _ => {
                                    return Err(RuntimeError::TypeError {
                                        expected: "Text".to_string(),
                                        got: format!("{}", obj.value),
                                    })
                                }
                            };
                            let val = if wrap {
                                Value::Html(text)
                            } else {
                                Value::Text(text)
                            };
                            Ok(ConfidentValue::derived(val, obj.confidence))
                        }
                        "trim" => {
                            let (text, wrap) = match &obj.value {
                                Value::Text(s) => (s.trim().to_string(), false),
                                Value::Html(s) => (s.trim().to_string(), true),
                                _ => {
                                    return Err(RuntimeError::TypeError {
                                        expected: "Text".to_string(),
                                        got: format!("{}", obj.value),
                                    })
                                }
                            };
                            let val = if wrap {
                                Value::Html(text)
                            } else {
                                Value::Text(text)
                            };
                            Ok(ConfidentValue::derived(val, obj.confidence))
                        }
                        "split" => {
                            if args.len() != 1 {
                                return Err(RuntimeError::TypeError {
                                    expected: "1 argument".to_string(),
                                    got: format!("{} arguments", args.len()),
                                });
                            }
                            let delim_val = self.eval_expr(&args[0].node.value, env).await?;
                            let delimiter = match &delim_val.value {
                                Value::Text(s) | Value::Html(s) => s.clone(),
                                _ => {
                                    return Err(RuntimeError::TypeError {
                                        expected: "Text delimiter".to_string(),
                                        got: format!("{:?}", delim_val.value),
                                    })
                                }
                            };
                            let text = match &obj.value {
                                Value::Text(s) | Value::Html(s) => s.as_str(),
                                _ => {
                                    return Err(RuntimeError::TypeError {
                                        expected: "Text".to_string(),
                                        got: format!("{}", obj.value),
                                    })
                                }
                            };
                            let parts: Vec<ConfidentValue> = text
                                .split(&delimiter)
                                .map(|part| {
                                    ConfidentValue::deterministic(Value::Text(part.to_string()))
                                })
                                .collect();
                            Ok(ConfidentValue::deterministic(Value::Array(parts)))
                        }
                        "join" => {
                            if args.len() != 1 {
                                return Err(RuntimeError::TypeError {
                                    expected: "1 argument".to_string(),
                                    got: format!("{} arguments", args.len()),
                                });
                            }
                            let delim_val = self.eval_expr(&args[0].node.value, env).await?;
                            let delimiter = match &delim_val.value {
                                Value::Text(s) | Value::Html(s) => s.clone(),
                                _ => {
                                    return Err(RuntimeError::TypeError {
                                        expected: "Text delimiter".to_string(),
                                        got: format!("{:?}", delim_val.value),
                                    })
                                }
                            };
                            let items = match &obj.value {
                                Value::Array(v) | Value::List(v) => v,
                                _ => {
                                    return Err(RuntimeError::TypeError {
                                        expected: "Array or List".to_string(),
                                        got: format!("{}", obj.value),
                                    })
                                }
                            };
                            let joined: String = items
                                .iter()
                                .map(|item| format!("{}", item.value))
                                .collect::<Vec<_>>()
                                .join(&delimiter);
                            Ok(ConfidentValue::deterministic(Value::Text(joined)))
                        }
                        other => Err(RuntimeError::Unsupported(format!("method .{}()", other))),
                    }
                }

                Expr::TypeAccess(type_name, variant) => {
                    if matches!(type_name.node, TypeName::AgentResult) && variant.node == "default"
                    {
                        return Ok(ConfidentValue::default_agent_result());
                    }
                    Ok(ConfidentValue::deterministic(Value::Text(
                        variant.node.clone(),
                    )))
                }

                Expr::GlobAccess(inner) => self.eval_expr(inner, env).await,

                // ── Calls ─────────────────────────────────────────────────────
                Expr::Call(call) => {
                    let name = &call.name.node;
                    let mut arg_vals = Vec::new();
                    for arg in &call.args {
                        arg_vals.push(self.eval_expr(&arg.node.value, env).await?);
                    }

                    if let Some(task_decl) = self.task_map.get(name).cloned() {
                        if let Some(ref tracer) = self.tracer {
                            tracer.task_call(name);
                        }
                        let result = self.call_task(&task_decl, arg_vals).await;
                        if let Some(ref tracer) = self.tracer {
                            tracer.task_return(name, result.is_ok());
                        }
                        result
                    } else if let Some(pure_decl) = self.pure_map.get(name).cloned() {
                        self.call_pure(&pure_decl, arg_vals).await
                    } else if let Some(flow_decl) = self.flow_map.get(name).cloned() {
                        self.call_flow(&flow_decl, arg_vals).await
                    } else if let Some(pool_decl) = self.pool_map.get(name).cloned() {
                        let pool = crate::runtime::pool::PoolExecutor::new(
                            pool_decl,
                            &self.program,
                            self.providers.clone(),
                            self.tracer.clone(),
                        )?;
                        let event = arg_vals
                            .first()
                            .map(|v| format!("{}", v.value))
                            .unwrap_or_else(|| "default".to_string());
                        let payload: Vec<ConfidentValue> = arg_vals.into_iter().skip(1).collect();
                        pool.send(&event, payload).await
                    } else if name == "asset" {
                        let path = arg_vals
                            .first()
                            .map(|v| format!("{}", v.value))
                            .unwrap_or_default();
                        let prefix = self
                            .config
                            .as_ref()
                            .and_then(|c| c.server.as_ref())
                            .and_then(|s| s.static_files.as_ref())
                            .map(|sc| sc.prefix_or_default().to_string())
                            .unwrap_or_else(|| "/static".to_string());
                        let prefix = prefix.trim_end_matches('/');
                        let path = if path.starts_with('/') {
                            path
                        } else {
                            format!("/{path}")
                        };
                        Ok(ConfidentValue::deterministic(Value::Text(format!(
                            "{prefix}{path}"
                        ))))
                    } else if name.starts_with(|c: char| c.is_uppercase()) {
                        // Uppercase names not in task/pure/flow/pool maps are type constructors
                        let mut fields = HashMap::new();
                        for (i, arg) in call.args.iter().enumerate() {
                            let val = self.eval_expr(&arg.node.value, env).await?;
                            let key = arg
                                .node
                                .label
                                .as_ref()
                                .map(|l| l.node.clone())
                                .unwrap_or_else(|| format!("_{}", i));
                            fields.insert(key, val);
                        }
                        if fields.is_empty() {
                            // No-arg variant: just a tag
                            Ok(ConfidentValue::deterministic(Value::Text(name.clone())))
                        } else {
                            // With args: Record tagged by position or label
                            // Wrap in an outer record with the type name for match dispatch
                            let inner = ConfidentValue::deterministic(Value::Record(fields));
                            let mut wrapper = HashMap::new();
                            wrapper.insert(
                                "_type".to_string(),
                                ConfidentValue::deterministic(Value::Text(name.clone())),
                            );
                            wrapper.insert("_value".to_string(), inner);
                            Ok(ConfidentValue::deterministic(Value::Record(wrapper)))
                        }
                    } else if name == "winning_lines" {
                        // Built-in: returns the 8 triplets for tic-tac-toe win detection
                        let board = arg_vals
                            .into_iter()
                            .next()
                            .unwrap_or(ConfidentValue::deterministic(Value::Unit));
                        let cells = match &board.value {
                            Value::Array(v) | Value::List(v) => v.clone(),
                            _ => {
                                return Err(RuntimeError::TypeError {
                                    expected: "Array[9]".to_string(),
                                    got: format!("{}", board.value),
                                })
                            }
                        };
                        let indices: &[&[usize]] = &[
                            &[0, 1, 2],
                            &[3, 4, 5],
                            &[6, 7, 8], // rows
                            &[0, 3, 6],
                            &[1, 4, 7],
                            &[2, 5, 8], // cols
                            &[0, 4, 8],
                            &[2, 4, 6], // diags
                        ];
                        let lines: Vec<ConfidentValue> = indices
                            .iter()
                            .map(|idx| {
                                let line: Vec<ConfidentValue> = idx
                                    .iter()
                                    .map(|&i| {
                                        cells.get(i).cloned().unwrap_or(
                                            ConfidentValue::deterministic(Value::Text(
                                                "_".to_string(),
                                            )),
                                        )
                                    })
                                    .collect();
                                ConfidentValue::deterministic(Value::Array(line))
                            })
                            .collect();
                        Ok(ConfidentValue::deterministic(Value::Array(lines)))
                    } else {
                        Err(RuntimeError::NotCallable { name: name.clone() })
                    }
                }

                Expr::Constructor(ctor) => {
                    let mut fields = HashMap::new();
                    for (i, arg) in ctor.args.iter().enumerate() {
                        let val = self.eval_expr(&arg.node.value, env).await?;
                        let key = arg
                            .node
                            .label
                            .as_ref()
                            .map(|l| l.node.clone())
                            .unwrap_or_else(|| format!("_{}", i));
                        fields.insert(key, val);
                    }
                    // AgentResult: merge with defaults and sync confidence
                    if matches!(ctor.type_name.node, TypeName::AgentResult) {
                        let mut defaults = ConfidentValue::default_agent_result_fields();
                        for (k, v) in fields {
                            defaults.insert(k, v);
                        }
                        let wrapper_conf = defaults
                            .get("confidence")
                            .and_then(|cv| match &cv.value {
                                Value::Number(n) => Some(*n as f32),
                                _ => None,
                            })
                            .unwrap_or(0.0);
                        let conf = wrapper_conf.clamp(0.0, 1.0);
                        return Ok(ConfidentValue {
                            value: Value::Record(defaults),
                            confidence: conf,
                            source: crate::types::ConfidenceSource::Derived(conf),
                        });
                    }
                    Ok(ConfidentValue::deterministic(Value::Record(fields)))
                }

                // ── CLI execution (issue #40) ────────────────────────────────
                Expr::Exec(command_expr) => {
                    let command = self.eval_expr(command_expr, env).await?;
                    let cmd_str = format!("{}", command.value);

                    if let Some(ref tracer) = self.tracer {
                        tracer.exec_call(&cmd_str);
                    }

                    let start = std::time::Instant::now();

                    let result = tokio::time::timeout(
                        std::time::Duration::from_secs(30),
                        tokio::process::Command::new("sh")
                            .arg("-c")
                            .arg(&cmd_str)
                            .output(),
                    )
                    .await;

                    let elapsed = start.elapsed();

                    match result {
                        Ok(Ok(output)) => {
                            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                            let success = output.status.success();
                            let confidence = if success { 0.9 } else { 0.3 };
                            let text = if stdout.is_empty() { stderr } else { stdout };

                            if let Some(ref tracer) = self.tracer {
                                tracer.exec_return(&cmd_str, success, elapsed.as_millis() as u64);
                            }

                            Ok(ConfidentValue::from_exec(Value::Text(text), confidence))
                        }
                        Ok(Err(e)) => Err(RuntimeError::FlowError(format!("exec failed: {}", e))),
                        Err(_) => Err(RuntimeError::FlowError(format!(
                            "exec timed out after 30s: {}",
                            cmd_str
                        ))),
                    }
                }

                // ── command expression (issue #160, runtime in #161) ─────────
                Expr::Command(cmd_expr) => {
                    // 1. Evaluate the command argument
                    let cmd_val = self.eval_expr(&cmd_expr.cmd, env).await?;

                    // 2. Build process based on argv vs string mode
                    let (mut process, cmd_display) = match &cmd_val.value {
                        Value::Array(items) | Value::List(items) => {
                            let argv: Vec<String> =
                                items.iter().map(|item| format!("{}", item.value)).collect();
                            if argv.is_empty() {
                                return Err(RuntimeError::FlowError(
                                    "command requires at least one element in argv array"
                                        .to_string(),
                                ));
                            }
                            let mut cmd = tokio::process::Command::new(&argv[0]);
                            cmd.args(&argv[1..]);
                            let display = argv.join(" ");
                            (cmd, display)
                        }
                        Value::Text(s) => {
                            let mut cmd = tokio::process::Command::new("sh");
                            cmd.arg("-c").arg(s);
                            (cmd, s.clone())
                        }
                        other => {
                            return Err(RuntimeError::TypeError {
                                expected: "Text or Array".to_string(),
                                got: format!("{}", other),
                            });
                        }
                    };

                    // 3. Apply working directory (explicit `in` overrides agent-level sandbox dir)
                    if let Some(ref wd_expr) = cmd_expr.working_dir {
                        let wd = self.eval_expr(wd_expr, env).await?;
                        process.current_dir(format!("{}", wd.value));
                    } else if let Some(ref default_wd) = self.working_dir {
                        process.current_dir(default_wd);
                    }

                    // 4. Apply environment variables
                    if let Some(ref entries) = cmd_expr.env {
                        for entry in entries {
                            let val = self.eval_expr(&entry.node.value, env).await?;
                            process.env(&entry.node.key.node, format!("{}", val.value));
                        }
                    }

                    // 5. Determine timeout (default 30s)
                    let timeout_dur = cmd_expr
                        .timeout
                        .as_ref()
                        .map(|t| t.node.to_std())
                        .unwrap_or(std::time::Duration::from_secs(30));

                    // 6. Background mode — spawn and return handle (issue #162)
                    if matches!(cmd_expr.background, Some(ref s) if s.node) {
                        let mgr = self.command_manager.as_ref().ok_or_else(|| {
                            RuntimeError::Unsupported(
                                "background commands require a command manager".to_string(),
                            )
                        })?;

                        process.stdout(std::process::Stdio::piped());
                        process.stderr(std::process::Stdio::piped());

                        if let Some(ref tracer) = self.tracer {
                            tracer.command_call(&cmd_display);
                        }

                        let child = process.spawn().map_err(|e| {
                            RuntimeError::FlowError(format!(
                                "background command failed to spawn: {}",
                                e
                            ))
                        })?;

                        let handle_id = mgr
                            .lock()
                            .unwrap()
                            .spawn_background(
                                child,
                                cmd_display.clone(),
                                Some(timeout_dur),
                                self.tracer.clone(),
                            )
                            .map_err(RuntimeError::FlowError)?;

                        if let Some(ref tracer) = self.tracer {
                            tracer.command_bg_spawn(&cmd_display, &handle_id);
                        }

                        return Ok(ConfidentValue::deterministic(Value::Text(handle_id)));
                    }

                    // 7. Trace and spawn (synchronous)
                    if let Some(ref tracer) = self.tracer {
                        tracer.command_call(&cmd_display);
                    }

                    let start = std::time::Instant::now();
                    let result = tokio::time::timeout(timeout_dur, process.output()).await;
                    let elapsed = start.elapsed();

                    // 8. Build structured result
                    match result {
                        Ok(Ok(output)) => {
                            let stdout = String::from_utf8_lossy(&output.stdout)
                                .trim_end()
                                .to_string();
                            let stderr = String::from_utf8_lossy(&output.stderr)
                                .trim_end()
                                .to_string();
                            let success = output.status.success();
                            let exit_code = output.status.code().unwrap_or(-1) as f64;
                            let confidence = if success { 0.9 } else { 0.3 };

                            let mut fields = HashMap::new();
                            fields.insert(
                                "stdout".to_string(),
                                ConfidentValue::from_exec(Value::Text(stdout), confidence),
                            );
                            fields.insert(
                                "stderr".to_string(),
                                ConfidentValue::from_exec(Value::Text(stderr), confidence),
                            );
                            fields.insert(
                                "exit_code".to_string(),
                                ConfidentValue::from_exec(Value::Number(exit_code), confidence),
                            );
                            fields.insert(
                                "success".to_string(),
                                ConfidentValue::from_exec(Value::Bool(success), confidence),
                            );

                            if let Some(ref tracer) = self.tracer {
                                tracer.command_return(
                                    &cmd_display,
                                    success,
                                    elapsed.as_millis() as u64,
                                );
                            }

                            Ok(ConfidentValue::from_exec(Value::Record(fields), confidence))
                        }
                        Ok(Err(e)) => Err(RuntimeError::FlowError(format!(
                            "command failed to spawn: {}",
                            e
                        ))),
                        Err(_) => Err(RuntimeError::FlowError(format!(
                            "command timed out after {:?}: {}",
                            timeout_dur, cmd_display
                        ))),
                    }
                }

                Expr::Session(session) => self.eval_session_expr(session, env).await,

                // ── command.method() expressions (issue #162) ────────────────
                Expr::CommandMethod(method, args) => {
                    let mut arg_vals = Vec::new();
                    for arg in args {
                        arg_vals.push(self.eval_expr(&arg.node.value, env).await?);
                    }
                    let mgr = self.command_manager.as_ref().ok_or_else(|| {
                        RuntimeError::Unsupported(
                            "command.* requires a command manager".to_string(),
                        )
                    })?;
                    let handle = arg_vals
                        .first()
                        .map(|v| format!("{}", v.value))
                        .unwrap_or_default();
                    match method.node.as_str() {
                        "status" => mgr
                            .lock()
                            .unwrap()
                            .status(&handle)
                            .map_err(RuntimeError::FlowError),
                        "output" => mgr
                            .lock()
                            .unwrap()
                            .output(&handle)
                            .map_err(RuntimeError::FlowError),
                        "cancel" => {
                            mgr.lock()
                                .unwrap()
                                .cancel(&handle)
                                .map_err(RuntimeError::FlowError)?;
                            Ok(ConfidentValue::deterministic(Value::Unit))
                        }
                        other => Err(RuntimeError::Unsupported(format!("command.{}()", other))),
                    }
                }

                // ── session.method() expressions (issue #192) ────────────────
                Expr::SessionMethod(method, args) => {
                    let mut arg_vals = Vec::new();
                    for arg in args {
                        arg_vals.push(self.eval_expr(&arg.node.value, env).await?);
                    }
                    let manager = self.session_manager.as_ref().ok_or_else(|| {
                        RuntimeError::Unsupported(
                            "session.* requires a session manager".to_string(),
                        )
                    })?;
                    let id = arg_vals
                        .first()
                        .map(|v| format!("{}", v.value))
                        .unwrap_or_default();
                    match method.node.as_str() {
                        "status" => match manager.session_state(&id) {
                            Some(state) => {
                                let text = |s: &str| {
                                    ConfidentValue::deterministic(Value::Text(s.to_string()))
                                };
                                let num = |n: f64| ConfidentValue::deterministic(Value::Number(n));
                                let mut fields = std::collections::HashMap::new();
                                fields.insert("status".to_string(), text(state.status.as_str()));
                                fields.insert("cost_usd".to_string(), num(state.cost_usd as f64));
                                fields.insert(
                                    "started_at".to_string(),
                                    text(&state.started_at.to_rfc3339()),
                                );
                                fields.insert(
                                    "updated_at".to_string(),
                                    text(&state.updated_at.to_rfc3339()),
                                );
                                if let Some(err) = &state.error {
                                    fields.insert("error".to_string(), text(err));
                                }
                                Ok(ConfidentValue::deterministic(Value::Record(fields)))
                            }
                            None => Ok(ConfidentValue::deterministic(Value::Text(format!(
                                "unknown session: {}",
                                id
                            )))),
                        },
                        other => Err(RuntimeError::Unsupported(format!("session.{}()", other))),
                    }
                }

                // ── LLM expressions ───────────────────────────────────────────
                Expr::Reason(prompt_expr) => {
                    let prompt = self.eval_expr(prompt_expr, env).await?;
                    let prompt_text = format!("{}", prompt.value);

                    if let Some(ref tracer) = self.tracer {
                        tracer.llm_request("reason", &prompt_text);
                    }

                    let request = CompletionRequest::simple(&prompt_text);
                    let hint = crate::llm::CapabilityHint {
                        quality: Some(crate::llm::QualityTier::Balanced),
                        ..Default::default()
                    };
                    let response = self
                        .providers
                        .resolve_and_complete(request, Some(&hint))
                        .await
                        .map_err(|e| RuntimeError::LLMError(e.to_string()))?;
                    let confidence = response.estimate_confidence();

                    if let Some(ref tracer) = self.tracer {
                        tracer.llm_response(&LLMResponseInfo {
                            operation: "reason",
                            provider: &response.provider_name,
                            model: &response.model_used,
                            tokens_in: response.tokens_in,
                            tokens_out: response.tokens_out,
                            cost_usd: response.cost_usd,
                            confidence,
                            agent_name: self.agent_name.as_deref(),
                        });
                    }

                    Ok(ConfidentValue::from_llm(
                        Value::Text(response.content),
                        confidence,
                    ))
                }

                Expr::Classify(classify) => {
                    let input = self.eval_expr(&classify.input, env).await?;
                    let labels: Vec<String> =
                        classify.labels.iter().map(|l| l.node.clone()).collect();
                    let prompt = format!(
                    "Classify the following into exactly one of these categories: {}\n\nInput: {}\n\nRespond with just the category name.",
                    labels.join(", "),
                    input.value,
                );

                    if let Some(ref tracer) = self.tracer {
                        tracer.llm_request("classify", &prompt);
                    }

                    let request = CompletionRequest::simple(&prompt).with_temperature(0.0);
                    let hint = crate::llm::CapabilityHint {
                        quality: Some(crate::llm::QualityTier::Fast),
                        ..Default::default()
                    };
                    let response = self
                        .providers
                        .resolve_and_complete(request, Some(&hint))
                        .await
                        .map_err(|e| RuntimeError::LLMError(e.to_string()))?;
                    let confidence = response.estimate_confidence();

                    if let Some(ref tracer) = self.tracer {
                        tracer.llm_response(&LLMResponseInfo {
                            operation: "classify",
                            provider: &response.provider_name,
                            model: &response.model_used,
                            tokens_in: response.tokens_in,
                            tokens_out: response.tokens_out,
                            cost_usd: response.cost_usd,
                            confidence,
                            agent_name: self.agent_name.as_deref(),
                        });
                    }

                    Ok(ConfidentValue::from_llm(
                        Value::Text(response.content.trim().to_string()),
                        confidence,
                    ))
                }

                Expr::Search(query_expr) => {
                    let query_val = self.eval_expr(query_expr, env).await?;
                    let query_text = format!("{}", query_val.value);
                    let web_config = self.config.as_ref().and_then(|c| c.web.as_ref());
                    let client = crate::runtime::http_client::ForgeHttpClient::new(web_config);
                    match crate::runtime::http_client::search(&client, &query_text, web_config)
                        .await
                    {
                        Ok(results) => {
                            let items: Vec<ConfidentValue> = results
                                .into_iter()
                                .map(|r| {
                                    let mut fields = HashMap::new();
                                    fields.insert(
                                        "title".into(),
                                        ConfidentValue::deterministic(Value::Text(r.title)),
                                    );
                                    fields.insert(
                                        "url".into(),
                                        ConfidentValue::deterministic(Value::Text(r.url)),
                                    );
                                    fields.insert(
                                        "snippet".into(),
                                        ConfidentValue::deterministic(Value::Text(r.snippet)),
                                    );
                                    ConfidentValue::deterministic(Value::Record(fields))
                                })
                                .collect();
                            Ok(ConfidentValue::deterministic(Value::List(items)))
                        }
                        Err(e) => Err(RuntimeError::FlowError(e)),
                    }
                }

                Expr::Recall(query_expr) => {
                    let query_val = self.eval_expr(query_expr, env).await?;
                    let query_text = format!("{}", query_val.value);

                    if let Some(ref ctx_arc) = self.agent_context {
                        let ks_arc = ctx_arc.lock().unwrap().knowledge_store.clone();
                        if let Some(ref ks_arc) = ks_arc {
                            // Default token budget for recall: 2000 tokens
                            let mut ks = ks_arc.lock().unwrap();
                            let result = ks.recall(&query_text, 2000);
                            Ok(result)
                        } else {
                            Err(RuntimeError::Unsupported(
                                "recall requires agent with knowledge store".into(),
                            ))
                        }
                    } else if let Some(ref ks_arc) = self.knowledge_store {
                        // Standalone knowledge store (endpoint/webhook context)
                        let mut ks = ks_arc.lock().unwrap();
                        let result = ks.recall(&query_text, 2000);
                        Ok(result)
                    } else {
                        // No knowledge store — return empty with low confidence
                        Ok(ConfidentValue {
                            value: Value::Text(String::new()),
                            confidence: 0.0,
                            source: crate::types::ConfidenceSource::KnowledgeRecall(0.0),
                        })
                    }
                }

                // ── Find (instance discovery, issue #84) ─────────────────────
                Expr::Find(find) => {
                    let ir = self.instance_registry.as_ref().ok_or_else(|| {
                        RuntimeError::Unsupported("find requires instance registry".into())
                    })?;
                    let registry = ir.read().await;
                    match &find.kind {
                        FindKind::ByAlias(alias_expr) => {
                            let alias_val = self.eval_expr(alias_expr, env).await?;
                            let alias_str = format!("{}", alias_val.value);
                            match registry.find_by_alias(&alias_str) {
                                Some(info) => Ok(instance_info_to_record(&info)),
                                None => Err(RuntimeError::FlowError(format!(
                                    "no agent instance found with alias \"{}\"",
                                    alias_str
                                ))),
                            }
                        }
                        FindKind::AllByTemplate(template) => {
                            let instances = registry.find_by_name(&template.node);
                            let records: Vec<ConfidentValue> =
                                instances.iter().map(instance_info_to_record).collect();
                            Ok(ConfidentValue::deterministic(Value::Array(records)))
                        }
                        FindKind::AllByTemplateFiltered(template, state) => {
                            let instances = registry.find_by_name(&template.node);
                            let records: Vec<ConfidentValue> = instances
                                .iter()
                                .filter(|info| {
                                    info.lifecycle_state.as_deref() == Some(state.node.as_str())
                                })
                                .map(instance_info_to_record)
                                .collect();
                            Ok(ConfidentValue::deterministic(Value::Array(records)))
                        }
                    }
                }

                // ── Composition ───────────────────────────────────────────────
                Expr::TryOr(primary, fallback) => match self.eval_expr(primary, env).await {
                    Ok(val) => Ok(val),
                    Err(_) => self.eval_expr(fallback, env).await,
                },

                Expr::Compose(parts) => {
                    if parts.is_empty() {
                        return Ok(ConfidentValue::deterministic(Value::Unit));
                    }
                    let mut current = self.eval_expr(&parts[0], env).await?;
                    for part in &parts[1..] {
                        env.push_scope();
                        env.bind("_pipe", current);
                        current = self.eval_expr(part, env).await?;
                        env.pop_scope();
                    }
                    Ok(current)
                }

                Expr::FanOut(parts) => {
                    let mut results = Vec::new();
                    for part in parts {
                        results.push(self.eval_expr(part, env).await?);
                    }
                    let min_conf = results.iter().map(|r| r.confidence).fold(1.0_f32, f32::min);
                    Ok(ConfidentValue::derived(Value::Array(results), min_conf))
                }
            }
        })
    }

    // ── Task/Pure call helpers ─────────────────────────────────────────────────

    pub(crate) async fn call_task(
        &self,
        decl: &TaskDecl,
        args: Vec<ConfidentValue>,
    ) -> Result<ConfidentValue, RuntimeError> {
        let mut env = Env::new();
        for (i, param) in decl.needs.iter().enumerate() {
            let val = args
                .get(i)
                .cloned()
                .unwrap_or(ConfidentValue::deterministic(Value::Unit));
            env.bind(&param.node.name, val);
        }

        // Set html_context if this task returns Html
        let prev_html = self.html_context.load(Ordering::Relaxed);
        if Self::returns_html_output(&decl.gives) {
            self.html_context.store(true, Ordering::Relaxed);
        }

        let result = match &decl.body.node {
            TaskBody::Do(stmts) => match self.exec_stmts(stmts, &mut env).await {
                Ok(val) => Ok(val),
                Err(RuntimeError::GiveSignal(val, ..)) => Ok(val),
                Err(e) => Err(e),
            },
            TaskBody::Is(expr) => self.eval_expr(expr, &mut env).await,
        };

        self.html_context.store(prev_html, Ordering::Relaxed);
        result
    }

    async fn call_pure(
        &self,
        decl: &PureDecl,
        args: Vec<ConfidentValue>,
    ) -> Result<ConfidentValue, RuntimeError> {
        let mut env = Env::new();
        for (i, param) in decl.needs.iter().enumerate() {
            let val = args
                .get(i)
                .cloned()
                .unwrap_or(ConfidentValue::deterministic(Value::Unit));
            env.bind(&param.node.name, val);
        }
        // Set html_context if this pure function returns Html
        let prev_html = self.html_context.load(Ordering::Relaxed);
        if Self::returns_html_output(&decl.gives) {
            self.html_context.store(true, Ordering::Relaxed);
        }
        let result = match self.exec_stmts(&decl.body, &mut env).await {
            Ok(val) => val,
            Err(RuntimeError::GiveSignal(val, ..)) => val,
            Err(e) => {
                self.html_context.store(prev_html, Ordering::Relaxed);
                return Err(e);
            }
        };
        self.html_context.store(prev_html, Ordering::Relaxed);
        // Pure functions always return deterministic confidence
        Ok(ConfidentValue::deterministic(result.value))
    }

    // ── Flow execution ────────────────────────────────────────────────────────

    async fn call_flow(
        &self,
        decl: &FlowDecl,
        args: Vec<ConfidentValue>,
    ) -> Result<ConfidentValue, RuntimeError> {
        use crate::planner::FlowPlanner;

        let graph = FlowPlanner::dependency_graph(decl)
            .map_err(|e| RuntimeError::FlowError(e.to_string()))?;
        let waves = FlowPlanner::execution_waves(&graph)
            .map_err(|e| RuntimeError::FlowError(e.to_string()))?;

        // Build stage lookup
        let stage_map: HashMap<String, StageDecl> = decl
            .stages
            .iter()
            .map(|s| (s.node.name.node.clone(), s.node.clone()))
            .collect();

        // Bind flow parameters
        let mut flow_args: HashMap<String, ConfidentValue> = HashMap::new();
        for (i, param) in decl.needs.iter().enumerate() {
            let val = args
                .get(i)
                .cloned()
                .unwrap_or(ConfidentValue::deterministic(Value::Unit));
            flow_args.insert(param.node.name.clone(), val);
        }

        let mut stage_outputs: HashMap<String, HashMap<String, ConfidentValue>> = HashMap::new();
        let mut last_result = ConfidentValue::deterministic(Value::Unit);

        if let Some(ref tracer) = self.tracer {
            tracer.flow_start(&decl.name.node, waves.len());
        }

        for (wave_idx, wave) in waves.iter().enumerate() {
            if let Some(ref tracer) = self.tracer {
                tracer.wave_start(wave_idx, wave);
            }

            if wave.len() == 1 {
                // Single stage — no spawn overhead
                let stage_name = &wave[0];
                let stage_decl = &stage_map[stage_name];
                let (bindings, give_val) = self
                    .execute_stage(stage_name, stage_decl, &flow_args, &stage_outputs)
                    .await?;
                if let Some(gv) = give_val {
                    last_result = gv;
                }
                stage_outputs.insert(stage_name.clone(), bindings);
            } else {
                // Multiple stages — run concurrently
                let mut handles = Vec::new();
                for stage_name in wave {
                    let stage_decl = stage_map[stage_name].clone();
                    let flow_args_clone = flow_args.clone();
                    let stage_outputs_clone = stage_outputs.clone();
                    let executor_clone = self.clone();
                    let name = stage_name.clone();

                    handles.push(tokio::spawn(async move {
                        let result = executor_clone
                            .execute_stage(
                                &name,
                                &stage_decl,
                                &flow_args_clone,
                                &stage_outputs_clone,
                            )
                            .await;
                        (name, result)
                    }));
                }

                for handle in handles {
                    let (name, result) = handle
                        .await
                        .map_err(|e| RuntimeError::FlowError(format!("stage join error: {}", e)))?;
                    let (bindings, give_val) = result?;
                    if let Some(gv) = give_val {
                        last_result = gv;
                    }
                    stage_outputs.insert(name, bindings);
                }
            }

            if let Some(ref tracer) = self.tracer {
                tracer.wave_complete(wave_idx);
            }
        }

        if let Some(ref tracer) = self.tracer {
            tracer.flow_complete(&decl.name.node);
        }

        Ok(last_result)
    }

    async fn execute_stage(
        &self,
        stage_name: &str,
        stage_decl: &StageDecl,
        flow_args: &HashMap<String, ConfidentValue>,
        stage_outputs: &HashMap<String, HashMap<String, ConfidentValue>>,
    ) -> Result<(HashMap<String, ConfidentValue>, Option<ConfidentValue>), RuntimeError> {
        if let Some(ref tracer) = self.tracer {
            tracer.stage_start(stage_name);
        }

        let mut env = Env::new();

        // Bind flow arguments
        for (name, val) in flow_args {
            env.bind(name, val.clone());
        }

        // Bind stage dependencies as Records
        let mut needed_stages: HashMap<String, Option<Vec<String>>> = HashMap::new();
        for needs_ref in &stage_decl.needs {
            let stage = &needs_ref.node.stage;
            match &needs_ref.node.field {
                NeedsRefField::Glob => {
                    needed_stages.insert(stage.clone(), None);
                }
                NeedsRefField::Named(field) => {
                    needed_stages
                        .entry(stage.clone())
                        .and_modify(|v| {
                            if let Some(fields) = v {
                                fields.push(field.clone());
                            }
                        })
                        .or_insert(Some(vec![field.clone()]));
                }
            }
        }

        for (dep_stage, fields_opt) in &needed_stages {
            let dep_bindings = stage_outputs.get(dep_stage).ok_or_else(|| {
                RuntimeError::FlowError(format!(
                    "stage '{}' output not available for stage '{}'",
                    dep_stage, stage_name
                ))
            })?;

            let record: HashMap<String, ConfidentValue> = match fields_opt {
                None => dep_bindings.clone(),
                Some(fields) => fields
                    .iter()
                    .filter_map(|f| dep_bindings.get(f).map(|v| (f.clone(), v.clone())))
                    .collect(),
            };

            env.bind(
                dep_stage,
                ConfidentValue::deterministic(Value::Record(record)),
            );
        }

        // Push scope so we can extract stage-produced bindings
        env.push_scope();

        // exec_stmts propagates GiveSignal as Err — catch it here at the
        // stage boundary so give values are captured correctly.
        let result = match self.exec_stmts(&stage_decl.body, &mut env).await {
            Ok(val) => val,
            Err(RuntimeError::GiveSignal(val, ..)) => val,
            Err(e) => return Err(e),
        };
        let give_value = match &result.value {
            Value::Unit => None,
            _ => Some(result),
        };

        let stage_bindings = env.top_scope_bindings();

        if let Some(ref tracer) = self.tracer {
            tracer.stage_complete(stage_name, give_value.is_some());
        }

        Ok((stage_bindings, give_value))
    }
}

// ── Helper functions ──────────────────────────────────────────────────────────

/// Convert an `InstanceInfo` into a deterministic `Value::Record` for the `find` expression.
fn instance_info_to_record(info: &InstanceInfo) -> ConfidentValue {
    let mut fields = HashMap::new();
    fields.insert(
        "id".to_string(),
        ConfidentValue::deterministic(Value::Text(info.instance_id.to_string())),
    );
    fields.insert(
        "name".to_string(),
        ConfidentValue::deterministic(Value::Text(info.agent_name.clone())),
    );
    fields.insert(
        "alias".to_string(),
        match &info.alias {
            Some(a) => ConfidentValue::deterministic(Value::Text(a.clone())),
            None => ConfidentValue::deterministic(Value::Unit),
        },
    );
    fields.insert(
        "status".to_string(),
        ConfidentValue::deterministic(Value::Text(match info.status {
            InstanceStatus::Running => "running".to_string(),
            InstanceStatus::Stopping => "stopping".to_string(),
        })),
    );
    fields.insert(
        "lifecycle".to_string(),
        match &info.lifecycle_state {
            Some(s) => ConfidentValue::deterministic(Value::Text(s.clone())),
            None => ConfidentValue::deterministic(Value::Unit),
        },
    );
    ConfidentValue::deterministic(Value::Record(fields))
}

fn truthy(cv: &ConfidentValue) -> bool {
    truthy_val(&cv.value)
}

fn truthy_val(val: &Value) -> bool {
    match val {
        Value::Bool(b) => *b,
        Value::Text(s) | Value::Html(s) => !s.is_empty(),
        Value::Number(n) => *n != 0.0,
        Value::Unit => false,
        Value::List(v) | Value::Array(v) => !v.is_empty(),
        Value::Record(m) => !m.is_empty(),
    }
}

fn values_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Text(a), Value::Text(b)) => a == b,
        (Value::Html(a), Value::Html(b)) => a == b,
        // Text/Html cross-comparison: same string content means equal
        (Value::Text(a), Value::Html(b)) | (Value::Html(a), Value::Text(b)) => a == b,
        (Value::Number(a), Value::Number(b)) => (a - b).abs() < f64::EPSILON,
        (Value::Bool(a), Value::Bool(b)) => a == b,
        (Value::Unit, Value::Unit) => true,
        _ => false,
    }
}

fn eval_binop(left: &Value, op: &BinOp, right: &Value) -> Result<Value, RuntimeError> {
    match (left, op, right) {
        // Arithmetic
        (Value::Number(a), BinOp::Add, Value::Number(b)) => Ok(Value::Number(a + b)),
        (Value::Number(a), BinOp::Sub, Value::Number(b)) => Ok(Value::Number(a - b)),
        (Value::Number(a), BinOp::Mul, Value::Number(b)) => Ok(Value::Number(a * b)),
        (Value::Number(a), BinOp::Div, Value::Number(b)) => {
            if *b == 0.0 {
                return Err(RuntimeError::DivisionByZero);
            }
            Ok(Value::Number(a / b))
        }
        // String concatenation (Text + Text → Text, Html + Html → Html, mixed → Text)
        (Value::Text(a), BinOp::Add, Value::Text(b)) => Ok(Value::Text(format!("{}{}", a, b))),
        (Value::Html(a), BinOp::Add, Value::Html(b)) => Ok(Value::Html(format!("{}{}", a, b))),
        (Value::Text(a), BinOp::Add, Value::Html(b)) => Ok(Value::Text(format!("{}{}", a, b))),
        (Value::Html(a), BinOp::Add, Value::Text(b)) => Ok(Value::Text(format!("{}{}", a, b))),
        // Array concatenation (Array + Array → Array, List variants too)
        (Value::Array(a), BinOp::Add, Value::Array(b)) => {
            let mut result = a.clone();
            result.extend(b.iter().cloned());
            Ok(Value::Array(result))
        }
        (Value::List(a), BinOp::Add, Value::List(b)) => {
            let mut result = a.clone();
            result.extend(b.iter().cloned());
            Ok(Value::List(result))
        }
        (Value::Array(a), BinOp::Add, Value::List(b))
        | (Value::List(a), BinOp::Add, Value::Array(b)) => {
            let mut result = a.clone();
            result.extend(b.iter().cloned());
            Ok(Value::Array(result))
        }
        // Numeric comparison
        (Value::Number(a), BinOp::Lt, Value::Number(b)) => Ok(Value::Bool(a < b)),
        (Value::Number(a), BinOp::Gt, Value::Number(b)) => Ok(Value::Bool(a > b)),
        (Value::Number(a), BinOp::Le, Value::Number(b)) => Ok(Value::Bool(a <= b)),
        (Value::Number(a), BinOp::Ge, Value::Number(b)) => Ok(Value::Bool(a >= b)),
        // Equality — works on all types
        (_, BinOp::Eq, _) => Ok(Value::Bool(values_equal(left, right))),
        (_, BinOp::Ne, _) => Ok(Value::Bool(!values_equal(left, right))),
        // Boolean logic
        (Value::Bool(a), BinOp::And, Value::Bool(b)) => Ok(Value::Bool(*a && *b)),
        (Value::Bool(a), BinOp::Or, Value::Bool(b)) => Ok(Value::Bool(*a || *b)),
        (_, BinOp::And, _) => Ok(Value::Bool(truthy_val(left) && truthy_val(right))),
        (_, BinOp::Or, _) => Ok(Value::Bool(truthy_val(left) || truthy_val(right))),
        _ => Err(RuntimeError::TypeError {
            expected: format!("compatible types for {:?}", op),
            got: format!("{} and {}", left, right),
        }),
    }
}

fn match_pattern(
    pattern: &Spanned<Pattern>,
    subject: &ConfidentValue,
) -> Option<Vec<(String, ConfidentValue)>> {
    match &pattern.node {
        Pattern::Wildcard => Some(vec![]),
        Pattern::Binding(name) => Some(vec![(name.clone(), subject.clone())]),
        Pattern::Constructor(name, sub_pats) => {
            match &subject.value {
                // Tagged record: { _type: "Winner", _value: { _0: val } }
                Value::Record(fields) => {
                    let type_match = fields
                        .get("_type")
                        .and_then(|t| match &t.value {
                            Value::Text(s) => Some(s.as_str()),
                            _ => None,
                        })
                        .map(|s| s == name)
                        .unwrap_or(false);
                    if !type_match {
                        return None;
                    }
                    let mut bindings = vec![];
                    if let Some(inner) = fields.get("_value") {
                        if let Value::Record(inner_fields) = &inner.value {
                            for (i, sub_pat) in sub_pats.iter().enumerate() {
                                let key = format!("_{}", i);
                                if let Some(val) = inner_fields.get(&key) {
                                    if let Some(mut sub_bindings) = match_pattern(sub_pat, val) {
                                        bindings.append(&mut sub_bindings);
                                    } else {
                                        return None;
                                    }
                                }
                            }
                        }
                    }
                    Some(bindings)
                }
                // Simple tag: Value::Text("Continue") matches Constructor("Continue")
                Value::Text(s) if s == name => Some(vec![]),
                _ => None,
            }
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::providers::mock::MockProvider;
    use crate::llm::registry::ProviderRegistry;
    use crate::parser;
    use crate::types::ConfidenceSource;

    async fn run_forge(source: &str) -> (Result<ConfidentValue, RuntimeError>, Vec<String>) {
        run_forge_with_mock(
            source,
            MockProvider::new("mock").with_default("mock response"),
        )
        .await
    }

    async fn run_forge_with_mock(
        source: &str,
        mock: MockProvider,
    ) -> (Result<ConfidentValue, RuntimeError>, Vec<String>) {
        let program = parser::parse(source).expect("parse failed");
        let mut registry = ProviderRegistry::new("mock");
        registry.register("mock", Arc::new(mock));
        let executor = TaskExecutor::new(program, Arc::new(registry), None);
        let result = executor.run().await;
        let outputs = executor.outputs();
        (result, outputs)
    }

    #[tokio::test]
    async fn test_say_template() {
        let (result, outputs) = run_forge(
            r#"
fn main
  say "hello world"
"#,
        )
        .await;
        assert!(result.is_ok());
        assert_eq!(outputs, vec!["hello world"]);
    }

    #[tokio::test]
    async fn test_bind_and_say() {
        let (result, outputs) = run_forge(
            r#"
fn main
  name = "FORGE"
  say "Hello, {name}!"
"#,
        )
        .await;
        assert!(result.is_ok());
        assert_eq!(outputs, vec!["Hello, FORGE!"]);
    }

    #[tokio::test]
    async fn test_template_escapes_render_as_control_characters() {
        let (result, outputs) = run_forge(
            r#"
fn main
  language = "rust"
  say "Line 1\nLanguage: {language}\tend\\"
"#,
        )
        .await;
        assert!(result.is_ok());
        assert_eq!(outputs, vec!["Line 1\nLanguage: rust\tend\\"]);
    }

    #[tokio::test]
    async fn test_give_returns_value() {
        let (result, _) = run_forge("fn main\n  give 42\n").await;
        let val = result.unwrap();
        assert!(matches!(val.value, Value::Number(n) if (n - 42.0).abs() < f64::EPSILON));
    }

    #[tokio::test]
    async fn test_binop_arithmetic() {
        let (result, _) = run_forge(
            r#"
fn main
  x = 10 + 5
  y = x * 2
  give y
"#,
        )
        .await;
        let val = result.unwrap();
        assert!(matches!(val.value, Value::Number(n) if (n - 30.0).abs() < f64::EPSILON));
    }

    #[tokio::test]
    async fn test_binop_comparison() {
        let (result, _) = run_forge("fn main\n  give 5 > 3\n").await;
        assert!(matches!(result.unwrap().value, Value::Bool(true)));
    }

    #[tokio::test]
    async fn test_unary_neg() {
        let (result, _) = run_forge("fn main\n  give -42\n").await;
        let val = result.unwrap();
        assert!(matches!(val.value, Value::Number(n) if (n - -42.0).abs() < f64::EPSILON));
    }

    #[tokio::test]
    async fn test_unary_not() {
        let (result, _) = run_forge("fn main\n  give not true\n").await;
        assert!(matches!(result.unwrap().value, Value::Bool(false)));
    }

    #[tokio::test]
    async fn test_undefined_variable_error() {
        let (result, _) = run_forge("fn main\n  say foo\n").await;
        assert!(matches!(
            result,
            Err(RuntimeError::UndefinedVariable { .. })
        ));
    }

    #[tokio::test]
    async fn test_no_main_error() {
        let (result, _) = run_forge(
            r#"
task greet
  needs name: Text
  gives Text
  do
    say "hello"
"#,
        )
        .await;
        assert!(matches!(result, Err(RuntimeError::NoMainFunction)));
    }

    #[tokio::test]
    async fn test_array_literal() {
        let (result, _) = run_forge("fn main\n  give [1, 2, 3]\n").await;
        assert!(matches!(result.unwrap().value, Value::Array(ref v) if v.len() == 3));
    }

    #[tokio::test]
    async fn test_if_else_true_branch() {
        // fn main: i1=2sp for stmts, if body at i3=6sp, else at i2=4sp
        let src =
            "fn main\n  x = 10\n  if x > 5\n      say \"big\"\n    else\n      say \"small\"\n";
        let (_, outputs) = run_forge(src).await;
        assert_eq!(outputs, vec!["big"]);
    }

    #[tokio::test]
    async fn test_if_else_false_branch() {
        let src =
            "fn main\n  x = 2\n  if x > 5\n      say \"big\"\n    else\n      say \"small\"\n";
        let (_, outputs) = run_forge(src).await;
        assert_eq!(outputs, vec!["small"]);
    }

    #[tokio::test]
    async fn test_for_loop() {
        // for body at i3=6sp
        let src = "fn main\n  items = [1, 2, 3]\n  for item in items\n      say \"{item}\"\n";
        let (_, outputs) = run_forge(src).await;
        assert_eq!(outputs, vec!["1", "2", "3"]);
    }

    #[tokio::test]
    async fn test_match_wildcard() {
        // match arms at i3=6sp
        let src = "fn main\n  x = \"hello\"\n  match x\n      _ -> say \"matched\"\n";
        let (_, outputs) = run_forge(src).await;
        assert_eq!(outputs, vec!["matched"]);
    }

    #[tokio::test]
    async fn test_match_binding() {
        let src = "fn main\n  x = \"hello\"\n  match x\n      val -> say val\n";
        let (_, outputs) = run_forge(src).await;
        assert_eq!(outputs, vec!["hello"]);
    }

    #[tokio::test]
    async fn test_task_call() {
        let (_, outputs) = run_forge(
            r#"
task greet
  needs name: Text
  gives Text
  do
    give "Hello, {name}!"

fn main
  result = greet("World")
  say result
"#,
        )
        .await;
        assert_eq!(outputs, vec!["Hello, World!"]);
    }

    #[tokio::test]
    async fn test_pure_call_deterministic() {
        let src = "pure add\n  needs a: Number, b: Number\n  gives Number\n  do\n    give a + b\n\nfn main\n  give add(3, 4)\n";
        let (result, _) = run_forge(src).await;
        let val = result.unwrap();
        assert!(matches!(val.value, Value::Number(n) if (n - 7.0).abs() < f64::EPSILON));
        assert_eq!(val.confidence, 1.0);
        assert!(matches!(val.source, ConfidenceSource::Deterministic));
    }

    #[tokio::test]
    async fn test_reason_mock() {
        let mock = MockProvider::new("mock").with_default("The answer is 42.");
        let (result, _) = run_forge_with_mock(
            r#"
fn main
  give reason "What is the meaning of life?"
"#,
            mock,
        )
        .await;
        let val = result.unwrap();
        assert!(matches!(val.value, Value::Text(ref s) if s == "The answer is 42."));
        assert!(val.confidence > 0.0);
        assert!(matches!(val.source, ConfidenceSource::LLMDirect(_)));
    }

    #[tokio::test]
    async fn test_when_sure_branch() {
        let mock = MockProvider::new("mock").with_default("support");
        let (_, outputs) = run_forge_with_mock(
            r#"
use
  llm.classify

task classify_intent
  needs message: Text
  gives Text
  do
    result = classify message into ["buy", "support", "cancel"]
    when result.sure -> give result
    when result.unsure -> give "unclear"
    else -> give "unknown"

fn main
  out = classify_intent("help me")
  say out
"#,
            mock,
        )
        .await;
        assert_eq!(outputs, vec!["support"]);
    }

    #[tokio::test]
    async fn test_when_else_branch() {
        let mock = MockProvider::new("mock")
            .with_default("I'm not sure, I think it might be possibly something, I don't know, it depends, unclear");
        let (_, outputs) = run_forge_with_mock(
            r#"
use
  llm.classify

task classify_intent
  needs message: Text
  gives Text
  do
    result = classify message into ["buy", "support", "cancel"]
    when result.sure -> give result
    else -> give "unknown"

fn main
  out = classify_intent("help")
  say out
"#,
            mock,
        )
        .await;
        assert_eq!(outputs, vec!["unknown"]);
    }

    #[tokio::test]
    async fn test_string_equality() {
        let src = "fn main\n  x = \"hello\"\n  if x == \"hello\"\n      say \"yes\"\n    else\n      say \"no\"\n";
        let (_, outputs) = run_forge(src).await;
        assert_eq!(outputs, vec!["yes"]);
    }

    #[tokio::test]
    async fn test_boolean_logic() {
        let (result, _) = run_forge("fn main\n  give true and false\n").await;
        assert!(matches!(result.unwrap().value, Value::Bool(false)));
    }

    #[tokio::test]
    async fn test_division_by_zero() {
        let (result, _) = run_forge("fn main\n  give 1 / 0\n").await;
        assert!(matches!(result, Err(RuntimeError::DivisionByZero)));
    }

    #[tokio::test]
    async fn test_text_concat() {
        let (result, _) = run_forge(
            r#"fn main
  give "hello" + " " + "world"
"#,
        )
        .await;
        assert!(matches!(result.unwrap().value, Value::Text(ref s) if s == "hello world"));
    }

    #[tokio::test]
    async fn test_try_or() {
        let (result, _) = run_forge("fn main\n  give try undefined_var or 42\n").await;
        let val = result.unwrap();
        assert!(matches!(val.value, Value::Number(n) if (n - 42.0).abs() < f64::EPSILON));
    }

    // ── Flow tests ───────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_flow_single_stage() {
        let (result, outputs) = run_forge(
            r#"
flow greetflow
  needs name: Text
  gives Text

  stage greet
    give "Hello, {name}!"

fn main
  result = greetflow("World")
  say result
"#,
        )
        .await;
        assert!(result.is_ok());
        assert_eq!(outputs, vec!["Hello, World!"]);
    }

    #[tokio::test]
    async fn test_flow_multi_wave_with_deps() {
        let (result, outputs) = run_forge(
            r#"
flow pipeline
  needs input: Text
  gives Text

  stage first
    msg = "{input} processed"

  stage second
    needs first.*
    give "{first.msg} and refined"

fn main
  result = pipeline("data")
  say result
"#,
        )
        .await;
        assert!(result.is_ok());
        assert_eq!(outputs, vec!["data processed and refined"]);
    }

    #[tokio::test]
    async fn test_flow_parallel_independent_stages() {
        let (result, _) = run_forge(
            r#"
flow parallel
  needs x: Text
  gives Text

  stage a
    val = "A:{x}"

  stage b
    val = "B:{x}"

  stage combine
    needs a.val, b.val
    give "{a.val}+{b.val}"

fn main
  give parallel("test")
"#,
        )
        .await;
        let val = result.unwrap();
        assert!(matches!(val.value, Value::Text(ref s) if s == "A:test+B:test"));
    }

    #[tokio::test]
    async fn test_flow_with_llm_reason() {
        let mock = MockProvider::new("mock")
            .with_response("synthesize", "synthesis result")
            .with_default("other response");
        let (result, outputs) = run_forge_with_mock(
            r#"
flow analyze
  needs topic: Text
  gives Text

  stage gather
    info = "facts about {topic}"

  stage synthesize
    needs gather.*
    give reason "synthesize {gather.info}"

fn main
  result = analyze("AI")
  say result
"#,
            mock,
        )
        .await;
        assert!(result.is_ok());
        assert_eq!(outputs, vec!["synthesis result"]);
    }

    #[tokio::test]
    async fn test_flow_cycle_detection() {
        let (result, _) = run_forge(
            r#"
flow cyclic
  needs x: Text
  gives Text

  stage a
    needs b.*
    val = "a"

  stage b
    needs a.*
    val = "b"

fn main
  give cyclic("test")
"#,
        )
        .await;
        assert!(matches!(result, Err(RuntimeError::FlowError(ref msg)) if msg.contains("cycle")));
    }

    // ── split / join tests ──────────────────────────────────────────

    #[tokio::test]
    async fn test_split_basic() {
        let (result, outputs) = run_forge(
            r#"
fn main
  text = "a,b,c"
  parts = text.split(",")
  say parts.length
  for p in parts
      say p
"#,
        )
        .await;
        assert!(result.is_ok(), "split failed: {:?}", result.err());
        assert_eq!(outputs, vec!["3", "a", "b", "c"]);
    }

    #[tokio::test]
    async fn test_split_newline() {
        let (result, outputs) = run_forge(
            r#"
fn main
  text = "line one
line two
line three"
  parts = text.split("\n")
  say parts.length
"#,
        )
        .await;
        assert!(result.is_ok(), "split newline failed: {:?}", result.err());
        assert_eq!(outputs, vec!["3"]);
    }

    #[tokio::test]
    async fn test_split_no_match() {
        let (result, outputs) = run_forge(
            r#"
fn main
  text = "no delimiter here"
  parts = text.split(",")
  say parts.length
"#,
        )
        .await;
        assert!(result.is_ok(), "split no match failed: {:?}", result.err());
        assert_eq!(outputs, vec!["1"]);
    }

    #[tokio::test]
    async fn test_join_basic() {
        let (result, outputs) = run_forge(
            r#"
fn main
  text = "a,b,c"
  parts = text.split(",")
  joined = parts.join(" | ")
  say joined
"#,
        )
        .await;
        assert!(result.is_ok(), "join failed: {:?}", result.err());
        assert_eq!(outputs, vec!["a | b | c"]);
    }

    #[tokio::test]
    async fn test_split_join_roundtrip() {
        let (result, outputs) = run_forge(
            r#"
fn main
  original = "hello world foo"
  parts = original.split(" ")
  back = parts.join(" ")
  say back
"#,
        )
        .await;
        assert!(result.is_ok(), "roundtrip failed: {:?}", result.err());
        assert_eq!(outputs, vec!["hello world foo"]);
    }
}
