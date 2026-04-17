// Task-history aggregator for the mastery Observer tile (issue #304).
// Subscribes to the FORGE event bus for `TaskCompleted` events and keeps a
// rolling window of task records per project. Serves the
// `/__forge/inspect/mastery` endpoint with per-task `review_rounds` data —
// the load-bearing metric behind the "10th task needs fewer approval asks
// than the 1st" proof point (#292).
//
// Mirrors `cost_aggregator.rs` in spirit: a lightweight accumulator that
// lives alongside the server and exposes a JSON snapshot. Unlike the cost
// aggregator, it subscribes to the typed event bus (where `TaskCompleted`
// arrives with full field values) rather than the trace SSE channel (where
// `event_emit` carries only the event name).

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Instant;

use chrono::{DateTime, Utc};
use tokio::sync::RwLock;

use crate::runtime::confidence::Value;
use crate::runtime::event_bus::{EventPayload, SharedEventBus};

pub type SharedTaskHistoryAggregator = Arc<RwLock<TaskHistoryAggregator>>;

/// One completed task, capturing the signals carried by `TaskCompleted`.
#[derive(Debug, Clone)]
pub struct TaskRecord {
    pub task_id: String,
    pub repo: String,
    pub outcome: String,
    pub review_rounds: u64,
    pub ci_passed_first_try: bool,
    pub time_to_merge: f64,
    pub reverted_within_7d: bool,
    pub completed_at: DateTime<Utc>,
}

impl TaskRecord {
    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "task_id": self.task_id,
            "repo": self.repo,
            "outcome": self.outcome,
            "review_rounds": self.review_rounds,
            "ci_passed_first_try": self.ci_passed_first_try,
            "time_to_merge": self.time_to_merge,
            "reverted_within_7d": self.reverted_within_7d,
            "completed_at": self.completed_at.to_rfc3339(),
        })
    }
}

pub struct TaskHistoryAggregator {
    records_by_project: HashMap<String, VecDeque<TaskRecord>>,
    capacity_per_project: usize,
    total_tasks: u64,
    started_at: Instant,
}

impl Default for TaskHistoryAggregator {
    fn default() -> Self {
        Self::new(100)
    }
}

impl TaskHistoryAggregator {
    pub fn new(capacity_per_project: usize) -> Self {
        Self {
            records_by_project: HashMap::new(),
            capacity_per_project,
            total_tasks: 0,
            started_at: Instant::now(),
        }
    }

    pub fn record(&mut self, record: TaskRecord) {
        let bucket = self
            .records_by_project
            .entry(record.repo.clone())
            .or_default();
        if bucket.len() >= self.capacity_per_project {
            bucket.pop_front();
        }
        bucket.push_back(record);
        self.total_tasks += 1;
    }

    pub fn record_from_payload(&mut self, payload: &EventPayload) {
        if let Some(record) = payload_to_task_record(payload) {
            self.record(record);
        }
    }

    pub fn snapshot(&self) -> serde_json::Value {
        let tasks_by_project: serde_json::Map<String, serde_json::Value> = self
            .records_by_project
            .iter()
            .map(|(project, records)| {
                let items: Vec<serde_json::Value> = records.iter().map(|r| r.to_json()).collect();
                (project.clone(), serde_json::Value::Array(items))
            })
            .collect();
        let projects: Vec<String> = self.records_by_project.keys().cloned().collect();
        serde_json::json!({
            "total_tasks": self.total_tasks,
            "projects": projects,
            "tasks_by_project": tasks_by_project,
            "uptime_secs": (self.started_at.elapsed().as_secs_f64() * 10.0).round() / 10.0,
        })
    }
}

/// Pull a `TaskRecord` out of an `EventPayload` whose name is `TaskCompleted`.
/// Returns `None` if required fields are missing — which would only happen
/// under a grammar/runtime regression, so we silently skip rather than panic.
fn payload_to_task_record(payload: &EventPayload) -> Option<TaskRecord> {
    if payload.event_name != "TaskCompleted" {
        return None;
    }
    let task_id = text_field(payload, "task_id")?;
    let repo = text_field(payload, "repo")?;
    let outcome = text_field(payload, "outcome").unwrap_or_else(|| "unknown".to_string());
    let ci_passed_first_try = bool_field(payload, "ci_passed_first_try").unwrap_or(false);
    let review_rounds = number_field(payload, "review_rounds").unwrap_or(0.0);
    let time_to_merge = number_field(payload, "time_to_merge").unwrap_or(0.0);
    let reverted_within_7d = bool_field(payload, "reverted_within_7d").unwrap_or(false);
    Some(TaskRecord {
        task_id,
        repo,
        outcome,
        review_rounds: review_rounds.max(0.0).round() as u64,
        ci_passed_first_try,
        time_to_merge,
        reverted_within_7d,
        completed_at: Utc::now(),
    })
}

fn text_field(payload: &EventPayload, name: &str) -> Option<String> {
    match payload.fields.get(name).map(|cv| &cv.value) {
        Some(Value::Text(s)) => Some(s.clone()),
        _ => None,
    }
}

fn bool_field(payload: &EventPayload, name: &str) -> Option<bool> {
    match payload.fields.get(name).map(|cv| &cv.value) {
        Some(Value::Bool(b)) => Some(*b),
        _ => None,
    }
}

fn number_field(payload: &EventPayload, name: &str) -> Option<f64> {
    match payload.fields.get(name).map(|cv| &cv.value) {
        Some(Value::Number(n)) => Some(*n),
        _ => None,
    }
}

/// Subscribe the aggregator to the event bus for `TaskCompleted` events.
/// Spawns a background task that drains the subscription channel.
pub async fn spawn_task_listener(
    bus: SharedEventBus,
    aggregator: SharedTaskHistoryAggregator,
) -> tokio::task::JoinHandle<()> {
    let mut rx = bus
        .write()
        .await
        .subscribe("TaskCompleted", "__task_history_aggregator", None);
    tokio::spawn(async move {
        while let Some(payload) = rx.recv().await {
            aggregator.write().await.record_from_payload(&payload);
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::confidence::ConfidentValue;

    fn det(v: Value) -> ConfidentValue {
        ConfidentValue::deterministic(v)
    }

    fn make_payload(repo: &str, task_id: &str, review_rounds: f64) -> EventPayload {
        let mut fields = HashMap::new();
        fields.insert("task_id".to_string(), det(Value::Text(task_id.to_string())));
        fields.insert("repo".to_string(), det(Value::Text(repo.to_string())));
        fields.insert(
            "outcome".to_string(),
            det(Value::Text("merged".to_string())),
        );
        fields.insert("ci_passed_first_try".to_string(), det(Value::Bool(true)));
        fields.insert(
            "review_rounds".to_string(),
            det(Value::Number(review_rounds)),
        );
        fields.insert("time_to_merge".to_string(), det(Value::Number(1800.0)));
        fields.insert("reverted_within_7d".to_string(), det(Value::Bool(false)));
        EventPayload {
            event_name: "TaskCompleted".to_string(),
            args: vec![],
            source_agent: "release_manager".to_string(),
            fields,
        }
    }

    #[test]
    fn records_task_and_snapshots() {
        let mut agg = TaskHistoryAggregator::new(100);
        agg.record_from_payload(&make_payload("ncmlabs-forge-playground", "task-1", 1.0));
        agg.record_from_payload(&make_payload("ncmlabs-forge-playground", "task-2", 2.0));
        let snapshot = agg.snapshot();
        assert_eq!(snapshot["total_tasks"].as_u64(), Some(2));
        let tasks = snapshot["tasks_by_project"]["ncmlabs-forge-playground"]
            .as_array()
            .expect("array");
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0]["task_id"].as_str(), Some("task-1"));
        assert_eq!(tasks[1]["review_rounds"].as_u64(), Some(2));
    }

    #[test]
    fn evicts_oldest_beyond_capacity() {
        let mut agg = TaskHistoryAggregator::new(2);
        agg.record_from_payload(&make_payload("repo", "t-1", 0.0));
        agg.record_from_payload(&make_payload("repo", "t-2", 0.0));
        agg.record_from_payload(&make_payload("repo", "t-3", 0.0));
        let snapshot = agg.snapshot();
        let tasks = snapshot["tasks_by_project"]["repo"].as_array().unwrap();
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0]["task_id"].as_str(), Some("t-2"));
        assert_eq!(tasks[1]["task_id"].as_str(), Some("t-3"));
    }

    #[test]
    fn ignores_non_task_completed_events() {
        let mut agg = TaskHistoryAggregator::new(10);
        let mut payload = make_payload("repo", "t", 0.0);
        payload.event_name = "SomethingElse".to_string();
        agg.record_from_payload(&payload);
        assert_eq!(agg.snapshot()["total_tasks"].as_u64(), Some(0));
    }
}
