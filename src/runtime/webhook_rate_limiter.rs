// FORGE webhook rate limiter — issue #335
// In-process token bucket, keyed by `(agent, trigger)`. Hand-rolled to avoid
// pulling in `governor`/`ratelimit` for a single use site. Accurate to the
// granularity of `Instant::now()`.
//
// Behaviour:
// - Each `(agent, trigger)` gets an independent bucket.
// - Bucket refills at `rps` tokens per second, capped at `burst`.
// - `check()` tries to consume one token and returns `true` on success,
//   `false` if the bucket is empty (caller should return 429).
// - Buckets are created lazily on first hit. They are not evicted; the key
//   space is bounded by the number of declared webhook triggers (static per
//   program load), so leak risk is nil.

use std::collections::HashMap;
use std::sync::RwLock;
use std::time::Instant;

#[derive(Debug)]
struct TokenBucket {
    tokens: f64,
    last_refill: Instant,
}

impl TokenBucket {
    fn new(burst: u32, now: Instant) -> Self {
        Self {
            tokens: burst as f64,
            last_refill: now,
        }
    }

    fn try_consume(&mut self, rps: f64, burst: u32, now: Instant) {
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        if elapsed > 0.0 {
            self.tokens = (self.tokens + elapsed * rps).min(burst as f64);
            self.last_refill = now;
        }
    }
}

pub struct WebhookRateLimiter {
    buckets: RwLock<HashMap<(String, String), TokenBucket>>,
    rps: f64,
    burst: u32,
}

impl WebhookRateLimiter {
    /// Construct a rate limiter with the given steady-state `rps` (requests
    /// per second per key) and `burst` (bucket size).
    pub fn new(rps: u32, burst: u32) -> Self {
        Self {
            buckets: RwLock::new(HashMap::new()),
            rps: rps as f64,
            burst,
        }
    }

    /// Default limiter — 10 rps / burst 20 per `(agent, trigger)`. Tuned to
    /// absorb GitHub's retry storms without letting a rogue source wake every
    /// specialist in a tight loop.
    pub fn default_for_webhooks() -> Self {
        Self::new(10, 20)
    }

    /// Consume a token for `(agent, trigger)`. Returns `true` if the request
    /// is allowed, `false` if it should be rejected with 429.
    pub fn check(&self, agent: &str, trigger: &str) -> bool {
        self.check_at(agent, trigger, Instant::now())
    }

    /// `check` with an explicit `now` for deterministic unit tests.
    pub fn check_at(&self, agent: &str, trigger: &str, now: Instant) -> bool {
        let key = (agent.to_string(), trigger.to_string());
        let mut map = self.buckets.write().expect("poisoned rate-limit lock");
        let bucket = map
            .entry(key)
            .or_insert_with(|| TokenBucket::new(self.burst, now));
        bucket.try_consume(self.rps, self.burst, now);
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn single_request_under_limit_passes() {
        let rl = WebhookRateLimiter::new(10, 5);
        assert!(rl.check("a", "t"));
    }

    #[test]
    fn burst_is_enforced_and_recovers() {
        let rl = WebhookRateLimiter::new(10, 3);
        let t0 = Instant::now();
        // Burst of 3 passes, 4th fails.
        for _ in 0..3 {
            assert!(rl.check_at("a", "t", t0));
        }
        assert!(!rl.check_at("a", "t", t0));

        // Half a second later — 5 new tokens, capped at burst=3. 3 more pass.
        let t1 = t0 + Duration::from_millis(500);
        for _ in 0..3 {
            assert!(rl.check_at("a", "t", t1));
        }
        assert!(!rl.check_at("a", "t", t1));
    }

    #[test]
    fn separate_keys_have_separate_budgets() {
        let rl = WebhookRateLimiter::new(1, 1);
        let t0 = Instant::now();
        assert!(rl.check_at("a", "t1", t0));
        assert!(!rl.check_at("a", "t1", t0));
        // Different trigger — fresh bucket.
        assert!(rl.check_at("a", "t2", t0));
        // Different agent — fresh bucket.
        assert!(rl.check_at("b", "t1", t0));
    }

    #[test]
    fn refill_is_continuous_not_stepwise() {
        let rl = WebhookRateLimiter::new(10, 1);
        let t0 = Instant::now();
        assert!(rl.check_at("a", "t", t0));
        assert!(!rl.check_at("a", "t", t0));
        // 100ms later — exactly one token back (10 rps * 0.1s).
        let t1 = t0 + Duration::from_millis(100);
        assert!(rl.check_at("a", "t", t1));
    }
}
