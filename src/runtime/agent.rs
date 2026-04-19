// FORGE agent process — issue #11
// Stateful agents with memory, handler dispatch, stuck detection,
// state machines, timers, events, and requires guards.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;

use crate::ast::*;
use crate::llm::registry::ProviderRegistry;
use crate::runtime::confidence::{ConfidentValue, Value};
use crate::runtime::event_bus::{EventPayload, SharedEventBus};
use crate::runtime::executor::{Env, RuntimeError, TaskExecutor};
use crate::runtime::knowledge_store::{KnowledgeStore, SharedKnowledgeStore};
use crate::runtime::memory::AgentMemory;
use crate::runtime::state_machine::StateMachine;
use crate::runtime::timer_engine::{TimerEngine, TimerFired};
use crate::tracer::Tracer;

// ── Timer Manager ────────────────────────────────────────────────────────────

/// Timer state for synchronous tracking. Issue #20 adds async tokio timers.
#[derive(Debug, Clone, PartialEq)]
pub enum TimerState {
    Idle,
    Running,
    Expired,
}

/// Basic timer state manager.
#[derive(Debug, Clone)]
pub struct TimerManager {
    timers: HashMap<String, TimerState>,
}

impl TimerManager {
    pub fn new(timer_fields: &[Spanned<TimerField>]) -> Self {
        let mut timers = HashMap::new();
        for tf in timer_fields {
            timers.insert(tf.node.name.node.clone(), TimerState::Idle);
        }
        Self { timers }
    }

    pub fn empty() -> Self {
        Self {
            timers: HashMap::new(),
        }
    }

    pub fn start(&mut self, name: &str) -> Result<(), RuntimeError> {
        if let Some(state) = self.timers.get_mut(name) {
            *state = TimerState::Running;
            Ok(())
        } else {
            Err(RuntimeError::Unsupported(format!(
                "unknown timer: {}",
                name
            )))
        }
    }

    pub fn cancel(&mut self, name: &str) -> Result<(), RuntimeError> {
        if let Some(state) = self.timers.get_mut(name) {
            *state = TimerState::Idle;
            Ok(())
        } else {
            Err(RuntimeError::Unsupported(format!(
                "unknown timer: {}",
                name
            )))
        }
    }

    pub fn reset(&mut self, name: &str) -> Result<(), RuntimeError> {
        if let Some(state) = self.timers.get_mut(name) {
            *state = TimerState::Running;
            Ok(())
        } else {
            Err(RuntimeError::Unsupported(format!(
                "unknown timer: {}",
                name
            )))
        }
    }

    pub fn state(&self, name: &str) -> Option<&TimerState> {
        self.timers.get(name)
    }

    /// Return all timer names and their current states (for introspection).
    pub fn all_states(&self) -> &HashMap<String, TimerState> {
        &self.timers
    }

    /// Mark a timer as expired (for testing and future async integration).
    pub fn expire(&mut self, name: &str) {
        if let Some(state) = self.timers.get_mut(name) {
            *state = TimerState::Expired;
        }
    }
}

// ── Event Sink ───────────────────────────────────────────────────────────────

/// A single emitted event with named fields for filter access.
#[derive(Debug, Clone)]
pub struct EmittedEvent {
    pub name: String,
    pub args: Vec<ConfidentValue>,
    pub fields: HashMap<String, ConfidentValue>,
}

/// Collects side effects (emitted events, escalations, forwards) for testing
/// and wiring to EventBus (issue #19).
#[derive(Debug, Clone, Default)]
pub struct EventSink {
    pub emitted: Vec<EmittedEvent>,
    pub escalations: Vec<String>,
    pub forwards: Vec<(ConfidentValue, ConfidentValue)>,
}

impl EventSink {
    pub fn new() -> Self {
        Self::default()
    }
}

// ── Warden Signal ───────────────────────────────────────────────────────────

/// Signal from a managed agent to its warden.
#[derive(Debug, Clone)]
pub enum AgentSignal {
    Stuck {
        agent_name: String,
    },
    Timeout {
        agent_name: String,
    },
    Hallucination {
        agent_name: String,
        detail: String,
    },
    Contradiction {
        agent_name: String,
        detail: String,
        severity: String,
    },
    BudgetExceeded {
        agent_name: String,
        detail: String,
    },
    Crash {
        agent_name: String,
    },
}

// ── Stuck Detector ───────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct TurnRecord {
    pub response_text: String,
    pub confidence: f32,
    pub memory_hash: u64,
}

/// Tracks last N turns to detect stuck loops.
#[derive(Debug, Clone)]
pub struct StuckDetector {
    history: Vec<TurnRecord>,
    threshold: usize,
}

impl StuckDetector {
    pub fn new(threshold: usize) -> Self {
        Self {
            history: Vec::new(),
            threshold,
        }
    }

    pub fn record_turn(&mut self, record: TurnRecord) {
        self.history.push(record);
        if self.history.len() > self.threshold * 2 {
            self.history
                .drain(..self.history.len() - self.threshold * 2);
        }
    }

    /// Returns true if the agent appears stuck.
    /// Stuck = last N turns have high similarity OR low confidence OR unchanged memory.
    pub fn is_stuck(&self) -> bool {
        if self.history.len() < self.threshold {
            return false;
        }
        let recent = &self.history[self.history.len() - self.threshold..];

        // Check 1: all responses highly similar (Jaccard > 0.8)
        let all_similar = recent
            .windows(2)
            .all(|pair| jaccard_similarity(&pair[0].response_text, &pair[1].response_text) > 0.8);

        // Check 2: average confidence below 0.5
        let avg_conf = recent.iter().map(|t| t.confidence).sum::<f32>() / recent.len() as f32;
        let low_confidence = avg_conf < 0.5;

        // Check 3: memory unchanged across all recent turns
        let memory_unchanged = recent
            .windows(2)
            .all(|pair| pair[0].memory_hash == pair[1].memory_hash);

        all_similar || low_confidence || (memory_unchanged && self.history.len() >= self.threshold)
    }

    /// Returns true if recent responses indicate hallucination.
    /// Hallucination = last N turns ALL have very low confidence (< 0.3).
    pub fn is_hallucinating(&self) -> bool {
        if self.history.len() < self.threshold {
            return false;
        }
        let recent = &self.history[self.history.len() - self.threshold..];
        recent.iter().all(|t| t.confidence < 0.3)
    }

    pub fn clear(&mut self) {
        self.history.clear();
    }
}

/// Jaccard similarity on word tokens.
pub(crate) fn jaccard_similarity(a: &str, b: &str) -> f64 {
    use std::collections::HashSet;
    let set_a: HashSet<&str> = a.split_whitespace().collect();
    let set_b: HashSet<&str> = b.split_whitespace().collect();
    if set_a.is_empty() && set_b.is_empty() {
        return 1.0;
    }
    let intersection = set_a.intersection(&set_b).count();
    let union = set_a.union(&set_b).count();
    if union == 0 {
        1.0
    } else {
        intersection as f64 / union as f64
    }
}

// ── Agent Context ────────────────────────────────────────────────────────────

/// Runtime context for an agent, shared with the executor via `Arc<Mutex<_>>`.
#[derive(Debug, Clone)]
pub struct AgentContext {
    pub memory: AgentMemory,
    pub knowledge_store: Option<SharedKnowledgeStore>,
    pub state_machine: Option<StateMachine>,
    pub timer_manager: TimerManager,
    pub event_sink: EventSink,
    pub stuck_detector: StuckDetector,
}

impl AgentContext {
    pub fn new(
        memory: AgentMemory,
        knowledge_store: Option<SharedKnowledgeStore>,
        state_machine: Option<StateMachine>,
        timer_manager: TimerManager,
        stuck_threshold: usize,
    ) -> Self {
        Self {
            memory,
            knowledge_store,
            state_machine,
            timer_manager,
            event_sink: EventSink::new(),
            stuck_detector: StuckDetector::new(stuck_threshold),
        }
    }
}

// ── Agent Process ────────────────────────────────────────────────────────────

/// Runtime agent process: holds declaration, context, and executor.
pub struct AgentProcess {
    pub decl: AgentDecl,
    context: Arc<Mutex<AgentContext>>,
    executor: TaskExecutor,
    event_bus: Option<SharedEventBus>,
    event_receivers: Vec<(Option<Spanned<Expr>>, mpsc::Receiver<EventPayload>)>,
    timer_engine: Arc<Mutex<TimerEngine>>,
    pub timer_rx: mpsc::Receiver<TimerFired>,
    warden_tx: Option<mpsc::Sender<AgentSignal>>,
    storage: Option<crate::runtime::storage::SharedStorage>,
    /// Worktree branch name for sandbox cleanup on agent exit (issue #194).
    worktree_branch: Option<String>,
}

impl AgentProcess {
    /// Create a new agent process from its declaration.
    ///
    /// If `shared_knowledge_store` is provided, the agent uses that instance
    /// instead of creating its own. This allows the executor and agent to
    /// share the same KnowledgeStore so that `learn` and `recall` operate
    /// on the same data (fixes #309).
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        decl: AgentDecl,
        states: Option<&StatesDecl>,
        registry: Arc<ProviderRegistry>,
        tracer: Option<Tracer>,
        program: Program,
        storage: Option<crate::runtime::storage::SharedStorage>,
        instance_registry: Option<crate::runtime::instance_registry::SharedInstanceRegistry>,
        shared_knowledge_store: Option<SharedKnowledgeStore>,
    ) -> Self {
        let mut memory = AgentMemory::new(&decl.memory);

        // Load memory from storage if available
        // For persistent agents: always load (ACID guarantee, issue #57)
        // For non-persistent agents: load if storage provided (CLI mode persistence)
        if let Some(ref store) = storage {
            let key = format!("agent:{}:memory", decl.name.node);
            if let Ok(Some(json)) = store.get(&key) {
                let _ = memory.restore_from_json(&json);
            }
        }
        let state_machine = states.map(|s| {
            let mut sm = StateMachine::new(s);
            // Restore lifecycle state from persistent storage if available
            if let Some(ref store) = storage {
                let key = format!("agent:{}:lifecycle", decl.name.node);
                if let Ok(Some(saved_state)) = store.get(&key) {
                    sm.set_current(&saved_state);
                }
            }
            sm
        });
        let timer_manager = TimerManager::new(&decl.timers);
        let stuck_threshold = decl
            .stuck_policy
            .as_ref()
            .and_then(|sp| sp.node.turns)
            .unwrap_or(3) as usize;

        // Use pre-created shared knowledge store if provided; otherwise create from declaration
        let knowledge_store: Option<SharedKnowledgeStore> =
            if let Some(shared) = shared_knowledge_store {
                Some(shared)
            } else {
                decl.knowledge.as_ref().map(|kd| {
                    let store_path = match &kd.node.store_path.node {
                        Expr::Template(parts) => {
                            // Extract plain text from template (no interpolation at init time)
                            parts
                                .iter()
                                .filter_map(|p| match &p.node {
                                    TemplatePart::Text(t) => Some(t.as_str()),
                                    _ => None,
                                })
                                .collect::<String>()
                        }
                        _ => ".forge-knowledge/default".to_string(),
                    };
                    let max_entries = kd.node.max_entries.as_ref().map(|m| m.node as usize);
                    let retention_days = kd.node.retention.as_ref().map(|r| {
                        let dur = &r.node;
                        match dur.unit {
                            DurationUnit::Days => dur.value,
                            DurationUnit::Hours => dur.value / 24,
                            DurationUnit::Minutes => dur.value / (24 * 60),
                            DurationUnit::Seconds => dur.value / (24 * 60 * 60),
                        }
                    });
                    Arc::new(Mutex::new(KnowledgeStore::new(
                        &store_path,
                        max_entries,
                        retention_days,
                    )))
                })
            };

        let context = Arc::new(Mutex::new(AgentContext::new(
            memory,
            knowledge_store,
            state_machine,
            timer_manager,
            stuck_threshold,
        )));

        // Async timer engine (issue #20)
        let (fire_tx, timer_rx) = mpsc::channel::<TimerFired>(64);
        let timer_engine = Arc::new(Mutex::new(TimerEngine::new(
            &decl.name.node,
            &decl.timers,
            fire_tx,
            tracer.clone(),
        )));

        let cmd_mgr = Arc::new(Mutex::new(
            crate::runtime::command_manager::CommandManager::new(),
        ));
        let session_mgr =
            crate::runtime::session_manager::new_shared_default_session_manager(tracer.clone());
        let mut executor = TaskExecutor::new(program, registry, tracer)
            .with_agent_context(context.clone())
            .with_timer_engine(timer_engine.clone())
            .with_command_manager(cmd_mgr)
            .with_session_manager(session_mgr);

        // Wire instance registry into executor (issue #82)
        if let Some(ir) = instance_registry {
            executor = executor.with_instance_registry(ir);
        }

        // Wire storage for data.store/data.get operations (#140)
        if let Some(ref store) = storage {
            executor = executor.with_storage(store.clone());
        }

        // Wire persistent memory storage into executor (issue #57)
        if decl.memory_persistent {
            if let Some(ref store) = storage {
                executor = executor.with_persistent_memory(store.clone(), decl.name.node.clone());
            }
        }

        Self {
            decl,
            context,
            executor,
            event_bus: None,
            event_receivers: Vec::new(),
            timer_engine,
            timer_rx,
            warden_tx: None,
            storage,
            worktree_branch: None,
        }
    }

    /// Set working directory for sandbox isolation (issue #194).
    pub fn with_working_dir(mut self, dir: std::path::PathBuf) -> Self {
        self.executor = self.executor.with_working_dir(dir);
        self
    }

    /// Attach a skill executor for LLM-mediated skill bridge (#276).
    pub fn with_skill_executor(
        mut self,
        skill_exec: std::sync::Arc<crate::runtime::skill_executor::SkillExecutor>,
    ) -> Self {
        self.executor = self.executor.with_skill_executor(skill_exec);
        self
    }

    /// Set worktree branch name for cleanup on agent exit (issue #194).
    pub fn with_worktree_branch(mut self, branch: String) -> Self {
        self.worktree_branch = Some(branch);
        self
    }

    /// Get a reference to the shared context.
    pub fn context(&self) -> &Arc<Mutex<AgentContext>> {
        &self.context
    }

    /// Attach a warden signal channel for reporting stuck status.
    /// Also wires the signal into the session manager for contradiction
    /// reporting (issue #205).
    pub fn with_warden_signal(mut self, tx: mpsc::Sender<AgentSignal>) -> Self {
        // Wire warden signal into the session manager so contradictions
        // detected during verification can reach the warden.
        if let Some(mgr) = self.executor.session_manager() {
            mgr.set_warden_signal(tx.clone());
        }
        self.warden_tx = Some(tx);
        self
    }

    /// Dispatch an event to the appropriate handler.
    pub async fn dispatch(
        &self,
        event: &str,
        params: HashMap<String, ConfidentValue>,
    ) -> Result<Option<ConfidentValue>, RuntimeError> {
        // Find matching handler
        let handler = self
            .decl
            .handlers
            .iter()
            .find(|h| h.node.event.node == event)
            .ok_or_else(|| {
                RuntimeError::Unsupported(format!("no handler for event '{}'", event))
            })?;

        // Build environment with handler params and memory
        let mut env = Env::new();
        for (name, val) in &params {
            env.bind(name, val.clone());
        }
        // Bind handler declared params from positional args
        for param in &handler.node.params {
            if let Some(val) = params.get(&param.node.name) {
                env.bind(&param.node.name, val.clone());
            }
        }
        // Bind memory as a record and lifecycle as current state name
        {
            let ctx = self.context.lock().unwrap();
            env.bind(
                "memory",
                ConfidentValue::deterministic(ctx.memory.to_record()),
            );
            if let Some(ref sm) = ctx.state_machine {
                env.bind(
                    "lifecycle",
                    ConfidentValue::deterministic(Value::Text(sm.current.clone())),
                );
                // Bind state names as self-referencing strings so
                // `requires lifecycle == waiting` works as expected
                for state_name in sm.graph.keys() {
                    env.bind(
                        state_name,
                        ConfidentValue::deterministic(Value::Text(state_name.clone())),
                    );
                }
            }
        }

        // Evaluate requires guards
        for req in &handler.node.requires {
            let cond_val = self
                .executor
                .eval_expr(&req.node.condition, &mut env)
                .await?;
            if !truthy(&cond_val) {
                if let Some(t) = self.executor.tracer() {
                    t.handler_completed(
                        &self.decl.name.node,
                        event,
                        "blocked_by_requires",
                        0,
                        None,
                    );
                }
                return self.apply_fail_policy(&req.node.on_fail, &mut env).await;
            }
        }

        // Emit HandlerStarted after requires pass (issue #255)
        if let Some(t) = self.executor.tracer() {
            t.handler_started(&self.decl.name.node, event);
        }
        let handler_started_at = std::time::Instant::now();

        // Execute handler body with timeout detection
        let handler_timeout = std::time::Duration::from_secs(60);
        let exec_future = self.executor.exec_stmts(&handler.node.body, &mut env);
        let result = match tokio::time::timeout(handler_timeout, exec_future).await {
            Ok(Ok(_)) => None,
            Ok(Err(RuntimeError::GiveSignal(val, ..))) => Some(val),
            Ok(Err(e)) => {
                if let Some(t) = self.executor.tracer() {
                    t.handler_completed(
                        &self.decl.name.node,
                        event,
                        "error",
                        handler_started_at.elapsed().as_millis() as u64,
                        None,
                    );
                }
                return Err(e);
            }
            Err(_elapsed) => {
                // Handler timed out — signal warden
                if let Some(ref tx) = self.warden_tx {
                    let _ = tx.try_send(AgentSignal::Timeout {
                        agent_name: self.decl.name.node.clone(),
                    });
                }
                if let Some(t) = self.executor.tracer() {
                    t.handler_completed(
                        &self.decl.name.node,
                        event,
                        "timeout",
                        handler_started_at.elapsed().as_millis() as u64,
                        None,
                    );
                }
                return Err(RuntimeError::Unsupported(format!(
                    "agent '{}' handler '{}' timed out after {}s",
                    self.decl.name.node,
                    event,
                    handler_timeout.as_secs()
                )));
            }
        };

        // Emit HandlerCompleted for success path (issue #255)
        if let Some(t) = self.executor.tracer() {
            let confidence = result.as_ref().map(|v| v.confidence);
            t.handler_completed(
                &self.decl.name.node,
                event,
                "success",
                handler_started_at.elapsed().as_millis() as u64,
                confidence,
            );
        }

        // Record turn for stuck detection — only handlers that actually
        // produced a response (hit a `give` clause) are meaningful signal.
        // Event-absorption handlers that return Unit would otherwise feed
        // identical empty response_texts to StuckDetector and trivially
        // satisfy the Jaccard "all similar" check, tripping the warden
        // after a handful of bounded side-effect dispatches. See #286.
        if let Some(ref v) = result {
            let response_text = format!("{}", v.value);
            let confidence = v.confidence;
            let mut ctx = self.context.lock().unwrap();
            let memory_hash = ctx.memory.snapshot_hash();
            ctx.stuck_detector.record_turn(TurnRecord {
                response_text,
                confidence,
                memory_hash,
            });
        }

        // Persist memory and lifecycle state after handler execution
        // Ensures both survive across CLI invocations (forge send / binary)
        {
            let ctx = self.context.lock().unwrap();
            if let Some(ref store) = self.storage {
                let key = format!("agent:{}:memory", self.decl.name.node);
                if let Ok(json) = ctx.memory.to_json() {
                    // Collect correlation upserts atomically with the memory
                    // blob so external events can never route into a session
                    // whose memory isn't yet on disk (issue #334).
                    let mut correlation_rows: Vec<(String, String, String, String)> =
                        Vec::with_capacity(self.decl.correlates.len());
                    for correlate_sp in &self.decl.correlates {
                        let field = correlate_sp.node.field_name.node.as_str();
                        if let Some(cv) = ctx.memory.get(field) {
                            if let Value::Text(s) = &cv.value {
                                if !s.is_empty() {
                                    correlation_rows.push((
                                        self.decl.name.node.clone(),
                                        field.to_string(),
                                        s.clone(),
                                        self.decl.name.node.clone(),
                                    ));
                                }
                            }
                        }
                    }
                    if correlation_rows.is_empty() {
                        let _ = store.store(&key, &json);
                    } else {
                        let rows_for_trace = correlation_rows.clone();
                        if store
                            .store_memory_with_correlations(&key, &json, &correlation_rows)
                            .is_ok()
                        {
                            if let Some(t) = self.executor.tracer() {
                                for (agent, field, value, target) in &rows_for_trace {
                                    t.correlation_registered(agent, field, value, target);
                                }
                            }
                        }
                    }
                }
                // Persist lifecycle state
                if let Some(ref sm) = ctx.state_machine {
                    let lc_key = format!("agent:{}:lifecycle", self.decl.name.node);
                    let _ = store.store(&lc_key, &sm.current);
                }
            }
        }

        // Check stuck detection and execute stuck policy if needed
        let is_stuck = self.context.lock().unwrap().stuck_detector.is_stuck();
        if is_stuck {
            // Signal warden if attached
            if let Some(ref tx) = self.warden_tx {
                let _ = tx.try_send(AgentSignal::Stuck {
                    agent_name: self.decl.name.node.clone(),
                });
            }
            if let Some(ref policy) = self.decl.stuck_policy {
                // Re-bind memory for stuck policy body
                {
                    let ctx = self.context.lock().unwrap();
                    env.bind(
                        "memory",
                        ConfidentValue::deterministic(ctx.memory.to_record()),
                    );
                }
                match self.executor.exec_stmts(&policy.node.body, &mut env).await {
                    Ok(_) => {}
                    Err(RuntimeError::GiveSignal(val, ..)) => return Ok(Some(val)),
                    Err(e) => return Err(e),
                }
            }
        }

        // Check hallucination detection — repeated very low confidence responses
        let is_hallucinating = self
            .context
            .lock()
            .unwrap()
            .stuck_detector
            .is_hallucinating();
        if is_hallucinating {
            if let Some(ref tx) = self.warden_tx {
                let _ = tx.try_send(AgentSignal::Hallucination {
                    agent_name: self.decl.name.node.clone(),
                    detail: format!("repeated low confidence responses in handler '{}'", event),
                });
            }
        }

        Ok(result)
    }

    // ── EventBus integration (issue #19) ──────────────────────────────────

    /// Attach an event bus and register all declared subscriptions.
    ///
    /// Each declared `schedule` also gets an implicit bus subscription on its
    /// own name — `WakeService` publishes scheduled fires as regular events,
    /// and the agent's `on <schedule_name>` handler consumes them through the
    /// same event loop that handles ordinary subscribes. This keeps the
    /// "schedules arrive as normal bus events" contract honest without
    /// forcing every agent author to write an explicit `subscribe` line
    /// alongside each `schedule` block (issue #332).
    pub async fn with_event_bus(mut self, bus: SharedEventBus) -> Self {
        let mut receivers = Vec::new();
        {
            let mut bus_guard = bus.write().await;
            for sub in &self.decl.subscriptions {
                let rx = bus_guard.subscribe(
                    &sub.node.event_name.node,
                    &self.decl.name.node,
                    sub.node.filter.clone(),
                );
                receivers.push((sub.node.filter.clone(), rx));
            }
            for schedule in &self.decl.schedules {
                // For `mode: wake`, WakeService publishes the schedule's
                // `emit:` event (or `{name}.tick` fallback) — not the
                // schedule name itself (#333). Subscribe to the actual
                // event the dispatcher will publish so the agent's handler
                // receives it.
                let event_name = match schedule.node.mode.as_ref().map(|m| m.node) {
                    Some(crate::ast::ScheduleMode::Wake) => schedule
                        .node
                        .emit
                        .as_ref()
                        .map(|e| e.node.clone())
                        .unwrap_or_else(|| format!("{}.tick", schedule.node.name.node)),
                    _ => schedule.node.name.node.clone(),
                };
                let rx = bus_guard.subscribe(&event_name, &self.decl.name.node, None);
                receivers.push((None, rx));
            }
            // Correlate blocks imply a subscription on the source event — the
            // whole point is routing that event into this agent (#334). When
            // `emit:` is declared, WakeService/CorrelationDriver publishes the
            // emit event after rehydration; otherwise the inbound event is
            // delivered directly, so the agent needs to be on the bus for it.
            for correlate in &self.decl.correlates {
                let event_name = correlate
                    .node
                    .emit
                    .as_ref()
                    .map(|e| e.node.clone())
                    .unwrap_or_else(|| correlate.node.event_type.node.clone());
                let rx = bus_guard.subscribe(&event_name, &self.decl.name.node, None);
                receivers.push((None, rx));
            }
        }
        self.event_bus = Some(bus.clone());
        self.executor = self.executor.with_event_bus(bus);
        self.event_receivers = receivers;
        self
    }

    /// Run the agent event loop: receive events from the bus and timer expiry
    /// events, dispatch to handlers, and drain emitted events back through the bus.
    /// Returns when all event channels are closed.
    pub async fn run(&mut self) -> Result<(), RuntimeError> {
        // Emit AgentStarted lifecycle event (issue #255)
        if let Some(t) = self.executor.tracer() {
            t.agent_started(&self.decl.name.node, std::process::id());
        }

        // Auto-dispatch the `on start` handler when the agent comes up (issue #273).
        // Closes a Composition gap: agents declaring `on start` previously needed an
        // external event-bus trigger to fire it, leaving the handler dormant under
        // `forge run`. Auto-firing makes start a true lifecycle event so agents can
        // self-initialize (memory setup, banners, kicking off work).
        let has_start_handler = self
            .decl
            .handlers
            .iter()
            .any(|h| h.node.event.node == "start");
        if has_start_handler {
            match self.dispatch("start", HashMap::new()).await {
                Ok(_) => {}
                Err(RuntimeError::RetireSignal) => {
                    // Agent retired during start. Skip event loop, run cleanup path below.
                    if let Some(t) = self.executor.tracer() {
                        t.agent_shutdown(&self.decl.name.node, "retire");
                    }
                    return Ok(());
                }
                Err(e) => {
                    if let Some(t) = self.executor.tracer() {
                        t.agent_shutdown(&self.decl.name.node, "error");
                    }
                    return Err(e);
                }
            }
            // Drain any events the start handler emitted into the bus.
            self.drain_event_sink().await?;
        }

        // Merge all receivers into a single stream via a helper channel
        let (merge_tx, mut merge_rx) = mpsc::channel::<(Option<Spanned<Expr>>, EventPayload)>(64);

        // Spawn forwarders for each subscription receiver
        let mut handles = Vec::new();
        for (filter, rx) in self.event_receivers.drain(..) {
            let tx = merge_tx.clone();
            handles.push(tokio::spawn(async move {
                let mut rx = rx;
                while let Some(payload) = rx.recv().await {
                    if tx.send((filter.clone(), payload)).await.is_err() {
                        break;
                    }
                }
            }));
        }
        // Drop our copy so merge_rx closes when all forwarders finish
        drop(merge_tx);

        let shutdown_reason: &'static str;
        loop {
            tokio::select! {
                msg = merge_rx.recv() => {
                    match msg {
                        Some((filter, payload)) => {
                            if self.should_handle(&payload, &filter).await? {
                                let event_name = payload.event_name.clone();
                                let mut params = payload.fields.clone();
                                for (i, val) in payload.args.iter().enumerate() {
                                    params.entry(format!("arg{}", i))
                                        .or_insert_with(|| val.clone());
                                }
                                params.insert("event".to_string(),
                                    ConfidentValue::deterministic(
                                        Value::Record(payload.fields.clone()),
                                    ));

                                match self.dispatch(&event_name, params).await {
                                    Ok(_) => {}
                                    Err(RuntimeError::RetireSignal) => {
                                        shutdown_reason = "retire";
                                        break;
                                    }
                                    Err(e) => {
                                        if let Some(t) = self.executor.tracer() {
                                            t.agent_shutdown(&self.decl.name.node, "error");
                                        }
                                        return Err(e);
                                    }
                                }
                                self.drain_event_sink().await?;
                            }
                        }
                        None => {
                            shutdown_reason = "channel_closed";
                            break;
                        }
                    }
                }
                fired = self.timer_rx.recv() => {
                    if let Some(timer_event) = fired {
                        match self.handle_timer_fired(timer_event).await {
                            Ok(()) => {}
                            Err(RuntimeError::RetireSignal) => {
                                shutdown_reason = "retire";
                                break;
                            }
                            Err(e) => {
                                if let Some(t) = self.executor.tracer() {
                                    t.agent_shutdown(&self.decl.name.node, "error");
                                }
                                return Err(e);
                            }
                        }
                        self.drain_event_sink().await?;
                    }
                }
            }
        }

        // Shutdown: cancel all active timers
        self.timer_engine.lock().unwrap().cancel_all();

        // Shutdown: cancel all background commands (issue #162)
        if let Some(mgr) = self.executor.command_manager() {
            mgr.lock().unwrap().shutdown_all();
        }

        // Wait for forwarder tasks to finish
        for h in handles {
            let _ = h.await;
        }

        // Cleanup worktree on agent exit (issue #194)
        if let Some(ref branch) = self.worktree_branch {
            let _ = crate::runtime::sandbox::remove_worktree(branch);
        }

        // Emit AgentShutdown lifecycle event (issue #255)
        if let Some(t) = self.executor.tracer() {
            t.agent_shutdown(&self.decl.name.node, shutdown_reason);
        }

        Ok(())
    }

    /// Handle a timer expiry: update state, dispatch to `on timer_name.expired` handler.
    pub async fn handle_timer_fired(&self, fired: TimerFired) -> Result<(), RuntimeError> {
        // Update synchronous timer state
        {
            let mut ctx = self.context.lock().unwrap();
            ctx.timer_manager.expire(&fired.timer_name);
        }

        // Build handler event name: "{timer_name}.expired"
        let event_name = format!("{}.expired", fired.timer_name);

        // Build params — include context if present
        let mut params = HashMap::new();
        if let Some(context_val) = fired.context {
            // Bind as first positional param name from handler, or "context"
            let handler = self
                .decl
                .handlers
                .iter()
                .find(|h| h.node.event.node == event_name);
            if let Some(h) = handler {
                if let Some(first_param) = h.node.params.first() {
                    params.insert(first_param.node.name.clone(), context_val);
                } else {
                    params.insert("context".to_string(), context_val);
                }
            } else {
                params.insert("context".to_string(), context_val);
            }
        }

        // Dispatch — missing handler is not an error for timers
        match self.dispatch(&event_name, params).await {
            Ok(_) => {}
            Err(RuntimeError::Unsupported(msg)) if msg.contains("no handler") => {}
            Err(e) => return Err(e),
        }

        Ok(())
    }

    /// Evaluate a subscription filter against an event payload.
    async fn should_handle(
        &self,
        payload: &EventPayload,
        filter: &Option<Spanned<Expr>>,
    ) -> Result<bool, RuntimeError> {
        match filter {
            None => Ok(true),
            Some(filter_expr) => {
                let mut env = Env::new();
                // Bind each field directly
                for (name, val) in &payload.fields {
                    env.bind(name, val.clone());
                }
                // Bind "event" as a record for dot-access (event.room_id)
                env.bind(
                    "event",
                    ConfidentValue::deterministic(Value::Record(payload.fields.clone())),
                );
                // Bind agent memory so filters like
                // `subscribe LearnedInsight where category == memory.topic`
                // can resolve `memory.*` the same way handler bodies do.
                // Without this binding the filter eval errors, the agent
                // task crashes, and the warden restarts it — with N crashes
                // within the retry window tripping the circuit breaker.
                {
                    let ctx = self.context.lock().unwrap();
                    env.bind(
                        "memory",
                        ConfidentValue::deterministic(ctx.memory.to_record()),
                    );
                }
                let result = self.executor.eval_expr(filter_expr, &mut env).await?;
                Ok(truthy(&result))
            }
        }
    }

    /// Drain emitted events from the EventSink and publish them through the bus.
    /// Lock ordering: AgentContext mutex released before EventBus RwLock acquired.
    async fn drain_event_sink(&self) -> Result<(), RuntimeError> {
        let (emitted, forwards) = {
            let mut ctx = self.context.lock().unwrap();
            let emitted = std::mem::take(&mut ctx.event_sink.emitted);
            let forwards = std::mem::take(&mut ctx.event_sink.forwards);
            (emitted, forwards)
        };

        if let Some(ref bus) = self.event_bus {
            let bus_guard = bus.read().await;
            for event in emitted {
                let payload = EventPayload {
                    event_name: event.name,
                    args: event.args,
                    source_agent: self.decl.name.node.clone(),
                    fields: event.fields,
                };
                bus_guard.publish(&payload);
            }
            for (val, target) in forwards {
                if let Value::Text(target_agent) = &target.value {
                    let payload = EventPayload {
                        event_name: "forward".to_string(),
                        args: vec![val],
                        source_agent: self.decl.name.node.clone(),
                        fields: HashMap::new(),
                    };
                    bus_guard.forward(&payload, target_agent);
                }
            }
        }
        Ok(())
    }

    /// Apply a fail policy when a requires guard fails.
    async fn apply_fail_policy(
        &self,
        on_fail: &Option<Spanned<FailPolicy>>,
        env: &mut Env,
    ) -> Result<Option<ConfidentValue>, RuntimeError> {
        match on_fail {
            None
            | Some(Spanned {
                node: FailPolicy::Silent,
                ..
            }) => {
                // Silent rejection
                Ok(None)
            }
            Some(Spanned {
                node: FailPolicy::Log,
                ..
            }) => {
                eprintln!("[forge] requires guard failed (log)");
                Ok(None)
            }
            Some(Spanned {
                node: FailPolicy::Give(expr),
                ..
            }) => {
                let val = self.executor.eval_expr(expr, env).await?;
                Ok(Some(val))
            }
            Some(Spanned {
                node: FailPolicy::Escalate,
                ..
            }) => {
                let mut ctx = self.context.lock().unwrap();
                ctx.event_sink
                    .escalations
                    .push("requires_guard".to_string());
                Ok(None)
            }
            Some(Spanned {
                node: FailPolicy::Crash,
                ..
            }) => Err(RuntimeError::FlowError(
                "requires guard failed: crash policy".to_string(),
            )),
        }
    }
}

fn truthy(cv: &ConfidentValue) -> bool {
    match &cv.value {
        Value::Bool(b) => *b,
        Value::Text(s) | Value::Html(s) => !s.is_empty(),
        Value::Number(n) => *n != 0.0,
        Value::Unit => false,
        Value::List(v) | Value::Array(v) => !v.is_empty(),
        Value::Record(m) => !m.is_empty(),
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Span, Spanned};

    fn spanned<T>(node: T) -> Spanned<T> {
        Spanned::new(node, Span { start: 0, end: 0 })
    }

    // ── StateMachine tests ───────────────────────────────────────

    #[test]
    fn state_machine_initial_state() {
        let decl = StatesDecl {
            name: spanned("Phase".into()),
            transitions: vec![spanned(StateTransition {
                from: spanned("idle".into()),
                to: spanned("active".into()),
                condition: None,
            })],
        };
        let sm = StateMachine::new(&decl);
        assert_eq!(sm.current, "idle");
    }

    #[test]
    fn state_machine_valid_transition() {
        let decl = StatesDecl {
            name: spanned("Phase".into()),
            transitions: vec![
                spanned(StateTransition {
                    from: spanned("idle".into()),
                    to: spanned("active".into()),
                    condition: None,
                }),
                spanned(StateTransition {
                    from: spanned("active".into()),
                    to: spanned("done".into()),
                    condition: None,
                }),
            ],
        };
        let mut sm = StateMachine::new(&decl);
        assert!(sm.transition("active").is_ok());
        assert_eq!(sm.current, "active");
        assert!(sm.transition("done").is_ok());
        assert_eq!(sm.current, "done");
    }

    #[test]
    fn state_machine_invalid_transition() {
        let decl = StatesDecl {
            name: spanned("Phase".into()),
            transitions: vec![spanned(StateTransition {
                from: spanned("idle".into()),
                to: spanned("active".into()),
                condition: None,
            })],
        };
        let mut sm = StateMachine::new(&decl);
        assert!(sm.transition("done").is_err());
    }

    // ── TimerManager tests ───────────────────────────────────────

    #[test]
    fn timer_start_cancel() {
        let fields = vec![spanned(TimerField {
            name: spanned("timeout".into()),
            duration: spanned(Duration {
                value: 10,
                unit: DurationUnit::Minutes,
            }),
        })];
        let mut tm = TimerManager::new(&fields);
        assert_eq!(tm.state("timeout"), Some(&TimerState::Idle));
        tm.start("timeout").unwrap();
        assert_eq!(tm.state("timeout"), Some(&TimerState::Running));
        tm.cancel("timeout").unwrap();
        assert_eq!(tm.state("timeout"), Some(&TimerState::Idle));
    }

    #[test]
    fn timer_unknown_errors() {
        let mut tm = TimerManager::empty();
        assert!(tm.start("bogus").is_err());
    }

    // ── StuckDetector tests ──────────────────────────────────────

    #[test]
    fn stuck_not_enough_turns() {
        let mut sd = StuckDetector::new(3);
        sd.record_turn(TurnRecord {
            response_text: "hi".into(),
            confidence: 0.9,
            memory_hash: 1,
        });
        sd.record_turn(TurnRecord {
            response_text: "hi".into(),
            confidence: 0.9,
            memory_hash: 1,
        });
        assert!(!sd.is_stuck());
    }

    #[test]
    fn stuck_similar_responses() {
        let mut sd = StuckDetector::new(3);
        for _ in 0..3 {
            sd.record_turn(TurnRecord {
                response_text: "I cannot help with that request".into(),
                confidence: 0.9,
                memory_hash: 42,
            });
        }
        assert!(sd.is_stuck());
    }

    #[test]
    fn stuck_low_confidence() {
        let mut sd = StuckDetector::new(3);
        for _ in 0..3 {
            sd.record_turn(TurnRecord {
                response_text: format!("response {}", rand_text()),
                confidence: 0.3,
                memory_hash: 42,
            });
        }
        assert!(sd.is_stuck());
    }

    #[test]
    fn not_stuck_different_responses() {
        let mut sd = StuckDetector::new(3);
        sd.record_turn(TurnRecord {
            response_text: "hello there friend".into(),
            confidence: 0.9,
            memory_hash: 1,
        });
        sd.record_turn(TurnRecord {
            response_text: "goodbye world now".into(),
            confidence: 0.9,
            memory_hash: 2,
        });
        sd.record_turn(TurnRecord {
            response_text: "something completely different here".into(),
            confidence: 0.9,
            memory_hash: 3,
        });
        assert!(!sd.is_stuck());
    }

    // ── Jaccard similarity tests ─────────────────────────────────

    #[test]
    fn jaccard_identical() {
        assert!((jaccard_similarity("hello world", "hello world") - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn jaccard_disjoint() {
        assert!((jaccard_similarity("hello world", "foo bar") - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn jaccard_partial() {
        let sim = jaccard_similarity("the quick brown fox", "the quick red fox");
        assert!(sim > 0.5 && sim < 1.0);
    }

    fn rand_text() -> String {
        // Just unique enough for testing
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        format!(
            "{}",
            COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        )
    }

    // ── EventSink tests ──────────────────────────────────────────

    #[test]
    fn event_sink_collects() {
        let mut sink = EventSink::new();
        sink.emitted.push(EmittedEvent {
            name: "TestEvent".into(),
            args: vec![],
            fields: HashMap::new(),
        });
        sink.escalations.push("human".into());
        assert_eq!(sink.emitted.len(), 1);
        assert_eq!(sink.escalations.len(), 1);
    }
}
