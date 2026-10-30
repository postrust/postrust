//! Vendored backend management from rpxy-lib: backend/*.rs
//!
//! This module provides load balancing and upstream management.

use crate::config::{Backend, LoadBalanceStrategy, Upstream};
use crate::health::HealthChecker;
use crate::vendored::types::{PathName, ServerName};
use dashmap::DashMap;
use rand::Rng;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use uuid::Uuid;

/// Load balance context for sticky sessions and weighted selection.
#[derive(Clone, Debug)]
pub struct LoadBalanceContext {
    /// Client IP for sticky sessions
    pub client_ip: Option<std::net::IpAddr>,
    /// Cookie value for sticky sessions
    pub sticky_cookie: Option<String>,
}

/// Pointer to a selected upstream backend.
#[derive(Clone, Debug)]
pub struct PointerToUpstream {
    /// Index in the backends array
    pub ptr: usize,
    /// Context for sticky sessions
    pub context: Option<String>,
}

/// Load balancer trait.
///
/// A balancer chooses among the backends it is *offered*, not among all of
/// them. It used to pick an index over the whole list, which the caller then
/// reduced modulo the number of healthy backends -- so once any backend was
/// down, weights applied to the wrong backends, and a least-connections pick
/// named a backend that was not the one it had counted.
pub trait LoadBalanceWithPointer: Send + Sync {
    /// Choose one of `eligible`, which are indices into the upstream's
    /// backends. `None` only when `eligible` is empty.
    fn get_ptr(
        &self,
        ctx: Option<&LoadBalanceContext>,
        eligible: &[usize],
    ) -> Option<PointerToUpstream>;
}

fn pointer(ptr: usize) -> Option<PointerToUpstream> {
    Some(PointerToUpstream { ptr, context: None })
}

/// Round-robin load balancer (from rpxy load_balance_main.rs).
#[derive(Default)]
pub struct LoadBalanceRoundRobin {
    ptr: AtomicUsize,
}

impl LoadBalanceRoundRobin {
    /// A round-robin balancer.
    pub fn new() -> Self {
        Self::default()
    }
}

impl LoadBalanceWithPointer for LoadBalanceRoundRobin {
    fn get_ptr(
        &self,
        _ctx: Option<&LoadBalanceContext>,
        eligible: &[usize],
    ) -> Option<PointerToUpstream> {
        if eligible.is_empty() {
            return None;
        }
        // One atomic step, so two concurrent requests cannot both read the
        // same position and go to the same backend.
        let turn = self.ptr.fetch_add(1, Ordering::Relaxed);
        pointer(eligible[turn % eligible.len()])
    }
}

/// Random load balancer (from rpxy load_balance_main.rs).
#[derive(Default)]
pub struct LoadBalanceRandom;

impl LoadBalanceRandom {
    /// A uniformly random balancer.
    pub fn new() -> Self {
        Self
    }
}

impl LoadBalanceWithPointer for LoadBalanceRandom {
    fn get_ptr(
        &self,
        _ctx: Option<&LoadBalanceContext>,
        eligible: &[usize],
    ) -> Option<PointerToUpstream> {
        if eligible.is_empty() {
            return None;
        }
        pointer(eligible[rand::rng().random_range(0..eligible.len())])
    }
}

/// Least connections load balancer (our addition).
pub struct LoadBalanceLeastConn {
    connections: Arc<DashMap<usize, AtomicUsize>>,
    /// Where to start looking, so that ties -- which is every pick until
    /// something counts connections -- rotate rather than all landing on the
    /// first backend.
    start: AtomicUsize,
}

impl LoadBalanceLeastConn {
    /// A least-connections balancer over `num_upstreams` backends.
    pub fn new(num_upstreams: usize) -> Self {
        let connections = Arc::new(DashMap::new());
        for i in 0..num_upstreams {
            connections.insert(i, AtomicUsize::new(0));
        }
        Self {
            connections,
            start: AtomicUsize::new(0),
        }
    }

    /// Increment connection count for a backend.
    pub fn increment(&self, idx: usize) {
        if let Some(count) = self.connections.get(&idx) {
            count.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Decrement connection count for a backend.
    pub fn decrement(&self, idx: usize) {
        if let Some(count) = self.connections.get(&idx) {
            count.fetch_sub(1, Ordering::Relaxed);
        }
    }
}

impl LoadBalanceWithPointer for LoadBalanceLeastConn {
    fn get_ptr(
        &self,
        _ctx: Option<&LoadBalanceContext>,
        eligible: &[usize],
    ) -> Option<PointerToUpstream> {
        if eligible.is_empty() {
            return None;
        }
        let start = self.start.fetch_add(1, Ordering::Relaxed);
        let mut best: Option<(usize, usize)> = None;
        for offset in 0..eligible.len() {
            let idx = eligible[(start + offset) % eligible.len()];
            let conns = self
                .connections
                .get(&idx)
                .map(|c| c.load(Ordering::Relaxed))
                .unwrap_or(0);
            if best.is_none_or(|(_, fewest)| conns < fewest) {
                best = Some((idx, conns));
            }
        }
        best.and_then(|(idx, _)| pointer(idx))
    }
}

/// Weighted load balancer (our addition).
pub struct LoadBalanceWeighted {
    /// By backend index.
    weights: Vec<u32>,
}

impl LoadBalanceWeighted {
    /// A balancer that picks each backend in proportion to its weight, where
    /// `weights[i]` belongs to backend `i`.
    pub fn new(weights: Vec<u32>) -> Self {
        Self { weights }
    }
}

impl LoadBalanceWithPointer for LoadBalanceWeighted {
    fn get_ptr(
        &self,
        _ctx: Option<&LoadBalanceContext>,
        eligible: &[usize],
    ) -> Option<PointerToUpstream> {
        if eligible.is_empty() {
            return None;
        }
        let weight = |idx: usize| u64::from(self.weights.get(idx).copied().unwrap_or(0));

        // The total is over the eligible backends only: a backend that is
        // down must not keep its share and hand it to whichever neighbour the
        // arithmetic lands on.
        let total: u64 = eligible.iter().map(|&idx| weight(idx)).sum();

        // Every eligible weight is zero. `random_range(0..0)` panicked here;
        // with nothing to tell the backends apart, treat them as equal. A zero
        // weight alongside non-zero ones still means "no traffic", which is
        // how one drains a backend.
        if total == 0 {
            return pointer(eligible[rand::rng().random_range(0..eligible.len())]);
        }

        let mut point = rand::rng().random_range(0..total);
        for &idx in eligible {
            let w = weight(idx);
            if point < w {
                return pointer(idx);
            }
            point -= w;
        }
        // Unreachable while `point < total`; the last eligible is the honest
        // answer if it ever is.
        eligible.last().copied().and_then(pointer)
    }
}

/// Load balancer enum (adapted from rpxy).
pub enum LoadBalance {
    /// Each backend in turn.
    RoundRobin(LoadBalanceRoundRobin),
    /// Uniformly at random.
    Random(LoadBalanceRandom),
    /// Whichever backend currently has the fewest connections.
    LeastConnections(LoadBalanceLeastConn),
    /// In proportion to each backend's weight.
    Weighted(LoadBalanceWeighted),
}

impl LoadBalance {
    /// Build the balancer a strategy names, sized to `backends`.
    ///
    /// Sticky falls back to round-robin: cookie affinity is behind the
    /// `sticky-cookie` feature and there is nothing to be sticky on without it.
    pub fn from_strategy(strategy: &LoadBalanceStrategy, backends: &[Backend]) -> Self {
        let num = backends.len();
        match strategy {
            LoadBalanceStrategy::RoundRobin => {
                LoadBalance::RoundRobin(LoadBalanceRoundRobin::new())
            }
            LoadBalanceStrategy::Random => LoadBalance::Random(LoadBalanceRandom::new()),
            LoadBalanceStrategy::LeastConnections => {
                LoadBalance::LeastConnections(LoadBalanceLeastConn::new(num))
            }
            LoadBalanceStrategy::Weighted => {
                let weights: Vec<u32> = backends.iter().map(|b| b.weight).collect();
                LoadBalance::Weighted(LoadBalanceWeighted::new(weights))
            }
            LoadBalanceStrategy::Sticky => {
                // For now, fall back to round-robin. Sticky requires cookie handling.
                LoadBalance::RoundRobin(LoadBalanceRoundRobin::new())
            }
        }
    }

    /// Choose the next backend among `eligible`, as an index into the
    /// upstream's list. `None` only when `eligible` is empty.
    pub fn get_ptr(
        &self,
        ctx: Option<&LoadBalanceContext>,
        eligible: &[usize],
    ) -> Option<PointerToUpstream> {
        match self {
            LoadBalance::RoundRobin(lb) => lb.get_ptr(ctx, eligible),
            LoadBalance::Random(lb) => lb.get_ptr(ctx, eligible),
            LoadBalance::LeastConnections(lb) => lb.get_ptr(ctx, eligible),
            LoadBalance::Weighted(lb) => lb.get_ptr(ctx, eligible),
        }
    }
}

/// Backend application manager (adapted from rpxy backend_main.rs).
pub struct BackendAppManager {
    /// Upstreams by ID
    upstreams: DashMap<Uuid, UpstreamEntry>,
    /// Route matcher: (host, path_prefix) -> upstream_id
    routes: DashMap<(ServerName, PathName), Uuid>,
    /// Health checker reference
    health_checker: Option<Arc<HealthChecker>>,
}

struct UpstreamEntry {
    upstream: Upstream,
    load_balance: LoadBalance,
}

impl BackendAppManager {
    /// An empty manager, with no health checking.
    pub fn new() -> Self {
        Self {
            upstreams: DashMap::new(),
            routes: DashMap::new(),
            health_checker: None,
        }
    }

    /// Attach a health checker, so unhealthy backends are skipped.
    pub fn with_health_checker(mut self, checker: Arc<HealthChecker>) -> Self {
        self.health_checker = Some(checker);
        self
    }

    /// Register an upstream.
    pub fn register_upstream(&self, upstream: Upstream) {
        let id = upstream.resolved_id();
        let load_balance = LoadBalance::from_strategy(&upstream.lb_strategy, &upstream.backends);
        self.upstreams.insert(
            id,
            UpstreamEntry {
                upstream,
                load_balance,
            },
        );
    }

    /// Register a route.
    pub fn register_route(&self, host: ServerName, path: PathName, upstream_id: Uuid) {
        self.routes.insert((host, path), upstream_id);
    }

    /// Select a backend from an upstream, considering health status.
    ///
    /// `None` -- which the caller answers with 503 -- when the upstream is
    /// disabled or has no enabled backend. `enabled = false` is how an
    /// operator takes something out of service, and it used to be ignored
    /// here: a disabled backend kept its share of traffic, and was even the
    /// one chosen when every backend was unhealthy.
    pub fn select_backend(
        &self,
        upstream_id: Uuid,
        ctx: Option<&LoadBalanceContext>,
    ) -> Option<Backend> {
        let entry = self.upstreams.get(&upstream_id)?;
        let upstream = &entry.upstream;

        if !upstream.enabled {
            return None;
        }

        let enabled: Vec<usize> = upstream
            .backends
            .iter()
            .enumerate()
            .filter(|(_, b)| b.enabled)
            .map(|(idx, _)| idx)
            .collect();

        let healthy: Vec<usize> = enabled
            .iter()
            .copied()
            .filter(
                |&idx| match (&self.health_checker, upstream.backends[idx].id) {
                    (Some(checker), Some(id)) => checker.is_healthy(id),
                    // No health checker or no ID means assume healthy
                    _ => true,
                },
            )
            .collect();

        // When every enabled backend is unhealthy, keep balancing across all
        // of them rather than refusing. A health check can be wrong in a way
        // the traffic is not -- a misconfigured check path, or a dependency
        // the check exercises and most requests do not -- and when it is,
        // serving 503 for everything turns a monitoring mistake into an
        // outage. If the backends really are down, the request fails at the
        // backend with a 502, which is no worse. Disabled backends stay out
        // even here: that was a decision, not a measurement.
        let candidates = if healthy.is_empty() {
            &enabled
        } else {
            &healthy
        };

        let ptr = entry.load_balance.get_ptr(ctx, candidates)?;
        upstream.backends.get(ptr.ptr).cloned()
    }

    /// Get upstream by ID.
    pub fn get_upstream(&self, id: Uuid) -> Option<Upstream> {
        self.upstreams.get(&id).map(|e| e.upstream.clone())
    }
}

impl Default for BackendAppManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{HealthCheckConfig, UpstreamHttpVersion};

    #[test]
    fn test_round_robin() {
        let lb = LoadBalanceRoundRobin::new();
        let all = [0, 1, 2];

        assert_eq!(lb.get_ptr(None, &all).unwrap().ptr, 0);
        assert_eq!(lb.get_ptr(None, &all).unwrap().ptr, 1);
        assert_eq!(lb.get_ptr(None, &all).unwrap().ptr, 2);
        assert_eq!(lb.get_ptr(None, &all).unwrap().ptr, 0); // Wraps around
    }

    #[test]
    fn test_random() {
        let lb = LoadBalanceRandom::new();
        let eligible = [1, 4, 7];

        // Only ever one of the offered backends.
        for _ in 0..100 {
            let ptr = lb.get_ptr(None, &eligible).unwrap().ptr;
            assert!(eligible.contains(&ptr));
        }
    }

    #[test]
    fn test_weighted() {
        let lb = LoadBalanceWeighted::new(vec![1, 2, 7]);

        // Run many iterations and check distribution
        let mut counts = [0u32; 3];
        for _ in 0..1000 {
            let ptr = lb.get_ptr(None, &[0, 1, 2]).unwrap().ptr;
            counts[ptr] += 1;
        }

        // Backend 2 should get roughly 70% of traffic
        assert!(counts[2] > counts[0] && counts[2] > counts[1]);
    }

    #[test]
    fn all_zero_weights_are_equal_weights_not_a_panic() {
        // `random_range(0..0)` panicked on every request to such an upstream.
        let lb = LoadBalanceWeighted::new(vec![0, 0, 0]);
        let mut seen = [false; 3];
        for _ in 0..300 {
            seen[lb.get_ptr(None, &[0, 1, 2]).unwrap().ptr] = true;
        }
        assert_eq!(seen, [true, true, true]);
    }

    #[test]
    fn weights_are_over_the_eligible_backends() {
        // Backend 2 carries nearly all the weight but is down. The pick used
        // to be made over all three and then reduced modulo the healthy
        // count, which sent the heavy share to whichever survivor it landed
        // on. Now the survivors split by their own weights.
        let lb = LoadBalanceWeighted::new(vec![1, 3, 1000]);
        let mut counts = [0u32; 3];
        for _ in 0..4000 {
            counts[lb.get_ptr(None, &[0, 1]).unwrap().ptr] += 1;
        }
        assert_eq!(counts[2], 0);
        // 1:3, with a wide margin for randomness.
        assert!(counts[1] > counts[0] * 2, "{counts:?}");
    }

    #[test]
    fn a_zero_weight_beside_others_gets_nothing() {
        let lb = LoadBalanceWeighted::new(vec![0, 5]);
        for _ in 0..200 {
            assert_eq!(lb.get_ptr(None, &[0, 1]).unwrap().ptr, 1);
        }
    }

    #[test]
    fn no_eligible_backend_is_no_pick() {
        assert!(LoadBalanceRoundRobin::new().get_ptr(None, &[]).is_none());
        assert!(LoadBalanceRandom::new().get_ptr(None, &[]).is_none());
        assert!(LoadBalanceLeastConn::new(2).get_ptr(None, &[]).is_none());
        assert!(LoadBalanceWeighted::new(vec![1])
            .get_ptr(None, &[])
            .is_none());
    }

    #[test]
    fn least_connections_rotates_through_ties() {
        let lb = LoadBalanceLeastConn::new(3);
        let mut seen = [false; 3];
        for _ in 0..3 {
            seen[lb.get_ptr(None, &[0, 1, 2]).unwrap().ptr] = true;
        }
        assert_eq!(seen, [true, true, true]);

        lb.increment(0);
        lb.increment(1);
        for _ in 0..5 {
            assert_eq!(lb.get_ptr(None, &[0, 1, 2]).unwrap().ptr, 2);
        }
    }

    fn backend(address: &str, enabled: bool) -> Backend {
        Backend {
            id: Some(Uuid::new_v4()),
            address: address.into(),
            scheme: "http".into(),
            weight: 1,
            enabled,
            http_version: UpstreamHttpVersion::Http11,
        }
    }

    fn upstream(backends: Vec<Backend>, enabled: bool) -> Upstream {
        Upstream {
            id: Some(Uuid::new_v4()),
            name: "u".into(),
            description: None,
            lb_strategy: LoadBalanceStrategy::RoundRobin,
            backends,
            health_check: HealthCheckConfig::default(),
            enabled,
        }
    }

    #[test]
    fn a_disabled_backend_is_never_selected() {
        let manager = BackendAppManager::new();
        let up = upstream(vec![backend("off:1", false), backend("on:1", true)], true);
        let id = up.resolved_id();
        manager.register_upstream(up);
        for _ in 0..10 {
            assert_eq!(manager.select_backend(id, None).unwrap().address, "on:1");
        }
    }

    #[test]
    fn no_enabled_backend_is_no_backend() {
        let manager = BackendAppManager::new();
        let up = upstream(vec![backend("off:1", false), backend("off:2", false)], true);
        let id = up.resolved_id();
        manager.register_upstream(up);
        assert!(manager.select_backend(id, None).is_none());
    }

    #[test]
    fn a_disabled_upstream_has_no_backend() {
        let manager = BackendAppManager::new();
        let up = upstream(vec![backend("on:1", true)], false);
        let id = up.resolved_id();
        manager.register_upstream(up);
        assert!(manager.select_backend(id, None).is_none());
    }

    #[test]
    fn all_unhealthy_falls_back_to_enabled_backends_only() {
        let checker = Arc::new(HealthChecker::new());
        let disabled = backend("off:1", false);
        let a = backend("sick:1", true);
        let b = backend("sick:2", true);
        for b in [&disabled, &a, &b] {
            checker.mark_unhealthy_for_test(b.id.unwrap());
        }

        let manager = BackendAppManager::new().with_health_checker(checker);
        let up = upstream(vec![disabled, a, b], true);
        let id = up.resolved_id();
        manager.register_upstream(up);

        let mut seen = std::collections::HashSet::new();
        for _ in 0..10 {
            seen.insert(manager.select_backend(id, None).unwrap().address);
        }
        // Both enabled backends, balanced -- not `backends[0]`, which was the
        // disabled one.
        assert_eq!(
            seen,
            ["sick:1".to_string(), "sick:2".to_string()]
                .into_iter()
                .collect()
        );
    }

    #[test]
    fn a_healthy_backend_is_preferred() {
        let checker = Arc::new(HealthChecker::new());
        let sick = backend("sick:1", true);
        let well = backend("well:1", true);
        checker.mark_unhealthy_for_test(sick.id.unwrap());

        let manager = BackendAppManager::new().with_health_checker(checker);
        let up = upstream(vec![sick, well], true);
        let id = up.resolved_id();
        manager.register_upstream(up);
        for _ in 0..10 {
            assert_eq!(manager.select_backend(id, None).unwrap().address, "well:1");
        }
    }
}
