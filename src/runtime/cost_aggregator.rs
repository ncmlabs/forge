// Cost aggregator for token economy visibility (issue #142, Principle III).
// Subscribes to the trace event broadcast channel and maintains running totals
// for tokens, cost, confidence, broken down by operation/provider/model/agent.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{broadcast, RwLock};

pub type SharedCostAggregator = Arc<RwLock<CostAggregator>>;

#[derive(Debug, Default, Clone)]
pub struct OpStats {
    pub calls: u64,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub cost_usd: f64,
    pub confidence_sum: f64,
}

impl OpStats {
    fn record(&mut self, tokens_in: u64, tokens_out: u64, cost_usd: f64, confidence: f64) {
        self.calls += 1;
        self.tokens_in += tokens_in;
        self.tokens_out += tokens_out;
        self.cost_usd += cost_usd;
        self.confidence_sum += confidence;
    }

    fn to_json(&self) -> serde_json::Value {
        let avg_confidence = if self.calls > 0 {
            self.confidence_sum / self.calls as f64
        } else {
            0.0
        };
        serde_json::json!({
            "calls": self.calls,
            "tokens_in": self.tokens_in,
            "tokens_out": self.tokens_out,
            "cost_usd": self.cost_usd,
            "avg_confidence": (avg_confidence * 1000.0).round() / 1000.0,
        })
    }
}

pub struct CostAggregator {
    pub total_calls: u64,
    pub total_tokens_in: u64,
    pub total_tokens_out: u64,
    pub total_cost_usd: f64,
    pub by_operation: HashMap<String, OpStats>,
    pub by_provider_model: HashMap<String, OpStats>,
    pub by_agent: HashMap<String, OpStats>,
    /// Confidence histogram: 10 buckets for [0.0–0.1), [0.1–0.2), ..., [0.9–1.0]
    pub confidence_buckets: [u64; 10],
    started_at: Instant,
}

impl Default for CostAggregator {
    fn default() -> Self {
        Self::new()
    }
}

impl CostAggregator {
    pub fn new() -> Self {
        Self {
            total_calls: 0,
            total_tokens_in: 0,
            total_tokens_out: 0,
            total_cost_usd: 0.0,
            by_operation: HashMap::new(),
            by_provider_model: HashMap::new(),
            by_agent: HashMap::new(),
            confidence_buckets: [0; 10],
            started_at: Instant::now(),
        }
    }

    pub fn record_llm_event(&mut self, event: &serde_json::Value) {
        let tokens_in = event["tokens_in"].as_u64().unwrap_or(0);
        let tokens_out = event["tokens_out"].as_u64().unwrap_or(0);
        let cost_usd = event["cost_usd"].as_f64().unwrap_or(0.0);
        let confidence = event["confidence"].as_f64().unwrap_or(0.0);

        self.total_calls += 1;
        self.total_tokens_in += tokens_in;
        self.total_tokens_out += tokens_out;
        self.total_cost_usd += cost_usd;

        // Confidence histogram
        let bucket = ((confidence * 10.0) as usize).min(9);
        self.confidence_buckets[bucket] += 1;

        // By operation
        if let Some(op) = event["operation"].as_str() {
            self.by_operation
                .entry(op.to_string())
                .or_default()
                .record(tokens_in, tokens_out, cost_usd, confidence);
        }

        // By provider/model
        if let (Some(provider), Some(model)) = (event["provider"].as_str(), event["model"].as_str())
        {
            let key = format!("{}/{}", provider, model);
            self.by_provider_model
                .entry(key)
                .or_default()
                .record(tokens_in, tokens_out, cost_usd, confidence);
        }

        // By agent
        if let Some(agent) = event["agent"].as_str() {
            self.by_agent
                .entry(agent.to_string())
                .or_default()
                .record(tokens_in, tokens_out, cost_usd, confidence);
        } else {
            self.by_agent
                .entry("(anonymous)".to_string())
                .or_default()
                .record(tokens_in, tokens_out, cost_usd, confidence);
        }
    }

    pub fn snapshot(&self) -> serde_json::Value {
        let uptime_secs = self.started_at.elapsed().as_secs_f64();
        let tokens_per_sec = if uptime_secs > 0.0 {
            (self.total_tokens_out as f64 / uptime_secs * 10.0).round() / 10.0
        } else {
            0.0
        };

        let by_operation: serde_json::Map<String, serde_json::Value> = self
            .by_operation
            .iter()
            .map(|(k, v)| (k.clone(), v.to_json()))
            .collect();

        let by_provider_model: serde_json::Map<String, serde_json::Value> = self
            .by_provider_model
            .iter()
            .map(|(k, v)| (k.clone(), v.to_json()))
            .collect();

        let by_agent: serde_json::Map<String, serde_json::Value> = self
            .by_agent
            .iter()
            .map(|(k, v)| (k.clone(), v.to_json()))
            .collect();

        serde_json::json!({
            "totals": {
                "calls": self.total_calls,
                "tokens_in": self.total_tokens_in,
                "tokens_out": self.total_tokens_out,
                "cost_usd": (self.total_cost_usd * 10000.0).round() / 10000.0,
            },
            "tokens_per_sec": tokens_per_sec,
            "by_operation": by_operation,
            "by_provider_model": by_provider_model,
            "by_agent": by_agent,
            "confidence_histogram": self.confidence_buckets,
            "uptime_secs": (uptime_secs * 10.0).round() / 10.0,
        })
    }
}

/// Spawn a background task that listens to trace events and aggregates LLM costs.
pub fn spawn_cost_listener(
    events_rx: broadcast::Receiver<String>,
    aggregator: SharedCostAggregator,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut rx = events_rx;
        loop {
            match rx.recv().await {
                Ok(json_str) => {
                    if let Ok(event) = serde_json::from_str::<serde_json::Value>(&json_str) {
                        if event["event"].as_str() == Some("llm_response") {
                            aggregator.write().await.record_llm_event(&event);
                        }
                    }
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    eprintln!("[cost_aggregator] lagged, skipped {} events", n);
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    })
}
