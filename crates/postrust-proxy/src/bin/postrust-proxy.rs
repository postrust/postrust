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
//!
//! When `DATABASE_URL` *is* set and `server.database_config` is left at its
//! default of true, the routes and upstreams in the database are loaded and
//! added to whatever the file declared. The trigger is the variable rather
//! than the flag alone: `database_config` defaults to true, so keying off it
//! by itself would make every file-configured proxy -- the conformance
//! harnesses included -- try to reach a database that is not there.

use std::net::SocketAddr;
use std::sync::Arc;

use postrust_proxy::config::{load_from_database, load_from_file, ProxyConfig};
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
    // health checker's background task runs or the database holds config.
    let database_url = std::env::var("DATABASE_URL").ok();
    let pool = PgPoolOptions::new().connect_lazy(
        database_url
            .as_deref()
            .unwrap_or("postgres://postgres:postgres@127.0.0.1:5432/postgres"),
    )?;

    let cancel = CancellationToken::new();
    #[cfg(feature = "acme")]
    let cancel_for_acme = cancel.clone();

    let mut config = config;
    if config.server.database_config && database_url.is_some() {
        let (routes, upstreams) = load_from_database(&pool).await?;
        tracing::info!(
            routes = routes.len(),
            upstreams = upstreams.len(),
            "loaded configuration from the database"
        );
        merge_from_database(&mut config, routes, upstreams)?;
    }
    let config = config;

    // Whether to serve HTTPS at all, decided before anything is built for it.
    //
    // Not "a database is configured": that would open 0.0.0.0:8443 on every
    // proxy that merely keeps its routes in PostgreSQL, and serve nothing on
    // it, because there would be no certificate to answer a handshake with.
    let https = wants_https(&config);

    // The certificate store, shared by whatever issues certificates and by the
    // listener that serves them. Created here rather than inside the ACME
    // worker because the two need the same one -- when they had one each, the
    // worker wrote certificates the listener could not see. It does not depend
    // on the `acme` feature: `POST /domains/{id}/ssl/upload` fills the same
    // store by hand.
    //
    // Built only when something will use it. `CertificateStore::new` creates
    // `tls.cert_dir` (`./certs` by default), which fails on a read-only root
    // filesystem -- so building it unconditionally would stop a proxy that
    // uses no TLS from starting at all.
    let cert_store = match (&database_url, https.needs_store()) {
        (Some(_), true) => Some(Arc::new(
            postrust_proxy::tls::CertificateStore::new(pool.clone(), &config.tls.cert_dir).await?,
        )),
        _ => None,
    };

    // The ACME issuance worker, if the config asks for it.
    //
    // Needs a database: the account, the pending challenges and the
    // certificates all live there, and the challenge the CA fetches is served
    // out of a table so that any instance can answer it. Without DATABASE_URL
    // there is nowhere to keep any of that, so the worker does not start and
    // says so rather than failing later on the first order.
    #[cfg(feature = "acme")]
    let acme_worker = if config.tls.acme_enabled {
        match &cert_store {
            Some(store) => Some(
                start_acme_worker(
                    &config,
                    pool.clone(),
                    store.clone(),
                    cancel_for_acme.clone(),
                )
                .await?,
            ),
            None => {
                tracing::warn!(
                    "tls.acme_enabled is set but DATABASE_URL is not;                      the ACME worker needs a database for the account,                      challenges and certificates, so it will not start"
                );
                None
            }
        }
    } else {
        None
    };

    let rate_limiter = Arc::new(RateLimiter::new(config.rate_limit.clone()));
    let health_checker = Arc::new(HealthChecker::new());
    let service = Arc::new(ProxyService::new(
        Arc::new(RwLock::new(config)),
        health_checker,
        rate_limiter,
    ));
    service.load_config().await;

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

    // The statically configured pair, if there is one. It becomes the fallback
    // for a handshake whose SNI names no stored certificate, and for a client
    // that sends no SNI at all.
    let static_key = match (&cert_file, &key_file) {
        (Some(cert), Some(key)) => {
            let cert_pem = tokio::fs::read(cert)
                .await
                .map_err(|e| format!("could not read {cert}: {e}"))?;
            let key_pem = tokio::fs::read(key)
                .await
                .map_err(|e| format!("could not read {key}: {e}"))?;
            Some(postrust_proxy::tls::SniCertResolver::certified_key(
                &cert_pem, &key_pem,
            )?)
        }
        (None, None) => None,
        _ => return Err("tls.cert_file and tls.key_file must be set together".into()),
    };

    // HTTPS. ALPN is what makes HTTP/2 reachable as browsers actually use it;
    // without this listener h2 is cleartext-only and WebSocket is ws:// only.
    let tls = if https.enabled() {
        let resolver = Arc::new(postrust_proxy::tls::SniCertResolver::new(static_key));

        if let Some(store) = &cert_store {
            match resolver.refresh(store).await {
                Ok(0) => {
                    tracing::info!("No stored certificates yet; serving the configured pair only")
                }
                Ok(_) => {}
                Err(e) => tracing::warn!("Could not load stored certificates: {e}"),
            }
            spawn_certificate_refresh(resolver.clone(), store.clone(), cancel.clone());
        }

        let tls_config = postrust_proxy::tls::build_server_config_with_resolver(resolver);
        let https_addr: SocketAddr = format!("{https_host}:{https_port}")
            .parse()
            .map_err(|e| format!("bad https listen address in {config_path}: {e}"))?;
        let tls_service = service.clone();
        let tls_cancel = cancel.clone();
        Some(tokio::spawn(async move {
            tls_service
                .serve_https(https_addr, tls_config, tls_cancel)
                .await
        }))
    } else {
        None
    };

    match h2_addr {
        Some(h2_addr) => {
            let h2_service = service.clone();
            let h2_cancel = cancel.clone();
            let h2 = tokio::spawn(async move { h2_service.serve_h2c(h2_addr, h2_cancel).await });
            let main = service.serve_http(addr, cancel).await;
            h2.await
                .map_err(|e| format!("http2 listener panicked: {e}"))??;
            main?;
        }
        None => service.serve_http(addr, cancel).await?,
    }

    if let Some(tls) = tls {
        tls.await
            .map_err(|e| format!("https listener panicked: {e}"))??;
    }

    #[cfg(feature = "acme")]
    if let Some(worker) = acme_worker {
        worker
            .await
            .map_err(|e| format!("ACME worker panicked: {e}"))?;
    }

    Ok(())
}

/// Start the ACME issuance worker.
///
/// Split out to keep `main` readable, and because everything in it is behind
/// the `acme` feature.
#[cfg(feature = "acme")]
async fn start_acme_worker(
    config: &ProxyConfig,
    pool: sqlx::PgPool,
    cert_store: Arc<postrust_proxy::tls::CertificateStore>,
    cancel: CancellationToken,
) -> Result<tokio::task::JoinHandle<()>, Box<dyn std::error::Error>> {
    use postrust_proxy::tls::{AcmeIssuer, LETS_ENCRYPT_PRODUCTION, LETS_ENCRYPT_STAGING};

    let directory = config.tls.acme_directory_url.clone().unwrap_or_else(|| {
        if config.tls.acme_staging {
            LETS_ENCRYPT_STAGING.to_string()
        } else {
            LETS_ENCRYPT_PRODUCTION.to_string()
        }
    });

    let mut issuer = AcmeIssuer::new(
        directory,
        config.tls.acme_email.clone(),
        pool.clone(),
        cert_store,
    );
    if let Some(root) = &config.tls.acme_root_pem {
        issuer = issuer.with_root_certificate(root);
    }

    Ok(tokio::spawn(postrust_proxy::saas::ssl::run(
        pool,
        Arc::new(issuer),
        cancel,
    )))
}

/// What, if anything, the HTTPS listener is being asked to serve.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Https {
    /// `tls.cert_file` and `tls.key_file` are set.
    static_pair: bool,
    /// Certificates are expected to arrive in the database: ACME issuance, or
    /// an operator saying so with `server.https_enabled`.
    stored: bool,
}

impl Https {
    fn enabled(self) -> bool {
        self.static_pair || self.stored
    }

    /// Whether to build a `CertificateStore`, which creates `tls.cert_dir`.
    fn needs_store(self) -> bool {
        self.stored
    }
}

/// Decide whether to serve HTTPS, from what the configuration actually asks
/// for rather than from whether a database happens to be reachable.
///
/// `server.https_enabled` is how a deployment says "serve tenant certificates
/// from the store" without also configuring a static pair. It used to be
/// declared and unread; this is what reads it.
fn wants_https(config: &ProxyConfig) -> Https {
    Https {
        static_pair: config.tls.cert_file.is_some() && config.tls.key_file.is_some(),
        stored: config.tls.acme_enabled || config.server.https_enabled,
    }
}

/// Re-read the certificate store periodically, so a certificate issued or
/// uploaded while the proxy is running starts being served.
///
/// Polling rather than LISTEN/NOTIFY: issuance is rare and a minute of delay
/// on a new tenant domain costs nothing, where a listening connection is one
/// more thing to reconnect and get wrong. `POST /domains/{id}/ssl/provision`
/// takes several round trips to the CA anyway, so this is not the slow part.
fn spawn_certificate_refresh(
    resolver: Arc<postrust_proxy::tls::SniCertResolver>,
    store: Arc<postrust_proxy::tls::CertificateStore>,
    cancel: CancellationToken,
) {
    const EVERY: std::time::Duration = std::time::Duration::from_secs(60);

    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = cancel.cancelled() => return,
                _ = tokio::time::sleep(EVERY) => {}
            }
            if let Err(e) = resolver.refresh(&store).await {
                tracing::warn!("Certificate refresh failed: {e}");
            }
        }
    });
}

/// Add database-held routes and upstreams to a file-bootstrapped config.
///
/// Names have to stay unique across both sources. `Upstream::resolved_id`
/// derives a UUID from the name when there is no id, and the routing tables are
/// keyed by it -- so two upstreams sharing a name would collide there, and one
/// would silently take the other's traffic. Refusing to start is the right
/// answer to a configuration that cannot be represented.
fn merge_from_database(
    config: &mut ProxyConfig,
    routes: Vec<postrust_proxy::config::Route>,
    upstreams: Vec<postrust_proxy::config::Upstream>,
) -> Result<(), String> {
    for upstream in &upstreams {
        if config.upstreams.iter().any(|u| u.name == upstream.name) {
            return Err(format!(
                "upstream {:?} is declared in both the config file and the database",
                upstream.name
            ));
        }
    }
    for route in &routes {
        if config.routes.iter().any(|r| r.name == route.name) {
            return Err(format!(
                "route {:?} is declared in both the config file and the database",
                route.name
            ));
        }
    }
    config.upstreams.extend(upstreams);
    config.routes.extend(routes);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> ProxyConfig {
        ProxyConfig::default()
    }

    #[test]
    fn a_plain_proxy_does_not_serve_https() {
        // The regression this guards: HTTPS was gated on whether a certificate
        // *store* could be built, and the store was built whenever
        // DATABASE_URL was set. A proxy that merely keeps its routes in
        // PostgreSQL then bound 0.0.0.0:8443 and failed every handshake on it,
        // having no certificate to offer.
        let https = wants_https(&config());
        assert!(!https.enabled());
        assert!(!https.needs_store());
    }

    #[test]
    fn a_static_pair_serves_https_without_a_store() {
        let mut config = config();
        config.tls.cert_file = Some("/etc/postrust/fullchain.pem".into());
        config.tls.key_file = Some("/etc/postrust/privkey.pem".into());

        let https = wants_https(&config);
        assert!(https.enabled());
        // No store, so `tls.cert_dir` is not created -- which is what lets
        // this run on a read-only root filesystem.
        assert!(!https.needs_store());
    }

    #[test]
    fn half_a_pair_is_not_a_pair() {
        let mut config = config();
        config.tls.cert_file = Some("/etc/postrust/fullchain.pem".into());
        // main() rejects this combination outright; wants_https must not
        // report it as usable in the meantime.
        assert!(!wants_https(&config).static_pair);
    }

    #[test]
    fn acme_serves_https_from_the_store_alone() {
        let mut config = config();
        config.tls.acme_enabled = true;

        let https = wants_https(&config);
        assert!(https.enabled());
        assert!(https.needs_store());
        // This is the multi-tenant case: no static pair configured at all.
        assert!(!https.static_pair);
    }

    #[test]
    fn https_enabled_is_read() {
        // It was declared and ignored, and documented as doing nothing. It is
        // how a deployment serves uploaded certificates without ACME.
        let mut config = config();
        config.server.https_enabled = true;

        let https = wants_https(&config);
        assert!(https.enabled());
        assert!(https.needs_store());
    }
}
