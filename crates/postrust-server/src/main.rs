//! Postrust HTTP Server.
//!
//! A PostgREST-compatible REST API server for PostgreSQL.

use anyhow::Result;
use axum::Router;
use tracing::{error, info};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

// musl's built-in allocator contends badly across threads, which shows up on
// exactly the paths that allocate per row. Building the Alpine image with it
// cost roughly 4-6x on the paged and embedded scenarios compared with the same
// code on glibc. glibc builds keep their own allocator, which performs fine.
#[cfg(target_env = "musl")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[tokio::main]
async fn main() -> Result<()> {
    // The log level is read here rather than from the loaded config, because
    // the subscriber has to exist before `from_env` runs -- it warns about
    // values it rejects, and a warning emitted before there is a subscriber
    // goes nowhere. `from_env` parses the same variable again for anything
    // else that wants it, and is the one that reports a bad value.
    //
    // `RUST_LOG` wins where both are set: it is the more specific instrument,
    // able to name targets and spans, and somebody who exported it is asking
    // for exactly what it says.
    let filter = std::env::var("RUST_LOG").unwrap_or_else(|_| {
        let level = std::env::var("PGRST_LOG_LEVEL")
            .ok()
            .and_then(|v| postrust_core::LogLevel::parse_config(&v))
            .unwrap_or_else(|| postrust_core::AppConfig::default().log_level);
        format!("postrust={}", level.to_tracing())
    });

    // `PGRST_LOG_FORMAT` is read here for the same reason as the level, and
    // a value `from_env` will warn about falls back to text meanwhile.
    let format = std::env::var("PGRST_LOG_FORMAT")
        .ok()
        .and_then(|v| postrust_core::LogFormat::parse_config(&v))
        .unwrap_or_default();

    init_subscriber(filter, format);

    let config = postrust_core::AppConfig::try_from_env()?;

    // Metrics, where they have somewhere to go: OTLP if it is configured,
    // and `/metrics` on the admin port if there is one. After the
    // configuration, which is what says whether there is an admin port.
    #[cfg(feature = "otel")]
    if postrust_server::otel::init_metrics(config.admin_server_port.is_some()) {
        info!("Metrics enabled");
    }

    info!("Starting Postrust server");

    let postrust_server::App {
        router: app, state, ..
    } = postrust_server::build(config.clone(), postrust_server::Host::SERVER).await?;

    // `admin_server_port`: liveness and readiness on a port of their own, so a
    // probe does not have to be able to reach the API.
    if let Some(admin_port) = config.admin_server_port {
        let admin_addr = format!("{}:{}", config.server_host, admin_port);
        let admin_listener = tokio::net::TcpListener::bind(&admin_addr).await?;
        info!("Admin server listening on http://{}", admin_addr);
        let admin_app = postrust_server::custom::admin_router().with_state(state.clone());
        tokio::spawn(async move {
            if let Err(e) = axum::serve(admin_listener, admin_app).await {
                error!("Admin server stopped: {}", e);
            }
        });
    }

    // Start server.
    //
    // Both listeners are wrapped so that a request target carrying a raw `>`
    // or `"` reaches the router instead of being refused by the URI parser.
    // See `lenient_uri`.
    match &config.server_unix_socket {
        Some(path) => serve_unix_socket(path, app).await?,
        None => {
            let addr = format!("{}:{}", config.server_host, config.server_port);
            let listener = tokio::net::TcpListener::bind(&addr).await?;
            info!("Listening on http://{}", addr);
            axum::serve(postrust_server::lenient_uri::LenientListener(listener), app).await?;
        }
    }

    Ok(())
}

/// Install the global subscriber: the log, and with the `otel` feature and an
/// OTLP endpoint, the exporter beside it.
///
/// Two shapes, chosen once. With nothing to export, the log's filter is the
/// subscriber's global filter, exactly as without the feature: a span or
/// event below the log level is turned away at its call site, before
/// anything is built. With an exporter the filter moves onto the log's own
/// layer, and the exporter has one of its own -- the only way the request's
/// debug-level spans can be exported while the log stays at its level.
fn init_subscriber(filter: String, format: postrust_core::LogFormat) {
    #[cfg(feature = "otel")]
    if let Some(otel) = postrust_server::otel::layer() {
        use tracing_subscriber::Layer as _;
        // The SDK reports its own failures -- an export the collector
        // refused, a batch dropped -- as events of its own target. Let the
        // warnings through to the log, unless `RUST_LOG` says otherwise.
        let filter = match std::env::var_os("RUST_LOG") {
            Some(_) => filter,
            None => format!("{filter},opentelemetry=warn,opentelemetry_sdk=warn"),
        };
        let (text, json) = log_layers(format);
        tracing_subscriber::registry()
            .with(
                text.with_filter(tracing_subscriber::EnvFilter::new(&filter))
                    .and_then(json.with_filter(tracing_subscriber::EnvFilter::new(&filter))),
            )
            .with(otel)
            .init();
        postrust_server::otel::log_startup();
        return;
    }

    // `Layer::and_then` spelled out: `Option` has an `and_then` of its own.
    let (text, json) = log_layers(format);
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(filter))
        .with(tracing_subscriber::Layer::and_then(text, json))
        .init();
    // An exporter that was asked for and could not be built says why.
    #[cfg(feature = "otel")]
    postrust_server::otel::log_startup();
}

/// The log's layer for `format`, as text or as JSON lines; the other is
/// `None`.
///
/// Both are written out and one of them is `None`, rather than one boxed
/// layer chosen at run time: an `Option` of a layer is a layer, and this way
/// neither is behind a pointer. JSON lines carry the current span and the
/// spans above it, each with its fields -- which is how every line logged
/// while serving a request names its `request_id` and `trace_id`.
#[allow(clippy::type_complexity)]
fn log_layers<S>(
    format: postrust_core::LogFormat,
) -> (
    Option<tracing_subscriber::fmt::Layer<S>>,
    Option<
        tracing_subscriber::fmt::Layer<
            S,
            tracing_subscriber::fmt::format::JsonFields,
            tracing_subscriber::fmt::format::Format<tracing_subscriber::fmt::format::Json>,
        >,
    >,
)
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
{
    match format {
        postrust_core::LogFormat::Text => (Some(tracing_subscriber::fmt::layer()), None),
        postrust_core::LogFormat::Json => (None, Some(tracing_subscriber::fmt::layer().json())),
    }
}

/// Serve on a Unix domain socket.
///
/// Unix only. `tokio::net::UnixListener` does not exist elsewhere, so the
/// alternative to this split is a binary that does not compile off Unix at
/// all -- and CI only builds Linux, so nothing would have caught that.
#[cfg(unix)]
async fn serve_unix_socket(path: &str, app: Router) -> Result<()> {
    remove_stale_socket(path).await?;
    let listener = tokio::net::UnixListener::bind(path)
        .map_err(|e| anyhow::anyhow!("could not bind unix socket {path}: {e}"))?;
    info!("Listening on unix:{}", path);
    axum::serve(
        postrust_server::lenient_uri::LenientUnixListener(listener),
        app,
    )
    .await?;
    Ok(())
}

/// Refuse the option rather than ignore it: silently falling back to the TCP
/// port would bind an address the operator did not ask for.
#[cfg(not(unix))]
async fn serve_unix_socket(path: &str, _app: Router) -> Result<()> {
    Err(anyhow::anyhow!(
        "server_unix_socket ({path}) is not supported on this platform"
    ))
}

/// Remove a leftover socket file so the bind does not fail with `EADDRINUSE`
/// against a server that is not running.
///
/// Only a socket is removed. If the path holds a regular file or a directory,
/// that is a configuration mistake and deleting it would be the wrong way to
/// find out.
#[cfg(unix)]
async fn remove_stale_socket(path: &str) -> Result<()> {
    use std::os::unix::fs::FileTypeExt;

    let metadata = match tokio::fs::symlink_metadata(path).await {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(anyhow::anyhow!("could not stat {path}: {e}")),
    };

    if !metadata.file_type().is_socket() {
        return Err(anyhow::anyhow!(
            "{path} exists and is not a socket; refusing to remove it"
        ));
    }

    tokio::fs::remove_file(path)
        .await
        .map_err(|e| anyhow::anyhow!("could not remove stale socket {path}: {e}"))?;
    info!("Removed stale socket at {}", path);
    Ok(())
}
