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
    let h2_port = config.server.http2_port;
    let http_host = config.server.http_host.clone();
    let https_host = config.server.https_host.clone();
    let https_port = config.server.https_port;
    let cert_file = config.tls.cert_file.clone();
    let key_file = config.tls.key_file.clone();
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

    // An optional HTTP/2-only listener alongside the main one, which keeps
    // serving HTTP/1.1 and h2c together. See ServerConfig::http2_port.
    let h2_addr: Option<SocketAddr> = match h2_port {
        Some(port) => Some(
            format!("{http_host}:{port}")
                .parse()
                .map_err(|e| format!("bad http2 listen address in {config_path}: {e}"))?,
        ),
        None => None,
    };

    // HTTPS, if a certificate was configured. ALPN is what makes HTTP/2 reachable
    // as browsers actually use it; without this listener h2 is cleartext-only
    // and WebSocket is ws:// only.
    let tls = match (cert_file, key_file) {
        (Some(cert), Some(key)) => {
            let tls_config = postrust_proxy::tls::load_server_config(&cert, &key).await?;
            let https_addr: SocketAddr = format!("{https_host}:{https_port}")
                .parse()
                .map_err(|e| format!("bad https listen address in {config_path}: {e}"))?;
            let tls_service = service.clone();
            let tls_cancel = cancel.clone();
            Some(tokio::spawn(async move {
                tls_service.serve_https(https_addr, tls_config, tls_cancel).await
            }))
        }
        (None, None) => None,
        _ => return Err("tls.cert_file and tls.key_file must be set together".into()),
    };

    match h2_addr {
        Some(h2_addr) => {
            let h2_service = service.clone();
            let h2_cancel = cancel.clone();
            let h2 = tokio::spawn(async move { h2_service.serve_h2c(h2_addr, h2_cancel).await });
            let main = service.serve_http(addr, cancel).await;
            h2.await.map_err(|e| format!("http2 listener panicked: {e}"))??;
            main?;
        }
        None => service.serve_http(addr, cancel).await?,
    }

    if let Some(tls) = tls {
        tls.await.map_err(|e| format!("https listener panicked: {e}"))??;
    }
    Ok(())
}
