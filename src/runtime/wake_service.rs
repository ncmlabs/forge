// FORGE WakeService — issue #332
//
// Durable, cross-session schedule dispatch. Sibling to `EventBus`; owned by
// `SystemRuntime`. Reads declared schedules from `AgentDecl.schedules`
// (parsed/checked by #331), persists per-schedule state in redb, and emits
// bus events when a schedule is due.
//
// v1 scope: `mode: spawn` only (bus event carrying the schedule's prompt).
// `mode: wake` lands in #333. `emit:` for wake is grammar-declared but not
// dispatched yet; reaching the dispatcher with wake is a checker-caught bug.
//
// Design notes:
// - Wall-clock reads via `Clock` trait (Principle II — determinism boundary).
// - Per-fire claim uses `ForgeStorage::try_claim_schedule` for within-process
//   serialization of concurrent tick loops. Cross-process isolation is enforced
//   by redb at database-open (a second `forge serve` on the same `.forge-data/`
//   fails with "Database already open" — stronger than a cooperative lock).
// - Budget gate queries a caller-supplied hook; v1 reads session-level
//   `CostTracker` state. Per-agent budgets are a future upgrade; the hook
//   signature already accepts an `agent` argument.
// - Failure policy at the schedule layer: track `consecutive_errors` in
//   `ScheduleState`; after `max_consecutive_errors`, mark the schedule
//   `Halted` and stop firing. Handler errors (inside the agent) flow through
//   the existing agent event loop and are already warden-supervised — no
//   duplicate path here.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration as StdDuration;

use chrono::{DateTime, Datelike, NaiveDate, TimeZone, Utc};
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

use crate::ast::{
    Duration as AstDuration, DurationUnit, Precision, ScheduleField, ScheduleMode, TimeOfDay,
    WhenExpr,
};
use crate::runtime::agent_lifecycle::AgentLifecycle;
use crate::runtime::clock::SharedClock;
use crate::runtime::confidence::{ConfidentValue, Value};
use crate::runtime::event_bus::{EventPayload, SharedEventBus};
use crate::runtime::storage::{ClaimOutcome, ScheduleState, ScheduleStatus, SharedStorage};
use crate::tracer::Tracer;

// ── Public types ────────────────────────────────────────────────────────────

/// A declared schedule owned by a specific agent instance (alias).
#[derive(Debug, Clone)]
pub struct ScheduleRegistration {
    pub agent: String,
    pub schedule: ScheduleField,
}

/// Budget gate. Returns `Some(reason)` if firing this agent's schedule should
/// be skipped; `None` to proceed. v1 ignores the agent argument and returns
/// based on session-level cost state.
pub type BudgetQuery = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

#[derive(Debug, Clone)]
pub struct WakeServiceConfig {
    pub instance_id: String,
    pub tick_interval: StdDuration,
    pub claim_ttl: StdDuration,
    pub catchup_stagger: StdDuration,
    pub max_consecutive_errors: u32,
}

impl Default for WakeServiceConfig {
    fn default() -> Self {
        Self {
            instance_id: uuid::Uuid::new_v4().to_string(),
            tick_interval: StdDuration::from_secs(60),
            claim_ttl: StdDuration::from_secs(300),
            catchup_stagger: StdDuration::from_secs(5),
            max_consecutive_errors: 3,
        }
    }
}

impl WakeServiceConfig {
    /// Override the tick interval. Useful for tests (e.g. 100ms) and for
    /// honouring `precision: high` at production (1s).
    pub fn with_tick_interval(mut self, tick: StdDuration) -> Self {
        self.tick_interval = tick;
        self
    }
}

/// Why did a schedule dispatch not produce any event delivery?
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FireOutcome {
    Delivered { subscribers: usize },
    NoSubscribers,
    ClaimLost { winner: String },
    SkippedBudget { reason: String },
    NotRegistered,
    Halted,
}

#[derive(Debug)]
pub enum WakeServiceError {
    Storage(crate::runtime::storage::StorageError),
    InvalidCron(String),
    InvalidDailyAt(String),
}

impl std::fmt::Display for WakeServiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WakeServiceError::Storage(e) => write!(f, "wake storage: {e}"),
            WakeServiceError::InvalidCron(e) => write!(f, "wake invalid cron: {e}"),
            WakeServiceError::InvalidDailyAt(e) => write!(f, "wake invalid daily at: {e}"),
        }
    }
}

impl std::error::Error for WakeServiceError {}

impl From<crate::runtime::storage::StorageError> for WakeServiceError {
    fn from(e: crate::runtime::storage::StorageError) -> Self {
        WakeServiceError::Storage(e)
    }
}

// ── CronDriver (pure functions) ─────────────────────────────────────────────

/// Compute the next firing instant strictly after `from`.
pub fn next_fire(when: &WhenExpr, from: DateTime<Utc>) -> Result<DateTime<Utc>, WakeServiceError> {
    match when {
        WhenExpr::Every(d) => Ok(from + ast_duration_to_chrono(d)),
        WhenExpr::DailyAt(tod) => next_daily_at(*tod, from),
        WhenExpr::Cron(expr) => next_cron(expr, from),
    }
}

fn ast_duration_to_chrono(d: &AstDuration) -> chrono::Duration {
    let secs = match d.unit {
        DurationUnit::Seconds => d.value,
        DurationUnit::Minutes => d.value.saturating_mul(60),
        DurationUnit::Hours => d.value.saturating_mul(3600),
        DurationUnit::Days => d.value.saturating_mul(86400),
    };
    chrono::Duration::seconds(secs as i64)
}

fn next_daily_at(tod: TimeOfDay, from: DateTime<Utc>) -> Result<DateTime<Utc>, WakeServiceError> {
    let date = from.date_naive();
    let today = NaiveDate::from_ymd_opt(date.year(), date.month(), date.day())
        .and_then(|d| d.and_hms_opt(u32::from(tod.hour), u32::from(tod.minute), 0))
        .and_then(|ndt| Utc.from_local_datetime(&ndt).single())
        .ok_or_else(|| {
            WakeServiceError::InvalidDailyAt(format!(
                "cannot build datetime for {:02}:{:02}",
                tod.hour, tod.minute
            ))
        })?;

    if today > from {
        Ok(today)
    } else {
        let tomorrow = today + chrono::Duration::days(1);
        Ok(tomorrow)
    }
}

fn next_cron(expr: &str, from: DateTime<Utc>) -> Result<DateTime<Utc>, WakeServiceError> {
    let parser = croner::parser::CronParser::builder()
        .seconds(croner::parser::Seconds::Disallowed)
        .year(croner::parser::Year::Disallowed)
        .build();
    let cron = parser
        .parse(expr)
        .map_err(|e| WakeServiceError::InvalidCron(format!("{e:?}")))?;
    cron.find_next_occurrence(&from, false)
        .map_err(|e| WakeServiceError::InvalidCron(format!("{e:?}")))
}

// ── WakeService ─────────────────────────────────────────────────────────────

pub struct WakeService {
    schedules: Vec<ScheduleRegistration>,
    event_bus: SharedEventBus,
    storage: SharedStorage,
    clock: SharedClock,
    tracer: Tracer,
    budget_query: BudgetQuery,
    config: WakeServiceConfig,
    /// Agent-lifecycle helper used by `mode: wake` to rehydrate dormant
    /// agents before publishing the wake event (#333). Optional so existing
    /// callers that only use `mode: spawn` need not construct one.
    lifecycle: Option<Arc<AgentLifecycle>>,
}

impl WakeService {
    pub fn new(
        schedules: Vec<ScheduleRegistration>,
        event_bus: SharedEventBus,
        storage: SharedStorage,
        clock: SharedClock,
        tracer: Tracer,
        budget_query: BudgetQuery,
        config: WakeServiceConfig,
    ) -> Self {
        Self {
            schedules,
            event_bus,
            storage,
            clock,
            tracer,
            budget_query,
            config,
            lifecycle: None,
        }
    }

    /// Attach an `AgentLifecycle` so `mode: wake` schedules can rehydrate
    /// dormant agents before publishing their wake event (#333). Without a
    /// lifecycle, `mode: wake` degrades to the `mode: spawn` event-publish
    /// path and traces `session_rehydrate_failed`.
    pub fn with_lifecycle(mut self, lifecycle: Arc<AgentLifecycle>) -> Self {
        self.lifecycle = Some(lifecycle);
        self
    }

    /// True if any registered schedule requests `precision: high`. Callers
    /// building `WakeServiceConfig` can use this to pick a smaller tick.
    pub fn wants_high_precision(&self) -> bool {
        self.schedules
            .iter()
            .any(|r| matches!(precision_of(&r.schedule), Some(Precision::High)))
    }

    /// Reconcile declared schedules against the redb ledger:
    /// - register fresh rows for newly-declared schedules;
    /// - GC rows whose schedule was removed from source;
    /// - leave untouched rows that are still declared.
    pub fn reconcile(&self) -> Result<(), WakeServiceError> {
        use std::collections::HashMap;
        let mut declared_by_agent: HashMap<&str, HashSet<&str>> = HashMap::new();
        for reg in &self.schedules {
            declared_by_agent
                .entry(reg.agent.as_str())
                .or_default()
                .insert(reg.schedule.name.node.as_str());
        }

        // Register fresh rows (if absent) for each declared schedule.
        let now_ms = self.clock.now_ms();
        let now_utc = self.clock.now_utc();
        for reg in &self.schedules {
            let existing = self
                .storage
                .get_schedule_state(&reg.agent, &reg.schedule.name.node)?;
            if existing.is_none() {
                let Some(when) = reg.schedule.when.as_ref() else {
                    // checker enforces `when:` is required for `mode: spawn`
                    // and `mode: wake`; reaching this branch means an invalid
                    // source slipped past — skip rather than panic.
                    continue;
                };
                let next = next_fire(&when.node, now_utc)?;
                let next_ms = next.timestamp_millis().max(0) as u64;
                let _ = now_ms;
                self.storage.upsert_schedule_state(
                    &reg.agent,
                    &reg.schedule.name.node,
                    &ScheduleState::fresh(next_ms),
                )?;
            }
        }

        // GC orphans: for every agent we know about, drop rows whose schedule
        // name is no longer declared.
        for (agent, declared) in &declared_by_agent {
            let rows = self.storage.list_schedules_for_agent(agent)?;
            for (schedule_name, _) in rows {
                if !declared.contains(schedule_name.as_str()) {
                    self.storage.delete_schedule(agent, &schedule_name)?;
                }
            }
        }
        Ok(())
    }

    /// One-shot catchup sweep: for each schedule whose `next_run_at_ms` is in
    /// the past, fire exactly once (policy `once`, the MVP default). 5s stagger
    /// between fires so the bus doesn't see a thundering herd after a restart.
    pub async fn catchup(&self) -> Result<(), WakeServiceError> {
        let now_ms = self.clock.now_ms();
        let mut fired_any = false;
        for reg in &self.schedules {
            let Some(state) = self
                .storage
                .get_schedule_state(&reg.agent, &reg.schedule.name.node)?
            else {
                continue;
            };
            if state.last_status == ScheduleStatus::Halted {
                continue;
            }
            if state.next_run_at_ms <= now_ms {
                if fired_any {
                    tokio::time::sleep(self.config.catchup_stagger).await;
                }
                self.fire_once(reg).await?;
                fired_any = true;
            }
        }
        Ok(())
    }

    /// One sweep of the tick loop: fire every schedule whose `next_run_at_ms`
    /// has been reached.
    pub async fn sweep(&self) -> Result<(), WakeServiceError> {
        let now_ms = self.clock.now_ms();
        for reg in &self.schedules {
            let Some(state) = self
                .storage
                .get_schedule_state(&reg.agent, &reg.schedule.name.node)?
            else {
                continue;
            };
            if state.last_status == ScheduleStatus::Halted {
                continue;
            }
            if state.next_run_at_ms <= now_ms {
                self.fire_once(reg).await?;
            }
        }
        Ok(())
    }

    /// Fire a single schedule: claim, budget gate, dispatch, update state.
    pub async fn fire_once(
        &self,
        reg: &ScheduleRegistration,
    ) -> Result<FireOutcome, WakeServiceError> {
        let now_ms = self.clock.now_ms();
        let ttl_ms = self.config.claim_ttl.as_millis() as u64;

        // 1. Try to take the claim.
        let claim = self.storage.try_claim_schedule(
            &reg.agent,
            &reg.schedule.name.node,
            &self.config.instance_id,
            now_ms,
            ttl_ms,
        )?;
        let mut state = match claim {
            ClaimOutcome::Claimed { state } => state,
            ClaimOutcome::Lost { winner, .. } => {
                self.tracer
                    .schedule_claim_lost(&reg.agent, &reg.schedule.name.node, &winner);
                return Ok(FireOutcome::ClaimLost { winner });
            }
            ClaimOutcome::NotRegistered => {
                return Ok(FireOutcome::NotRegistered);
            }
        };

        if state.last_status == ScheduleStatus::Halted {
            return Ok(FireOutcome::Halted);
        }

        // 2. Budget gate.
        if let Some(reason) = (self.budget_query)(&reg.agent) {
            self.tracer
                .schedule_skipped_budget(&reg.agent, &reg.schedule.name.node, &reason);
            // Record the skip, advance the next fire time, release claim.
            state.last_status = ScheduleStatus::SkippedBudget;
            state.claimed_by = None;
            state.claim_expires_at_ms = None;
            self.advance_next_fire(&mut state, reg)?;
            self.storage
                .upsert_schedule_state(&reg.agent, &reg.schedule.name.node, &state)?;
            return Ok(FireOutcome::SkippedBudget { reason });
        }

        // 3. Dispatch. Tracer ordering is load-bearing (#333):
        //    `schedule_fired` → (for wake) `schedule_rehydrated` → bus deliver.
        //    We emit `schedule_fired` BEFORE dispatch so the rehydration event
        //    (emitted inside `dispatch_wake`) lands AFTER it, and the bus
        //    delivery events land last. Replay consumers depend on this.
        let scheduled_at_ms = state.next_run_at_ms;
        let mode = mode_of(&reg.schedule).unwrap_or(ScheduleMode::Spawn);
        self.tracer.schedule_fired(
            &reg.agent,
            &reg.schedule.name.node,
            match mode {
                ScheduleMode::Spawn => "spawn",
                ScheduleMode::Wake => "wake",
            },
            scheduled_at_ms,
            now_ms,
        );
        let delivered = self.dispatch(reg, mode).await;

        // 4. Update state based on delivery outcome.
        let outcome = if delivered == 0 {
            state.consecutive_errors = state.consecutive_errors.saturating_add(1);
            state.last_status = ScheduleStatus::Error;
            self.tracer.schedule_errored(
                &reg.agent,
                &reg.schedule.name.node,
                "no subscribers",
                state.consecutive_errors,
            );
            if state.consecutive_errors >= self.config.max_consecutive_errors {
                state.last_status = ScheduleStatus::Halted;
            }
            FireOutcome::NoSubscribers
        } else {
            state.consecutive_errors = 0;
            state.last_status = ScheduleStatus::Success;
            state.last_run_at_ms = Some(now_ms);
            FireOutcome::Delivered {
                subscribers: delivered,
            }
        };

        state.claimed_by = None;
        state.claim_expires_at_ms = None;
        self.advance_next_fire(&mut state, reg)?;
        self.storage
            .upsert_schedule_state(&reg.agent, &reg.schedule.name.node, &state)?;

        Ok(outcome)
    }

    fn advance_next_fire(
        &self,
        state: &mut ScheduleState,
        reg: &ScheduleRegistration,
    ) -> Result<(), WakeServiceError> {
        let Some(when) = reg.schedule.when.as_ref() else {
            return Ok(());
        };
        let from = self.clock.now_utc();
        let next = next_fire(&when.node, from)?;
        let next_ms = next.timestamp_millis().max(0) as u64;
        state.next_run_at_ms = next_ms;
        Ok(())
    }

    async fn dispatch(&self, reg: &ScheduleRegistration, mode: ScheduleMode) -> usize {
        match mode {
            ScheduleMode::Spawn => self.dispatch_spawn(reg).await,
            ScheduleMode::Wake => self.dispatch_wake(reg).await,
        }
    }

    /// `mode: spawn` dispatch: publish a bus event named after the schedule,
    /// carrying the prompt (if any). Handlers run as stateless one-shot turns.
    async fn dispatch_spawn(&self, reg: &ScheduleRegistration) -> usize {
        let event_name = reg.schedule.name.node.clone();
        let mut fields = std::collections::HashMap::new();
        if let Some(prompt_text) = extract_prompt_text(reg.schedule.prompt.as_ref()) {
            fields.insert(
                "prompt".to_string(),
                ConfidentValue::deterministic(Value::Text(prompt_text)),
            );
        }
        let payload = EventPayload {
            event_name,
            args: Vec::new(),
            source_agent: reg.agent.clone(),
            fields,
        };
        let bus = self.event_bus.read().await;
        bus.publish(&payload)
    }

    /// `mode: wake` dispatch: ensure the agent is live (restoring `memory
    /// persistent` and re-subscribing it to the bus if not) before publishing
    /// the wake event. Handlers observe the last persisted memory value —
    /// Principle I (honesty).
    ///
    /// Event name is `schedule.emit` if declared, otherwise the default
    /// paired handler name `{schedule_name}.tick` (the checker enforces that
    /// at least one of these exists for `mode: wake`).
    async fn dispatch_wake(&self, reg: &ScheduleRegistration) -> usize {
        // Without a lifecycle helper we cannot rehydrate a dormant agent.
        // Trace the failure and decline to publish (delivery == 0 → the fire
        // is counted as an error by the caller).
        let Some(lifecycle) = self.lifecycle.as_ref() else {
            self.tracer.session_rehydrate_failed(
                &reg.agent,
                &reg.schedule.name.node,
                "wake dispatcher has no AgentLifecycle wired",
            );
            return 0;
        };

        let handle = match lifecycle.rehydrate_or_spawn(&reg.agent).await {
            Ok(h) => h,
            Err(e) => {
                self.tracer.session_rehydrate_failed(
                    &reg.agent,
                    &reg.schedule.name.node,
                    &e.to_string(),
                );
                return 0;
            }
        };

        // Emit the rehydration event BEFORE publishing. Order:
        //   schedule_fired → schedule_rehydrated → event_delivered (from bus).
        self.tracer.schedule_rehydrated(
            &reg.agent,
            &reg.schedule.name.node,
            &handle.memory_keys_restored,
        );

        let event_name = reg
            .schedule
            .emit
            .as_ref()
            .map(|e| e.node.clone())
            .unwrap_or_else(|| format!("{}.tick", reg.schedule.name.node));
        let payload = EventPayload {
            event_name,
            args: Vec::new(),
            source_agent: reg.agent.clone(),
            fields: std::collections::HashMap::new(),
        };
        let bus = self.event_bus.read().await;
        bus.publish(&payload)
    }

    /// Drive the service under a tokio runtime. Returns once the shutdown
    /// channel signals or the receiver is dropped.
    pub async fn run(
        self,
        mut shutdown_rx: broadcast::Receiver<()>,
    ) -> Result<(), WakeServiceError> {
        self.reconcile()?;
        self.catchup().await?;
        let mut ticker = tokio::time::interval(self.config.tick_interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // Prime the first tick so we don't immediately sweep after catchup.
        ticker.tick().await;
        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    if let Err(e) = self.sweep().await {
                        eprintln!("wake_service sweep error: {e}");
                    }
                }
                _ = shutdown_rx.recv() => break,
            }
        }
        Ok(())
    }

    /// Spawn the service on the current tokio runtime. Returns a handle and
    /// the shutdown sender the caller should hold.
    pub fn spawn(self) -> WakeServiceHandle {
        let (shutdown_tx, shutdown_rx) = broadcast::channel(1);
        let join = tokio::spawn(async move { self.run(shutdown_rx).await });
        WakeServiceHandle { shutdown_tx, join }
    }
}

pub struct WakeServiceHandle {
    shutdown_tx: broadcast::Sender<()>,
    pub join: JoinHandle<Result<(), WakeServiceError>>,
}

impl WakeServiceHandle {
    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(());
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn mode_of(schedule: &ScheduleField) -> Option<ScheduleMode> {
    schedule.mode.as_ref().map(|m| m.node)
}

fn precision_of(schedule: &ScheduleField) -> Option<Precision> {
    schedule.precision.as_ref().map(|p| p.node)
}

/// Extract the plain-text portion of a prompt expression. v1 only supports
/// plain templates (no interpolation) because the scheduler has no agent
/// context to resolve variables against; a future release can define what
/// context a scheduled prompt sees.
fn extract_prompt_text(prompt: Option<&crate::ast::Spanned<crate::ast::Expr>>) -> Option<String> {
    use crate::ast::{Expr, TemplatePart};
    let spanned = prompt?;
    match &spanned.node {
        Expr::Template(parts) => {
            let mut out = String::new();
            for p in parts {
                if let TemplatePart::Text(t) = &p.node {
                    out.push_str(t);
                }
            }
            if out.is_empty() {
                None
            } else {
                Some(out)
            }
        }
        _ => None,
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Span, Spanned};
    use crate::runtime::clock::MockClock;
    use crate::runtime::event_bus::EventBus;
    use crate::runtime::storage::ForgeStorage;
    use chrono::{TimeZone, Utc};
    use std::sync::Arc;

    fn sp<T>(node: T) -> Spanned<T> {
        Spanned {
            node,
            span: Span { start: 0, end: 0 },
        }
    }

    fn noop_budget() -> BudgetQuery {
        Arc::new(|_| None)
    }

    fn over_budget(reason: &'static str) -> BudgetQuery {
        Arc::new(move |_| Some(reason.to_string()))
    }

    fn temp_storage() -> (tempfile::TempDir, SharedStorage) {
        let dir = tempfile::tempdir().unwrap();
        let s = ForgeStorage::open(&dir.path().join("t.redb")).unwrap();
        (dir, Arc::new(s))
    }

    fn schedule_every(name: &str, secs: u64) -> ScheduleField {
        ScheduleField {
            name: sp(name.to_string()),
            when: Some(sp(WhenExpr::Every(AstDuration {
                value: secs,
                unit: DurationUnit::Seconds,
            }))),
            mode: Some(sp(ScheduleMode::Spawn)),
            prompt: None,
            emit: None,
            precision: None,
            duplicates: Vec::new(),
        }
    }

    // ── CronDriver unit tests ───────────────────────────────────

    #[test]
    fn next_fire_every_10s() {
        let when = WhenExpr::Every(AstDuration {
            value: 10,
            unit: DurationUnit::Seconds,
        });
        let from = Utc.with_ymd_and_hms(2026, 4, 18, 12, 0, 0).unwrap();
        let next = next_fire(&when, from).unwrap();
        assert_eq!(next, from + chrono::Duration::seconds(10));
    }

    #[test]
    fn next_fire_every_hour() {
        let when = WhenExpr::Every(AstDuration {
            value: 1,
            unit: DurationUnit::Hours,
        });
        let from = Utc.with_ymd_and_hms(2026, 4, 18, 12, 30, 0).unwrap();
        assert_eq!(
            next_fire(&when, from).unwrap(),
            Utc.with_ymd_and_hms(2026, 4, 18, 13, 30, 0).unwrap()
        );
    }

    #[test]
    fn next_fire_daily_at_later_today() {
        let when = WhenExpr::DailyAt(TimeOfDay {
            hour: 14,
            minute: 0,
        });
        let from = Utc.with_ymd_and_hms(2026, 4, 18, 9, 0, 0).unwrap();
        assert_eq!(
            next_fire(&when, from).unwrap(),
            Utc.with_ymd_and_hms(2026, 4, 18, 14, 0, 0).unwrap()
        );
    }

    #[test]
    fn next_fire_daily_at_rolls_to_tomorrow_when_past() {
        let when = WhenExpr::DailyAt(TimeOfDay { hour: 9, minute: 0 });
        let from = Utc.with_ymd_and_hms(2026, 4, 18, 14, 0, 0).unwrap();
        assert_eq!(
            next_fire(&when, from).unwrap(),
            Utc.with_ymd_and_hms(2026, 4, 19, 9, 0, 0).unwrap()
        );
    }

    #[test]
    fn next_fire_daily_at_exact_time_rolls_to_tomorrow() {
        // If called at the exact scheduled moment, the next fire is tomorrow.
        let when = WhenExpr::DailyAt(TimeOfDay { hour: 9, minute: 0 });
        let from = Utc.with_ymd_and_hms(2026, 4, 18, 9, 0, 0).unwrap();
        assert_eq!(
            next_fire(&when, from).unwrap(),
            Utc.with_ymd_and_hms(2026, 4, 19, 9, 0, 0).unwrap()
        );
    }

    #[test]
    fn next_fire_cron_every_5_minutes() {
        // "0,5,10,...,55 * * * *" — fires every 5 minutes at the top of each
        // minute. 5-field Unix cron.
        let when = WhenExpr::Cron("*/5 * * * *".to_string());
        let from = Utc.with_ymd_and_hms(2026, 4, 18, 12, 3, 0).unwrap();
        let next = next_fire(&when, from).unwrap();
        assert_eq!(next, Utc.with_ymd_and_hms(2026, 4, 18, 12, 5, 0).unwrap());
    }

    #[test]
    fn next_fire_cron_invalid_is_error() {
        let when = WhenExpr::Cron("not a cron".to_string());
        let from = Utc.with_ymd_and_hms(2026, 4, 18, 12, 0, 0).unwrap();
        assert!(next_fire(&when, from).is_err());
    }

    // ── WakeService integration tests ───────────────────────────

    fn make_service(
        schedules: Vec<ScheduleRegistration>,
        clock: MockClock,
        budget: BudgetQuery,
    ) -> (
        WakeService,
        Tracer,
        tempfile::TempDir,
        SharedStorage,
        SharedEventBus,
    ) {
        let tracer = Tracer::with_capture();
        let bus = EventBus::new_shared(Some(tracer.clone()));
        let (dir, storage) = temp_storage();
        let config = WakeServiceConfig {
            instance_id: "test-instance".into(),
            tick_interval: StdDuration::from_millis(50),
            claim_ttl: StdDuration::from_secs(60),
            catchup_stagger: StdDuration::from_millis(1),
            max_consecutive_errors: 3,
        };
        let service = WakeService::new(
            schedules,
            bus.clone(),
            storage.clone(),
            Arc::new(clock),
            tracer.clone(),
            budget,
            config,
        );
        (service, tracer, dir, storage, bus)
    }

    async fn subscribe(
        bus: &SharedEventBus,
        event: &str,
        agent: &str,
    ) -> tokio::sync::mpsc::Receiver<EventPayload> {
        bus.write().await.subscribe(event, agent, None)
    }

    #[tokio::test]
    async fn reconcile_registers_fresh_rows_with_next_fire() {
        let clock = MockClock::new(0);
        let regs = vec![ScheduleRegistration {
            agent: "sensei".into(),
            schedule: schedule_every("heartbeat", 60),
        }];
        let (service, _t, _d, storage, _bus) = make_service(regs, clock.clone(), noop_budget());
        service.reconcile().unwrap();
        let state = storage
            .get_schedule_state("sensei", "heartbeat")
            .unwrap()
            .unwrap();
        assert_eq!(state.next_run_at_ms, 60_000);
        assert_eq!(state.consecutive_errors, 0);
    }

    #[tokio::test]
    async fn reconcile_gcs_orphaned_rows() {
        let clock = MockClock::new(0);
        let (_dir, storage) = temp_storage();
        // Seed a stale row for a schedule no longer declared.
        storage
            .upsert_schedule_state("sensei", "gone", &ScheduleState::fresh(999))
            .unwrap();
        let regs = vec![ScheduleRegistration {
            agent: "sensei".into(),
            schedule: schedule_every("alive", 60),
        }];
        let tracer = Tracer::with_capture();
        let bus = EventBus::new_shared(Some(tracer.clone()));
        let config = WakeServiceConfig {
            instance_id: "t".into(),
            tick_interval: StdDuration::from_millis(50),
            claim_ttl: StdDuration::from_secs(60),
            catchup_stagger: StdDuration::from_millis(1),
            max_consecutive_errors: 3,
        };
        let service = WakeService::new(
            regs,
            bus,
            storage.clone(),
            Arc::new(clock),
            tracer,
            noop_budget(),
            config,
        );
        service.reconcile().unwrap();
        assert!(storage
            .get_schedule_state("sensei", "gone")
            .unwrap()
            .is_none());
        assert!(storage
            .get_schedule_state("sensei", "alive")
            .unwrap()
            .is_some());
    }

    #[tokio::test]
    async fn fire_once_delivers_bus_event_and_advances_next_run() {
        let clock = MockClock::new(0);
        let regs = vec![ScheduleRegistration {
            agent: "sensei".into(),
            schedule: schedule_every("mastery_review", 30),
        }];
        let (service, tracer, _d, storage, bus) =
            make_service(regs.clone(), clock.clone(), noop_budget());
        let mut rx = subscribe(&bus, "mastery_review", "sensei").await;
        service.reconcile().unwrap();

        // Advance past the first scheduled time.
        clock.set(60_000);
        let outcome = service.fire_once(&regs[0]).await.unwrap();
        assert!(matches!(outcome, FireOutcome::Delivered { subscribers: 1 }));

        let payload = rx.recv().await.expect("subscriber received the event");
        assert_eq!(payload.event_name, "mastery_review");
        assert_eq!(payload.source_agent, "sensei");

        let state = storage
            .get_schedule_state("sensei", "mastery_review")
            .unwrap()
            .unwrap();
        assert_eq!(state.last_status, ScheduleStatus::Success);
        assert_eq!(state.last_run_at_ms, Some(60_000));
        assert_eq!(state.consecutive_errors, 0);
        assert_eq!(state.next_run_at_ms, 90_000);

        // Tracer saw the firing.
        let events = tracer.captured_events();
        assert!(events.contains(&"schedule_fired".to_string()));
    }

    #[tokio::test]
    async fn fire_once_with_no_subscribers_increments_errors_then_halts() {
        let clock = MockClock::new(0);
        let regs = vec![ScheduleRegistration {
            agent: "ghost".into(),
            schedule: schedule_every("nope", 30),
        }];
        let (service, tracer, _d, storage, _bus) =
            make_service(regs.clone(), clock.clone(), noop_budget());
        service.reconcile().unwrap();

        for tick in 1..=4u64 {
            clock.set(tick * 60_000);
            let outcome = service.fire_once(&regs[0]).await.unwrap();
            match outcome {
                FireOutcome::NoSubscribers => {}
                FireOutcome::Halted => break,
                other => panic!("unexpected: {other:?}"),
            }
        }

        let state = storage
            .get_schedule_state("ghost", "nope")
            .unwrap()
            .unwrap();
        assert_eq!(state.last_status, ScheduleStatus::Halted);
        assert!(state.consecutive_errors >= 3);
        assert!(
            tracer
                .captured_events()
                .iter()
                .filter(|e| e.as_str() == "schedule_errored")
                .count()
                >= 3
        );
    }

    #[tokio::test]
    async fn budget_gate_skips_dispatch_and_advances() {
        let clock = MockClock::new(0);
        let regs = vec![ScheduleRegistration {
            agent: "sensei".into(),
            schedule: schedule_every("expensive", 30),
        }];
        let (service, tracer, _d, storage, bus) =
            make_service(regs.clone(), clock.clone(), over_budget("session cap"));
        // Subscribe so we can detect (un)delivery.
        let mut rx = subscribe(&bus, "expensive", "sensei").await;
        service.reconcile().unwrap();
        clock.set(60_000);

        let outcome = service.fire_once(&regs[0]).await.unwrap();
        match outcome {
            FireOutcome::SkippedBudget { reason } => assert_eq!(reason, "session cap"),
            other => panic!("expected SkippedBudget, got {other:?}"),
        }
        // No event should have been published.
        assert!(rx.try_recv().is_err());

        let state = storage
            .get_schedule_state("sensei", "expensive")
            .unwrap()
            .unwrap();
        assert_eq!(state.last_status, ScheduleStatus::SkippedBudget);
        assert_eq!(
            state.next_run_at_ms, 90_000,
            "budget skip still advances next_run"
        );
        assert!(tracer
            .captured_events()
            .contains(&"schedule_skipped_budget".to_string()));
    }

    #[tokio::test]
    async fn claim_loss_is_traced_and_returns_claim_lost() {
        // Simulate a live foreign claim by pre-writing ScheduleState.
        let clock = MockClock::new(0);
        let regs = vec![ScheduleRegistration {
            agent: "sensei".into(),
            schedule: schedule_every("mastery", 30),
        }];
        let (service, tracer, _d, storage, _bus) =
            make_service(regs.clone(), clock.clone(), noop_budget());
        service.reconcile().unwrap();

        // Pre-write a live claim by a different instance.
        let mut state = storage
            .get_schedule_state("sensei", "mastery")
            .unwrap()
            .unwrap();
        state.claimed_by = Some("other-instance".into());
        state.claim_expires_at_ms = Some(10_000_000);
        storage
            .upsert_schedule_state("sensei", "mastery", &state)
            .unwrap();

        clock.set(60_000);
        let outcome = service.fire_once(&regs[0]).await.unwrap();
        match outcome {
            FireOutcome::ClaimLost { winner } => assert_eq!(winner, "other-instance"),
            other => panic!("expected ClaimLost, got {other:?}"),
        }
        assert!(tracer
            .captured_events()
            .contains(&"schedule_claim_lost".to_string()));
    }

    #[tokio::test]
    async fn catchup_fires_past_due_schedules_once() {
        let clock = MockClock::new(0);
        let regs = vec![ScheduleRegistration {
            agent: "sensei".into(),
            schedule: schedule_every("hourly", 3600),
        }];
        let (service, _t, _d, storage, bus) =
            make_service(regs.clone(), clock.clone(), noop_budget());
        let mut rx = subscribe(&bus, "hourly", "sensei").await;

        // First, reconcile at t=0 to seed next_run_at = 3_600_000.
        service.reconcile().unwrap();
        // Then jump forward ~2h. The schedule is now due.
        clock.set(7_200_000);

        service.catchup().await.unwrap();
        // Exactly one delivery happens during catchup (policy `once`).
        assert!(rx.recv().await.is_some());
        assert!(rx.try_recv().is_err(), "catchup must fire only once");

        let state = storage
            .get_schedule_state("sensei", "hourly")
            .unwrap()
            .unwrap();
        assert_eq!(state.last_status, ScheduleStatus::Success);
        assert!(state.next_run_at_ms > 7_200_000);
    }

    #[tokio::test]
    async fn wants_high_precision_reflects_declared_schedules() {
        let mut s_lo = schedule_every("a", 60);
        s_lo.precision = None;
        let mut s_hi = schedule_every("b", 60);
        s_hi.precision = Some(sp(Precision::High));

        let clock = MockClock::new(0);
        let (svc_lo, _t, _d, _s, _b) = make_service(
            vec![ScheduleRegistration {
                agent: "x".into(),
                schedule: s_lo,
            }],
            clock.clone(),
            noop_budget(),
        );
        assert!(!svc_lo.wants_high_precision());

        let (svc_hi, _t2, _d2, _s2, _b2) = make_service(
            vec![ScheduleRegistration {
                agent: "x".into(),
                schedule: s_hi,
            }],
            clock,
            noop_budget(),
        );
        assert!(svc_hi.wants_high_precision());
    }
}
