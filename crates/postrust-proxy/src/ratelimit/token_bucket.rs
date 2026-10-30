//! Token bucket algorithm for rate limiting.

use parking_lot::Mutex;
use std::time::Instant;

/// Token bucket rate limiter.
///
/// Implements the token bucket algorithm where tokens are added at a fixed rate
/// up to a maximum capacity. Each request consumes one token.
///
/// The rate is a fraction of a token per second, not a whole number. A limit
/// of 3 requests a minute is 0.05 tokens a second; when the rate was carried
/// as an integer that rounded to 0, the bucket never refilled, and a client
/// that spent its burst was locked out until the process restarted.
///
/// The count and the time it was last brought up to date live under one lock.
/// They used to be two atomics updated by separate compare-and-swaps, which
/// let two concurrent requests both credit the same elapsed time.
pub struct TokenBucket {
    /// Maximum number of tokens (burst capacity)
    capacity: f64,
    /// Tokens added per second
    refill_rate: f64,
    state: Mutex<BucketState>,
}

struct BucketState {
    tokens: f64,
    last_refill: Instant,
}

impl TokenBucket {
    /// Create a new token bucket, full.
    ///
    /// # Arguments
    /// * `capacity` - Maximum tokens (burst size)
    /// * `refill_rate` - Tokens added per second; may be fractional
    pub fn new(capacity: u64, refill_rate: f64) -> Self {
        let capacity = capacity as f64;
        Self {
            capacity,
            // A negative or NaN rate would drain the bucket or poison the
            // arithmetic; neither is a rate anyone meant.
            refill_rate: if refill_rate.is_finite() && refill_rate > 0.0 {
                refill_rate
            } else {
                0.0
            },
            state: Mutex::new(BucketState {
                tokens: capacity,
                last_refill: Instant::now(),
            }),
        }
    }

    /// Try to consume a token.
    ///
    /// Returns `true` if a token was available and consumed, `false` otherwise.
    pub fn try_acquire(&self) -> bool {
        self.try_acquire_n(1)
    }

    /// Try to consume N tokens.
    ///
    /// Returns `true` if N tokens were available and consumed, `false` otherwise.
    pub fn try_acquire_n(&self, n: u64) -> bool {
        self.try_acquire_n_at(n, Instant::now())
    }

    /// [`try_acquire_n`](Self::try_acquire_n) as of `now`, so tests can step
    /// time rather than sleep through it.
    pub(crate) fn try_acquire_n_at(&self, n: u64, now: Instant) -> bool {
        let mut state = self.state.lock();
        self.refill(&mut state, now);
        let cost = n as f64;
        if state.tokens >= cost {
            state.tokens -= cost;
            true
        } else {
            false
        }
    }

    /// Get current available tokens (approximate).
    pub fn available(&self) -> u64 {
        let mut state = self.state.lock();
        self.refill(&mut state, Instant::now());
        state.tokens.floor() as u64
    }

    /// Whether the bucket has refilled completely.
    ///
    /// A full bucket is indistinguishable from a fresh one, so the limiter can
    /// forget it without changing any future answer.
    pub fn is_full(&self) -> bool {
        self.is_full_at(Instant::now())
    }

    pub(crate) fn is_full_at(&self, now: Instant) -> bool {
        let mut state = self.state.lock();
        self.refill(&mut state, now);
        state.tokens >= self.capacity
    }

    fn refill(&self, state: &mut BucketState, now: Instant) {
        let elapsed = now.saturating_duration_since(state.last_refill);
        state.tokens = (state.tokens + elapsed.as_secs_f64() * self.refill_rate).min(self.capacity);
        // Only move forward: an `Instant` captured before the lock was taken
        // can be older than the one another request has already stored.
        if now > state.last_refill {
            state.last_refill = now;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_token_bucket_basic() {
        let bucket = TokenBucket::new(10, 1.0); // 10 capacity, 1 token/sec

        // Should be able to acquire up to capacity
        for _ in 0..10 {
            assert!(bucket.try_acquire());
        }

        // Should fail after exhausting tokens
        assert!(!bucket.try_acquire());
    }

    #[test]
    fn test_token_bucket_burst() {
        let bucket = TokenBucket::new(5, 10.0); // 5 burst, 10 tokens/sec

        // Burst of 5 should succeed
        assert!(bucket.try_acquire_n(5));

        // Next request should fail
        assert!(!bucket.try_acquire());
    }

    #[test]
    fn a_rate_below_one_per_second_still_refills() {
        // 3 requests a minute. As an integer rate this was 3 / 60 = 0, and the
        // bucket never refilled once the burst was spent.
        let bucket = TokenBucket::new(3, 3.0 / 60.0);
        // After the bucket's own clock starts, so the steps below are at
        // least as long as they say.
        let start = Instant::now();
        for _ in 0..3 {
            assert!(bucket.try_acquire_n_at(1, start));
        }
        assert!(!bucket.try_acquire_n_at(1, start));

        // One token every 20 seconds.
        assert!(!bucket.try_acquire_n_at(1, start + Duration::from_secs(19)));
        assert!(bucket.try_acquire_n_at(1, start + Duration::from_secs(20)));
        assert!(!bucket.try_acquire_n_at(1, start + Duration::from_secs(21)));

        // And it refills to the burst, no further.
        let later = start + Duration::from_secs(3600);
        assert!(bucket.is_full_at(later));
        for _ in 0..3 {
            assert!(bucket.try_acquire_n_at(1, later));
        }
        assert!(!bucket.try_acquire_n_at(1, later));
    }

    #[test]
    fn a_zero_or_nonsense_rate_does_not_refill_or_panic() {
        let start = Instant::now();
        for rate in [0.0, -1.0, f64::NAN] {
            let bucket = TokenBucket::new(1, rate);
            assert!(bucket.try_acquire_n_at(1, start));
            assert!(!bucket.try_acquire_n_at(1, start + Duration::from_secs(3600)));
        }
    }
}
