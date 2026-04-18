// FORGE structured execution tracer
// Emits JSON trace events to stderr for accountability (Principle VIII).
// See issue #9.

use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::sync::broadcast;

type TraceLog = Vec<(String, serde_json::Value)>;

#[derive(Clone)]
pub struct Tracer {
    start: Instant,
    captured: Option<Arc<Mutex<TraceLog>>>,
    live_tx: Option<broadcast::Sender<String>>,
}

impl Default for Tracer {
    fn default() -> Self {
        Self::new()
    }
}

pub struct LLMResponseInfo<'a> {
    pub operation: &'a str,
    pub provider: &'a str,
    pub model: &'a str,
    pub tokens_in: u32,
    pub tokens_out: u32,
    pub cost_usd: f32,
    pub confidence: f32,
    pub agent_name: Option<&'a str>,
}

impl Tracer {
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
            captured: None,
            live_tx: None,
        }
    }

    /// Create a tracer that captures events in memory (for conformance tests).
    pub fn with_capture() -> Self {
        Self {
            start: Instant::now(),
            captured: Some(Arc::new(Mutex::new(Vec::new()))),
            live_tx: None,
        }
    }

    /// Create a tracer that broadcasts events to an SSE channel (for live streaming).
    pub fn with_live(tx: broadcast::Sender<String>) -> Self {
        Self {
            start: Instant::now(),
            captured: None,
            live_tx: Some(tx),
        }
    }

    /// Return captured event type names in order.
    pub fn captured_events(&self) -> Vec<String> {
        self.captured.as_ref().map_or_else(Vec::new, |buf| {
            buf.lock()
                .unwrap()
                .iter()
                .map(|(name, _)| name.clone())
                .collect()
        })
    }

    /// Return captured (name, payload) pairs in order — for tests.
    pub fn captured_log(&self) -> Vec<(String, serde_json::Value)> {
        self.captured
            .as_ref()
            .map_or_else(Vec::new, |buf| buf.lock().unwrap().clone())
    }

    fn ts_ms(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }

    fn emit(&self, event: &str, data: serde_json::Value) {
        let mut obj = serde_json::json!({
            "ts_ms": self.ts_ms(),
            "event": event,
        });
        if let serde_json::Value::Object(map) = data {
            if let serde_json::Value::Object(ref mut root) = obj {
                root.extend(map);
            }
        }
        if let Some(ref buf) = self.captured {
            buf.lock().unwrap().push((event.to_string(), obj.clone()));
        }
        if let Some(ref tx) = self.live_tx {
            let _ = tx.send(obj.to_string());
        }
        eprintln!("{}", obj);
    }

    pub fn llm_request(&self, operation: &str, prompt: &str) {
        self.emit(
            "llm_request",
            serde_json::json!({
                "operation": operation,
                "prompt_len": prompt.len(),
            }),
        );
    }

    pub fn llm_response(&self, info: &LLMResponseInfo) {
        let mut data = serde_json::json!({
            "operation": info.operation,
            "provider": info.provider,
            "model": info.model,
            "tokens_in": info.tokens_in,
            "tokens_out": info.tokens_out,
            "cost_usd": info.cost_usd,
            "confidence": info.confidence,
        });
        if let Some(agent) = info.agent_name {
            data["agent"] = serde_json::json!(agent);
        }
        self.emit("llm_response", data);
    }

    pub fn when_dispatch(&self, subject: &str, level: &str, matched: bool) {
        self.emit(
            "when_dispatch",
            serde_json::json!({
                "subject": subject,
                "level": level,
                "matched": matched,
            }),
        );
    }

    pub fn task_call(&self, name: &str) {
        self.emit(
            "task_call",
            serde_json::json!({
                "task": name,
            }),
        );
    }

    pub fn task_return(&self, name: &str, success: bool) {
        self.emit(
            "task_return",
            serde_json::json!({
                "task": name,
                "success": success,
            }),
        );
    }

    // ── Say tracing (issue #138) ──────────────────────────────────────────────

    pub fn say(&self, text: &str) {
        self.emit("say", serde_json::json!({ "text": text }));
    }

    // ── Flow / wave / stage tracing ──────────────────────────────────────────

    pub fn flow_start(&self, name: &str, wave_count: usize) {
        self.emit(
            "flow_start",
            serde_json::json!({
                "flow": name,
                "waves": wave_count,
            }),
        );
    }

    pub fn flow_complete(&self, name: &str) {
        self.emit(
            "flow_complete",
            serde_json::json!({
                "flow": name,
            }),
        );
    }

    pub fn wave_start(&self, wave_idx: usize, stages: &[String]) {
        self.emit(
            "wave_start",
            serde_json::json!({
                "wave": wave_idx,
                "stages": stages,
            }),
        );
    }

    pub fn wave_complete(&self, wave_idx: usize) {
        self.emit(
            "wave_complete",
            serde_json::json!({
                "wave": wave_idx,
            }),
        );
    }

    pub fn stage_start(&self, name: &str) {
        self.emit(
            "stage_start",
            serde_json::json!({
                "stage": name,
            }),
        );
    }

    pub fn stage_complete(&self, name: &str, has_give: bool) {
        self.emit(
            "stage_complete",
            serde_json::json!({
                "stage": name,
                "has_give": has_give,
            }),
        );
    }

    // ── Pool tracing ────────────────────────────────────────────────────────

    pub fn pool_send(&self, pool_name: &str, event: &str, worker_count: usize, strategy: &str) {
        self.emit(
            "pool_send",
            serde_json::json!({
                "pool": pool_name,
                "event": event,
                "workers": worker_count,
                "strategy": strategy,
            }),
        );
    }

    pub fn pool_resolved(
        &self,
        pool_name: &str,
        strategy: &str,
        success: bool,
        agreement: Option<f32>,
    ) {
        self.emit(
            "pool_resolved",
            serde_json::json!({
                "pool": pool_name,
                "strategy": strategy,
                "success": success,
                "agreement": agreement,
            }),
        );
    }

    // ── Event bus tracing (issue #19) ───────────────────────────────────────

    pub fn event_emit(&self, source_agent: &str, event_name: &str, subscriber_count: usize) {
        self.emit(
            "event_emit",
            serde_json::json!({
                "source_agent": source_agent,
                "event": event_name,
                "subscribers": subscriber_count,
            }),
        );
    }

    pub fn event_delivered(&self, event_name: &str, target_agent: &str) {
        self.emit(
            "event_delivered",
            serde_json::json!({
                "event": event_name,
                "target_agent": target_agent,
            }),
        );
    }

    pub fn event_delivery_failed(&self, event_name: &str, target_agent: &str, reason: &str) {
        self.emit(
            "event_delivery_failed",
            serde_json::json!({
                "event": event_name,
                "target_agent": target_agent,
                "reason": reason,
            }),
        );
    }

    // ── Timer tracing (issue #20) ──────────────────────────────────────────

    pub fn timer_started(&self, agent: &str, timer_name: &str, duration_secs: u64) {
        self.emit(
            "timer_started",
            serde_json::json!({
                "agent": agent,
                "timer": timer_name,
                "duration_secs": duration_secs,
            }),
        );
    }

    pub fn timer_cancelled(&self, agent: &str, timer_name: &str) {
        self.emit(
            "timer_cancelled",
            serde_json::json!({
                "agent": agent,
                "timer": timer_name,
            }),
        );
    }

    pub fn timer_fired(&self, agent: &str, timer_name: &str) {
        self.emit(
            "timer_fired",
            serde_json::json!({
                "agent": agent,
                "timer": timer_name,
            }),
        );
    }

    // ── Schedule tracing (issue #332) ───────────────────────────────────────
    //
    // Schedules are the WakeService-owned counterpart to timers. Each fire,
    // skip, error, and claim contention emits a discrete event so the observer
    // (and replay harness) can reconstruct the cadence deterministically
    // — per Principle II, wall-clock is an oracle on the determinism boundary.

    pub fn schedule_fired(
        &self,
        agent: &str,
        schedule: &str,
        mode: &str,
        scheduled_at_ms: u64,
        wall_time_ms: u64,
    ) {
        self.emit(
            "schedule_fired",
            serde_json::json!({
                "agent": agent,
                "schedule": schedule,
                "mode": mode,
                "scheduled_at_ms": scheduled_at_ms,
                "wall_time_ms": wall_time_ms,
            }),
        );
    }

    pub fn schedule_skipped_concurrent(&self, agent: &str, schedule: &str, held_by: &str) {
        self.emit(
            "schedule_skipped_concurrent",
            serde_json::json!({
                "agent": agent,
                "schedule": schedule,
                "held_by": held_by,
            }),
        );
    }

    pub fn schedule_skipped_budget(&self, agent: &str, schedule: &str, budget_state: &str) {
        self.emit(
            "schedule_skipped_budget",
            serde_json::json!({
                "agent": agent,
                "schedule": schedule,
                "budget_state": budget_state,
            }),
        );
    }

    pub fn schedule_errored(&self, agent: &str, schedule: &str, error: &str, retry_count: u32) {
        self.emit(
            "schedule_errored",
            serde_json::json!({
                "agent": agent,
                "schedule": schedule,
                "error": error,
                "retry_count": retry_count,
            }),
        );
    }

    pub fn schedule_claim_lost(&self, agent: &str, schedule: &str, winner: &str) {
        self.emit(
            "schedule_claim_lost",
            serde_json::json!({
                "agent": agent,
                "schedule": schedule,
                "winner": winner,
            }),
        );
    }

    // ── HTTP server tracing (issue #43) ─────────────────────────────────────

    pub fn http_request(&self, endpoint: &str, method: &str, path: &str) {
        self.emit(
            "http_request",
            serde_json::json!({
                "endpoint": endpoint,
                "method": method,
                "path": path,
            }),
        );
    }

    pub fn http_response(&self, endpoint: &str, status: u16, duration_ms: u64) {
        self.emit(
            "http_response",
            serde_json::json!({
                "endpoint": endpoint,
                "status": status,
                "duration_ms": duration_ms,
            }),
        );
    }

    // ── Exec tracing (issue #40) ──────────────────────────────────────────

    pub fn exec_call(&self, command: &str) {
        self.emit(
            "exec_call",
            serde_json::json!({
                "command": command,
            }),
        );
    }

    pub fn exec_return(&self, command: &str, success: bool, duration_ms: u64) {
        self.emit(
            "exec_return",
            serde_json::json!({
                "command": command,
                "success": success,
                "duration_ms": duration_ms,
            }),
        );
    }

    // ── Command tracing (issue #161) ────────────────────────────────────────

    pub fn command_call(&self, command: &str) {
        self.emit(
            "command_call",
            serde_json::json!({
                "command": command,
            }),
        );
    }

    pub fn command_return(&self, command: &str, success: bool, duration_ms: u64) {
        self.emit(
            "command_return",
            serde_json::json!({
                "command": command,
                "success": success,
                "duration_ms": duration_ms,
            }),
        );
    }

    pub fn command_bg_spawn(&self, command: &str, handle: &str) {
        self.emit(
            "command_bg_spawn",
            serde_json::json!({
                "command": command,
                "handle": handle,
            }),
        );
    }

    pub fn command_bg_complete(&self, handle: &str, success: bool, duration_ms: u64) {
        self.emit(
            "command_bg_complete",
            serde_json::json!({
                "handle": handle,
                "success": success,
                "duration_ms": duration_ms,
            }),
        );
    }

    // ── Session tracing (issue #190) ───────────────────────────────────────

    pub fn session_spawned(&self, session_id: &str) {
        self.emit(
            "session_spawned",
            serde_json::json!({
                "session_id": session_id,
            }),
        );
    }

    pub fn session_state_changed(&self, session_id: &str, status: &str) {
        self.emit(
            "session_state_changed",
            serde_json::json!({
                "session_id": session_id,
                "status": status,
            }),
        );
    }

    pub fn session_progress(&self, session_id: &str, total_cost_usd: f32) {
        self.emit(
            "session_progress",
            serde_json::json!({
                "session_id": session_id,
                "total_cost_usd": total_cost_usd,
            }),
        );
    }

    pub fn session_budget_updated(&self, session_id: &str, total_cost_usd: f32) {
        self.emit(
            "session_budget_updated",
            serde_json::json!({
                "session_id": session_id,
                "total_cost_usd": total_cost_usd,
            }),
        );
    }

    pub fn session_completed(&self, session_id: &str, total_cost_usd: f32) {
        self.emit(
            "session_completed",
            serde_json::json!({
                "session_id": session_id,
                "total_cost_usd": total_cost_usd,
            }),
        );
    }

    pub fn session_failed(&self, session_id: &str, error: &str) {
        self.emit(
            "session_failed",
            serde_json::json!({
                "session_id": session_id,
                "error": error,
            }),
        );
    }

    pub fn session_cancelled(&self, session_id: &str) {
        self.emit(
            "session_cancelled",
            serde_json::json!({
                "session_id": session_id,
            }),
        );
    }

    pub fn session_resume_attempted(&self, session_id: &str) {
        self.emit(
            "session_resume_attempted",
            serde_json::json!({
                "session_id": session_id,
            }),
        );
    }

    pub fn session_resume_failed(&self, session_id: &str, error: &str) {
        self.emit(
            "session_resume_failed",
            serde_json::json!({
                "session_id": session_id,
                "error": error,
            }),
        );
    }

    // ── Skill tracing (issue #40) ──────────────────────────────────────────

    pub fn skill_call(&self, skill_name: &str) {
        self.emit(
            "skill_call",
            serde_json::json!({
                "skill": skill_name,
            }),
        );
    }

    pub fn skill_return(&self, skill_name: &str, success: bool, duration_ms: u64) {
        self.emit(
            "skill_return",
            serde_json::json!({
                "skill": skill_name,
                "success": success,
                "duration_ms": duration_ms,
            }),
        );
    }

    pub fn supervision_tree(&self, warden: &str, active: &[&str], degraded: &[&str]) {
        self.emit(
            "supervision_tree",
            serde_json::json!({
                "warden": warden,
                "active_agents": active,
                "degraded_agents": degraded,
            }),
        );
    }

    pub fn ward_action(
        &self,
        warden: &str,
        agent: &str,
        failure_type: &str,
        response: &str,
        scope: &str,
        retry_count: u64,
    ) {
        self.emit(
            "ward_action",
            serde_json::json!({
                "warden": warden,
                "agent": agent,
                "failure_type": failure_type,
                "response": response,
                "scope": scope,
                "retry_count": retry_count,
            }),
        );
    }

    // ── Agent lifecycle tracing (issue #255) ───────────────────────────────

    pub fn agent_started(&self, agent: &str, pid: u32) {
        self.emit(
            "AgentStarted",
            serde_json::json!({
                "agent": agent,
                "pid": pid,
            }),
        );
    }

    pub fn handler_started(&self, agent: &str, handler: &str) {
        self.emit(
            "HandlerStarted",
            serde_json::json!({
                "agent": agent,
                "handler": handler,
            }),
        );
    }

    pub fn handler_completed(
        &self,
        agent: &str,
        handler: &str,
        status: &str,
        duration_ms: u64,
        confidence: Option<f32>,
    ) {
        self.emit(
            "HandlerCompleted",
            serde_json::json!({
                "agent": agent,
                "handler": handler,
                "status": status,
                "duration_ms": duration_ms,
                "confidence": confidence,
            }),
        );
    }

    pub fn agent_shutdown(&self, agent: &str, reason: &str) {
        self.emit(
            "AgentShutdown",
            serde_json::json!({
                "agent": agent,
                "reason": reason,
            }),
        );
    }
}
