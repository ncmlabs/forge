// FORGE task executor
// Walks the AST, evaluates expressions, dispatches on confidence predicates,
// calls LLM providers. See issue #9.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::ast::*;
use crate::llm::registry::ProviderRegistry;
use crate::llm::CompletionRequest;
use crate::runtime::agent::{AgentContext, EmittedEvent};
use crate::runtime::confidence::{ConfidentValue, Value};
use crate::runtime::instance_registry::{InstanceInfo, InstanceStatus};
use crate::runtime::timer_engine::TimerEngine;
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
}

/// Result from executing an endpoint, including optional response metadata.
#[derive(Debug, Clone)]
pub struct EndpointResult {
    pub value: ConfidentValue,
    pub status: Option<u16>,
    pub content_type: Option<String>,
}

// ── Environment (scope stack) ─────────────────────────────────────────────────

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

#[derive(Clone)]
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
            tracer,
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
        }
    }

    /// Attach a ForgeConfig for system runtime configuration.
    pub fn with_config(mut self, config: crate::config::ForgeConfig) -> Self {
        self.config = Some(config);
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
        self.event_bus = Some(bus);
        self
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

        let mut env = Env::new();
        if let Some(req) = request {
            env.bind("request", req);
        }
        for (k, v) in args {
            env.bind(&k, v);
        }

        match self.exec_stmts(&endpoint.body, &mut env).await {
            Ok(val) => Ok(EndpointResult {
                value: val,
                status: None,
                content_type: None,
            }),
            Err(RuntimeError::GiveSignal(val, status, content_type)) => Ok(EndpointResult {
                value: val,
                status,
                content_type,
            }),
            Err(e) => Err(e),
        }
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
                                if let Value::Text(s) = &meta_val.value {
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
                    if let Some(ref ctx_arc) = self.agent_context {
                        let mut arg_vals = Vec::new();
                        let mut fields = std::collections::HashMap::new();
                        for arg in args {
                            let val = self.eval_expr(&arg.node.value, env).await?;
                            if let Some(ref label) = arg.node.label {
                                fields.insert(label.node.clone(), val.clone());
                            }
                            arg_vals.push(val);
                        }
                        ctx_arc
                            .lock()
                            .unwrap()
                            .event_sink
                            .emitted
                            .push(EmittedEvent {
                                name: name.node.clone(),
                                args: arg_vals,
                                fields,
                            });
                    } else {
                        return Err(RuntimeError::Unsupported("emit outside agent".into()));
                    }
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
                        // Check knowledge store exists (quick lock/unlock)
                        {
                            let ctx = ctx_arc.lock().unwrap();
                            if ctx.knowledge_store.is_none() {
                                return Err(RuntimeError::Unsupported(
                                    "learn requires agent with knowledge store".into(),
                                ));
                            }
                        }

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
                                let mut ctx = ctx_arc.lock().unwrap();
                                if let Some(ref mut ks) = ctx.knowledge_store {
                                    if let Some(ref cat) = category {
                                        ks.learn_direct_categorized(&text, cat);
                                    } else {
                                        ks.learn_direct(&text);
                                    }
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
                                let mut ctx = ctx_arc.lock().unwrap();
                                if let Some(ref mut ks) = ctx.knowledge_store {
                                    if let Some(ref cat) = category {
                                        ks.learn_from_interaction_categorized(
                                            &question, &answer, confidence, cat,
                                        );
                                    } else {
                                        ks.learn_from_interaction(&question, &answer, confidence);
                                    }
                                }
                            }
                            LearnSource::FromDocument(expr) => {
                                let val = self.eval_expr(expr, env).await?;
                                let path = format!("{}", val.value);
                                let mut ctx = ctx_arc.lock().unwrap();
                                if let Some(ref mut ks) = ctx.knowledge_store {
                                    if let Some(ref cat) = category {
                                        ks.learn_from_document_categorized(&path, cat)
                                            .map_err(RuntimeError::Unsupported)?;
                                    } else {
                                        ks.learn_from_document(&path)
                                            .map_err(RuntimeError::Unsupported)?;
                                    }
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
                            TopLevel::States(s) => {
                                // Collect states decls for lifecycle lookup
                                if states_decl.is_none() {
                                    if let Some(ref ad) = agent_decl {
                                        if ad.lifecycle.as_ref().map(|l| &l.node)
                                            == Some(&s.name.node)
                                        {
                                            states_decl = Some(s.clone());
                                        }
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
                        }
                    }

                    // 4. Create the child agent process
                    let mut child_process = crate::runtime::agent::AgentProcess::new(
                        agent_decl,
                        states_decl.as_ref(),
                        self.providers.clone(),
                        self.tracer.clone(),
                        self.program.clone(),
                        self.storage.clone(),
                        self.instance_registry.clone(),
                    );

                    // 5. Transfer knowledge from parent to child
                    if let Some(ref cat) = knowledge_category {
                        // Get filtered entries from parent's knowledge store
                        let mut transferred_entries = Vec::new();
                        if let Some(ref ctx_arc) = self.agent_context {
                            let ctx = ctx_arc.lock().unwrap();
                            if let Some(ref ks) = ctx.knowledge_store {
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
                            let mut ctx = child_ctx.lock().unwrap();
                            if let Some(ref mut ks) = ctx.knowledge_store {
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

                    // 7. Wire event bus
                    if let Some(ref bus) = self.event_bus {
                        child_process = child_process.with_event_bus(bus.clone()).await;
                    }

                    // 8. Register in instance registry
                    let alias = if instance_name != *template_name {
                        Some(instance_name.as_str())
                    } else {
                        None
                    };
                    let instance_id = if let Some(ref ir) = self.instance_registry {
                        let id = ir.write().await.register(template_name, alias);
                        Some(id)
                    } else {
                        None
                    };

                    // 9. Spawn as tokio task
                    tokio::spawn(async move {
                        let _ = child_process.run().await;
                    });

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
                            let ctx = ctx_arc.lock().unwrap();
                            if let Some(ref ks) = ctx.knowledge_store {
                                let entries = ks.export_entries();
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

                    // 3. Unregister from instance registry
                    if let Some(ref ir) = self.instance_registry {
                        let mut registry = ir.write().await;
                        if let Some(ref alias) = target_alias {
                            // Retire a specific instance by alias
                            if let Some(info) = registry.find_by_alias(alias) {
                                registry.unregister(&info.instance_id);
                            }
                        }
                        // If no target, self-retirement — unregister happens via
                        // the RetireSignal propagation in the agent event loop.
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
                    let mut result = String::new();
                    let mut min_conf: f32 = 1.0;
                    for part in parts {
                        match &part.node {
                            TemplatePart::Text(s) => result.push_str(s),
                            TemplatePart::Interp(inner_expr) => {
                                let val = self.eval_expr(inner_expr, env).await?;
                                min_conf = min_conf.min(val.confidence);
                                result.push_str(&format!("{}", val.value));
                            }
                        }
                    }
                    if min_conf >= 1.0 {
                        Ok(ConfidentValue::deterministic(Value::Text(result)))
                    } else {
                        Ok(ConfidentValue::derived(Value::Text(result), min_conf))
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
                        (Value::Array(v) | Value::List(v), "len" | "count") => {
                            Ok(ConfidentValue::deterministic(Value::Number(v.len() as f64)))
                        }
                        // Record field access
                        (Value::Record(fields), _) => {
                            fields.get(&field.node).cloned().ok_or_else(|| {
                                RuntimeError::UndefinedVariable {
                                    name: format!(".{}", field.node),
                                }
                            })
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
                                Value::Text(s) => s.len(),
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
                                Value::Text(s) => {
                                    if let Value::Text(needle_s) = &needle.value {
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
                            let text = match &obj.value {
                                Value::Text(s) => s.to_lowercase(),
                                _ => {
                                    return Err(RuntimeError::TypeError {
                                        expected: "Text".to_string(),
                                        got: format!("{}", obj.value),
                                    })
                                }
                            };
                            Ok(ConfidentValue::derived(Value::Text(text), obj.confidence))
                        }
                        "upper" => {
                            let text = match &obj.value {
                                Value::Text(s) => s.to_uppercase(),
                                _ => {
                                    return Err(RuntimeError::TypeError {
                                        expected: "Text".to_string(),
                                        got: format!("{}", obj.value),
                                    })
                                }
                            };
                            Ok(ConfidentValue::derived(Value::Text(text), obj.confidence))
                        }
                        "trim" => {
                            let text = match &obj.value {
                                Value::Text(s) => s.trim().to_string(),
                                _ => {
                                    return Err(RuntimeError::TypeError {
                                        expected: "Text".to_string(),
                                        got: format!("{}", obj.value),
                                    })
                                }
                            };
                            Ok(ConfidentValue::derived(Value::Text(text), obj.confidence))
                        }
                        other => Err(RuntimeError::Unsupported(format!("method .{}()", other))),
                    }
                }

                Expr::TypeAccess(_type_name, variant) => Ok(ConfidentValue::deterministic(
                    Value::Text(variant.node.clone()),
                )),

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
                    Ok(ConfidentValue::deterministic(Value::Record(fields)))
                }

                // ── LLM expressions ───────────────────────────────────────────
                Expr::Reason(prompt_expr) => {
                    let prompt = self.eval_expr(prompt_expr, env).await?;
                    let prompt_text = format!("{}", prompt.value);

                    if let Some(ref tracer) = self.tracer {
                        tracer.llm_request("reason", &prompt_text);
                    }

                    let request = CompletionRequest::simple(&prompt_text);
                    let response = self
                        .providers
                        .resolve_and_complete(request, None)
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
                    let response = self
                        .providers
                        .resolve_and_complete(request, None)
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
                        });
                    }

                    Ok(ConfidentValue::from_llm(
                        Value::Text(response.content.trim().to_string()),
                        confidence,
                    ))
                }

                Expr::Search(_) => Ok(ConfidentValue::deterministic(Value::List(vec![]))),

                Expr::Recall(query_expr) => {
                    let query_val = self.eval_expr(query_expr, env).await?;
                    let query_text = format!("{}", query_val.value);

                    if let Some(ref ctx_arc) = self.agent_context {
                        let mut ctx = ctx_arc.lock().unwrap();
                        if let Some(ref mut ks) = ctx.knowledge_store {
                            // Default token budget for recall: 2000 tokens
                            let result = ks.recall(&query_text, 2000);
                            Ok(result)
                        } else {
                            Err(RuntimeError::Unsupported(
                                "recall requires agent with knowledge store".into(),
                            ))
                        }
                    } else {
                        Err(RuntimeError::Unsupported("recall outside agent".into()))
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

        match &decl.body.node {
            TaskBody::Do(stmts) => match self.exec_stmts(stmts, &mut env).await {
                Ok(val) => Ok(val),
                Err(RuntimeError::GiveSignal(val, ..)) => Ok(val),
                Err(e) => Err(e),
            },
            TaskBody::Is(expr) => self.eval_expr(expr, &mut env).await,
        }
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
        let result = match self.exec_stmts(&decl.body, &mut env).await {
            Ok(val) => val,
            Err(RuntimeError::GiveSignal(val, ..)) => val,
            Err(e) => return Err(e),
        };
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
        Value::Text(s) => !s.is_empty(),
        Value::Number(n) => *n != 0.0,
        Value::Unit => false,
        Value::List(v) | Value::Array(v) => !v.is_empty(),
        Value::Record(m) => !m.is_empty(),
    }
}

fn values_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Text(a), Value::Text(b)) => a == b,
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
        // String concatenation
        (Value::Text(a), BinOp::Add, Value::Text(b)) => Ok(Value::Text(format!("{}{}", a, b))),
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
}
