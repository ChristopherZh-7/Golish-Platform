//! Per-provider request pacing.
//!
//! Uses leaky-bucket-style enforcement: each `acquire()` ensures at least
//! `min_interval` has elapsed since the previous successful acquire. The
//! limiter is `Send + Sync`, so a single instance can serve all concurrent
//! calls to one provider (each `Provider` impl owns one of these).

use std::time::Duration;

use tokio::sync::Mutex;
use tokio::time::{sleep_until, Instant};

/// Per-provider rate limiter.
///
/// # Example
///
/// ```
/// use golish_intel_providers::shared::RateLimiter;
/// use std::time::Duration;
///
/// # async fn example() {
/// let limiter = RateLimiter::new(Duration::from_millis(500));  // 2 req/s
/// limiter.acquire().await;
/// // ... make HTTP request ...
/// # }
/// ```
#[derive(Debug)]
pub struct RateLimiter {
    min_interval: Duration,
    next_allowed: Mutex<Instant>,
}

impl RateLimiter {
    /// Create a limiter that enforces `min_interval` between successive
    /// acquires.
    pub fn new(min_interval: Duration) -> Self {
        Self {
            min_interval,
            next_allowed: Mutex::new(Instant::now()),
        }
    }

    /// Convenience: 2 requests per second. Matches 0.zone's documented limit.
    pub fn two_per_second() -> Self {
        Self::new(Duration::from_millis(500))
    }

    /// Block until the limiter is ready, then mark the next acquire as
    /// blocked for `min_interval`. Multiple concurrent callers serialize
    /// through the internal mutex.
    pub async fn acquire(&self) {
        let mut next = self.next_allowed.lock().await;
        let now = Instant::now();
        if now < *next {
            sleep_until(*next).await;
        }
        *next = Instant::now() + self.min_interval;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn first_acquire_is_immediate() {
        let limiter = RateLimiter::new(Duration::from_secs(1));
        let t0 = Instant::now();
        limiter.acquire().await;
        assert!(t0.elapsed() < Duration::from_millis(50));
    }

    #[tokio::test]
    async fn enforces_interval_between_acquires() {
        let limiter = RateLimiter::new(Duration::from_millis(50));
        let t0 = Instant::now();
        limiter.acquire().await;
        limiter.acquire().await;
        limiter.acquire().await;
        let elapsed = t0.elapsed();
        // 3 acquires with 50ms interval: first immediate + 2 waits of ~50ms each = ~100ms
        assert!(
            elapsed >= Duration::from_millis(95),
            "expected at least ~100ms, got {elapsed:?}"
        );
        assert!(
            elapsed < Duration::from_millis(300),
            "expected under 300ms, got {elapsed:?}"
        );
    }

    #[tokio::test]
    async fn two_per_second_constructor_uses_500ms() {
        let limiter = RateLimiter::two_per_second();
        let t0 = Instant::now();
        limiter.acquire().await;
        limiter.acquire().await;
        let elapsed = t0.elapsed();
        assert!(
            elapsed >= Duration::from_millis(490),
            "expected at least ~500ms between two acquires, got {elapsed:?}"
        );
    }
}
