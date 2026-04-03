// FORGE pool executor — issue #12
// Runs N workers concurrently and resolves results via strategy
// (fastest, all, majority, quorum, first).

use std::collections::HashMap;
use std::sync::Arc;

use tokio::task::JoinSet;

use crate::ast::*;
use crate::llm::registry::ProviderRegistry;
use crate::runtime::agent::{jaccard_similarity, AgentProcess};
use crate::runtime::confidence::{ConfidentValue, Value};
use crate::runtime::executor::{RuntimeError, TaskExecutor};
use crate::tracer::Tracer;

// ── Worker kind ──────────────────────────────────────────────────────────────

enum WorkerKind {
    Task(TaskDecl),
    Agent(Box<AgentDecl>),
}

// ── Pool executor ────────────────────────────────────────────────────────────

pub struct PoolExecutor {
    decl: PoolDecl,
    worker_kind: WorkerKind,
    worker_count: usize,
    providers: Arc<ProviderRegistry>,
    tracer: Option<Tracer>,
    program: Program,
}

impl PoolExecutor {
    /// Build a pool executor by resolving the worker type against the program.
    pub fn new(
        decl: PoolDecl,
        program: &Program,
        providers: Arc<ProviderRegistry>,
        tracer: Option<Tracer>,
    ) -> Result<Self, RuntimeError> {
        let worker_type = &decl.worker_type.node;
        let worker_count = decl.worker_count.node as usize;

        // Resolve worker kind from program declarations
        let worker_kind = program
            .items
            .iter()
            .find_map(|item| match &item.node {
                TopLevel::Task(t) if t.name.node == *worker_type => {
                    Some(WorkerKind::Task(t.clone()))
                }
                TopLevel::Agent(a) if a.name.node == *worker_type => {
                    Some(WorkerKind::Agent(a.clone()))
                }
                _ => None,
            })
            .ok_or_else(|| {
                RuntimeError::PoolError(format!(
                    "pool '{}': worker type '{}' not found as task or agent",
                    decl.name.node, worker_type
                ))
            })?;

        Ok(Self {
            decl,
            worker_kind,
            worker_count,
            providers,
            tracer,
            program: program.clone(),
        })
    }

    /// Dispatch work to the pool and resolve via the declared strategy.
    pub async fn send(
        &self,
        event: &str,
        args: Vec<ConfidentValue>,
    ) -> Result<ConfidentValue, RuntimeError> {
        let strategy_name = strategy_label(&self.decl.strategy.node);

        if let Some(ref tracer) = self.tracer {
            tracer.pool_send(
                &self.decl.name.node,
                event,
                self.worker_count,
                &strategy_name,
            );
        }

        // Spawn workers
        let mut join_set = JoinSet::new();
        self.spawn_workers(&mut join_set, event, &args);

        // Compute timeout
        let timeout_dur = self
            .decl
            .timeout
            .as_ref()
            .map(|t| ast_duration_to_tokio(&t.node));

        // Resolve via strategy
        let result = match timeout_dur {
            Some(dur) => match tokio::time::timeout(dur, self.resolve(&mut join_set)).await {
                Ok(r) => r,
                Err(_) => {
                    join_set.abort_all();
                    Err(RuntimeError::PoolError(format!(
                        "pool '{}': timeout after {}",
                        self.decl.name.node,
                        format_duration(&self.decl.timeout.as_ref().unwrap().node),
                    )))
                }
            },
            None => self.resolve(&mut join_set).await,
        };

        // Cancel remaining workers
        join_set.abort_all();

        // On failure, try fallback
        match result {
            Ok(val) => {
                if let Some(ref tracer) = self.tracer {
                    let agreement = match &val.source {
                        crate::types::ConfidenceSource::ConsensusAgreement(a) => Some(*a),
                        _ => None,
                    };
                    tracer.pool_resolved(&self.decl.name.node, &strategy_name, true, agreement);
                }
                Ok(val)
            }
            Err(e) => {
                if let Some(ref tracer) = self.tracer {
                    tracer.pool_resolved(&self.decl.name.node, &strategy_name, false, None);
                }
                self.try_fallback(&args).await.ok_or(e)
            }
        }
    }

    // ── Worker spawning ──────────────────────────────────────────────────────

    fn spawn_workers(
        &self,
        join_set: &mut JoinSet<Result<ConfidentValue, RuntimeError>>,
        event: &str,
        args: &[ConfidentValue],
    ) {
        for _ in 0..self.worker_count {
            match &self.worker_kind {
                WorkerKind::Task(task_decl) => {
                    let executor = TaskExecutor::new(
                        self.program.clone(),
                        self.providers.clone(),
                        self.tracer.clone(),
                    );
                    let decl = task_decl.clone();
                    let args = args.to_vec();
                    join_set.spawn(async move { executor.call_task(&decl, args).await });
                }
                WorkerKind::Agent(agent_decl) => {
                    let decl = agent_decl.as_ref().clone();
                    let providers = self.providers.clone();
                    let tracer = self.tracer.clone();
                    let program = self.program.clone();
                    let event = event.to_string();
                    let args = args.to_vec();
                    join_set.spawn(async move {
                        let process = AgentProcess::new(decl, None, providers, tracer, program);
                        let mut params = HashMap::new();
                        for (i, arg) in args.into_iter().enumerate() {
                            params.insert(format!("arg_{}", i), arg);
                        }
                        process
                            .dispatch(&event, params)
                            .await
                            .map(|opt| opt.unwrap_or(ConfidentValue::deterministic(Value::Unit)))
                    });
                }
            }
        }
    }

    // ── Strategy dispatch ────────────────────────────────────────────────────

    async fn resolve(
        &self,
        join_set: &mut JoinSet<Result<ConfidentValue, RuntimeError>>,
    ) -> Result<ConfidentValue, RuntimeError> {
        match &self.decl.strategy.node {
            PoolStrategy::Fastest => self.resolve_fastest(join_set).await,
            PoolStrategy::All => self.resolve_all(join_set).await,
            PoolStrategy::Majority => self.resolve_majority(join_set).await,
            PoolStrategy::Quorum(n) => self.resolve_quorum(*n as usize, join_set).await,
            PoolStrategy::First(n) => self.resolve_first(*n as usize, join_set).await,
        }
    }

    /// Return the first `.sure()` result; fall back to highest confidence.
    async fn resolve_fastest(
        &self,
        join_set: &mut JoinSet<Result<ConfidentValue, RuntimeError>>,
    ) -> Result<ConfidentValue, RuntimeError> {
        let mut best: Option<ConfidentValue> = None;

        while let Some(result) = join_set.join_next().await {
            if let Ok(Ok(cv)) = result {
                if cv.sure() {
                    return Ok(cv);
                }
                // Track highest confidence as fallback
                if best.as_ref().is_none_or(|b| cv.confidence > b.confidence) {
                    best = Some(cv);
                }
            }
        }

        best.ok_or_else(|| {
            RuntimeError::PoolError(format!(
                "pool '{}': all workers failed",
                self.decl.name.node
            ))
        })
    }

    /// Collect all results into an array.
    async fn resolve_all(
        &self,
        join_set: &mut JoinSet<Result<ConfidentValue, RuntimeError>>,
    ) -> Result<ConfidentValue, RuntimeError> {
        let mut results = Vec::new();

        while let Some(result) = join_set.join_next().await {
            match result {
                Ok(Ok(cv)) => results.push(cv),
                Ok(Err(e)) => return Err(e),
                Err(e) => return Err(RuntimeError::PoolError(format!("worker panicked: {}", e))),
            }
        }

        if results.is_empty() {
            return Err(RuntimeError::PoolError(format!(
                "pool '{}': no results",
                self.decl.name.node
            )));
        }

        let min_conf = results.iter().map(|r| r.confidence).fold(1.0_f32, f32::min);
        Ok(ConfidentValue::derived(Value::Array(results), min_conf))
    }

    /// Return the majority-agreed answer using text similarity clustering.
    async fn resolve_majority(
        &self,
        join_set: &mut JoinSet<Result<ConfidentValue, RuntimeError>>,
    ) -> Result<ConfidentValue, RuntimeError> {
        let results = self.collect_results(join_set).await?;
        let clusters = cluster_by_similarity(&results);
        let total = results.len();

        // Find largest cluster
        let largest = clusters.iter().max_by_key(|c| c.len()).ok_or_else(|| {
            RuntimeError::PoolError(format!(
                "pool '{}': no results to compare",
                self.decl.name.node
            ))
        })?;

        // Always return a consensus value — the agreement ratio determines
        // whether it's .sure(), .unsure(), or .conflicted() downstream.
        let agreement = largest.len() as f32 / total as f32;
        let best_idx = *largest
            .iter()
            .max_by(|&&a, &&b| {
                results[a]
                    .confidence
                    .partial_cmp(&results[b].confidence)
                    .unwrap()
            })
            .unwrap();
        Ok(ConfidentValue::from_consensus(
            results[best_idx].value.clone(),
            agreement,
        ))
    }

    /// Return when n workers agree on the same answer.
    async fn resolve_quorum(
        &self,
        n: usize,
        join_set: &mut JoinSet<Result<ConfidentValue, RuntimeError>>,
    ) -> Result<ConfidentValue, RuntimeError> {
        let results = self.collect_results(join_set).await?;
        let clusters = cluster_by_similarity(&results);

        // Find any cluster with >= n members
        if let Some(cluster) = clusters.iter().find(|c| c.len() >= n) {
            let agreement = cluster.len() as f32 / results.len() as f32;
            let best_idx = *cluster
                .iter()
                .max_by(|&&a, &&b| {
                    results[a]
                        .confidence
                        .partial_cmp(&results[b].confidence)
                        .unwrap()
                })
                .unwrap();
            Ok(ConfidentValue::from_consensus(
                results[best_idx].value.clone(),
                agreement,
            ))
        } else {
            Err(RuntimeError::PoolError(format!(
                "pool '{}': quorum of {} not reached",
                self.decl.name.node, n
            )))
        }
    }

    /// Return after the first n workers succeed, pick highest confidence.
    async fn resolve_first(
        &self,
        n: usize,
        join_set: &mut JoinSet<Result<ConfidentValue, RuntimeError>>,
    ) -> Result<ConfidentValue, RuntimeError> {
        let mut results = Vec::new();

        while results.len() < n {
            match join_set.join_next().await {
                Some(Ok(Ok(cv))) => results.push(cv),
                Some(Ok(Err(_))) | Some(Err(_)) => continue,
                None => break,
            }
        }

        results
            .into_iter()
            .max_by(|a, b| a.confidence.partial_cmp(&b.confidence).unwrap())
            .ok_or_else(|| {
                RuntimeError::PoolError(format!(
                    "pool '{}': fewer than {} workers succeeded",
                    self.decl.name.node, n
                ))
            })
    }

    // ── Helpers ──────────────────────────────────────────────────────────────

    async fn collect_results(
        &self,
        join_set: &mut JoinSet<Result<ConfidentValue, RuntimeError>>,
    ) -> Result<Vec<ConfidentValue>, RuntimeError> {
        let mut results = Vec::new();
        while let Some(result) = join_set.join_next().await {
            if let Ok(Ok(cv)) = result {
                results.push(cv);
            }
        }
        if results.is_empty() {
            return Err(RuntimeError::PoolError(format!(
                "pool '{}': all workers failed",
                self.decl.name.node
            )));
        }
        Ok(results)
    }

    async fn try_fallback(&self, args: &[ConfidentValue]) -> Option<ConfidentValue> {
        let fallback_name = self.decl.fallback.as_ref()?;
        let executor = TaskExecutor::new(
            self.program.clone(),
            self.providers.clone(),
            self.tracer.clone(),
        );

        // Look up the fallback as a task
        let task = self
            .program
            .items
            .iter()
            .find_map(|item| match &item.node {
                TopLevel::Task(t) if t.name.node == fallback_name.node => Some(t.clone()),
                _ => None,
            })?;

        executor.call_task(&task, args.to_vec()).await.ok()
    }
}

// ── Similarity clustering ────────────────────────────────────────────────────

/// Group results into clusters where members have Jaccard similarity > 0.5
/// on their text representation.
fn cluster_by_similarity(results: &[ConfidentValue]) -> Vec<Vec<usize>> {
    let texts: Vec<String> = results.iter().map(|r| format!("{}", r.value)).collect();

    let mut clusters: Vec<Vec<usize>> = Vec::new();

    for (i, text) in texts.iter().enumerate() {
        let mut placed = false;
        for cluster in &mut clusters {
            let rep = &texts[cluster[0]];
            if jaccard_similarity(text, rep) > 0.5 {
                cluster.push(i);
                placed = true;
                break;
            }
        }
        if !placed {
            clusters.push(vec![i]);
        }
    }

    clusters
}

// ── Duration conversion ──────────────────────────────────────────────────────

fn ast_duration_to_tokio(d: &Duration) -> tokio::time::Duration {
    let secs = match d.unit {
        DurationUnit::Seconds => d.value,
        DurationUnit::Minutes => d.value * 60,
        DurationUnit::Hours => d.value * 3600,
        DurationUnit::Days => d.value * 86400,
    };
    tokio::time::Duration::from_secs(secs)
}

fn format_duration(d: &Duration) -> String {
    match d.unit {
        DurationUnit::Seconds => format!("{}s", d.value),
        DurationUnit::Minutes => format!("{}m", d.value),
        DurationUnit::Hours => format!("{}h", d.value),
        DurationUnit::Days => format!("{}d", d.value),
    }
}

fn strategy_label(s: &PoolStrategy) -> String {
    match s {
        PoolStrategy::Fastest => "fastest".to_string(),
        PoolStrategy::All => "all".to_string(),
        PoolStrategy::Majority => "majority".to_string(),
        PoolStrategy::Quorum(n) => format!("quorum({})", n),
        PoolStrategy::First(n) => format!("first({})", n),
    }
}

// ── Unit tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::confidence::ConfidentValue;

    #[test]
    fn cluster_identical_texts() {
        let results = vec![
            ConfidentValue::deterministic(Value::Text("yes".to_string())),
            ConfidentValue::deterministic(Value::Text("yes".to_string())),
            ConfidentValue::deterministic(Value::Text("no".to_string())),
        ];
        let clusters = cluster_by_similarity(&results);
        assert_eq!(clusters.len(), 2);
        // First cluster has 2 "yes" results
        assert_eq!(clusters[0].len(), 2);
        assert_eq!(clusters[1].len(), 1);
    }

    #[test]
    fn cluster_similar_texts() {
        let results = vec![
            ConfidentValue::deterministic(Value::Text(
                "the speed of light is about 300000 km/s".to_string(),
            )),
            ConfidentValue::deterministic(Value::Text(
                "the speed of light is approximately 300000 km/s".to_string(),
            )),
            ConfidentValue::deterministic(Value::Text("I don't know the answer".to_string())),
        ];
        let clusters = cluster_by_similarity(&results);
        // First two should cluster together (high word overlap)
        assert_eq!(clusters.len(), 2);
        assert_eq!(clusters[0].len(), 2);
    }

    #[test]
    fn cluster_all_different() {
        let results = vec![
            ConfidentValue::deterministic(Value::Text("alpha".to_string())),
            ConfidentValue::deterministic(Value::Text("beta".to_string())),
            ConfidentValue::deterministic(Value::Text("gamma".to_string())),
        ];
        let clusters = cluster_by_similarity(&results);
        assert_eq!(clusters.len(), 3);
    }

    #[test]
    fn duration_conversion() {
        let d = Duration {
            value: 30,
            unit: DurationUnit::Seconds,
        };
        assert_eq!(
            ast_duration_to_tokio(&d),
            tokio::time::Duration::from_secs(30)
        );

        let d = Duration {
            value: 2,
            unit: DurationUnit::Minutes,
        };
        assert_eq!(
            ast_duration_to_tokio(&d),
            tokio::time::Duration::from_secs(120)
        );

        let d = Duration {
            value: 1,
            unit: DurationUnit::Hours,
        };
        assert_eq!(
            ast_duration_to_tokio(&d),
            tokio::time::Duration::from_secs(3600)
        );
    }

    #[test]
    fn strategy_labels() {
        assert_eq!(strategy_label(&PoolStrategy::Fastest), "fastest");
        assert_eq!(strategy_label(&PoolStrategy::All), "all");
        assert_eq!(strategy_label(&PoolStrategy::Majority), "majority");
        assert_eq!(strategy_label(&PoolStrategy::Quorum(3.0)), "quorum(3)");
        assert_eq!(strategy_label(&PoolStrategy::First(2.0)), "first(2)");
    }
}
