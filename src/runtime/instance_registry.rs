// FORGE agent instance registry — issue #82
// Tracks all living agent instances at runtime for discovery and composition.
// Principle VI (Self-Reference): agents need to discover and compose with other agents.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use uuid::Uuid;

// ── Instance Status ─────────────────────────────────────────────────────────

/// Current status of an agent instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstanceStatus {
    Running,
    Stopping,
}

// ── Instance Info ───────────────────────────────────────────────────────────

/// Metadata about a single running agent instance.
#[derive(Debug, Clone)]
pub struct InstanceInfo {
    pub instance_id: Uuid,
    pub agent_name: String,
    pub alias: Option<String>,
    pub lifecycle_state: Option<String>,
    pub spawned_at: Instant,
    pub status: InstanceStatus,
}

// ── Instance Registry ───────────────────────────────────────────────────────

/// Registry of all living agent instances, supporting lookup by ID, name, or alias.
pub struct InstanceRegistry {
    by_id: HashMap<Uuid, InstanceInfo>,
    by_name: HashMap<String, Vec<Uuid>>,
    by_alias: HashMap<String, Uuid>,
}

/// Thread-safe shared reference to the instance registry.
pub type SharedInstanceRegistry = Arc<tokio::sync::RwLock<InstanceRegistry>>;

impl Default for InstanceRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl InstanceRegistry {
    pub fn new() -> Self {
        Self {
            by_id: HashMap::new(),
            by_name: HashMap::new(),
            by_alias: HashMap::new(),
        }
    }

    /// Register a new agent instance with an optional alias. Returns the generated instance ID.
    pub fn register(&mut self, agent_name: &str, alias: Option<&str>) -> Uuid {
        let id = Uuid::new_v4();
        let info = InstanceInfo {
            instance_id: id,
            agent_name: agent_name.to_string(),
            alias: alias.map(|s| s.to_string()),
            lifecycle_state: None,
            spawned_at: Instant::now(),
            status: InstanceStatus::Running,
        };
        self.by_id.insert(id, info);
        self.by_name
            .entry(agent_name.to_string())
            .or_default()
            .push(id);
        if let Some(a) = alias {
            self.by_alias.insert(a.to_string(), id);
        }
        id
    }

    /// Unregister an agent instance by ID. Cleans up all maps.
    pub fn unregister(&mut self, instance_id: &Uuid) {
        if let Some(info) = self.by_id.remove(instance_id) {
            if let Some(ids) = self.by_name.get_mut(&info.agent_name) {
                ids.retain(|id| id != instance_id);
                if ids.is_empty() {
                    self.by_name.remove(&info.agent_name);
                }
            }
            if let Some(ref alias) = info.alias {
                self.by_alias.remove(alias);
            }
        }
    }

    /// Find a single instance by its alias.
    pub fn find_by_alias(&self, alias: &str) -> Option<InstanceInfo> {
        self.by_alias
            .get(alias)
            .and_then(|id| self.by_id.get(id).cloned())
    }

    /// Update the lifecycle state of an instance.
    pub fn update_lifecycle(&mut self, instance_id: &Uuid, state: &str) {
        if let Some(info) = self.by_id.get_mut(instance_id) {
            info.lifecycle_state = Some(state.to_string());
        }
    }

    /// Find all instances with the given agent/template name.
    pub fn find_by_name(&self, name: &str) -> Vec<InstanceInfo> {
        self.by_name
            .get(name)
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| self.by_id.get(id).cloned())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Find all instances spawned from a given template (agent declaration name).
    /// Equivalent to `find_by_name` — both look up by the agent declaration name.
    pub fn find_all_by_template(&self, template_name: &str) -> Vec<InstanceInfo> {
        self.find_by_name(template_name)
    }

    /// Return all live instances.
    pub fn find_all(&self) -> Vec<InstanceInfo> {
        self.by_id.values().cloned().collect()
    }

    /// Look up a single instance by ID.
    pub fn get(&self, id: &Uuid) -> Option<InstanceInfo> {
        self.by_id.get(id).cloned()
    }

    /// Number of registered instances.
    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_and_find_by_name() {
        let mut reg = InstanceRegistry::new();
        let id = reg.register("greeter", None);

        let found = reg.find_by_name("greeter");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].instance_id, id);
        assert_eq!(found[0].agent_name, "greeter");
        assert_eq!(found[0].status, InstanceStatus::Running);
    }

    #[test]
    fn register_multiple_same_name() {
        let mut reg = InstanceRegistry::new();
        let id1 = reg.register("worker", None);
        let id2 = reg.register("worker", None);
        assert_ne!(id1, id2);

        let found = reg.find_by_name("worker");
        assert_eq!(found.len(), 2);
        assert_eq!(reg.len(), 2);
    }

    #[test]
    fn unregister_cleans_both_maps() {
        let mut reg = InstanceRegistry::new();
        let id1 = reg.register("worker", None);
        let id2 = reg.register("worker", None);

        reg.unregister(&id1);
        assert_eq!(reg.len(), 1);
        assert!(reg.get(&id1).is_none());
        assert!(reg.get(&id2).is_some());

        let found = reg.find_by_name("worker");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].instance_id, id2);
    }

    #[test]
    fn unregister_last_removes_name_entry() {
        let mut reg = InstanceRegistry::new();
        let id = reg.register("solo", None);

        reg.unregister(&id);
        assert!(reg.is_empty());
        assert!(reg.find_by_name("solo").is_empty());
    }

    #[test]
    fn find_by_name_empty() {
        let reg = InstanceRegistry::new();
        assert!(reg.find_by_name("nonexistent").is_empty());
    }

    #[test]
    fn find_all_returns_all_instances() {
        let mut reg = InstanceRegistry::new();
        reg.register("alpha", None);
        reg.register("beta", None);
        reg.register("alpha", None);

        assert_eq!(reg.find_all().len(), 3);
    }

    #[test]
    fn register_with_alias() {
        let mut reg = InstanceRegistry::new();
        let id = reg.register("worker", Some("room_42_bot"));

        let found = reg.find_by_alias("room_42_bot");
        assert!(found.is_some());
        let info = found.unwrap();
        assert_eq!(info.instance_id, id);
        assert_eq!(info.alias.as_deref(), Some("room_42_bot"));
    }

    #[test]
    fn find_by_alias_not_found() {
        let reg = InstanceRegistry::new();
        assert!(reg.find_by_alias("nonexistent").is_none());
    }

    #[test]
    fn unregister_cleans_alias() {
        let mut reg = InstanceRegistry::new();
        let id = reg.register("worker", Some("my_alias"));
        assert!(reg.find_by_alias("my_alias").is_some());

        reg.unregister(&id);
        assert!(reg.find_by_alias("my_alias").is_none());
    }

    #[test]
    fn update_lifecycle() {
        let mut reg = InstanceRegistry::new();
        let id = reg.register("worker", None);
        assert!(reg.get(&id).unwrap().lifecycle_state.is_none());

        reg.update_lifecycle(&id, "expert");
        assert_eq!(
            reg.get(&id).unwrap().lifecycle_state.as_deref(),
            Some("expert")
        );
    }

    #[test]
    fn unregister_nonexistent_is_noop() {
        let mut reg = InstanceRegistry::new();
        reg.register("a", None);
        reg.unregister(&Uuid::new_v4());
        assert_eq!(reg.len(), 1);
    }
}
