// FORGE warded runtime — issue #24
// Orchestrates agent lifecycle management: spawns agents as tokio tasks,
// monitors for crashes/stuck, and executes warden response decisions.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::ast::*;
use crate::llm::registry::ProviderRegistry;
use uuid::Uuid;

use serde::Serialize;
use tokio::sync::RwLock;

use crate::runtime::agent::{AgentProcess, AgentSignal};
use crate::runtime::event_bus::{EventBus, SharedEventBus};
use crate::runtime::executor::RuntimeError;
use crate::runtime::instance_registry::{InstanceRegistry, SharedInstanceRegistry};
use crate::runtime::warden::{FailureSignal, WardAction, Warden};
use crate::tracer::Tracer;

// ── Introspection Snapshot ─────────────────────────────────────────────────

/// Read-only snapshot of warden state for introspection (issue #139).
#[derive(Debug, Clone, Serialize)]
pub struct WardenSnapshot {
    pub name: String,
    pub managed_agents: Vec<String>,
    pub degraded_agents: Vec<String>,
    pub retry_counts: HashMap<String, u64>,
    pub circuit_breaker_tripped: bool,
}

/// Shared handle for HTTP handlers to read warden snapshots.
pub type SharedWardenSnapshots = Arc<RwLock<Vec<WardenSnapshot>>>;

// ── AgentBlueprint ──────────────────────────────────────────────────────────

/// Everything needed to (re)create an agent process.
#[derive(Clone)]
pub struct AgentBlueprint {
    pub decl: AgentDecl,
    pub states: Option<StatesDecl>,
    pub program: Program,
    pub registry: Arc<ProviderRegistry>,
    pub tracer: Option<Tracer>,
}

// ── ManagedAgent ────────────────────────────────────────────────────────────

/// A running agent managed by a warden.
pub struct ManagedAgent {
    pub blueprint: AgentBlueprint,
    pub handle: JoinHandle<Result<(), RuntimeError>>,
    pub instance_id: Uuid,
}

// ── WardedRuntime ───────────────────────────────────────────────────────────

/// Async orchestrator that spawns agents and monitors them using the Warden policy engine.
pub struct WardedRuntime {
    pub warden: Warden,
    agents: HashMap<String, ManagedAgent>,
    blueprints: HashMap<String, AgentBlueprint>,
    event_bus: SharedEventBus,
    instance_registry: SharedInstanceRegistry,
    storage: Option<crate::runtime::storage::SharedStorage>,
    signal_tx: mpsc::Sender<AgentSignal>,
    signal_rx: mpsc::Receiver<AgentSignal>,
    start: Instant,
    /// Agents that have been escalated and removed from active supervision.
    /// The wiki continues running with degraded features for these agents.
    pub degraded_agents: HashSet<String>,
    /// Shared snapshot handle updated on state changes (for introspection).
    shared_snapshots: Option<SharedWardenSnapshots>,
}

impl WardedRuntime {
    /// Build a WardedRuntime from a WardenDecl and a Program.
    /// Finds AgentDecl and StatesDecl by name from the program's items.
    pub fn new(
        warden_decl: WardenDecl,
        program: &Program,
        registry: Arc<ProviderRegistry>,
        tracer: Option<Tracer>,
    ) -> Self {
        // Collect agent and states declarations from the program
        let mut agent_decls: HashMap<String, AgentDecl> = HashMap::new();
        let mut states_decls: HashMap<String, StatesDecl> = HashMap::new();

        for item in &program.items {
            match &item.node {
                TopLevel::Agent(a) => {
                    agent_decls.insert(a.name.node.clone(), a.as_ref().clone());
                }
                TopLevel::States(s) => {
                    states_decls.insert(s.name.node.clone(), s.clone());
                }
                _ => {}
            }
        }

        // Build blueprints for each managed agent
        let mut blueprints = HashMap::new();
        for managed_name in &warden_decl.manages {
            if let Some(agent_decl) = agent_decls.get(&managed_name.node) {
                let states = agent_decl
                    .lifecycle
                    .as_ref()
                    .and_then(|lc| states_decls.get(&lc.node))
                    .cloned();

                blueprints.insert(
                    managed_name.node.clone(),
                    AgentBlueprint {
                        decl: agent_decl.clone(),
                        states,
                        program: program.clone(),
                        registry: registry.clone(),
                        tracer: tracer.clone(),
                    },
                );
            }
        }

        let (signal_tx, signal_rx) = mpsc::channel::<AgentSignal>(64);

        let event_bus: SharedEventBus =
            Arc::new(tokio::sync::RwLock::new(EventBus::new(tracer.clone())));

        let instance_registry: SharedInstanceRegistry =
            Arc::new(tokio::sync::RwLock::new(InstanceRegistry::new()));

        Self {
            warden: Warden::new(warden_decl, tracer),
            agents: HashMap::new(),
            blueprints,
            event_bus,
            instance_registry,
            signal_tx,
            signal_rx,
            start: Instant::now(),
            degraded_agents: HashSet::new(),
            storage: None,
            shared_snapshots: None,
        }
    }

    /// Build a WardedRuntime that shares an existing event bus and instance registry,
    /// used by SystemRuntime to coordinate across multiple wardens.
    pub fn with_shared_infrastructure(
        warden_decl: WardenDecl,
        program: &Program,
        registry: Arc<ProviderRegistry>,
        tracer: Option<Tracer>,
        event_bus: SharedEventBus,
        instance_registry: SharedInstanceRegistry,
    ) -> Self {
        // Collect agent and states declarations from the program
        let mut agent_decls: HashMap<String, AgentDecl> = HashMap::new();
        let mut states_decls: HashMap<String, StatesDecl> = HashMap::new();

        for item in &program.items {
            match &item.node {
                TopLevel::Agent(a) => {
                    agent_decls.insert(a.name.node.clone(), a.as_ref().clone());
                }
                TopLevel::States(s) => {
                    states_decls.insert(s.name.node.clone(), s.clone());
                }
                _ => {}
            }
        }

        // Build blueprints for each managed agent
        let mut blueprints = HashMap::new();
        for managed_name in &warden_decl.manages {
            if let Some(agent_decl) = agent_decls.get(&managed_name.node) {
                let states = agent_decl
                    .lifecycle
                    .as_ref()
                    .and_then(|lc| states_decls.get(&lc.node))
                    .cloned();

                blueprints.insert(
                    managed_name.node.clone(),
                    AgentBlueprint {
                        decl: agent_decl.clone(),
                        states,
                        program: program.clone(),
                        registry: registry.clone(),
                        tracer: tracer.clone(),
                    },
                );
            }
        }

        let (signal_tx, signal_rx) = mpsc::channel::<AgentSignal>(64);

        Self {
            warden: Warden::new(warden_decl, tracer),
            agents: HashMap::new(),
            blueprints,
            event_bus,
            instance_registry,
            signal_tx,
            signal_rx,
            start: Instant::now(),
            degraded_agents: HashSet::new(),
            storage: None,
            shared_snapshots: None,
        }
    }

    /// Get a reference to the shared event bus.
    pub fn event_bus(&self) -> &SharedEventBus {
        &self.event_bus
    }

    /// Get a reference to the shared instance registry.
    pub fn instance_registry(&self) -> &SharedInstanceRegistry {
        &self.instance_registry
    }

    /// Clone the signal sender for external injection (issue #143).
    pub fn signal_sender(&self) -> mpsc::Sender<AgentSignal> {
        self.signal_tx.clone()
    }

    /// Return the list of managed agent names (from blueprints).
    pub fn managed_agent_names(&self) -> Vec<String> {
        self.blueprints.keys().cloned().collect()
    }

    /// Attach a shared snapshot handle for introspection.
    pub fn with_shared_snapshots(mut self, snaps: SharedWardenSnapshots) -> Self {
        self.shared_snapshots = Some(snaps);
        self
    }

    /// Replace the event bus (call before spawning agents).
    pub fn set_event_bus(&mut self, bus: SharedEventBus) {
        self.event_bus = bus;
    }

    /// Replace the instance registry (call before spawning agents).
    pub fn set_instance_registry(&mut self, registry: SharedInstanceRegistry) {
        self.instance_registry = registry;
    }

    /// Inject shared storage so agents can use data.store/data.get.
    pub fn set_storage(&mut self, storage: crate::runtime::storage::SharedStorage) {
        self.storage = Some(storage);
    }

    /// Build a read-only snapshot of current warden state.
    pub fn snapshot(&self) -> WardenSnapshot {
        let now_ms = self.timestamp_ms();
        let retry_counts: HashMap<String, u64> = self
            .warden
            .retry_tracker
            .all_counts()
            .iter()
            .map(|((agent, ft), count)| (format!("{}:{:?}", agent, ft), *count))
            .collect();
        WardenSnapshot {
            name: self.warden.decl.name.node.clone(),
            managed_agents: self.agents.keys().cloned().collect(),
            degraded_agents: self.degraded_agents.iter().cloned().collect(),
            retry_counts,
            circuit_breaker_tripped: self.warden.circuit_breaker_tripped(now_ms),
        }
    }

    /// Push current snapshot to the shared handle (if wired).
    async fn update_shared_snapshot(&self) {
        if let Some(ref shared) = self.shared_snapshots {
            let snap = self.snapshot();
            let mut guard = shared.write().await;
            // Replace our entry (find by name) or append
            if let Some(pos) = guard.iter().position(|s| s.name == snap.name) {
                guard[pos] = snap;
            } else {
                guard.push(snap);
            }
        }
    }

    fn timestamp_ms(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }

    /// Spawn all managed agents as tokio tasks.
    pub async fn spawn_all(&mut self) -> Result<(), RuntimeError> {
        let names: Vec<String> = self.blueprints.keys().cloned().collect();
        for name in names {
            self.spawn_one(&name).await?;
        }
        // Trace supervision tree after startup
        self.trace_supervision_tree();
        self.update_shared_snapshot().await;
        Ok(())
    }

    /// Emit a supervision tree trace event showing active and degraded agents.
    fn trace_supervision_tree(&self) {
        if let Some(tracer) = self.warden.tracer() {
            let active: Vec<&str> = self.agents.keys().map(|s| s.as_str()).collect();
            let degraded: Vec<&str> = self.degraded_agents.iter().map(|s| s.as_str()).collect();
            tracer.supervision_tree(&self.warden.decl.name.node, &active, &degraded);
        }
    }

    /// Spawn a single agent by name.
    async fn spawn_one(&mut self, name: &str) -> Result<(), RuntimeError> {
        let blueprint = self.blueprints.get(name).ok_or_else(|| {
            RuntimeError::Unsupported(format!("no blueprint for agent '{}'", name))
        })?;

        let mut process = AgentProcess::new(
            blueprint.decl.clone(),
            blueprint.states.as_ref(),
            blueprint.registry.clone(),
            blueprint.tracer.clone(),
            blueprint.program.clone(),
            self.storage.clone(),
            Some(self.instance_registry.clone()),
        )
        .with_warden_signal(self.signal_tx.clone());

        process = process.with_event_bus(self.event_bus.clone()).await;

        let context_ref = process.context().clone();
        let instance_id =
            self.instance_registry
                .write()
                .await
                .register_with_context(name, None, context_ref);

        let handle = tokio::spawn(async move { process.run().await });

        self.agents.insert(
            name.to_string(),
            ManagedAgent {
                blueprint: blueprint.clone(),
                handle,
                instance_id,
            },
        );

        Ok(())
    }

    /// Stop an agent by aborting its tokio task and unregistering from the instance registry.
    async fn stop_agent(&mut self, name: &str) {
        if let Some(agent) = self.agents.remove(name) {
            self.instance_registry
                .write()
                .await
                .unregister(&agent.instance_id);
            agent.handle.abort();
        }
    }

    /// The main monitoring loop.
    /// Watches for agent crashes (JoinHandle completion) and stuck signals.
    /// Returns when all agents have exited normally or escalation is triggered.
    pub async fn run(&mut self) -> Result<(), RuntimeError> {
        loop {
            // If no agents left, we're done
            if self.agents.is_empty() {
                break;
            }

            tokio::select! {
                // Check for agent signals (stuck, timeout, hallucination, budget)
                signal = self.signal_rx.recv() => {
                    match signal {
                        Some(AgentSignal::Stuck { agent_name }) => {
                            let fs = FailureSignal {
                                agent_name,
                                failure_type: FailureType::Stuck,
                                detail: "stuck detector triggered".to_string(),
                            };
                            self.handle_signal(fs).await?;
                        }
                        Some(AgentSignal::Timeout { agent_name }) => {
                            let fs = FailureSignal {
                                agent_name,
                                failure_type: FailureType::Timeout,
                                detail: "handler execution timed out".to_string(),
                            };
                            self.handle_signal(fs).await?;
                        }
                        Some(AgentSignal::Hallucination { agent_name, detail }) => {
                            let fs = FailureSignal {
                                agent_name,
                                failure_type: FailureType::Hallucination,
                                detail,
                            };
                            self.handle_signal(fs).await?;
                        }
                        Some(AgentSignal::Contradiction { agent_name, detail, severity }) => {
                            let fs = FailureSignal {
                                agent_name,
                                failure_type: FailureType::Contradiction,
                                detail: format!("[{}] {}", severity, detail),
                            };
                            self.handle_signal(fs).await?;
                        }
                        Some(AgentSignal::BudgetExceeded { agent_name, detail }) => {
                            let fs = FailureSignal {
                                agent_name,
                                failure_type: FailureType::Budget,
                                detail,
                            };
                            self.handle_signal(fs).await?;
                        }
                        Some(AgentSignal::Crash { agent_name }) => {
                            let fs = FailureSignal {
                                agent_name,
                                failure_type: FailureType::Crash,
                                detail: "injected crash signal".to_string(),
                            };
                            self.handle_signal(fs).await?;
                        }
                        None => break,
                    }
                }
                // Poll for completed agent tasks
                result = poll_agents(&mut self.agents) => {
                    let (name, result) = result;
                    match result {
                        Ok(Ok(())) => {
                            // Normal exit — agent finished cleanly
                            self.agents.remove(&name);
                        }
                        Ok(Err(runtime_err)) => {
                            self.agents.remove(&name);
                            let fs = FailureSignal {
                                agent_name: name,
                                failure_type: FailureType::Crash,
                                detail: format!("{:?}", runtime_err),
                            };
                            self.handle_signal(fs).await?;
                        }
                        Err(join_err) => {
                            self.agents.remove(&name);
                            let fs = FailureSignal {
                                agent_name: name,
                                failure_type: FailureType::Crash,
                                detail: format!("panic: {}", join_err),
                            };
                            self.handle_signal(fs).await?;
                        }
                    }
                }
            }

            // Check circuit breaker after every signal
            let now = self.timestamp_ms();
            if self.warden.circuit_breaker_tripped(now) {
                // Graceful degradation: stop all agents, mark as degraded, but don't crash
                eprintln!(
                    "[warden] CIRCUIT BREAKER: warden '{}' tripped — stopping all agents. \
                     Deterministic endpoints continue serving.",
                    self.warden.decl.name.node,
                );
                let names: Vec<String> = self.agents.keys().cloned().collect();
                for name in names {
                    self.stop_agent(&name).await;
                    self.degraded_agents.insert(name);
                }
                self.trace_supervision_tree();
                self.update_shared_snapshot().await;
                break;
            }

            // Update shared snapshot after state changes
            self.update_shared_snapshot().await;
        }

        Ok(())
    }

    /// Handle a failure signal: resolve policy, execute response, apply scope.
    async fn handle_signal(&mut self, signal: FailureSignal) -> Result<(), RuntimeError> {
        let agent_name = signal.agent_name.clone();

        // Look up agent overrides
        let overrides = self
            .blueprints
            .get(&agent_name)
            .map(|bp| bp.decl.warden_override.as_slice())
            .unwrap_or(&[]);

        let now = self.timestamp_ms();
        let action = self.warden.handle_failure(&signal, overrides, now);

        if let Some(action) = action {
            self.execute_action(action).await?;
        }

        Ok(())
    }

    /// Execute a WardAction: nudge/restart/replace/escalate + scope.
    async fn execute_action(&mut self, action: WardAction) -> Result<(), RuntimeError> {
        let agent_name = action.agent_name.clone();

        match action.response {
            WardResponse::Nudge
            | WardResponse::Downgrade
            | WardResponse::Restart
            | WardResponse::Replace => {
                // v1: all four are restart (nudge with memory hint, downgrade model tier, and replace with config deferred)
                self.stop_agent(&agent_name).await;
                if self.blueprints.contains_key(&agent_name) {
                    self.spawn_one(&agent_name).await?;
                }
            }
            WardResponse::Escalate => {
                // Graceful degradation: stop the agent but continue running.
                // The wiki serves deterministic endpoints while the agent is degraded.
                self.stop_agent(&agent_name).await;
                self.degraded_agents.insert(agent_name.clone());

                eprintln!(
                    "[warden] ESCALATED: warden '{}' escalated agent '{}' \
                     (failure: {:?}, retries: {}). Agent removed from supervision.",
                    action.warden_name, agent_name, action.failure_type, action.retry_count
                );

                self.trace_supervision_tree();
                return Ok(()); // Continue running — do not crash the runtime
            }
        }

        // Apply scope to other agents
        self.apply_scope(action.scope, &agent_name).await?;

        Ok(())
    }

    /// Adopt a running agent into warden supervision.
    pub fn adopt(&mut self, name: &str, blueprint: AgentBlueprint) {
        self.warden.adopt(name);
        self.blueprints.insert(name.to_string(), blueprint);
    }

    /// Release an agent from warden supervision (does NOT stop the agent).
    pub fn release(&mut self, name: &str) {
        self.warden.release(name);
        self.blueprints.remove(name);
    }

    /// Apply scope: restart affected agents beyond the failing one.
    async fn apply_scope(
        &mut self,
        scope: WardScope,
        failing_agent: &str,
    ) -> Result<(), RuntimeError> {
        match scope {
            WardScope::This => { /* already handled the failing agent */ }
            WardScope::Downstream | WardScope::All => {
                // v1: downstream treated as all
                let other_names: Vec<String> = self
                    .agents
                    .keys()
                    .filter(|n| n.as_str() != failing_agent)
                    .cloned()
                    .collect();
                for name in other_names {
                    self.stop_agent(&name).await;
                    self.spawn_one(&name).await?;
                }
            }
        }
        Ok(())
    }
}

/// Poll all managed agents, returning the first one that completes.
/// This is a helper for use in `tokio::select!`.
async fn poll_agents(
    agents: &mut HashMap<String, ManagedAgent>,
) -> (
    String,
    Result<Result<(), RuntimeError>, tokio::task::JoinError>,
) {
    // We need to poll all JoinHandles. Use a simple approach:
    // find the first completed one.
    loop {
        for (name, agent) in agents.iter_mut() {
            // Check if the task is finished without blocking
            if agent.handle.is_finished() {
                let name = name.clone();
                let agent = agents.remove(&name).unwrap();
                let result = agent.handle.await;
                // Re-insert a dummy to avoid borrow issues — actually we removed it
                return (name, result);
            }
        }
        // None finished yet, yield and try again
        tokio::task::yield_now().await;
    }
}
