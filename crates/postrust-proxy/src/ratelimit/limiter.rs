//! Rate limiter with per-key tracking.

use crate::config::RateLimitDefaults;
use crate::ratelimit::TokenBucket;
use dashmap::DashMap;
use parking_lot::Mutex;
use sha2::{Digest, Sha256};
use std::net::{IpAddr, Ipv6Addr};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};
use uuid::Uuid;

/// Key for rate limiting: who a bucket is counting.
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub enum RateLimitKey {
    /// Rate limit by IP address. Build it with [`RateLimitKey::client_ip`],
    /// which groups an IPv6 client by its /64.
    Ip(IpAddr),
    /// Rate limit by custom header value. Build it with
    /// [`RateLimitKey::header_value`], which stores a digest rather than the
    /// value itself.
    Header(String),
    /// Rate limit by route ID
    Route(uuid::Uuid),
    /// Global rate limit
    Global,
}

impl RateLimitKey {
    /// The key for a client connecting from `ip`.
    ///
    /// An IPv6 client is keyed by its /64. A single host is normally handed a
    /// whole /64, and can pick any address in it for each connection; keyed by
    /// the full address, every request could come from a fresh bucket, which
    /// is the same as no limit at all.
    ///
    /// An IPv4 client arriving on an IPv6 socket appears as `::ffff:a.b.c.d`.
    /// That is canonicalised back to IPv4 first, or every IPv4 client would
    /// share the one /64 that holds all the mapped addresses.
    pub fn client_ip(ip: IpAddr) -> Self {
        match ip.to_canonical() {
            IpAddr::V4(v4) => RateLimitKey::Ip(IpAddr::V4(v4)),
            IpAddr::V6(v6) => {
                let s = v6.segments();
                RateLimitKey::Ip(IpAddr::V6(Ipv6Addr::new(
                    s[0], s[1], s[2], s[3], 0, 0, 0, 0,
                )))
            }
        }
    }

    /// The key for a header value.
    ///
    /// A digest, not the value: header values are client-chosen and can be
    /// kilobytes long, and the table holds up to
    /// [`RateLimitDefaults::max_tracked_keys`] of them. It also keeps API keys
    /// and the like out of the limiter's memory.
    pub fn header_value(value: &[u8]) -> Self {
        RateLimitKey::Header(hex::encode(Sha256::digest(value)))
    }
}

/// Requests per window as a refill rate, in tokens per second.
///
/// A fraction, never rounded: see [`TokenBucket`]. A window of zero seconds is
/// read as one, which is what the integer version effectively did.
pub fn per_second(requests: u32, window_secs: u32) -> f64 {
    f64::from(requests) / f64::from(window_secs.max(1))
}

/// Which limit a bucket belongs to, and whom it counts.
///
/// The scope is the route. Without it every route shared one bucket per
/// client, sized by whichever route that client happened to reach first: a
/// generous limit on one route could be spent on another, and a strict one
/// could lock a client out of everything.
type BucketId = (Uuid, RateLimitKey);

/// Rate limiter with per-key token buckets.
///
/// Entries are forgotten once their bucket has refilled, which loses nothing:
/// a full bucket answers exactly as a new one would. That happens in
/// [`start_cleanup`](Self::start_cleanup), and on demand when the table
/// reaches its cap. [`crate::vendored::ProxyService`] starts the cleanup task
/// when it begins serving; anything else that holds a `RateLimiter` for long
/// should start it too.
pub struct RateLimiter {
    /// Per-key token buckets
    buckets: DashMap<BucketId, TokenBucket>,
    /// Default configuration
    defaults: RateLimitDefaults,
    /// Most entries held at once.
    max_keys: usize,
    /// When the table was last swept because it was full, so that a flood of
    /// new clients costs one sweep a second rather than one each.
    last_full_sweep: Mutex<Option<Instant>>,
}

/// The scope [`RateLimiter::check`] counts in: the global defaults, which
/// belong to no route.
const DEFAULT_SCOPE: Uuid = Uuid::nil();

impl RateLimiter {
    /// Create a new rate limiter.
    pub fn new(defaults: RateLimitDefaults) -> Self {
        Self {
            buckets: DashMap::new(),
            max_keys: defaults.max_tracked_keys.max(1),
            defaults,
            last_full_sweep: Mutex::new(None),
        }
    }

    /// Check if a request should be allowed, against the configured defaults.
    ///
    /// Returns `true` if the request is allowed, `false` if rate limited.
    pub fn check(&self, key: RateLimitKey) -> bool {
        let rate = per_second(self.defaults.requests, self.defaults.window_secs);
        self.check_with_config(DEFAULT_SCOPE, key, rate, self.defaults.burst)
    }

    /// Check against a limit of `rate_per_sec` tokens a second and a burst of
    /// `burst`, in the bucket for `key` within `scope`.
    ///
    /// `scope` keeps limits apart: pass the route's identity, so that one
    /// route's traffic is not counted against another's.
    pub fn check_with_config(
        &self,
        scope: Uuid,
        key: RateLimitKey,
        rate_per_sec: f64,
        burst: u32,
    ) -> bool {
        let id = (scope, key);
        if let Some(bucket) = self.buckets.get(&id) {
            return bucket.try_acquire();
        }

        if self.buckets.len() >= self.max_keys && !self.make_room() {
            // Refuse rather than let the request through untracked. Failing
            // open here would mean that filling the table with junk keys turns
            // the limit off for every client that arrives after.
            warn!(
                max_keys = self.max_keys,
                "Rate limiter is tracking as many clients as it may; refusing a new one"
            );
            return false;
        }

        self.buckets
            .entry(id)
            .or_insert_with(|| TokenBucket::new(u64::from(burst), rate_per_sec))
            .try_acquire()
    }

    /// Get remaining tokens for a key under the defaults (approximate).
    pub fn remaining(&self, key: &RateLimitKey) -> Option<u64> {
        self.buckets
            .get(&(DEFAULT_SCOPE, key.clone()))
            .map(|bucket| bucket.available())
    }

    /// How many buckets are held.
    pub fn tracked_keys(&self) -> usize {
        self.buckets.len()
    }

    /// Start background cleanup task.
    pub async fn start_cleanup(self: Arc<Self>, cancel_token: CancellationToken) {
        let cleanup_interval = Duration::from_secs(60);

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

    /// Sweep a full table, at most once a second. Whether there is room now.
    fn make_room(&self) -> bool {
        {
            let mut last = self.last_full_sweep.lock();
            let now = Instant::now();
            if last.is_some_and(|at| now.duration_since(at) < Duration::from_secs(1)) {
                return self.buckets.len() < self.max_keys;
            }
            *last = Some(now);
        }
        self.cleanup_expired();
        self.buckets.len() < self.max_keys
    }

    /// Forget every bucket that has refilled completely.
    fn cleanup_expired(&self) {
        let before = self.buckets.len();
        self.buckets.retain(|_, bucket| !bucket.is_full());
        let removed = before.saturating_sub(self.buckets.len());
        if removed > 0 {
            debug!("Rate limiter cleanup: removed {} refilled entries", removed);
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

    fn defaults(requests: u32, window_secs: u32, burst: u32) -> RateLimitDefaults {
        RateLimitDefaults {
            requests,
            window_secs,
            burst,
            ..RateLimitDefaults::default()
        }
    }

    #[test]
    fn test_rate_limiter_by_ip() {
        let limiter = RateLimiter::new(defaults(600, 60, 5));

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
        let limiter = RateLimiter::new(defaults(600, 60, 10));

        let key = RateLimitKey::Global;
        assert!(limiter.check(key.clone()));

        let remaining = limiter.remaining(&key).unwrap();
        assert!(remaining < 10);
    }

    #[test]
    fn the_rate_is_a_fraction() {
        assert_eq!(per_second(3, 60), 0.05);
        assert_eq!(per_second(600, 60), 10.0);
        // A zero window is read as one second rather than dividing by zero.
        assert_eq!(per_second(5, 0), 5.0);
    }

    #[test]
    fn scopes_do_not_share_a_bucket() {
        // The bug: one bucket per IP across every route, sized by whichever
        // route was hit first.
        let limiter = RateLimiter::default();
        let ip = RateLimitKey::client_ip("10.0.0.1".parse().unwrap());
        let strict = Uuid::new_v4();
        let generous = Uuid::new_v4();

        assert!(limiter.check_with_config(strict, ip.clone(), 0.01, 1));
        assert!(!limiter.check_with_config(strict, ip.clone(), 0.01, 1));

        // Spending the strict route's burst leaves the generous route alone,
        // and the generous route gets its own size of bucket.
        for _ in 0..100 {
            assert!(limiter.check_with_config(generous, ip.clone(), 0.01, 100));
        }
        assert!(!limiter.check_with_config(generous, ip.clone(), 0.01, 100));
    }

    #[test]
    fn an_ipv6_client_is_keyed_by_its_slash_64() {
        let a = RateLimitKey::client_ip("2001:db8:1:2:aaaa::1".parse().unwrap());
        let b = RateLimitKey::client_ip("2001:db8:1:2:ffff:1:2:3".parse().unwrap());
        let other = RateLimitKey::client_ip("2001:db8:1:3::1".parse().unwrap());
        assert_eq!(a, b);
        assert_ne!(a, other);
    }

    #[test]
    fn a_mapped_ipv4_client_is_keyed_as_ipv4() {
        // Not by the /64 of ::ffff:0:0/96, which every IPv4 client shares.
        let mapped = RateLimitKey::client_ip("::ffff:192.0.2.1".parse().unwrap());
        let plain = RateLimitKey::client_ip("192.0.2.1".parse().unwrap());
        let other = RateLimitKey::client_ip("::ffff:192.0.2.2".parse().unwrap());
        assert_eq!(mapped, plain);
        assert_ne!(mapped, other);
    }

    #[test]
    fn a_header_value_is_stored_as_a_digest() {
        let key = RateLimitKey::header_value(&[b'k'; 8192]);
        match key {
            RateLimitKey::Header(digest) => assert_eq!(digest.len(), 64),
            other => panic!("expected a header key, got {other:?}"),
        }
        assert_eq!(
            RateLimitKey::header_value(b"a"),
            RateLimitKey::header_value(b"a")
        );
        assert_ne!(
            RateLimitKey::header_value(b"a"),
            RateLimitKey::header_value(b"b")
        );
    }

    #[test]
    fn the_table_is_capped() {
        let limiter = RateLimiter::new(RateLimitDefaults {
            max_tracked_keys: 3,
            ..RateLimitDefaults::default()
        });
        let scope = Uuid::new_v4();
        let key = |n: u8| RateLimitKey::Ip(IpAddr::V4(Ipv4Addr::new(10, 0, 0, n)));

        // Three clients, each with an empty bucket that will not refill soon.
        for n in 0..3 {
            assert!(limiter.check_with_config(scope, key(n), 0.0001, 1));
        }
        assert_eq!(limiter.tracked_keys(), 3);

        // A fourth is refused rather than tracked or waved through, and the
        // table does not grow.
        assert!(!limiter.check_with_config(scope, key(3), 0.0001, 1));
        assert_eq!(limiter.tracked_keys(), 3);

        // Clients already tracked are still answered from their buckets.
        assert!(!limiter.check_with_config(scope, key(0), 0.0001, 1));
    }

    #[test]
    fn a_full_table_makes_room_by_dropping_refilled_buckets() {
        let limiter = RateLimiter::new(RateLimitDefaults {
            max_tracked_keys: 2,
            ..RateLimitDefaults::default()
        });
        let scope = Uuid::new_v4();
        let key = |n: u8| RateLimitKey::Ip(IpAddr::V4(Ipv4Addr::new(10, 0, 0, n)));

        // A burst of 2 with one spent leaves the bucket refilling; a huge
        // rate refills it before the next check.
        assert!(limiter.check_with_config(scope, key(0), 1e9, 2));
        assert!(limiter.check_with_config(scope, key(1), 1e9, 2));
        std::thread::sleep(Duration::from_millis(5));

        assert!(limiter.check_with_config(scope, key(2), 1e9, 2));
        assert!(limiter.tracked_keys() <= 2);
    }

    #[test]
    fn cleanup_forgets_only_refilled_buckets() {
        let limiter = RateLimiter::default();
        let scope = Uuid::new_v4();
        let spent = RateLimitKey::Ip(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)));
        let refilled = RateLimitKey::Ip(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2)));

        assert!(limiter.check_with_config(scope, spent.clone(), 0.0001, 1));
        assert!(limiter.check_with_config(scope, refilled, 1e9, 1));
        std::thread::sleep(Duration::from_millis(5));

        limiter.cleanup_expired();
        assert_eq!(limiter.tracked_keys(), 1);
        // The spent one is remembered: still refused.
        assert!(!limiter.check_with_config(scope, spent, 0.0001, 1));
    }
}
