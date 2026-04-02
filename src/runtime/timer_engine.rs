// FORGE async timer engine — issue #20
// Named countdown timers that live on agents and fire handlers independently.
// Uses tokio tasks for async countdowns with oneshot cancellation.

use std::collections::HashMap;

use tokio::sync::{mpsc, oneshot};

use crate::ast::{DurationUnit, Spanned, TimerField};
use crate::runtime::confidence::ConfidentValue;
use crate::tracer::Tracer;

// ── Types ───────────────────────────────────────────────────────────────────

/// A timer expiry event delivered to the agent's main loop.
#[derive(Debug)]
pub struct TimerFired {
    pub timer_name: String,
    pub context: Option<ConfidentValue>,
    pub agent_id: String,
}

/// A single active timer instance (one per `start` call).
/// Multiple instances can exist for the same timer name with different contexts.
struct TimerInstance {
    context: Option<ConfidentValue>,
    cancel_tx: oneshot::Sender<()>,
}

/// Async timer engine that spawns tokio tasks for countdown timers.
pub struct TimerEngine {
    agent_id: String,
    durations: HashMap<String, std::time::Duration>,
    active: HashMap<String, Vec<TimerInstance>>,
    fire_tx: mpsc::Sender<TimerFired>,
    tracer: Option<Tracer>,
}

// ── Duration conversion ─────────────────────────────────────────────────────

fn ast_duration_to_std(dur: &crate::ast::Duration) -> std::time::Duration {
    let secs = match dur.unit {
        DurationUnit::Seconds => dur.value,
        DurationUnit::Minutes => dur.value * 60,
        DurationUnit::Hours => dur.value * 3600,
    };
    std::time::Duration::from_secs(secs)
}

// ── Implementation ──────────────────────────────────────────────────────────

impl TimerEngine {
    /// Create a new timer engine from agent timer declarations.
    pub fn new(
        agent_id: &str,
        timer_fields: &[Spanned<TimerField>],
        fire_tx: mpsc::Sender<TimerFired>,
        tracer: Option<Tracer>,
    ) -> Self {
        let mut durations = HashMap::new();
        for tf in timer_fields {
            durations.insert(
                tf.node.name.node.clone(),
                ast_duration_to_std(&tf.node.duration.node),
            );
        }
        Self {
            agent_id: agent_id.to_string(),
            durations,
            active: HashMap::new(),
            fire_tx,
            tracer,
        }
    }

    /// Start a named timer. Spawns a tokio task that fires after the declared duration.
    pub fn start(&mut self, name: &str, context: Option<ConfidentValue>) -> Result<(), TimerError> {
        let duration = self
            .durations
            .get(name)
            .copied()
            .ok_or_else(|| TimerError::UnknownTimer(name.to_string()))?;

        let (cancel_tx, cancel_rx) = oneshot::channel::<()>();

        let fire_tx = self.fire_tx.clone();
        let agent_id = self.agent_id.clone();
        let timer_name = name.to_string();
        let ctx_clone = context.clone();

        if let Some(ref tracer) = self.tracer {
            tracer.timer_started(&self.agent_id, name, duration.as_secs());
        }

        tokio::spawn(async move {
            tokio::select! {
                _ = tokio::time::sleep(duration) => {
                    let _ = fire_tx.send(TimerFired {
                        timer_name,
                        context: ctx_clone,
                        agent_id,
                    }).await;
                }
                _ = cancel_rx => {
                    // Cancelled — do nothing
                }
            }
        });

        let instance = TimerInstance { context, cancel_tx };
        self.active
            .entry(name.to_string())
            .or_default()
            .push(instance);
        Ok(())
    }

    /// Cancel a named timer. If context is provided, only cancel instances with
    /// matching context. If context is None, cancel all instances of this timer.
    pub fn cancel(
        &mut self,
        name: &str,
        context: &Option<ConfidentValue>,
    ) -> Result<(), TimerError> {
        if !self.durations.contains_key(name) {
            return Err(TimerError::UnknownTimer(name.to_string()));
        }

        if let Some(ref tracer) = self.tracer {
            tracer.timer_cancelled(&self.agent_id, name);
        }

        if let Some(instances) = self.active.remove(name) {
            match context {
                None => {
                    // Cancel all instances
                    for inst in instances {
                        let _ = inst.cancel_tx.send(());
                    }
                }
                Some(ctx) => {
                    let mut remaining = Vec::new();
                    for inst in instances {
                        if context_matches(&inst.context, ctx) {
                            let _ = inst.cancel_tx.send(());
                        } else {
                            remaining.push(inst);
                        }
                    }
                    if !remaining.is_empty() {
                        self.active.insert(name.to_string(), remaining);
                    }
                }
            }
        }
        Ok(())
    }

    /// Reset a named timer: cancel existing + start fresh with same duration.
    pub fn reset(&mut self, name: &str, context: Option<ConfidentValue>) -> Result<(), TimerError> {
        self.cancel(name, &context)?;
        self.start(name, context)
    }

    /// Cancel all active timers (for agent shutdown).
    pub fn cancel_all(&mut self) {
        for (_name, instances) in self.active.drain() {
            for inst in instances {
                let _ = inst.cancel_tx.send(());
            }
        }
    }
}

/// Compare timer context values for cancel matching.
/// Uses string representation since ConfidentValue doesn't implement PartialEq.
fn context_matches(instance_ctx: &Option<ConfidentValue>, cancel_ctx: &ConfidentValue) -> bool {
    match instance_ctx {
        None => false,
        Some(inst_val) => format!("{}", inst_val.value) == format!("{}", cancel_ctx.value),
    }
}

// ── Errors ──────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum TimerError {
    UnknownTimer(String),
}

impl std::fmt::Display for TimerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TimerError::UnknownTimer(name) => write!(f, "unknown timer: {}", name),
        }
    }
}

impl std::error::Error for TimerError {}
