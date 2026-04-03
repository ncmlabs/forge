// FORGE warded runtime — issue #24
// Orchestrates agent lifecycle management: spawns agents as tokio tasks,
// monitors for crashes/stuck, and executes warden response decisions.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::ast::*;
use crate::llm::registry::ProviderRegistry;
use crate::runtime::agent::{AgentProcess, AgentSignal};
use crate::runtime::event_bus::{EventBus, SharedEventBus};
use crate::runtime::executor::RuntimeError;
use crate::runtime::warden::{FailureSignal, WardAction, Warden};
use crate::tracer::Tracer;

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
}

// ── WardedRuntime ───────────────────────────────────────────────────────────

/// Async orchestrator that spawns agents and monitors them using the Warden policy engine.
pub struct WardedRuntime {
    pub warden: Warden,
    agents: HashMap<String, ManagedAgent>,
    blueprints: HashMap<String, AgentBlueprint>,
    event_bus: SharedEventBus,
    signal_tx: mpsc::Sender<AgentSignal>,
    signal_rx: mpsc::Receiver<AgentSignal>,
    start: Instant,
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

        Self {
            warden: Warden::new(warden_decl, tracer),
            agents: HashMap::new(),
            blueprints,
            event_bus,
            signal_tx,
            signal_rx,
            start: Instant::now(),
        }
    }

    /// Get a reference to the shared event bus.
    pub fn event_bus(&self) -> &SharedEventBus {
        &self.event_bus
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
        Ok(())
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
        )
        .with_warden_signal(self.signal_tx.clone());

        process = process.with_event_bus(self.event_bus.clone()).await;

        let handle = tokio::spawn(async move { process.run().await });

        self.agents.insert(
            name.to_string(),
            ManagedAgent {
                blueprint: blueprint.clone(),
                handle,
            },
        );

        Ok(())
    }

    /// Stop an agent by aborting its tokio task.
    fn stop_agent(&mut self, name: &str) {
        if let Some(agent) = self.agents.remove(name) {
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
                // Check for stuck signals from agents
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
                return Err(RuntimeError::Unsupported(format!(
                    "warden '{}' circuit breaker tripped",
                    self.warden.decl.name.node,
                )));
            }
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
            WardResponse::Nudge | WardResponse::Restart | WardResponse::Replace => {
                // v1: all three are restart (nudge with memory hint and replace with config deferred)
                self.stop_agent(&agent_name);
                if self.blueprints.contains_key(&agent_name) {
                    self.spawn_one(&agent_name).await?;
                }
            }
            WardResponse::Escalate => {
                self.stop_agent(&agent_name);
                return Err(RuntimeError::Unsupported(format!(
                    "warden '{}' escalated for agent '{}'",
                    action.warden_name, agent_name
                )));
            }
        }

        // Apply scope to other agents
        self.apply_scope(action.scope, &agent_name).await?;

        Ok(())
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
                    self.stop_agent(&name);
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
