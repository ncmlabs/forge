// FORGE correlation driver — issue #334
// Peer driver to `WakeService::CronDriver`. Inbound events matching a
// persisted correlation key rehydrate the owning specialist instead of
// spawning fresh. Event-driven (not tick-driven): runs at dispatch time
// against the shared redb storage.

use std::collections::HashMap;
use std::sync::Arc;

use crate::ast::ScheduleMode;
use crate::runtime::confidence::Value;
use crate::runtime::event_bus::EventPayload;
use crate::runtime::storage::{SharedStorage, StorageError};

/// One declared correlation on a specific agent: incoming events of
/// `event_type` whose `field_name` matches a persisted row route to
/// `agent_alias`.
#[derive(Debug, Clone)]
pub struct CorrelationRegistration {
    pub agent_alias: String,
    pub event_type: String,
    pub field_name: String,
    pub mode: ScheduleMode,
    pub emit: Option<String>,
}

/// Outcome when an inbound event matches a persisted correlation row.
#[derive(Debug, Clone)]
pub struct CorrelationHit {
    pub target_alias: String,
    pub event_type: String,
    pub field_name: String,
    pub field_value: String,
    pub mode: ScheduleMode,
    pub emit: Option<String>,
}

/// Central correlation dispatcher. Cheap to clone — internals are behind `Arc`.
#[derive(Clone)]
pub struct CorrelationDriver {
    storage: SharedStorage,
    /// event_type -> list of registrations (one per agent that correlates on it).
    registry: Arc<HashMap<String, Vec<CorrelationRegistration>>>,
}

impl CorrelationDriver {
    pub fn new(storage: SharedStorage, registrations: Vec<CorrelationRegistration>) -> Self {
        let mut registry: HashMap<String, Vec<CorrelationRegistration>> = HashMap::new();
        for reg in registrations {
            registry
                .entry(reg.event_type.clone())
                .or_default()
                .push(reg);
        }
        Self {
            storage,
            registry: Arc::new(registry),
        }
    }

    /// Attempt to match an inbound event to a persisted correlation row.
    /// Returns `Ok(None)` if no registration covers this event name, if the
    /// declared field is absent or not Text, or if no row exists for the value.
    pub fn match_event(
        &self,
        payload: &EventPayload,
    ) -> Result<Option<CorrelationHit>, StorageError> {
        let Some(regs) = self.registry.get(&payload.event_name) else {
            return Ok(None);
        };
        for reg in regs {
            let Some(cv) = payload.fields.get(&reg.field_name) else {
                continue;
            };
            let Value::Text(field_value) = &cv.value else {
                continue;
            };
            if field_value.is_empty() {
                continue;
            }
            if let Some(target) =
                self.storage
                    .lookup_correlation(&reg.agent_alias, &reg.field_name, field_value)?
            {
                return Ok(Some(CorrelationHit {
                    target_alias: target,
                    event_type: reg.event_type.clone(),
                    field_name: reg.field_name.clone(),
                    field_value: field_value.clone(),
                    mode: reg.mode,
                    emit: reg.emit.clone(),
                }));
            }
        }
        Ok(None)
    }

    /// Return `(field_name, field_value)` for the first registered field on
    /// this event that has a matching Text value in the payload. Used by the
    /// miss-tracer so we only log misses for events that are actually
    /// subject to correlation — unrelated events skip the trace.
    pub fn first_registered_field(&self, payload: &EventPayload) -> Option<(String, String)> {
        let regs = self.registry.get(&payload.event_name)?;
        for reg in regs {
            if let Some(cv) = payload.fields.get(&reg.field_name) {
                if let Value::Text(s) = &cv.value {
                    if !s.is_empty() {
                        return Some((reg.field_name.clone(), s.clone()));
                    }
                }
            }
        }
        None
    }

    /// Return registrations keyed by `(agent_alias, field_name)` for the
    /// memory-write path to know which fields to persist into the
    /// correlations table.
    pub fn agent_field_targets(&self, agent_alias: &str) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for regs in self.registry.values() {
            for reg in regs {
                if reg.agent_alias == agent_alias {
                    out.push((reg.field_name.clone(), reg.agent_alias.clone()));
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::confidence::ConfidentValue;
    use crate::runtime::storage::ForgeStorage;
    use std::sync::Arc;
    use tempfile::tempdir;

    fn mk_storage() -> (tempfile::TempDir, SharedStorage) {
        let dir = tempdir().unwrap();
        let db = ForgeStorage::open(&dir.path().join("test.redb")).unwrap();
        (dir, Arc::new(db))
    }

    fn payload(event: &str, field: &str, value: &str) -> EventPayload {
        let mut fields = HashMap::new();
        fields.insert(
            field.to_string(),
            ConfidentValue::deterministic(Value::Text(value.to_string())),
        );
        EventPayload {
            event_name: event.to_string(),
            args: Vec::new(),
            source_agent: "tester".to_string(),
            fields,
        }
    }

    fn reg(agent: &str, event: &str, field: &str) -> CorrelationRegistration {
        CorrelationRegistration {
            agent_alias: agent.to_string(),
            event_type: event.to_string(),
            field_name: field.to_string(),
            mode: ScheduleMode::Wake,
            emit: None,
        }
    }

    #[test]
    fn match_event_returns_none_when_event_not_registered() {
        let (_dir, storage) = mk_storage();
        let driver = CorrelationDriver::new(storage, vec![]);
        let p = payload("SlackMention", "thread_ts", "T1");
        assert!(driver.match_event(&p).unwrap().is_none());
    }

    #[test]
    fn match_event_returns_none_when_no_row_exists() {
        let (_dir, storage) = mk_storage();
        let driver = CorrelationDriver::new(
            storage,
            vec![reg("slack_specialist", "SlackMention", "thread_ts")],
        );
        let p = payload("SlackMention", "thread_ts", "T-new");
        assert!(driver.match_event(&p).unwrap().is_none());
    }

    #[test]
    fn match_event_returns_hit_when_row_exists() {
        let (_dir, storage) = mk_storage();
        storage
            .upsert_correlation("slack_specialist", "thread_ts", "T1", "slack_specialist")
            .unwrap();
        let driver = CorrelationDriver::new(
            storage,
            vec![reg("slack_specialist", "SlackMention", "thread_ts")],
        );
        let p = payload("SlackMention", "thread_ts", "T1");
        let hit = driver.match_event(&p).unwrap().expect("expected hit");
        assert_eq!(hit.target_alias, "slack_specialist");
        assert_eq!(hit.event_type, "SlackMention");
        assert_eq!(hit.field_name, "thread_ts");
        assert_eq!(hit.field_value, "T1");
        assert_eq!(hit.mode, ScheduleMode::Wake);
    }

    #[test]
    fn match_event_skips_when_field_absent_or_wrong_type() {
        let (_dir, storage) = mk_storage();
        storage
            .upsert_correlation("a", "thread_ts", "T1", "a")
            .unwrap();
        let driver = CorrelationDriver::new(storage, vec![reg("a", "SlackMention", "thread_ts")]);

        // Missing field
        let mut p1 = payload("SlackMention", "other", "T1");
        p1.fields.remove("thread_ts");
        assert!(driver.match_event(&p1).unwrap().is_none());

        // Wrong-type field (Number instead of Text)
        let mut fields = HashMap::new();
        fields.insert(
            "thread_ts".to_string(),
            ConfidentValue::deterministic(Value::Number(1.0)),
        );
        let p2 = EventPayload {
            event_name: "SlackMention".to_string(),
            args: Vec::new(),
            source_agent: "t".to_string(),
            fields,
        };
        assert!(driver.match_event(&p2).unwrap().is_none());
    }

    #[test]
    fn agent_field_targets_lists_declared_fields() {
        let (_dir, storage) = mk_storage();
        let driver = CorrelationDriver::new(
            storage,
            vec![
                reg("slack_specialist", "SlackMention", "thread_ts"),
                reg("other_agent", "OtherEvent", "key"),
            ],
        );
        let targets = driver.agent_field_targets("slack_specialist");
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].0, "thread_ts");
    }
}
