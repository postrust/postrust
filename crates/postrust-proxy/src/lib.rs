//! # Postrust Proxy
//!
//! High-performance reverse proxy module for Postrust with:
//! - HTTP/1.1 and HTTP/2 support
//! - Load balancing (round-robin, random, least-connections, weighted, sticky)
//! - Active health checking
//! - Rate limiting
//! - Automatic TLS via Let's Encrypt (ACME)
//! - Zero-downtime configuration updates
//!
//! ## Architecture
//!
//! The proxy is built on vendored code from [rust-rpxy](https://github.com/junkurihara/rust-rpxy),
//! with additional features for database-backed configuration, health checking, and rate limiting.
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                     Postrust Proxy                          │
//! ├─────────────────────────────────────────────────────────────┤
//! │  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────┐  │
//! │  │   Config    │  │   Health    │  │     Rate Limit      │  │
//! │  │ TOML + DB   │  │   Checker   │  │    Token Bucket     │  │
//! │  └──────┬──────┘  └──────┬──────┘  └──────────┬──────────┘  │
//! │         │                │                    │              │
//! │         └────────────────┼────────────────────┘              │
//! │                          │                                   │
//! │              ┌───────────▼───────────┐                       │
//! │              │    Vendored Core      │                       │
//! │              │  (from rust-rpxy)     │                       │
//! │              │  - Proxy handler      │                       │
//! │              │  - Load balancer      │                       │
//! │              │  - HTTP forwarding    │                       │
//! │              └───────────────────────┘                       │
//! └─────────────────────────────────────────────────────────────┘
//! ```

#![warn(clippy::all)]
// `postrust-proxy` is a beta module with wiring still in progress. The following
// lints are relaxed until it stabilizes (tighten before GA):
// - missing_docs / dead_code: some public items and scaffolding are not yet wired up.
// - result_large_err / large_enum_variant: rooted in the rich `ProxyError` enum.
// - type_complexity: a few internal signatures pending refactor into type aliases.
#![allow(missing_docs)]
#![allow(dead_code)]
#![allow(clippy::result_large_err)]
#![allow(clippy::large_enum_variant)]
#![allow(clippy::type_complexity)]

pub mod admin;
pub mod config;
pub mod health;
pub mod ratelimit;
pub mod saas;
pub mod tls;
pub mod vendored;

mod error;

pub use error::{ProxyError, ProxyResult};

// Re-export key types for convenience
pub use config::{Backend, ProxyConfig, Route, Upstream};
pub use health::HealthChecker;
pub use ratelimit::RateLimiter;

/// Proxy server state shared across handlers.
pub struct ProxyState {
    /// Database connection pool
    pub pool: sqlx::PgPool,
    /// Current proxy configuration
    pub config: std::sync::Arc<tokio::sync::RwLock<ProxyConfig>>,
    /// Health checker instance
    pub health_checker: std::sync::Arc<HealthChecker>,
    /// Rate limiter instance
    pub rate_limiter: std::sync::Arc<RateLimiter>,
    /// Configuration reloader
    pub config_reloader: std::sync::Arc<config::ConfigReloader>,
}

impl ProxyState {
    /// Create a new proxy state.
    pub async fn new(pool: sqlx::PgPool, config: ProxyConfig) -> ProxyResult<Self> {
        let rate_limit_defaults = config.rate_limit.clone();
        let config = std::sync::Arc::new(tokio::sync::RwLock::new(config));
        let health_checker = std::sync::Arc::new(HealthChecker::new(pool.clone()));
        let rate_limiter = std::sync::Arc::new(RateLimiter::new(rate_limit_defaults));
        let config_reloader = std::sync::Arc::new(config::ConfigReloader::new(config.clone()));

        Ok(Self {
            pool,
            config,
            health_checker,
            rate_limiter,
            config_reloader,
        })
    }
}
