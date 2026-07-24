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
pub trait LoadBalanceWithPointer: Send + Sync {
    /// Get pointer to the next upstream backend.
    fn get_ptr(&self, ctx: Option<&LoadBalanceContext>) -> PointerToUpstream;
}

/// Round-robin load balancer (from rpxy load_balance_main.rs).
pub struct LoadBalanceRoundRobin {
    ptr: Arc<AtomicUsize>,
    num_upstreams: usize,
}

impl LoadBalanceRoundRobin {
    pub fn new(num_upstreams: usize) -> Self {
        Self {
            ptr: Arc::new(AtomicUsize::new(0)),
            num_upstreams,
        }
    }
}

impl LoadBalanceWithPointer for LoadBalanceRoundRobin {
    fn get_ptr(&self, _ctx: Option<&LoadBalanceContext>) -> PointerToUpstream {
        let current = self.ptr.load(Ordering::Relaxed);
        let next = (current + 1) % self.num_upstreams;
        self.ptr.store(next, Ordering::Relaxed);

        PointerToUpstream {
            ptr: current,
            context: None,
        }
    }
}

/// Random load balancer (from rpxy load_balance_main.rs).
pub struct LoadBalanceRandom {
    num_upstreams: usize,
}

impl LoadBalanceRandom {
    pub fn new(num_upstreams: usize) -> Self {
        Self { num_upstreams }
    }
}

impl LoadBalanceWithPointer for LoadBalanceRandom {
    fn get_ptr(&self, _ctx: Option<&LoadBalanceContext>) -> PointerToUpstream {
        let ptr = rand::rng().random_range(0..self.num_upstreams);
        PointerToUpstream { ptr, context: None }
    }
}

/// Least connections load balancer (our addition).
pub struct LoadBalanceLeastConn {
    connections: Arc<DashMap<usize, AtomicUsize>>,
    num_upstreams: usize,
}

impl LoadBalanceLeastConn {
    pub fn new(num_upstreams: usize) -> Self {
        let connections = Arc::new(DashMap::new());
        for i in 0..num_upstreams {
            connections.insert(i, AtomicUsize::new(0));
        }
        Self {
            connections,
            num_upstreams,
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
    fn get_ptr(&self, _ctx: Option<&LoadBalanceContext>) -> PointerToUpstream {
        let mut min_idx = 0;
        let mut min_conns = usize::MAX;

        for i in 0..self.num_upstreams {
            if let Some(count) = self.connections.get(&i) {
                let conns = count.load(Ordering::Relaxed);
                if conns < min_conns {
                    min_conns = conns;
                    min_idx = i;
                }
            }
        }

        PointerToUpstream {
            ptr: min_idx,
            context: None,
        }
    }
}

/// Weighted load balancer (our addition).
pub struct LoadBalanceWeighted {
    weights: Vec<u32>,
    total_weight: u32,
}

impl LoadBalanceWeighted {
    pub fn new(weights: Vec<u32>) -> Self {
        let total_weight = weights.iter().sum();
        Self {
            weights,
            total_weight,
        }
    }
}

impl LoadBalanceWithPointer for LoadBalanceWeighted {
    fn get_ptr(&self, _ctx: Option<&LoadBalanceContext>) -> PointerToUpstream {
        let mut rng = rand::rng();
        let random = rng.random_range(0..self.total_weight);

        let mut cumulative = 0;
        for (idx, weight) in self.weights.iter().enumerate() {
            cumulative += weight;
            if random < cumulative {
                return PointerToUpstream {
                    ptr: idx,
                    context: None,
                };
            }
        }

        // Fallback to last backend
        PointerToUpstream {
            ptr: self.weights.len() - 1,
            context: None,
        }
    }
}

/// Load balancer enum (adapted from rpxy).
pub enum LoadBalance {
    RoundRobin(LoadBalanceRoundRobin),
    Random(LoadBalanceRandom),
    LeastConnections(LoadBalanceLeastConn),
    Weighted(LoadBalanceWeighted),
}

impl LoadBalance {
    pub fn from_strategy(strategy: &LoadBalanceStrategy, backends: &[Backend]) -> Self {
        let num = backends.len();
        match strategy {
            LoadBalanceStrategy::RoundRobin => {
                LoadBalance::RoundRobin(LoadBalanceRoundRobin::new(num))
            }
            LoadBalanceStrategy::Random => LoadBalance::Random(LoadBalanceRandom::new(num)),
            LoadBalanceStrategy::LeastConnections => {
                LoadBalance::LeastConnections(LoadBalanceLeastConn::new(num))
            }
            LoadBalanceStrategy::Weighted => {
                let weights: Vec<u32> = backends.iter().map(|b| b.weight).collect();
                LoadBalance::Weighted(LoadBalanceWeighted::new(weights))
            }
            LoadBalanceStrategy::Sticky => {
                // For now, fall back to round-robin. Sticky requires cookie handling.
                LoadBalance::RoundRobin(LoadBalanceRoundRobin::new(num))
            }
        }
    }

    pub fn get_ptr(&self, ctx: Option<&LoadBalanceContext>) -> PointerToUpstream {
        match self {
            LoadBalance::RoundRobin(lb) => lb.get_ptr(ctx),
            LoadBalance::Random(lb) => lb.get_ptr(ctx),
            LoadBalance::LeastConnections(lb) => lb.get_ptr(ctx),
            LoadBalance::Weighted(lb) => lb.get_ptr(ctx),
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
    pub fn new() -> Self {
        Self {
            upstreams: DashMap::new(),
            routes: DashMap::new(),
            health_checker: None,
        }
    }

    pub fn with_health_checker(mut self, checker: Arc<HealthChecker>) -> Self {
        self.health_checker = Some(checker);
        self
    }

    /// Register an upstream.
    pub fn register_upstream(&self, upstream: Upstream) {
        if let Some(id) = upstream.id {
            let load_balance =
                LoadBalance::from_strategy(&upstream.lb_strategy, &upstream.backends);
            self.upstreams.insert(
                id,
                UpstreamEntry {
                    upstream,
                    load_balance,
                },
            );
        }
    }

    /// Register a route.
    pub fn register_route(&self, host: ServerName, path: PathName, upstream_id: Uuid) {
        self.routes.insert((host, path), upstream_id);
    }

    /// Find the best matching upstream for a request.
    pub fn find_upstream(&self, host: &str, path: &str) -> Option<Uuid> {
        let _host_name = ServerName::new(host);
        let _path_name = PathName::new(path);

        // Find all matching routes and select the one with longest path prefix
        let mut best_match: Option<(usize, Uuid)> = None;

        for entry in self.routes.iter() {
            let ((route_host, route_path), upstream_id) = entry.pair();

            if route_host.matches(host) && route_path.matches(path) {
                let path_len = route_path.len();
                if best_match.is_none() || path_len > best_match.unwrap().0 {
                    best_match = Some((path_len, *upstream_id));
                }
            }
        }

        best_match.map(|(_, id)| id)
    }

    /// Select a backend from an upstream, considering health status.
    pub fn select_backend(
        &self,
        upstream_id: Uuid,
        ctx: Option<&LoadBalanceContext>,
    ) -> Option<Backend> {
        let entry = self.upstreams.get(&upstream_id)?;
        let upstream = &entry.upstream;

        if upstream.backends.is_empty() {
            return None;
        }

        // Get healthy backends
        let healthy_backends: Vec<(usize, &Backend)> = upstream
            .backends
            .iter()
            .enumerate()
            .filter(|(_, b)| {
                if let (Some(checker), Some(id)) = (&self.health_checker, b.id) {
                    checker.is_healthy(id)
                } else {
                    true // No health checker or no ID means assume healthy
                }
            })
            .collect();

        if healthy_backends.is_empty() {
            // All backends unhealthy, fall back to first backend
            return Some(upstream.backends[0].clone());
        }

        // Use load balancer to select
        let ptr = entry.load_balance.get_ptr(ctx);
        let idx = ptr.ptr % healthy_backends.len();
        Some(healthy_backends[idx].1.clone())
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

    #[test]
    fn test_round_robin() {
        let lb = LoadBalanceRoundRobin::new(3);

        assert_eq!(lb.get_ptr(None).ptr, 0);
        assert_eq!(lb.get_ptr(None).ptr, 1);
        assert_eq!(lb.get_ptr(None).ptr, 2);
        assert_eq!(lb.get_ptr(None).ptr, 0); // Wraps around
    }

    #[test]
    fn test_random() {
        let lb = LoadBalanceRandom::new(10);

        // Just verify it returns valid indices
        for _ in 0..100 {
            let ptr = lb.get_ptr(None).ptr;
            assert!(ptr < 10);
        }
    }

    #[test]
    fn test_weighted() {
        let lb = LoadBalanceWeighted::new(vec![1, 2, 7]);

        // Run many iterations and check distribution
        let mut counts = [0u32; 3];
        for _ in 0..1000 {
            let ptr = lb.get_ptr(None).ptr;
            counts[ptr] += 1;
        }

        // Backend 2 should get roughly 70% of traffic
        assert!(counts[2] > counts[0] && counts[2] > counts[1]);
    }
}
