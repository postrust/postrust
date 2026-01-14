//! Token bucket algorithm for rate limiting.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

/// Token bucket rate limiter.
///
/// Implements the token bucket algorithm where tokens are added at a fixed rate
/// up to a maximum capacity. Each request consumes one token.
pub struct TokenBucket {
    /// Maximum number of tokens (burst capacity)
    capacity: u64,
    /// Tokens added per second
    refill_rate: f64,
    /// Current token count (scaled by 1000 for precision)
    tokens: AtomicU64,
    /// Last refill time in milliseconds since epoch
    last_refill: AtomicU64,
}

impl TokenBucket {
    /// Create a new token bucket.
    ///
    /// # Arguments
    /// * `capacity` - Maximum tokens (burst size)
    /// * `refill_rate` - Tokens added per second
    pub fn new(capacity: u64, refill_rate: f64) -> Self {
        let now_ms = Self::now_ms();
        Self {
            capacity,
            refill_rate,
            tokens: AtomicU64::new(capacity * 1000), // Scale for precision
            last_refill: AtomicU64::new(now_ms),
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
        let now_ms = Self::now_ms();
        let cost = n * 1000; // Scale for precision

        loop {
            let last = self.last_refill.load(Ordering::Relaxed);
            let current_tokens = self.tokens.load(Ordering::Relaxed);

            // Calculate tokens to add based on elapsed time
            let elapsed_ms = now_ms.saturating_sub(last);
            let tokens_to_add = (elapsed_ms as f64 * self.refill_rate).round() as u64;

            // Calculate new token count (capped at capacity)
            let new_tokens = (current_tokens + tokens_to_add).min(self.capacity * 1000);

            // Check if we have enough tokens
            if new_tokens < cost {
                return false;
            }

            // Try to atomically update
            let final_tokens = new_tokens - cost;
            if self
                .tokens
                .compare_exchange_weak(
                    current_tokens,
                    final_tokens,
                    Ordering::SeqCst,
                    Ordering::Relaxed,
                )
                .is_ok()
            {
                // Update last refill time
                let _ = self.last_refill.compare_exchange(
                    last,
                    now_ms,
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                );
                return true;
            }
            // CAS failed, retry
        }
    }

    /// Get current available tokens (approximate).
    pub fn available(&self) -> u64 {
        let now_ms = Self::now_ms();
        let last = self.last_refill.load(Ordering::Relaxed);
        let current_tokens = self.tokens.load(Ordering::Relaxed);

        let elapsed_ms = now_ms.saturating_sub(last);
        let tokens_to_add = (elapsed_ms as f64 * self.refill_rate).round() as u64;

        (current_tokens + tokens_to_add).min(self.capacity * 1000) / 1000
    }

    fn now_ms() -> u64 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
