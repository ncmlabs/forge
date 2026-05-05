// FORGE agent lifecycle — issue #333
//
// `AgentLifecycle` owns "is this agent live? if not, bring it up" logic. It
// exists so that `WakeService::dispatch_wake` (and future drivers #334/#335)
// can restore an agent's `memory persistent` and re-subscribe it to the bus
// BEFORE publishing a wake event — handlers must never observe empty state
// (Principle I, honesty).
//
// The module extracts the canonical "build an AgentProcess + wire bus +
// register + tokio::spawn" flow that `SystemRuntime::spawn_unsupervised`
// already implements, and exposes it as `rehydrate_or_spawn` for re-entry.
//
// Design notes:
// - `rehydrate_or_spawn` is idempotent on a per-alias basis: it first checks
//   `InstanceRegistry::find_by_name(decl_name)`; if any live instance exists,
//   it returns a handle without spawning a duplicate.
// - Memory restoration is reused verbatim from `AgentProcess::new` (the redb
//   key `agent:{decl_name}:memory` + `AgentMemory::restore_from_json`). To
//   surface `memory_keys_restored` for the tracer, we peek at the stored JSON
//   before constructing the process.
// - The wake path does NOT publish the event here; that's WakeService's job
//   once the handle is returned. Keeps single-responsibility.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value as JsonValue;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::runtime::agent::AgentProcess;
use crate::runtime::event_bus::SharedEventBus;
use crate::runtime::executor::RuntimeError;
use crate::runtime::instance_registry::SharedInstanceRegistry;
use crate::runtime::storage::SharedStorage;
use crate::runtime::warded::AgentBlueprint;

/// Result of a `rehydrate_or_spawn` call.
#[derive(Debug, Clone)]
pub struct AgentHandle {
    /// The blueprint key used to look up the agent (system alias).
    pub alias: String,
    /// The agent declaration name (used for redb storage keys and InstanceRegistry).
    pub agent_name: String,
    /// InstanceRegistry UUID of the live instance.
    pub instance_id: Uuid,
    /// `memory persistent` keys that were overwritten from redb during this
    /// call. Empty if the agent was already live (no new restore performed)
    /// or if no persisted memory exists yet.
    pub memory_keys_restored: Vec<String>,
    /// True if we reused an existing live instance rather than spawning one.
    pub was_already_live: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum AgentLifecycleError {
    #[error("no blueprint declared for alias '{0}'")]
    NotDeclared(String),
    #[error("max_agents limit ({0}) reached")]
    MaxAgents(usize),
    #[error("spawn failed: {0}")]
    Spawn(String),
}

/// Reusable spawner for in-process FORGE agents. Cloneable (shares state via `Arc`).
#[derive(Clone)]
pub struct AgentLifecycle {
    blueprints: Arc<HashMap<String, AgentBlueprint>>,
    instance_registry: SharedInstanceRegistry,
    event_bus: SharedEventBus,
    storage: Option<SharedStorage>,
    max_agents: Option<usize>,
    /// Tokio handles for agents we spawned. Kept so callers can join on shutdown.
    spawned: Arc<Mutex<HashMap<String, SpawnedAgent>>>,
}

struct SpawnedAgent {
    #[allow(dead_code)]
    handle: JoinHandle<Result<(), RuntimeError>>,
    #[allow(dead_code)]
    instance_id: Uuid,
}

impl AgentLifecycle {
    pub fn new(
        blueprints: Arc<HashMap<String, AgentBlueprint>>,
        instance_registry: SharedInstanceRegistry,
        event_bus: SharedEventBus,
        storage: Option<SharedStorage>,
        max_agents: Option<usize>,
    ) -> Self {
        Self {
            blueprints,
            instance_registry,
            event_bus,
            storage,
            max_agents,
            spawned: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// If a live instance of the agent bound to `alias` exists in the
    /// InstanceRegistry, return a handle referencing it. Otherwise, spawn a
    /// fresh `AgentProcess` (which restores `memory persistent` from redb and
    /// subscribes to the bus), register it, and launch its run loop.
    ///
    /// When this method returns `Ok`, the bus subscription is already live —
    /// so a subsequent `bus.publish(event)` is guaranteed to reach the agent
    /// (ordering is required by Principle I for `mode: wake` handlers).
    pub async fn rehydrate_or_spawn(
        &self,
        alias: &str,
    ) -> Result<AgentHandle, AgentLifecycleError> {
        // Blueprints are keyed by system-binding alias (e.g. "specialist"), but
        // correlation rows are written keyed by the agent declaration name
        // (e.g. "slack_specialist"). Fall back to a decl-name scan so either
        // identifier routes correctly.
        let blueprint = match self.blueprints.get(alias) {
            Some(bp) => bp,
            None => self
                .blueprints
                .values()
                .find(|bp| bp.decl.name.node == alias)
                .ok_or_else(|| AgentLifecycleError::NotDeclared(alias.to_string()))?,
        };
        let agent_name = blueprint.decl.name.node.clone();

        // Fast path: already live?
        let existing = {
            let reg = self.instance_registry.read().await;
            reg.find_by_name(&agent_name).into_iter().next()
        };
        if let Some(info) = existing {
            return Ok(AgentHandle {
                alias: alias.to_string(),
                agent_name,
                instance_id: info.instance_id,
                memory_keys_restored: Vec::new(),
                was_already_live: true,
            });
        }

        // Enforce max_agents (mirrors SystemRuntime::spawn_unsupervised).
        if let Some(max) = self.max_agents {
            let current = self.instance_registry.read().await.len();
            if current >= max {
                return Err(AgentLifecycleError::MaxAgents(max));
            }
        }

        // Peek at persisted memory so we can report which keys were actually
        // restored (intersection of stored JSON keys with declared fields).
        let memory_keys_restored = self.peek_restored_keys(&agent_name, &blueprint.decl.memory);

        // Build AgentProcess (restores memory + state machine internally).
        let mut process = AgentProcess::new(
            blueprint.decl.clone(),
            blueprint.states.as_ref(),
            blueprint.registry.clone(),
            blueprint.tracer.clone(),
            blueprint.program.clone(),
            self.storage.clone(),
            Some(self.instance_registry.clone()),
            blueprint.shared_knowledge_store.clone(),
        );
        if let Some(ref se) = blueprint.skill_executor {
            process = process.with_skill_executor(se.clone());
        }

        // Subscribe to the bus BEFORE spawning the run loop. `with_event_bus`
        // holds the bus write lock while inserting subscribers, so once it
        // returns the agent is observable to any future `publish`.
        process = process.with_event_bus(self.event_bus.clone()).await;

        let context_ref = process.context().clone();
        let instance_id = self.instance_registry.write().await.register_with_context(
            &agent_name,
            Some(alias),
            context_ref,
        );

        let handle = tokio::spawn(async move { process.run().await });

        let mut spawned = self.spawned.lock().await;
        spawned.insert(
            alias.to_string(),
            SpawnedAgent {
                handle,
                instance_id,
            },
        );

        Ok(AgentHandle {
            alias: alias.to_string(),
            agent_name,
            instance_id,
            memory_keys_restored,
            was_already_live: false,
        })
    }

    /// Intersect the JSON keys stored at `agent:{name}:memory` with the
    /// declared `memory` field names. Returns keys that will be overwritten
    /// during `AgentMemory::restore_from_json`.
    fn peek_restored_keys(
        &self,
        agent_name: &str,
        declared: &[crate::ast::Spanned<crate::ast::FieldDef>],
    ) -> Vec<String> {
        let Some(store) = self.storage.as_ref() else {
            return Vec::new();
        };
        let key = format!("agent:{}:memory", agent_name);
        let Ok(Some(json)) = store.get(&key) else {
            return Vec::new();
        };
        let Ok(JsonValue::Object(map)) = serde_json::from_str::<JsonValue>(&json) else {
            return Vec::new();
        };
        let declared_names: std::collections::HashSet<&str> =
            declared.iter().map(|fd| fd.node.name.as_str()).collect();
        let mut keys: Vec<String> = map
            .keys()
            .filter(|k| declared_names.contains(k.as_str()))
            .cloned()
            .collect();
        keys.sort();
        keys
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{AgentDecl, Program, Span, Spanned};
    use crate::llm::registry::ProviderRegistry;
    use crate::runtime::event_bus::EventBus;
    use crate::runtime::instance_registry::InstanceRegistry;
    use crate::runtime::storage::ForgeStorage;

    fn sp<T>(node: T) -> Spanned<T> {
        Spanned {
            node,
            span: Span { start: 0, end: 0 },
        }
    }

    fn empty_program() -> Program {
        Program {
            boundary: None,
            items: Vec::new(),
        }
    }

    fn minimal_decl(name: &str) -> AgentDecl {
        AgentDecl {
            exportable: false,
            name: sp(name.to_string()),
            lifecycle: None,
            memory: Vec::new(),
            memory_persistent: false,
            knowledge: None,
            allows: Vec::new(),
            timers: Vec::new(),
            schedules: Vec::new(),
            correlates: Vec::new(),
            webhooks: vec![],
            subscriptions: Vec::new(),
            warden_override: Vec::new(),
            handlers: Vec::new(),
            stuck_policy: None,
        }
    }

    fn blueprint_for(decl: AgentDecl) -> AgentBlueprint {
        AgentBlueprint {
            decl,
            states: None,
            program: empty_program(),
            registry: Arc::new(ProviderRegistry::new("test")),
            tracer: None,
            skill_executor: None,
            shared_knowledge_store: None,
        }
    }

    fn temp_storage() -> (tempfile::TempDir, SharedStorage) {
        let dir = tempfile::tempdir().unwrap();
        let s = ForgeStorage::open(&dir.path().join("t.redb")).unwrap();
        (dir, Arc::new(s))
    }

    #[tokio::test]
    async fn rehydrate_or_spawn_fails_when_alias_undeclared() {
        let blueprints = Arc::new(HashMap::new());
        let reg = Arc::new(tokio::sync::RwLock::new(InstanceRegistry::new()));
        let bus = Arc::new(tokio::sync::RwLock::new(EventBus::new(None)));
        let lifecycle = AgentLifecycle::new(blueprints, reg, bus, None, None);

        let err = lifecycle.rehydrate_or_spawn("missing").await.unwrap_err();
        assert!(matches!(err, AgentLifecycleError::NotDeclared(ref a) if a == "missing"));
    }

    #[tokio::test]
    async fn rehydrate_or_spawn_reuses_existing_live_instance() {
        let decl = minimal_decl("probe");
        let mut bps = HashMap::new();
        bps.insert("probe".to_string(), blueprint_for(decl.clone()));
        let blueprints = Arc::new(bps);
        let reg = Arc::new(tokio::sync::RwLock::new(InstanceRegistry::new()));
        let bus = Arc::new(tokio::sync::RwLock::new(EventBus::new(None)));
        let lifecycle = AgentLifecycle::new(blueprints, reg.clone(), bus, None, None);

        let first = lifecycle.rehydrate_or_spawn("probe").await.unwrap();
        assert!(!first.was_already_live);
        let second = lifecycle.rehydrate_or_spawn("probe").await.unwrap();
        assert!(second.was_already_live);
        assert_eq!(first.instance_id, second.instance_id);
        // Only one instance in the registry.
        assert_eq!(reg.read().await.find_by_name("probe").len(), 1);
    }

    #[tokio::test]
    async fn rehydrate_or_spawn_enforces_max_agents() {
        let decl = minimal_decl("probe");
        let mut bps = HashMap::new();
        bps.insert("probe".to_string(), blueprint_for(decl));
        let blueprints = Arc::new(bps);
        let reg = Arc::new(tokio::sync::RwLock::new(InstanceRegistry::new()));
        let bus = Arc::new(tokio::sync::RwLock::new(EventBus::new(None)));
        // Pre-populate registry so max_agents=1 is already reached.
        reg.write().await.register("other", None);

        let lifecycle = AgentLifecycle::new(blueprints, reg, bus, None, Some(1));
        let err = lifecycle.rehydrate_or_spawn("probe").await.unwrap_err();
        assert!(matches!(err, AgentLifecycleError::MaxAgents(1)));
    }

    #[tokio::test]
    async fn rehydrated_agent_subscribes_to_wake_schedule_emit_event() {
        use crate::ast::{ScheduleField, ScheduleMode, WhenExpr};
        // Regression: for `mode: wake` schedules, the rehydrated agent must
        // subscribe to the `emit:` event name (or `{name}.tick` fallback),
        // not the schedule name itself, because that's what WakeService
        // publishes. Live serve caught `subscribers: 0` on DriftCheckDue
        // before this was fixed in `with_event_bus` (#333).
        let schedule = ScheduleField {
            name: sp("drift_check".to_string()),
            when: Some(sp(WhenExpr::Every(crate::ast::Duration {
                value: 30,
                unit: crate::ast::DurationUnit::Seconds,
            }))),
            mode: Some(sp(ScheduleMode::Wake)),
            prompt: None,
            emit: Some(sp("DriftCheckDue".to_string())),
            precision: None,
            duplicates: Vec::new(),
        };
        let mut decl = minimal_decl("drift_watcher");
        decl.schedules = vec![sp(schedule)];

        let mut bps = HashMap::new();
        bps.insert("drift_watcher".to_string(), blueprint_for(decl));
        let reg = Arc::new(tokio::sync::RwLock::new(InstanceRegistry::new()));
        let bus = Arc::new(tokio::sync::RwLock::new(EventBus::new(None)));
        let lifecycle = AgentLifecycle::new(Arc::new(bps), reg, bus.clone(), None, None);

        let _ = lifecycle.rehydrate_or_spawn("drift_watcher").await.unwrap();

        let bus_guard = bus.read().await;
        assert_eq!(
            bus_guard.subscriber_count("DriftCheckDue"),
            1,
            "agent must subscribe to the schedule's emit event, not its name"
        );
        assert_eq!(
            bus_guard.subscriber_count("drift_check"),
            0,
            "agent must NOT subscribe to the schedule name under mode: wake"
        );
    }

    #[tokio::test]
    async fn rehydrated_agent_subscribes_to_tick_fallback_without_emit() {
        use crate::ast::{ScheduleField, ScheduleMode, WhenExpr};
        let schedule = ScheduleField {
            name: sp("heartbeat".to_string()),
            when: Some(sp(WhenExpr::Every(crate::ast::Duration {
                value: 30,
                unit: crate::ast::DurationUnit::Seconds,
            }))),
            mode: Some(sp(ScheduleMode::Wake)),
            prompt: None,
            emit: None,
            precision: None,
            duplicates: Vec::new(),
        };
        let mut decl = minimal_decl("beat_watcher");
        decl.schedules = vec![sp(schedule)];

        let mut bps = HashMap::new();
        bps.insert("beat_watcher".to_string(), blueprint_for(decl));
        let reg = Arc::new(tokio::sync::RwLock::new(InstanceRegistry::new()));
        let bus = Arc::new(tokio::sync::RwLock::new(EventBus::new(None)));
        let lifecycle = AgentLifecycle::new(Arc::new(bps), reg, bus.clone(), None, None);

        let _ = lifecycle.rehydrate_or_spawn("beat_watcher").await.unwrap();

        let bus_guard = bus.read().await;
        assert_eq!(bus_guard.subscriber_count("heartbeat.tick"), 1);
    }

    #[tokio::test]
    async fn peek_restored_keys_returns_intersection_with_declared_fields() {
        use crate::ast::{FieldDef, TypeName};
        let (_guard, storage) = temp_storage();
        // Write a JSON blob with two known fields and one unknown.
        let stored = serde_json::json!({
            "known_a": { "value": { "Text": "hello" }, "confidence": 1.0, "provenance": [] },
            "known_b": { "value": { "Number": 3.0 }, "confidence": 1.0, "provenance": [] },
            "not_declared": { "value": { "Text": "noise" }, "confidence": 1.0, "provenance": [] },
        });
        storage
            .store("agent:probe:memory", &stored.to_string())
            .unwrap();

        let declared = vec![
            sp(FieldDef {
                name: "known_a".to_string(),
                type_name: sp(TypeName::Text),
            }),
            sp(FieldDef {
                name: "known_b".to_string(),
                type_name: sp(TypeName::Number),
            }),
            sp(FieldDef {
                name: "undeclared_in_storage".to_string(),
                type_name: sp(TypeName::Text),
            }),
        ];

        let blueprints = Arc::new(HashMap::new());
        let reg = Arc::new(tokio::sync::RwLock::new(InstanceRegistry::new()));
        let bus = Arc::new(tokio::sync::RwLock::new(EventBus::new(None)));
        let lifecycle = AgentLifecycle::new(blueprints, reg, bus, Some(storage), None);

        let keys = lifecycle.peek_restored_keys("probe", &declared);
        assert_eq!(keys, vec!["known_a".to_string(), "known_b".to_string()]);
    }
}
