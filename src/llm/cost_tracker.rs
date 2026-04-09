use crate::llm::CompletionResponse;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;

#[derive(Clone)]
pub struct CostTracker {
    total_tokens_in: Arc<AtomicU32>,
    total_tokens_out: Arc<AtomicU32>,
    total_cost_usd: Arc<AtomicU64>, // stored as microdollars (×1_000_000)
    budget_usd: Option<f32>,
    alert_at_pct: u32,
}

impl CostTracker {
    pub fn new(budget_usd: Option<f32>, alert_at_pct: u32) -> Self {
        Self {
            total_tokens_in: Arc::new(AtomicU32::new(0)),
            total_tokens_out: Arc::new(AtomicU32::new(0)),
            total_cost_usd: Arc::new(AtomicU64::new(0)),
            budget_usd,
            alert_at_pct,
        }
    }

    pub fn record(&self, resp: &CompletionResponse) -> Result<(), BudgetError> {
        self.total_tokens_in
            .fetch_add(resp.tokens_in, Ordering::Relaxed);
        self.total_tokens_out
            .fetch_add(resp.tokens_out, Ordering::Relaxed);

        let microdollars = (resp.cost_usd * 1_000_000.0) as u64;
        let new_total = self
            .total_cost_usd
            .fetch_add(microdollars, Ordering::Relaxed)
            + microdollars;
        let new_total_usd = new_total as f32 / 1_000_000.0;

        if let Some(budget) = self.budget_usd {
            let pct = (new_total_usd / budget * 100.0) as u32;

            if pct >= 100 {
                return Err(BudgetError::Exceeded {
                    spent: new_total_usd,
                    budget,
                });
            }

            if pct >= self.alert_at_pct {
                eprintln!(
                    "[forge] budget warning: ${:.4} spent of ${:.4} ({:.0}%)",
                    new_total_usd, budget, pct
                );
            }
        }

        Ok(())
    }

    pub fn summary(&self) -> CostSummary {
        CostSummary {
            total_tokens_in: self.total_tokens_in.load(Ordering::Relaxed),
            total_tokens_out: self.total_tokens_out.load(Ordering::Relaxed),
            total_cost_usd: self.total_cost_usd.load(Ordering::Relaxed) as f32 / 1_000_000.0,
            budget_usd: self.budget_usd,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CostSummary {
    pub total_tokens_in: u32,
    pub total_tokens_out: u32,
    pub total_cost_usd: f32,
    pub budget_usd: Option<f32>,
}

impl std::fmt::Display for CostSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "tokens: {}↑ {}↓  cost: ${:.4}{}",
            self.total_tokens_in,
            self.total_tokens_out,
            self.total_cost_usd,
            self.budget_usd
                .map(|b| format!(" / ${:.4}", b))
                .unwrap_or_default()
        )
    }
}

#[derive(Debug, thiserror::Error)]
pub enum BudgetError {
    #[error("budget exceeded: spent ${spent:.4} of ${budget:.4}")]
    Exceeded { spent: f32, budget: f32 },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mock_response(cost: f32, tokens_in: u32, tokens_out: u32) -> CompletionResponse {
        CompletionResponse {
            content: "test".to_string(),
            tool_calls: vec![],
            tokens_in,
            tokens_out,
            latency_ms: 1,
            model_used: "mock".to_string(),
            provider_name: "mock".to_string(),
            cost_usd: cost,
        }
    }

    #[test]
    fn budget_exceeded() {
        let tracker = CostTracker::new(Some(0.001), 80);
        let resp = mock_response(0.002, 100, 50);
        let result = tracker.record(&resp);
        assert!(matches!(result, Err(BudgetError::Exceeded { .. })));
    }

    #[test]
    fn no_budget_never_exceeds() {
        let tracker = CostTracker::new(None, 80);
        let resp = mock_response(100.0, 1000, 1000);
        assert!(tracker.record(&resp).is_ok());
    }

    #[test]
    fn summary_accumulates() {
        let tracker = CostTracker::new(Some(10.0), 80);
        tracker.record(&mock_response(0.001, 100, 50)).unwrap();
        tracker.record(&mock_response(0.002, 200, 100)).unwrap();
        let s = tracker.summary();
        assert_eq!(s.total_tokens_in, 300);
        assert_eq!(s.total_tokens_out, 150);
        assert!((s.total_cost_usd - 0.003).abs() < 0.0001);
    }

    #[test]
    fn summary_display() {
        let tracker = CostTracker::new(Some(5.0), 80);
        tracker.record(&mock_response(0.0012, 500, 200)).unwrap();
        let display = tracker.summary().to_string();
        assert!(display.contains("500↑"));
        assert!(display.contains("200↓"));
        assert!(display.contains("$5.0000"));
    }
}
