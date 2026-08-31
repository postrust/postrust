//! Runnable entry point for the proxy.
//!
//! The crate is a library with no binary, which means nothing external can be
//! pointed at it -- not a conformance suite, not a load generator, not a
//! browser. This binary exists so that `ProxyService` can be exercised as a
//! real listener.
//!
//! Configuration is a TOML file in the shape of [`ProxyConfig`]:
//!
//!     postrust-proxy path/to/config.toml
//!     POSTRUST_PROXY_CONFIG=path/to/config.toml postrust-proxy
//!
//! `DATABASE_URL` is optional. The health checker holds a `PgPool` but only
//! touches it from its background task, which this binary does not start, so a
//! lazily-connected pool to a database that does not exist is fine for
//! file-configured runs. `HealthChecker::is_healthy` treats an unknown backend
//! as healthy, so backends are used as configured.

use std::net::SocketAddr;
use std::sync::Arc;

use postrust_proxy::config::{load_from_file, ProxyConfig};
use postrust_proxy::health::HealthChecker;
use postrust_proxy::ratelimit::RateLimiter;
use postrust_proxy::vendored::ProxyService;
use sqlx::postgres::PgPoolOptions;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "postrust_proxy=info".into()),
        )
        .init();

    let config_path = std::env::args()
        .nth(1)
        .or_else(|| std::env::var("POSTRUST_PROXY_CONFIG").ok())
        .ok_or("usage: postrust-proxy <config.toml> (or set POSTRUST_PROXY_CONFIG)")?;

    let config: ProxyConfig = load_from_file(&config_path).await?;
    let addr: SocketAddr = format!("{}:{}", config.server.http_host, config.server.http_port)
        .parse()
        .map_err(|e| format!("bad listen address in {config_path}: {e}"))?;

    // Lazy: no connection is attempted here, and none is needed unless the
    // health checker's background task runs.
    let pool = PgPoolOptions::new().connect_lazy(
        &std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgres://postgres:postgres@127.0.0.1:5432/postgres".into()),
    )?;

    let rate_limiter = Arc::new(RateLimiter::new(config.rate_limit.clone()));
    let health_checker = Arc::new(HealthChecker::new(pool));
    let service = Arc::new(ProxyService::new(
        Arc::new(RwLock::new(config)),
        health_checker,
        rate_limiter,
    ));
    service.load_config().await;

    let cancel = CancellationToken::new();
    {
        let cancel = cancel.clone();
        tokio::spawn(async move {
            let _ = tokio::signal::ctrl_c().await;
            cancel.cancel();
        });
    }

    service.serve_http(addr, cancel).await?;
    Ok(())
}
