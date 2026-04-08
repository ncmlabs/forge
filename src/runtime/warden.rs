// FORGE warden runtime — issue #24
// Manages agent lifecycles with failure detection, response dispatch,
// scope enforcement, retry tracking, and escalation ladders.

use std::collections::HashMap;

use crate::ast::*;
use crate::tracer::Tracer;

// ── Failure & Response Types ────────────────────────────────────────────────

/// A failure signal from a managed agent.
#[derive(Debug, Clone)]
pub struct FailureSignal {
    pub agent_name: String,
    pub failure_type: FailureType,
    pub detail: String,
}

/// A warden's response action, resolved from policy.
#[derive(Debug, Clone)]
pub struct WardAction {
    pub warden_name: String,
    pub agent_name: String,
    pub failure_type: FailureType,
    pub response: WardResponse,
    pub scope: WardScope,
    pub retry_count: u64,
}

// ── Retry Tracker ───────────────────────────────────────────────────────────

/// Tracks retry counts per agent per failure type for escalation ladder.
#[derive(Debug, Clone, Default)]
pub struct RetryTracker {
    /// (agent_name, failure_type) -> total failure count
    counts: HashMap<(String, FailureType), u64>,
    /// Group-level failure log: (timestamp_ms, agent_name)
    group_failures: Vec<(u64, String)>,
}

impl RetryTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a failure and return the new count for this agent+type.
    pub fn record(&mut self, agent: &str, ft: FailureType, timestamp_ms: u64) -> u64 {
        let key = (agent.to_string(), ft);
        let count = self.counts.entry(key).or_insert(0);
        *count += 1;
        self.group_failures.push((timestamp_ms, agent.to_string()));
        *count
    }

    /// Get current count for an agent+type.
    pub fn count(&self, agent: &str, ft: FailureType) -> u64 {
        self.counts
            .get(&(agent.to_string(), ft))
            .copied()
            .unwrap_or(0)
    }

    /// Count total group failures within a time window.
    pub fn group_count_in_window(&self, now_ms: u64, window_ms: u64) -> u64 {
        self.group_failures
            .iter()
            .filter(|(ts, _)| now_ms.saturating_sub(window_ms) <= *ts)
            .count() as u64
    }

    /// Reset counts for a specific agent (after successful recovery).
    pub fn reset_agent(&mut self, agent: &str, ft: FailureType) {
        self.counts.remove(&(agent.to_string(), ft));
    }

    /// Return all failure counts as (agent, failure_type) → count.
    pub fn all_counts(&self) -> &HashMap<(String, FailureType), u64> {
        &self.counts
    }
}

// ── Policy Resolver ─────────────────────────────────────────────────────────

/// Resolves the effective policy for a given agent and failure type,
/// checking agent overrides first, then warden defaults.
pub fn resolve_policy<'a>(
    warden: &'a WardenDecl,
    agent_overrides: &'a [Spanned<WardPolicy>],
    failure_type: FailureType,
) -> Option<&'a WardPolicy> {
    // Agent overrides take precedence
    if let Some(p) = agent_overrides
        .iter()
        .find(|p| p.node.failure_type.node == failure_type)
    {
        return Some(&p.node);
    }

    // Fall back to warden defaults
    warden
        .policies
        .iter()
        .find(|p| p.node.failure_type.node == failure_type)
        .map(|p| &p.node)
}

/// Given a policy and the current retry count, determine the effective response
/// by walking the escalation ladder.
pub fn effective_response(policy: &WardPolicy, retry_count: u64) -> WardResponse {
    let mut response = policy.response.node;

    for after in &policy.after_clauses {
        if retry_count >= after.node.count {
            response = after.node.response.node;
        }
    }

    response
}

// ── Warden Runtime ──────────────────────────────────────────────────────────

/// The runtime warden that manages a group of agents/wardens.
pub struct Warden {
    pub decl: WardenDecl,
    pub retry_tracker: RetryTracker,
    tracer: Option<Tracer>,
}

impl Warden {
    pub fn new(decl: WardenDecl, tracer: Option<Tracer>) -> Self {
        Self {
            decl,
            retry_tracker: RetryTracker::new(),
            tracer,
        }
    }

    /// Handle a failure signal from a managed agent.
    /// Returns the action taken, or None if no policy covers this failure type.
    pub fn handle_failure(
        &mut self,
        signal: &FailureSignal,
        agent_overrides: &[Spanned<WardPolicy>],
        timestamp_ms: u64,
    ) -> Option<WardAction> {
        let policy = resolve_policy(&self.decl, agent_overrides, signal.failure_type)?;

        let retry_count =
            self.retry_tracker
                .record(&signal.agent_name, signal.failure_type, timestamp_ms);

        let response = effective_response(policy, retry_count);

        let action = WardAction {
            warden_name: self.decl.name.node.clone(),
            agent_name: signal.agent_name.clone(),
            failure_type: signal.failure_type,
            response,
            scope: policy.scope.node,
            retry_count,
        };

        // Trace the decision (Principle VIII)
        if let Some(tracer) = &self.tracer {
            tracer.ward_action(
                &action.warden_name,
                &action.agent_name,
                &format!("{:?}", action.failure_type),
                &format!("{:?}", action.response),
                &format!("{:?}", action.scope),
                action.retry_count,
            );
        }

        Some(action)
    }

    /// Check if the group-level circuit breaker has tripped.
    pub fn circuit_breaker_tripped(&self, now_ms: u64) -> bool {
        if let Some(ref mr) = self.decl.max_retries {
            let window_ms = duration_to_ms(&mr.node.window.node);
            let count = self.retry_tracker.group_count_in_window(now_ms, window_ms);
            count >= mr.node.count
        } else {
            false
        }
    }

    /// Get a reference to the tracer, if any.
    pub fn tracer(&self) -> Option<&Tracer> {
        self.tracer.as_ref()
    }

    /// Get the list of managed names.
    pub fn managed_names(&self) -> Vec<&str> {
        self.decl.manages.iter().map(|m| m.node.as_str()).collect()
    }

    /// Dynamically add an agent name to the manages list.
    pub fn adopt(&mut self, agent_name: &str) {
        // Avoid duplicates
        if !self.decl.manages.iter().any(|m| m.node == agent_name) {
            self.decl.manages.push(Spanned::new(
                agent_name.to_string(),
                Span { start: 0, end: 0 },
            ));
        }
    }

    /// Remove an agent name from the manages list and clear its retry state.
    pub fn release(&mut self, agent_name: &str) {
        self.decl.manages.retain(|m| m.node != agent_name);
        // Clear all retry tracker state for the released agent
        for ft in [
            FailureType::Stuck,
            FailureType::Crash,
            FailureType::Hallucination,
            FailureType::Budget,
            FailureType::Timeout,
        ] {
            self.retry_tracker.reset_agent(agent_name, ft);
        }
    }
}

fn duration_to_ms(d: &Duration) -> u64 {
    match d.unit {
        DurationUnit::Seconds => d.value * 1000,
        DurationUnit::Minutes => d.value * 60 * 1000,
        DurationUnit::Hours => d.value * 3600 * 1000,
        DurationUnit::Days => d.value * 86400 * 1000,
    }
}
