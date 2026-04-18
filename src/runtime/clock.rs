// FORGE clock abstraction — issue #332
// Wall-clock is an oracle on the determinism boundary (Principle II).
// Three implementations:
//   - SystemClock   : production, reads std::time::SystemTime
//   - MockClock     : tests, advance programmatically
//   - RecordedClock : replay, reads timestamps from a recorded oracle
//                     (same contract as recorded LLM responses)

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, TimeZone, Utc};

pub trait Clock: Send + Sync {
    /// Milliseconds since the Unix epoch.
    fn now_ms(&self) -> u64;

    fn now_utc(&self) -> DateTime<Utc> {
        let ms = self.now_ms() as i64;
        Utc.timestamp_millis_opt(ms)
            .single()
            .unwrap_or_else(Utc::now)
    }
}

pub type SharedClock = Arc<dyn Clock>;

// ── SystemClock ─────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }
}

// ── MockClock ───────────────────────────────────────────────────────────────

/// Monotonic mock clock driven by tests. Cheap to clone; internal state is
/// shared across clones so one handle advances every observer.
#[derive(Clone, Default)]
pub struct MockClock {
    ms: Arc<AtomicU64>,
}

impl MockClock {
    pub fn new(start_ms: u64) -> Self {
        Self {
            ms: Arc::new(AtomicU64::new(start_ms)),
        }
    }

    pub fn set(&self, ms: u64) {
        self.ms.store(ms, Ordering::SeqCst);
    }

    pub fn advance_ms(&self, delta: u64) {
        self.ms.fetch_add(delta, Ordering::SeqCst);
    }
}

impl Clock for MockClock {
    fn now_ms(&self) -> u64 {
        self.ms.load(Ordering::SeqCst)
    }
}

// ── RecordedClock ───────────────────────────────────────────────────────────

/// Reads wall-clock samples from a recorded trace. On exhaustion, returns the
/// last sample — identical to how recorded LLM responses hold steady after the
/// last captured turn.
pub struct RecordedClock {
    samples: Mutex<std::vec::IntoIter<u64>>,
    last: AtomicU64,
}

impl RecordedClock {
    pub fn new(samples: Vec<u64>) -> Self {
        let last = samples.first().copied().unwrap_or(0);
        Self {
            samples: Mutex::new(samples.into_iter()),
            last: AtomicU64::new(last),
        }
    }
}

impl Clock for RecordedClock {
    fn now_ms(&self) -> u64 {
        let mut it = self.samples.lock().expect("RecordedClock mutex poisoned");
        if let Some(next) = it.next() {
            self.last.store(next, Ordering::SeqCst);
            next
        } else {
            self.last.load(Ordering::SeqCst)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_clock_is_monotonic_and_nonzero() {
        let c = SystemClock;
        let a = c.now_ms();
        let b = c.now_ms();
        assert!(a > 0);
        assert!(b >= a);
    }

    #[test]
    fn system_clock_now_utc_roundtrips_ms() {
        let c = SystemClock;
        let ms = c.now_ms();
        let dt = c.now_utc();
        // Allow a few ms of drift between the two reads.
        let diff = (dt.timestamp_millis() - ms as i64).abs();
        assert!(diff < 1_000, "unexpected drift: {diff}ms");
    }

    #[test]
    fn mock_clock_starts_at_value_and_advances() {
        let c = MockClock::new(1_000);
        assert_eq!(c.now_ms(), 1_000);
        c.advance_ms(500);
        assert_eq!(c.now_ms(), 1_500);
        c.set(42);
        assert_eq!(c.now_ms(), 42);
    }

    #[test]
    fn mock_clock_clone_shares_state() {
        let a = MockClock::new(0);
        let b = a.clone();
        a.advance_ms(100);
        assert_eq!(
            b.now_ms(),
            100,
            "clones must share state for shared-handle semantics"
        );
    }

    #[test]
    fn recorded_clock_replays_samples_in_order() {
        let c = RecordedClock::new(vec![1, 2, 3]);
        assert_eq!(c.now_ms(), 1);
        assert_eq!(c.now_ms(), 2);
        assert_eq!(c.now_ms(), 3);
    }

    #[test]
    fn recorded_clock_holds_last_sample_after_exhaustion() {
        let c = RecordedClock::new(vec![10, 20]);
        assert_eq!(c.now_ms(), 10);
        assert_eq!(c.now_ms(), 20);
        assert_eq!(c.now_ms(), 20, "exhausted oracle holds last value");
        assert_eq!(c.now_ms(), 20);
    }

    #[test]
    fn recorded_clock_empty_returns_zero_until_samples_arrive() {
        let c = RecordedClock::new(vec![]);
        assert_eq!(c.now_ms(), 0);
    }
}
