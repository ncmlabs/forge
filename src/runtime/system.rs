// FORGE system runtime — issue #87
// Orchestration root: interprets SystemDecl bindings and wiring,
// creates shared infrastructure, spawns initial agents, and
// wires event routing between them.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use uuid::Uuid;

use serde::Serialize;

use crate::ast::*;
use crate::config::SystemConfig;
use crate::llm::registry::ProviderRegistry;
use crate::runtime::agent::{AgentProcess, AgentSignal};
use crate::runtime::event_bus::{EventBus, SharedEventBus};
use crate::runtime::executor::RuntimeError;
use crate::runtime::instance_registry::{InstanceRegistry, SharedInstanceRegistry};
use crate::runtime::warded::{AgentBlueprint, SharedWardenSnapshots, WardedRuntime};
use crate::tracer::Tracer;

/// Shared map of agent name → signal sender for failure injection (issue #143).
pub type SharedSignalSenders = Arc<tokio::sync::RwLock<HashMap<String, mpsc::Sender<AgentSignal>>>>;

// ── Introspection Snapshot ─────────────────────────────────────────────────

/// Static topology snapshot for introspection (issue #139).
#[derive(Debug, Clone, Serialize)]
pub struct TopologySnapshot {
    pub system_name: String,
    pub bindings: Vec<(String, String)>,
    pub wiring: Vec<Vec<String>>,
}

// ── SystemRuntime ────────────────────────────────────────────────────────────

/// A running agent managed directly by the system (no warden supervision).
struct UnsupervisedAgent {
    handle: JoinHandle<Result<(), RuntimeError>>,
    instance_id: Uuid,
}

/// The system runtime: orchestrates agent spawning, warden supervision,
/// and event routing based on a system declaration.
pub struct SystemRuntime {
    name: String,
    bindings: Vec<(String, String)>, // (alias, agent_decl_name)
    wiring: Vec<Vec<String>>,        // parsed compose chains as alias sequences
    event_bus: SharedEventBus,
    instance_registry: SharedInstanceRegistry,
    storage: Option<crate::runtime::storage::SharedStorage>,
    blueprints: HashMap<String, AgentBlueprint>,
    warded_runtimes: Vec<WardedRuntime>,
    unsupervised_blueprints: Vec<String>, // aliases not covered by any warden
    unsupervised_agents: HashMap<String, UnsupervisedAgent>,
    max_agents: Option<usize>,
}

impl SystemRuntime {
    /// Build a SystemRuntime from a SystemDecl and the full program.
    pub fn new(
        system_decl: &SystemDecl,
        program: &Program,
        providers: Arc<ProviderRegistry>,
        tracer: Option<Tracer>,
        config: Option<&SystemConfig>,
    ) -> Result<Self, RuntimeError> {
        let name = system_decl.name.node.clone();

        // Collect agent and states declarations from the program
        let mut agent_decls: HashMap<String, AgentDecl> = HashMap::new();
        let mut states_decls: HashMap<String, StatesDecl> = HashMap::new();
        let mut warden_decls: Vec<WardenDecl> = Vec::new();

        for item in &program.items {
            match &item.node {
                TopLevel::Agent(a) => {
                    agent_decls.insert(a.name.node.clone(), a.as_ref().clone());
                }
                TopLevel::States(s) => {
                    states_decls.insert(s.name.node.clone(), s.clone());
                }
                TopLevel::Warden(w) => {
                    warden_decls.push(w.clone());
                }
                _ => {}
            }
        }

        // Resolve bindings: alias → agent declaration name
        let mut bindings = Vec::new();
        for binding in &system_decl.bindings {
            let alias = &binding.node.alias;
            let target = &binding.node.target;
            if !agent_decls.contains_key(target) {
                return Err(RuntimeError::Unsupported(format!(
                    "system '{}': binding '{}' references unknown agent '{}'",
                    name, alias, target
                )));
            }
            bindings.push((alias.clone(), target.clone()));
        }

        // Build blueprints for each bound agent
        let mut blueprints = HashMap::new();
        for (alias, target) in &bindings {
            let agent_decl = &agent_decls[target];
            let states = agent_decl
                .lifecycle
                .as_ref()
                .and_then(|lc| states_decls.get(&lc.node))
                .cloned();

            blueprints.insert(
                alias.clone(),
                AgentBlueprint {
                    decl: agent_decl.clone(),
                    states,
                    program: program.clone(),
                    registry: providers.clone(),
                    tracer: tracer.clone(),
                },
            );
        }

        // Parse wiring expressions into alias chains
        let wiring = Self::parse_wiring(&system_decl.wiring)?;

        // Create shared infrastructure
        let event_bus: SharedEventBus =
            Arc::new(tokio::sync::RwLock::new(EventBus::new(tracer.clone())));
        let instance_registry: SharedInstanceRegistry =
            Arc::new(tokio::sync::RwLock::new(InstanceRegistry::new()));

        // Discover which agents are covered by wardens
        let binding_targets: HashMap<&str, &str> = bindings
            .iter()
            .map(|(a, t)| (t.as_str(), a.as_str()))
            .collect();

        let mut supervised_aliases: HashSet<String> = HashSet::new();
        let mut warded_runtimes = Vec::new();

        for warden_decl in warden_decls {
            // Check if this warden manages any agents in our system bindings
            let manages_system_agents = warden_decl
                .manages
                .iter()
                .any(|m| binding_targets.contains_key(m.node.as_str()));

            if manages_system_agents {
                for managed in &warden_decl.manages {
                    if let Some(alias) = binding_targets.get(managed.node.as_str()) {
                        supervised_aliases.insert(alias.to_string());
                    }
                }

                let warded = WardedRuntime::with_shared_infrastructure(
                    warden_decl,
                    program,
                    providers.clone(),
                    tracer.clone(),
                    event_bus.clone(),
                    instance_registry.clone(),
                );
                warded_runtimes.push(warded);
            }
        }

        // Unsupervised agents: those in bindings but not covered by any warden
        let unsupervised_blueprints: Vec<String> = bindings
            .iter()
            .filter(|(alias, _)| !supervised_aliases.contains(alias))
            .map(|(alias, _)| alias.clone())
            .collect();

        let max_agents = config.and_then(|c| c.max_agents);

        Ok(Self {
            name,
            bindings,
            wiring,
            event_bus,
            instance_registry,
            storage: None,
            blueprints,
            warded_runtimes,
            unsupervised_blueprints,
            unsupervised_agents: HashMap::new(),
            max_agents,
        })
    }

    /// Build a static topology snapshot (call before start()).
    pub fn topology_snapshot(&self) -> TopologySnapshot {
        TopologySnapshot {
            system_name: self.name.clone(),
            bindings: self.bindings.clone(),
            wiring: self.wiring.clone(),
        }
    }

    /// Attach shared warden snapshot handles to all warded runtimes.
    pub fn with_shared_warden_snapshots(mut self, snaps: SharedWardenSnapshots) -> Self {
        self.warded_runtimes = self
            .warded_runtimes
            .into_iter()
            .map(|wr| wr.with_shared_snapshots(snaps.clone()))
            .collect();
        self
    }

    /// Replace event bus and instance registry with pre-existing shared instances.
    /// Call BEFORE `.start()` to share infrastructure with the HTTP server.
    pub fn with_shared_infrastructure(
        mut self,
        event_bus: SharedEventBus,
        instance_registry: SharedInstanceRegistry,
    ) -> Self {
        self.event_bus = event_bus.clone();
        self.instance_registry = instance_registry.clone();
        for wr in &mut self.warded_runtimes {
            wr.set_event_bus(event_bus.clone());
            wr.set_instance_registry(instance_registry.clone());
        }
        self
    }

    /// Inject shared storage so system agents can use data.store/data.get.
    pub fn with_shared_storage(mut self, storage: crate::runtime::storage::SharedStorage) -> Self {
        self.storage = Some(storage.clone());
        for wr in &mut self.warded_runtimes {
            wr.set_storage(storage.clone());
        }
        self
    }

    /// Parse wiring compose expressions into alias chains.
    /// `a >> b >> c` becomes `["a", "b", "c"]`.
    fn parse_wiring(exprs: &[Spanned<Expr>]) -> Result<Vec<Vec<String>>, RuntimeError> {
        let mut chains = Vec::new();
        for expr in exprs {
            let chain = Self::extract_compose_chain(&expr.node)?;
            if chain.len() >= 2 {
                chains.push(chain);
            }
        }
        Ok(chains)
    }

    /// Recursively extract identifiers from a compose expression.
    fn extract_compose_chain(expr: &Expr) -> Result<Vec<String>, RuntimeError> {
        match expr {
            Expr::Compose(parts) => {
                let mut chain = Vec::new();
                for part in parts {
                    chain.extend(Self::extract_compose_chain(&part.node)?);
                }
                Ok(chain)
            }
            Expr::Ident(name) => Ok(vec![name.clone()]),
            other => Err(RuntimeError::Unsupported(format!(
                "system wiring: unsupported expression in compose chain: {:?}",
                other
            ))),
        }
    }

    /// Set up event routing based on wiring chains.
    /// For chain [a, b, c]: events from a are forwarded to b, events from b forwarded to c.
    /// Uses the EventBus routing table so forwarding happens inline during publish().
    async fn setup_routing(&self) -> Result<(), RuntimeError> {
        let mut bus_guard = self.event_bus.write().await;

        for chain in &self.wiring {
            for window in chain.windows(2) {
                let source_alias = &window[0];
                let target_alias = &window[1];

                // Resolve agent names from aliases
                let source_agent = self
                    .bindings
                    .iter()
                    .find(|(a, _)| a == source_alias)
                    .map(|(_, t)| t.clone())
                    .ok_or_else(|| {
                        RuntimeError::Unsupported(format!(
                            "system '{}': wiring references unknown alias '{}'",
                            self.name, source_alias
                        ))
                    })?;

                let target_agent = self
                    .bindings
                    .iter()
                    .find(|(a, _)| a == target_alias)
                    .map(|(_, t)| t.clone())
                    .ok_or_else(|| {
                        RuntimeError::Unsupported(format!(
                            "system '{}': wiring references unknown alias '{}'",
                            self.name, target_alias
                        ))
                    })?;

                bus_guard.add_route(&source_agent, &target_agent);
            }
        }

        Ok(())
    }

    /// Spawn an unsupervised agent by alias.
    async fn spawn_unsupervised(&mut self, alias: &str) -> Result<(), RuntimeError> {
        let blueprint = self.blueprints.get(alias).ok_or_else(|| {
            RuntimeError::Unsupported(format!("no blueprint for alias '{}'", alias))
        })?;

        // Enforce max_agents limit
        if let Some(max) = self.max_agents {
            let current_count = self.instance_registry.read().await.len();
            if current_count >= max {
                return Err(RuntimeError::Unsupported(format!(
                    "system '{}': max_agents limit ({}) reached",
                    self.name, max
                )));
            }
        }

        let mut process = AgentProcess::new(
            blueprint.decl.clone(),
            blueprint.states.as_ref(),
            blueprint.registry.clone(),
            blueprint.tracer.clone(),
            blueprint.program.clone(),
            self.storage.clone(),
            Some(self.instance_registry.clone()),
        );

        process = process.with_event_bus(self.event_bus.clone()).await;

        let context_ref = process.context().clone();
        let instance_id = self.instance_registry.write().await.register_with_context(
            &blueprint.decl.name.node,
            Some(alias),
            context_ref,
        );

        let handle = tokio::spawn(async move { process.run().await });

        self.unsupervised_agents.insert(
            alias.to_string(),
            UnsupervisedAgent {
                handle,
                instance_id,
            },
        );

        Ok(())
    }

    /// Start the system: spawn all agents, set up routing, and monitor until completion.
    pub async fn start(mut self) -> Result<(), RuntimeError> {
        eprintln!("system '{}': starting", self.name);

        // Set up event routing from wiring chains
        self.setup_routing().await?;

        // Spawn supervised agents via warded runtimes
        for warded in &mut self.warded_runtimes {
            warded.spawn_all().await?;
        }

        // Spawn unsupervised agents
        let unsupervised_aliases: Vec<String> = self.unsupervised_blueprints.clone();
        for alias in &unsupervised_aliases {
            self.spawn_unsupervised(alias).await?;
        }

        let total_agents = self.bindings.len(); // all bound agents were spawned
        eprintln!(
            "system '{}': spawned {} agent(s), {} warden(s)",
            self.name,
            total_agents,
            self.warded_runtimes.len()
        );

        // Run all warded runtimes concurrently with unsupervised agent monitoring
        let mut warded_handles: Vec<JoinHandle<Result<(), RuntimeError>>> = Vec::new();
        for mut warded in self.warded_runtimes {
            warded_handles.push(tokio::spawn(async move { warded.run().await }));
        }

        // Monitor until all agents have exited
        loop {
            // Check unsupervised agents
            let mut finished = Vec::new();
            for (alias, agent) in &self.unsupervised_agents {
                if agent.handle.is_finished() {
                    finished.push(alias.clone());
                }
            }
            for alias in &finished {
                if let Some(agent) = self.unsupervised_agents.remove(alias) {
                    self.instance_registry
                        .write()
                        .await
                        .unregister(&agent.instance_id);
                    match agent.handle.await {
                        Ok(Ok(())) => {}
                        Ok(Err(e)) => {
                            eprintln!(
                                "system '{}': unsupervised agent '{}' failed: {}",
                                self.name, alias, e
                            );
                        }
                        Err(e) => {
                            eprintln!(
                                "system '{}': unsupervised agent '{}' panicked: {}",
                                self.name, alias, e
                            );
                        }
                    }
                }
            }

            // Check warded runtimes
            warded_handles.retain(|h| !h.is_finished());

            // All done?
            if self.unsupervised_agents.is_empty() && warded_handles.is_empty() {
                break;
            }

            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }

        eprintln!("system '{}': all agents exited", self.name);
        Ok(())
    }

    /// Get a reference to the shared event bus.
    pub fn event_bus(&self) -> &SharedEventBus {
        &self.event_bus
    }

    /// Get a reference to the shared instance registry.
    pub fn instance_registry(&self) -> &SharedInstanceRegistry {
        &self.instance_registry
    }

    /// Collect signal senders from all warded runtimes (issue #143).
    /// Must be called **before** `start()` consumes self.
    pub fn collect_signal_senders(&self) -> SharedSignalSenders {
        let mut map = HashMap::new();
        for warded in &self.warded_runtimes {
            let tx = warded.signal_sender();
            for agent_name in warded.managed_agent_names() {
                map.insert(agent_name, tx.clone());
            }
        }
        Arc::new(tokio::sync::RwLock::new(map))
    }
}
