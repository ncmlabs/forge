// FORGE structured execution tracer
// Emits JSON trace events to stderr for accountability (Principle VIII).
// See issue #9.

use std::time::Instant;

#[derive(Clone)]
pub struct Tracer {
    start: Instant,
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
}

impl Tracer {
    pub fn new() -> Self {
        Self { start: Instant::now() }
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
        eprintln!("{}", obj);
    }

    pub fn llm_request(&self, operation: &str, prompt: &str) {
        self.emit("llm_request", serde_json::json!({
            "operation": operation,
            "prompt_len": prompt.len(),
        }));
    }

    pub fn llm_response(&self, info: &LLMResponseInfo) {
        self.emit("llm_response", serde_json::json!({
            "operation": info.operation,
            "provider": info.provider,
            "model": info.model,
            "tokens_in": info.tokens_in,
            "tokens_out": info.tokens_out,
            "cost_usd": info.cost_usd,
            "confidence": info.confidence,
        }));
    }

    pub fn when_dispatch(&self, subject: &str, level: &str, matched: bool) {
        self.emit("when_dispatch", serde_json::json!({
            "subject": subject,
            "level": level,
            "matched": matched,
        }));
    }

    pub fn task_call(&self, name: &str) {
        self.emit("task_call", serde_json::json!({
            "task": name,
        }));
    }

    pub fn task_return(&self, name: &str, success: bool) {
        self.emit("task_return", serde_json::json!({
            "task": name,
            "success": success,
        }));
    }

    // ── Flow / wave / stage tracing ──────────────────────────────────────────

    pub fn flow_start(&self, name: &str, wave_count: usize) {
        self.emit("flow_start", serde_json::json!({
            "flow": name,
            "waves": wave_count,
        }));
    }

    pub fn flow_complete(&self, name: &str) {
        self.emit("flow_complete", serde_json::json!({
            "flow": name,
        }));
    }

    pub fn wave_start(&self, wave_idx: usize, stages: &[String]) {
        self.emit("wave_start", serde_json::json!({
            "wave": wave_idx,
            "stages": stages,
        }));
    }

    pub fn wave_complete(&self, wave_idx: usize) {
        self.emit("wave_complete", serde_json::json!({
            "wave": wave_idx,
        }));
    }

    pub fn stage_start(&self, name: &str) {
        self.emit("stage_start", serde_json::json!({
            "stage": name,
        }));
    }

    pub fn stage_complete(&self, name: &str, has_give: bool) {
        self.emit("stage_complete", serde_json::json!({
            "stage": name,
            "has_give": has_give,
        }));
    }
}
