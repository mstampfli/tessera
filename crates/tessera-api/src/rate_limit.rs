//! A small fixed-window rate limiter keyed by a string (a client IP).
//!
//! Used to bound login attempts (STRIDE spoofing / brute force). It is
//! in-process and lock-guarded; for a single-node service that is sufficient. A
//! distributed deployment would move this to Postgres or Redis.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Fixed-window counter: at most `max` events per `window` per key.
pub struct RateLimiter {
    max: u32,
    window: Duration,
    buckets: Mutex<HashMap<String, Window>>,
}

struct Window {
    start: Instant,
    count: u32,
}

impl RateLimiter {
    #[must_use]
    pub fn new(max: u32, window: Duration) -> Self {
        Self {
            max,
            window,
            buckets: Mutex::new(HashMap::new()),
        }
    }

    /// Record an attempt for `key`; returns true if it is within the limit.
    pub fn check(&self, key: &str) -> bool {
        let now = Instant::now();
        let mut buckets = self.buckets.lock().expect("rate limiter mutex poisoned");

        // Opportunistically evict stale windows so the map cannot grow without
        // bound under many distinct keys.
        if buckets.len() > 4096 {
            buckets.retain(|_, w| now.duration_since(w.start) < self.window);
        }

        let window = buckets.entry(key.to_string()).or_insert(Window {
            start: now,
            count: 0,
        });
        if now.duration_since(window.start) >= self.window {
            window.start = now;
            window.count = 0;
        }
        window.count += 1;
        window.count <= self.max
    }
}

#[cfg(test)]
mod tests {
    use super::RateLimiter;
    use std::time::Duration;

    #[test]
    fn allows_up_to_max_then_blocks() {
        let rl = RateLimiter::new(3, Duration::from_mins(1));
        assert!(rl.check("1.2.3.4"));
        assert!(rl.check("1.2.3.4"));
        assert!(rl.check("1.2.3.4"));
        assert!(!rl.check("1.2.3.4")); // 4th within window is blocked
                                       // A different key has its own budget.
        assert!(rl.check("5.6.7.8"));
    }
}
