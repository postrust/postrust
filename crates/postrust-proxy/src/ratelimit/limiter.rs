//! Rate limiter with per-key tracking.

use crate::config::RateLimitDefaults;
use crate::ratelimit::TokenBucket;
use dashmap::DashMap;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info};

/// Key for rate limiting.
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub enum RateLimitKey {
    /// Rate limit by IP address
    Ip(IpAddr),
    /// Rate limit by custom header value
    Header(String),
    /// Rate limit by route ID
    Route(uuid::Uuid),
    /// Global rate limit
    Global,
}

/// Entry in the rate limiter cache.
struct RateLimitEntry {
    bucket: TokenBucket,
    last_access: Instant,
}

/// Rate limiter with per-key token buckets.
pub struct RateLimiter {
    /// Per-key token buckets
    buckets: DashMap<RateLimitKey, RateLimitEntry>,
    /// Default configuration
    defaults: RateLimitDefaults,
    /// Entry TTL for cleanup
    entry_ttl: Duration,
}

impl RateLimiter {
    /// Create a new rate limiter.
    pub fn new(defaults: RateLimitDefaults) -> Self {
        Self {
            buckets: DashMap::new(),
            defaults,
            entry_ttl: Duration::from_secs(3600), // 1 hour TTL
        }
    }

    /// Check if a request should be allowed.
    ///
    /// Returns `true` if the request is allowed, `false` if rate limited.
    pub fn check(&self, key: RateLimitKey) -> bool {
        // Convert requests per window to requests per second
        let rps = if self.defaults.window_secs > 0 {
            self.defaults.requests / self.defaults.window_secs
        } else {
            self.defaults.requests
        };
        self.check_with_config(key, rps, self.defaults.burst)
    }

    /// Check with custom rate limit configuration.
    pub fn check_with_config(&self, key: RateLimitKey, rps: u32, burst: u32) -> bool {
        let mut entry = self.buckets.entry(key).or_insert_with(|| RateLimitEntry {
            bucket: TokenBucket::new(burst as u64, rps as f64),
            last_access: Instant::now(),
        });

        entry.last_access = Instant::now();
        entry.bucket.try_acquire()
    }

    /// Get remaining tokens for a key (approximate).
    pub fn remaining(&self, key: &RateLimitKey) -> Option<u64> {
        self.buckets.get(key).map(|entry| entry.bucket.available())
    }

    /// Start background cleanup task.
    pub async fn start_cleanup(self: Arc<Self>, cancel_token: CancellationToken) {
        let cleanup_interval = Duration::from_secs(300); // 5 minutes

        info!("Rate limiter cleanup task started");

        loop {
            tokio::select! {
                _ = cancel_token.cancelled() => {
                    info!("Rate limiter cleanup task stopped");
                    break;
                }
                _ = tokio::time::sleep(cleanup_interval) => {
                    self.cleanup_expired();
                }
            }
        }
    }

    /// Remove expired entries.
    fn cleanup_expired(&self) {
        let now = Instant::now();
        let before = self.buckets.len();

        self.buckets.retain(|_, entry| {
            now.duration_since(entry.last_access) < self.entry_ttl
        });

        let removed = before - self.buckets.len();
        if removed > 0 {
            debug!("Rate limiter cleanup: removed {} expired entries", removed);
        }
    }
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new(RateLimitDefaults::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn test_rate_limiter_by_ip() {
        let limiter = RateLimiter::new(RateLimitDefaults {
            requests: 600,  // 10 per second
            window_secs: 60,
            burst: 5,
        });

        let ip1 = RateLimitKey::Ip(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)));
        let ip2 = RateLimitKey::Ip(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 2)));

        // Each IP gets its own bucket
        for _ in 0..5 {
            assert!(limiter.check(ip1.clone()));
            assert!(limiter.check(ip2.clone()));
        }

        // Both exhausted
        assert!(!limiter.check(ip1.clone()));
        assert!(!limiter.check(ip2.clone()));
    }

    #[test]
    fn test_rate_limiter_remaining() {
        let limiter = RateLimiter::new(RateLimitDefaults {
            requests: 600,  // 10 per second
            window_secs: 60,
            burst: 10,
        });

        let key = RateLimitKey::Global;
        assert!(limiter.check(key.clone()));

        let remaining = limiter.remaining(&key).unwrap();
        assert!(remaining < 10);
    }
}
