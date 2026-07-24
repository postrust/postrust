//! Background health checker for backend servers.

use crate::config::HealthCheckConfig;
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use sqlx::PgPool;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};
use uuid::Uuid;

/// Health status for a backend.
#[derive(Clone, Debug, serde::Serialize)]
pub struct BackendHealth {
    /// Whether the backend is healthy
    pub is_healthy: bool,
    /// Consecutive successful health checks
    pub consecutive_successes: u32,
    /// Consecutive failed health checks
    pub consecutive_failures: u32,
    /// Last health check time
    pub last_check: DateTime<Utc>,
    /// Last response time in milliseconds
    pub response_time_ms: Option<u64>,
    /// Last error message
    pub last_error: Option<String>,
}

impl Default for BackendHealth {
    fn default() -> Self {
        Self {
            is_healthy: true,
            consecutive_successes: 0,
            consecutive_failures: 0,
            last_check: Utc::now(),
            response_time_ms: None,
            last_error: None,
        }
    }
}

/// Backend info for health checking.
#[derive(Clone, Debug)]
pub struct BackendInfo {
    /// Backend ID
    pub id: Uuid,
    /// Backend address
    pub address: String,
    /// Backend scheme (http/https)
    pub scheme: String,
    /// Health check path
    pub health_path: String,
}

/// Background health checker.
pub struct HealthChecker {
    /// Database pool for persisting health status
    pool: PgPool,
    /// Backend health status
    health: DashMap<Uuid, BackendHealth>,
    /// Backend info
    backends: DashMap<Uuid, BackendInfo>,
    /// HTTP client
    client: reqwest::Client,
}

impl HealthChecker {
    /// Create a new health checker.
    pub fn new(pool: PgPool) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("Failed to build HTTP client");

        Self {
            pool,
            health: DashMap::new(),
            backends: DashMap::new(),
            client,
        }
    }

    /// Check if a backend is healthy.
    pub fn is_healthy(&self, backend_id: Uuid) -> bool {
        self.health
            .get(&backend_id)
            .map(|h| h.is_healthy)
            .unwrap_or(true)
    }

    /// Get health status for a backend.
    pub fn get_health(&self, backend_id: Uuid) -> Option<BackendHealth> {
        self.health.get(&backend_id).map(|h| h.clone())
    }

    /// Register a backend for health checking.
    pub fn register_backend(&self, info: BackendInfo) {
        let id = info.id;
        self.backends.insert(id, info);
        self.health.insert(id, BackendHealth::default());
    }

    /// Unregister a backend.
    pub fn unregister_backend(&self, backend_id: Uuid) {
        self.backends.remove(&backend_id);
        self.health.remove(&backend_id);
    }

    /// Start the health checker background task.
    pub async fn start(
        self: Arc<Self>,
        config: HealthCheckConfig,
        cancel_token: CancellationToken,
    ) {
        if !config.enabled {
            info!("Health checking disabled");
            return;
        }

        let interval = Duration::from_secs(config.interval_secs as u64);
        let timeout = Duration::from_secs(config.timeout_secs as u64);
        let healthy_threshold = config.healthy_threshold;
        let unhealthy_threshold = config.unhealthy_threshold;

        info!(
            "Health checker started with {}s interval, {}s timeout",
            config.interval_secs, config.timeout_secs
        );

        loop {
            tokio::select! {
                _ = cancel_token.cancelled() => {
                    info!("Health checker stopped");
                    break;
                }
                _ = tokio::time::sleep(interval) => {
                    self.check_all_backends(timeout, healthy_threshold, unhealthy_threshold).await;
                }
            }
        }
    }

    async fn check_all_backends(
        &self,
        timeout: Duration,
        healthy_threshold: u32,
        unhealthy_threshold: u32,
    ) {
        for entry in self.backends.iter() {
            let backend = entry.value();
            let health_url = format!(
                "{}://{}{}",
                backend.scheme, backend.address, backend.health_path
            );

            let start = Instant::now();
            let result = self.client.get(&health_url).timeout(timeout).send().await;
            let response_time = start.elapsed().as_millis() as u64;

            self.update_health(
                backend.id,
                result,
                response_time,
                healthy_threshold,
                unhealthy_threshold,
            );
        }
    }

    fn update_health(
        &self,
        backend_id: Uuid,
        result: Result<reqwest::Response, reqwest::Error>,
        response_time: u64,
        healthy_threshold: u32,
        unhealthy_threshold: u32,
    ) {
        let mut health = self.health.entry(backend_id).or_default();

        health.last_check = Utc::now();
        health.response_time_ms = Some(response_time);

        match result {
            Ok(response) if response.status().is_success() => {
                health.consecutive_successes += 1;
                health.consecutive_failures = 0;
                health.last_error = None;

                if health.consecutive_successes >= healthy_threshold && !health.is_healthy {
                    info!("Backend {} is now healthy", backend_id);
                    health.is_healthy = true;
                }
            }
            Ok(response) => {
                let error = format!("HTTP {}", response.status());
                warn!("Health check failed for {}: {}", backend_id, error);

                health.consecutive_failures += 1;
                health.consecutive_successes = 0;
                health.last_error = Some(error);

                if health.consecutive_failures >= unhealthy_threshold && health.is_healthy {
                    warn!("Backend {} is now unhealthy", backend_id);
                    health.is_healthy = false;
                }
            }
            Err(e) => {
                warn!("Health check failed for {}: {}", backend_id, e);

                health.consecutive_failures += 1;
                health.consecutive_successes = 0;
                health.last_error = Some(e.to_string());

                if health.consecutive_failures >= unhealthy_threshold && health.is_healthy {
                    warn!("Backend {} is now unhealthy", backend_id);
                    health.is_healthy = false;
                }
            }
        }
    }
}
