//! Per-issue JSON metrics sink for T11.3 (#372) clone-dev proof runs.
//!
//! Opt-in via env `FORGE_PROOF_RUN_ID` — when unset, [`spawn_proof_run_sink`]
//! returns `None` and no listeners are wired. When set, the sink:
//!
//! 1. Subscribes to `IssueAssigned` on the event bus. Stamps `assigned_at`
//!    and snapshots the current per-specialist mastery levels for the
//!    issue's project. State is keyed by `issue_id`.
//! 2. Subscribes to `TaskCompleted`. After a [`SETTLE_WINDOW`] deferral
//!    (so `MasterySignal` → tuple update → `MasteryUpdated` can land),
//!    snapshots mastery levels again, computes `time_to_merge_mins`, and
//!    writes `metrics/proof-runs/<run_id>/issue-<task_id>.json` with the
//!    DoD fields plus the mastery before/after deltas.
//!
//! The proof criterion (issue #10's `approval_asks` < issue #1's) is
//! verified by hand from the written JSON files; this sink's job is just
//! to produce them honestly. Any decision logic stays outside the
//! runtime.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde_json::json;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;

use crate::runtime::confidence::Value;
use crate::runtime::event_bus::{EventPayload, SharedEventBus};
use crate::runtime::knowledge_store::SharedKnowledgeStore;
use crate::runtime::mastery_view;

/// Environment variable that activates the sink. Set this in the
/// proof-run launch script (see `docs/proof-runs/...`).
pub const RUN_ID_ENV: &str = "FORGE_PROOF_RUN_ID";

/// Base directory (relative to cwd) for proof-run output.
pub const METRICS_BASE: &str = "metrics/proof-runs";

/// Deferral before snapshotting `mastery_after` on `TaskCompleted`.
/// 1 s is comfortable for the `TaskCompleted` → `MasterySignal` →
/// `swarm_mastery_tuple` update → `MasteryUpdated` chain; document the
/// trade-off if the chain ever grows past that.
pub const SETTLE_WINDOW: Duration = Duration::from_millis(1000);

#[derive(Debug, Clone)]
struct InFlight {
    assigned_at: DateTime<Utc>,
    repo: String,
    levels_before: HashMap<String, String>,
}

/// Per-issue metrics record matching the T11.3 DoD shape.
#[derive(Debug, Clone)]
pub struct IssueMetrics {
    pub run_id: String,
    pub task_id: String,
    pub repo: String,
    pub outcome: String,
    pub assigned_at: DateTime<Utc>,
    pub merged_at: DateTime<Utc>,
    pub time_to_merge_mins: f64,
    pub approval_asks: u64,
    pub ci_passed_first_try: bool,
    pub review_rounds: u64,
    pub reverted_within_7d: bool,
    pub mastery_level_before: HashMap<String, String>,
    pub mastery_level_after: HashMap<String, String>,
}

impl IssueMetrics {
    pub fn to_json(&self) -> serde_json::Value {
        json!({
            "run_id": self.run_id,
            "task_id": self.task_id,
            "repo": self.repo,
            "outcome": self.outcome,
            "assigned_at": self.assigned_at.to_rfc3339(),
            "merged_at": self.merged_at.to_rfc3339(),
            "time_to_merge_mins": round1(self.time_to_merge_mins),
            "approval_asks": self.approval_asks,
            "ci_passed_first_try": self.ci_passed_first_try,
            "review_rounds": self.review_rounds,
            "reverted_within_7d": self.reverted_within_7d,
            "mastery_level_before": self.mastery_level_before,
            "mastery_level_after": self.mastery_level_after,
        })
    }
}

fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

/// Sink state, shared across the assigned/completed handler tasks.
pub struct ProofRunSink {
    run_id: String,
    run_dir: PathBuf,
    in_flight: HashMap<String, InFlight>,
    knowledge_store: Option<SharedKnowledgeStore>,
}

impl ProofRunSink {
    /// Build a sink if `FORGE_PROOF_RUN_ID` is set and the run directory
    /// can be created. Returns `None` otherwise — opt-in is the whole
    /// point.
    pub fn try_new(knowledge_store: Option<SharedKnowledgeStore>) -> Option<Self> {
        let run_id = std::env::var(RUN_ID_ENV).ok().filter(|s| !s.is_empty())?;
        let run_dir = PathBuf::from(METRICS_BASE).join(&run_id);
        Self::new_with_dir(run_id, run_dir, knowledge_store)
    }

    /// Explicit constructor that bypasses the env var. Used by tests
    /// (and by anything that wants the sink wired without touching the
    /// process environment).
    pub fn new_with_dir(
        run_id: String,
        run_dir: PathBuf,
        knowledge_store: Option<SharedKnowledgeStore>,
    ) -> Option<Self> {
        if let Err(e) = std::fs::create_dir_all(&run_dir) {
            eprintln!(
                "[proof_run_sink] could not create {}: {} — sink disabled",
                run_dir.display(),
                e
            );
            return None;
        }
        eprintln!(
            "[proof_run_sink] active — run_id={} dir={}",
            run_id,
            run_dir.display()
        );
        Some(Self {
            run_id,
            run_dir,
            in_flight: HashMap::new(),
            knowledge_store,
        })
    }

    /// Stamp `assigned_at` and capture `mastery_level_before`.
    pub fn handle_issue_assigned(&mut self, payload: &EventPayload) {
        let Some(issue_id) = text_field(payload, "issue_id") else {
            return;
        };
        let repo = text_field(payload, "repo").unwrap_or_default();
        let levels = current_levels_for(self.knowledge_store.as_ref(), &repo);
        self.in_flight.insert(
            issue_id.clone(),
            InFlight {
                assigned_at: Utc::now(),
                repo,
                levels_before: levels,
            },
        );
    }

    /// Materialise `IssueMetrics` for the just-completed task. Returns
    /// `None` if there's no matching `IssueAssigned` on file — that
    /// can happen for tasks not assigned through the dev-cycle inbound
    /// (e.g., webhook-translated `GithubPrMerged` events).
    pub fn finalize_task_completed(&mut self, payload: &EventPayload) -> Option<IssueMetrics> {
        let task_id = text_field(payload, "task_id")?;
        let in_flight = self.in_flight.remove(&task_id)?;
        let merged_at = Utc::now();
        let elapsed_secs = (merged_at - in_flight.assigned_at).num_milliseconds() as f64 / 1000.0;
        let levels_after = current_levels_for(self.knowledge_store.as_ref(), &in_flight.repo);
        Some(IssueMetrics {
            run_id: self.run_id.clone(),
            task_id,
            repo: text_field(payload, "repo").unwrap_or(in_flight.repo),
            outcome: text_field(payload, "outcome").unwrap_or_else(|| "unknown".to_string()),
            assigned_at: in_flight.assigned_at,
            merged_at,
            time_to_merge_mins: elapsed_secs / 60.0,
            approval_asks: number_field(payload, "approval_asks")
                .unwrap_or(0.0)
                .max(0.0)
                .round() as u64,
            ci_passed_first_try: bool_field(payload, "ci_passed_first_try").unwrap_or(false),
            review_rounds: number_field(payload, "review_rounds")
                .unwrap_or(0.0)
                .max(0.0)
                .round() as u64,
            reverted_within_7d: bool_field(payload, "reverted_within_7d").unwrap_or(false),
            mastery_level_before: in_flight.levels_before,
            mastery_level_after: levels_after,
        })
    }

    /// Persist a single issue's metrics under
    /// `<run_dir>/issue-<task_id>.json` and append to `summary.json`.
    pub fn write_metrics(&self, metrics: &IssueMetrics) -> std::io::Result<()> {
        let safe = sanitise_task_id(&metrics.task_id);
        let issue_path = self.run_dir.join(format!("issue-{safe}.json"));
        std::fs::write(
            &issue_path,
            serde_json::to_string_pretty(&metrics.to_json()).unwrap_or_default(),
        )?;
        let summary_path = self.run_dir.join("summary.json");
        let mut summary = read_summary(&summary_path);
        if let Some(arr) = summary["issues"].as_array_mut() {
            // Replace existing record for this task_id if present (rerun
            // semantics) so the summary stays unique per task.
            arr.retain(|v| v["task_id"].as_str() != Some(metrics.task_id.as_str()));
            arr.push(metrics.to_json());
        }
        let _ = std::fs::write(
            &summary_path,
            serde_json::to_string_pretty(&summary).unwrap_or_default(),
        );
        Ok(())
    }
}

fn current_levels_for(store: Option<&SharedKnowledgeStore>, repo: &str) -> HashMap<String, String> {
    let snap = mastery_view::snapshot_current_levels(store, repo);
    snap.into_iter().map(|(k, v)| (k, v.level)).collect()
}

fn read_summary(path: &PathBuf) -> serde_json::Value {
    if let Ok(raw) = std::fs::read_to_string(path) {
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&raw) {
            if parsed["issues"].is_array() {
                return parsed;
            }
        }
    }
    json!({
        "issues": [],
    })
}

fn sanitise_task_id(task_id: &str) -> String {
    task_id
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
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

/// Spawn the proof-run JSON sink. Returns `None` when `FORGE_PROOF_RUN_ID`
/// is unset (the sink is opt-in by design).
pub async fn spawn_proof_run_sink(
    bus: SharedEventBus,
    knowledge_store: Option<SharedKnowledgeStore>,
) -> Option<JoinHandle<()>> {
    let sink = ProofRunSink::try_new(knowledge_store)?;
    let sink = Arc::new(RwLock::new(sink));

    let mut assigned_rx = bus
        .write()
        .await
        .subscribe("IssueAssigned", "__proof_run_sink", None);
    let mut completed_rx = bus
        .write()
        .await
        .subscribe("TaskCompleted", "__proof_run_sink", None);

    let handle = tokio::spawn(async move {
        loop {
            tokio::select! {
                payload = assigned_rx.recv() => {
                    match payload {
                        Some(p) => sink.write().await.handle_issue_assigned(&p),
                        None => break,
                    }
                }
                payload = completed_rx.recv() => {
                    match payload {
                        Some(p) => {
                            // Defer mastery_after snapshot until the
                            // MasterySignal → MasteryUpdated chain
                            // settles. Spawned so the loop keeps
                            // accepting events.
                            let sink_clone = sink.clone();
                            tokio::spawn(async move {
                                tokio::time::sleep(SETTLE_WINDOW).await;
                                let metrics_opt = sink_clone
                                    .write()
                                    .await
                                    .finalize_task_completed(&p);
                                if let Some(metrics) = metrics_opt {
                                    if let Err(e) = sink_clone.read().await.write_metrics(&metrics) {
                                        eprintln!(
                                            "[proof_run_sink] write failed for {}: {}",
                                            metrics.task_id, e
                                        );
                                    }
                                }
                            });
                        }
                        None => break,
                    }
                }
            }
        }
    });
    Some(handle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::confidence::ConfidentValue;
    use std::collections::HashMap as Map;

    fn det(v: Value) -> ConfidentValue {
        ConfidentValue::deterministic(v)
    }

    fn issue_assigned(issue_id: &str, repo: &str) -> EventPayload {
        let mut fields = Map::new();
        fields.insert("issue_id".to_string(), det(Value::Text(issue_id.into())));
        fields.insert("repo".to_string(), det(Value::Text(repo.into())));
        EventPayload {
            event_name: "IssueAssigned".to_string(),
            args: vec![],
            source_agent: "test".to_string(),
            fields,
        }
    }

    fn task_completed(
        task_id: &str,
        repo: &str,
        approval_asks: f64,
        review_rounds: f64,
        ci_first: bool,
    ) -> EventPayload {
        let mut fields = Map::new();
        fields.insert("task_id".to_string(), det(Value::Text(task_id.into())));
        fields.insert("repo".to_string(), det(Value::Text(repo.into())));
        fields.insert(
            "outcome".to_string(),
            det(Value::Text("merged".to_string())),
        );
        fields.insert(
            "ci_passed_first_try".to_string(),
            det(Value::Bool(ci_first)),
        );
        fields.insert(
            "review_rounds".to_string(),
            det(Value::Number(review_rounds)),
        );
        fields.insert("time_to_merge".to_string(), det(Value::Number(0.0)));
        fields.insert("reverted_within_7d".to_string(), det(Value::Bool(false)));
        fields.insert(
            "approval_asks".to_string(),
            det(Value::Number(approval_asks)),
        );
        EventPayload {
            event_name: "TaskCompleted".to_string(),
            args: vec![],
            source_agent: "release_manager".to_string(),
            fields,
        }
    }

    use std::sync::Mutex as StdMutex;

    /// Guard for the `try_new_returns_none_without_env_var` test. Other
    /// tests use `new_with_dir` and don't touch the env, so they're
    /// safe to run in parallel.
    static ENV_TEST_LOCK: StdMutex<()> = StdMutex::new(());

    #[test]
    fn try_new_returns_none_without_env_var() {
        let _g = ENV_TEST_LOCK.lock().unwrap();
        let prev = std::env::var(RUN_ID_ENV).ok();
        std::env::remove_var(RUN_ID_ENV);
        assert!(ProofRunSink::try_new(None).is_none());
        if let Some(v) = prev {
            std::env::set_var(RUN_ID_ENV, v);
        }
    }

    #[test]
    fn finalize_returns_none_when_no_matching_issue_assigned() {
        let (mut sink, _dir) = sink_for("no_match");
        // No IssueAssigned beforehand.
        let payload = task_completed("ISSUE-99", "test-org/test-repo", 0.0, 1.0, true);
        assert!(sink.finalize_task_completed(&payload).is_none());
    }

    #[test]
    fn round_trip_writes_per_issue_json_with_dod_fields() {
        let (mut sink, run_dir) = sink_for("round_trip");
        sink.handle_issue_assigned(&issue_assigned("ISSUE-1", "test-org/test-repo"));
        let payload = task_completed("ISSUE-1", "test-org/test-repo", 3.0, 2.0, true);
        let metrics = sink
            .finalize_task_completed(&payload)
            .expect("should finalize with prior IssueAssigned");
        assert_eq!(metrics.task_id, "ISSUE-1");
        assert_eq!(metrics.repo, "test-org/test-repo");
        assert_eq!(metrics.approval_asks, 3);
        assert_eq!(metrics.review_rounds, 2);
        assert!(metrics.ci_passed_first_try);
        assert!(!metrics.reverted_within_7d);
        assert_eq!(metrics.mastery_level_before.len(), 5);
        assert_eq!(metrics.mastery_level_after.len(), 5);
        // Defaults to "novice" because no knowledge_store was wired.
        for s in mastery_view::SPECIALISTS {
            assert_eq!(
                metrics.mastery_level_before.get(*s).map(String::as_str),
                Some(mastery_view::DEFAULT_LEVEL)
            );
            assert_eq!(
                metrics.mastery_level_after.get(*s).map(String::as_str),
                Some(mastery_view::DEFAULT_LEVEL)
            );
        }
        sink.write_metrics(&metrics).expect("write");

        // Verify the file exists and contains the DoD-required fields.
        let issue_file = run_dir.join("issue-ISSUE-1.json");
        let raw = std::fs::read_to_string(&issue_file).expect("issue file present");
        let parsed: serde_json::Value = serde_json::from_str(&raw).expect("valid json");
        for key in [
            "run_id",
            "task_id",
            "repo",
            "approval_asks",
            "ci_passed_first_try",
            "review_rounds",
            "time_to_merge_mins",
            "mastery_level_before",
            "mastery_level_after",
        ] {
            assert!(
                parsed.get(key).is_some(),
                "expected field {key} in issue json: {parsed}"
            );
        }
        assert_eq!(parsed["approval_asks"].as_u64(), Some(3));

        let summary: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(run_dir.join("summary.json")).unwrap())
                .unwrap();
        let arr = summary["issues"].as_array().expect("array");
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["task_id"].as_str(), Some("ISSUE-1"));
        std::fs::remove_dir_all(&run_dir).ok();
    }

    #[test]
    fn write_metrics_is_idempotent_per_task_id() {
        let (mut sink, run_dir) = sink_for("idempotent");
        sink.handle_issue_assigned(&issue_assigned("ISSUE-7", "test-org/test-repo"));
        let m1 = sink
            .finalize_task_completed(&task_completed(
                "ISSUE-7",
                "test-org/test-repo",
                2.0,
                1.0,
                true,
            ))
            .expect("finalize");
        sink.write_metrics(&m1).expect("write 1");

        // Re-run the same task — re-stamp assigned_at, finalize again.
        sink.handle_issue_assigned(&issue_assigned("ISSUE-7", "test-org/test-repo"));
        let m2 = sink
            .finalize_task_completed(&task_completed(
                "ISSUE-7",
                "test-org/test-repo",
                5.0,
                3.0,
                false,
            ))
            .expect("finalize 2");
        sink.write_metrics(&m2).expect("write 2");

        let summary: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(run_dir.join("summary.json")).unwrap())
                .unwrap();
        let arr = summary["issues"].as_array().unwrap();
        assert_eq!(
            arr.len(),
            1,
            "summary must hold a single record per task_id"
        );
        assert_eq!(arr[0]["approval_asks"].as_u64(), Some(5));
        std::fs::remove_dir_all(&run_dir).ok();
    }

    /// Build a sink with a unique tempdir as its run_dir. No env mutation,
    /// no cwd mutation — safe to run in parallel.
    fn sink_for(name: &str) -> (ProofRunSink, PathBuf) {
        let run_id = format!(
            "test-{}-{}-{}",
            std::process::id(),
            name,
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        let run_dir = std::env::temp_dir()
            .join("forge-proof-sink-tests")
            .join(&run_id);
        let sink = ProofRunSink::new_with_dir(run_id, run_dir.clone(), None)
            .expect("sink must construct with explicit dir");
        (sink, run_dir)
    }
}
