//! Postrust Lambda library.
//!
//! What the `postrust-lambda` binary is made of, for a Lambda binary of your
//! own: one that serves routes of yours beside the API, added with
//! [`postrust_server::Builder`].
//!
//! ```no_run
//! use axum::{routing::post, Router};
//! use postrust_lambda::postrust_server::{AppState, Builder, Host};
//! use std::sync::Arc;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), postrust_lambda::Error> {
//!     postrust_lambda::init_tracing();
//!     let hooks: Router<Arc<AppState>> = Router::new().route("/stripe", post(|| async { "ok" }));
//!     let app = Builder::new(postrust_lambda::config_from_env()?, Host::LAMBDA)
//!         .nest("/hooks", hooks)
//!         .build()
//!         .await?;
//!     postrust_lambda::run(app).await
//! }
//! ```
//!
//! Build the application before calling [`run`], in the init phase rather
//! than on the first request: the connection and the schema cache are paid
//! for once per sandbox, and a database that cannot be reached fails the init
//! -- which Lambda reports and retries -- instead of failing every invocation.

#![warn(missing_docs)]

pub use postrust_auth;
pub use postrust_core;
pub use postrust_response;
pub use postrust_server;
pub use postrust_sql;

/// The error a Lambda handler returns, as the runtime takes it.
pub use lambda_http::Error;

/// Log as the server does: `RUST_LOG` if set, otherwise `PGRST_LOG_LEVEL`,
/// as text or as JSON lines by `PGRST_LOG_FORMAT`.
///
/// No timestamps, which CloudWatch adds to every line itself. Installs the
/// global subscriber, so call it once, first.
pub fn init_tracing() {
    let filter = std::env::var("RUST_LOG").unwrap_or_else(|_| {
        let level = std::env::var("PGRST_LOG_LEVEL")
            .ok()
            .and_then(|v| postrust_core::LogLevel::parse_config(&v))
            .unwrap_or_else(|| postrust_core::AppConfig::default().log_level);
        format!("postrust={}", level.to_tracing())
    });
    // `PGRST_LOG_FORMAT=json` as on the server, which CloudWatch Logs
    // Insights can query by field. The request span and its `request_id` and
    // `trace_id` come with the router, which is the server's.
    let format = std::env::var("PGRST_LOG_FORMAT")
        .ok()
        .and_then(|v| postrust_core::LogFormat::parse_config(&v))
        .unwrap_or_default();

    // With the `otel` feature and an OTLP endpoint -- the ADOT collector
    // extension, usually -- spans are exported beside the log. The log's
    // filter then moves onto its own layer, as on the server, so the
    // request's debug-level spans can be exported while the log stays at its
    // level. Without one the subscriber is exactly what it always was.
    #[cfg(feature = "otel")]
    if let Some(otel) = postrust_server::otel::layer() {
        use tracing_subscriber::layer::SubscriberExt as _;
        use tracing_subscriber::util::SubscriberInitExt as _;
        use tracing_subscriber::Layer as _;
        let filter = match std::env::var_os("RUST_LOG") {
            Some(_) => filter,
            None => format!("{filter},opentelemetry=warn,opentelemetry_sdk=warn"),
        };
        let log = tracing_subscriber::fmt::layer()
            .with_target(false)
            .without_time();
        let (text, json) = match format {
            postrust_core::LogFormat::Text => (Some(log), None),
            postrust_core::LogFormat::Json => (None, Some(log.json())),
        };
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

    let builder = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new(filter))
        .with_target(false)
        .without_time();
    match format {
        postrust_core::LogFormat::Text => builder.init(),
        postrust_core::LogFormat::Json => builder.json().init(),
    }
    #[cfg(feature = "otel")]
    postrust_server::otel::log_startup();
}

/// The configuration from the environment, sized for a Lambda sandbox.
///
/// A sandbox answers one invocation at a time, so the server's default pool
/// holds connections nothing will use -- and every concurrent sandbox holds
/// its own, which is how a burst of Lambdas exhausts `max_connections`. The
/// pool is two connections unless `PGRST_DB_POOL` or `PGRST_DB_POOL_SIZE`
/// says otherwise; two rather than one because a GraphQL mutation holds a
/// transaction while its resolvers read.
pub fn config_from_env() -> Result<postrust_core::AppConfig, Error> {
    let mut config = postrust_core::AppConfig::try_from_env()?;
    if ["PGRST_DB_POOL", "PGRST_DB_POOL_SIZE"]
        .iter()
        .all(|var| std::env::var_os(var).is_none())
    {
        config.db_pool_size = 2;
    }
    Ok(config)
}

/// Serve `app` to the Lambda runtime until the sandbox is shut down.
///
/// With the `otel` feature, metrics are started here (over OTLP, if it is
/// configured; a function has no admin port to serve `/metrics` on), and
/// whatever was recorded during an invocation is exported before its response
/// goes back. See [`flush_after_each_invocation`].
pub async fn run(app: postrust_server::App) -> Result<(), Error> {
    #[cfg(feature = "otel")]
    {
        postrust_server::otel::init_metrics(false);
        if postrust_server::otel::exporting() {
            let router = app.router.layer(FlushLayer);
            return lambda_http::run(router).await;
        }
    }
    lambda_http::run(app.router).await
}

/// Lambda freezes a sandbox the moment an invocation's response is handed
/// back, and may never thaw it. The SDK exports in batches from a thread of
/// its own, on a timer, so whatever an invocation recorded would sit in a
/// frozen buffer -- for minutes, or for ever. So each invocation flushes
/// before it answers: spans and metrics go to the collector (the ADOT
/// extension, on localhost) first, and the response after.
///
/// That puts the export on the invocation's latency: a round trip to a
/// collector in the same sandbox, a millisecond or two. It is the price the
/// ADOT documentation describes, and the only way not to lose the data.
///
/// The request span closes before the flush -- it belongs to the router
/// inside this layer -- so the invocation's own trace is in what is sent.
#[cfg(feature = "otel")]
#[derive(Clone, Copy, Debug, Default)]
pub struct FlushLayer;

#[cfg(feature = "otel")]
impl<S> tower::Layer<S> for FlushLayer {
    type Service = FlushAfter<S>;

    fn layer(&self, inner: S) -> Self::Service {
        FlushAfter { inner }
    }
}

/// The service [`FlushLayer`] produces.
#[cfg(feature = "otel")]
#[derive(Clone, Debug)]
pub struct FlushAfter<S> {
    inner: S,
}

#[cfg(feature = "otel")]
impl<S, R> tower::Service<R> for FlushAfter<S>
where
    S: tower::Service<R>,
    S::Future: Send + 'static,
    S::Response: Send + 'static,
    S::Error: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future =
        std::pin::Pin<Box<dyn std::future::Future<Output = Result<S::Response, S::Error>> + Send>>;

    fn poll_ready(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: R) -> Self::Future {
        let answer = self.inner.call(request);
        Box::pin(async move {
            // Awaited to completion and dropped before the flush, and with
            // it the request's span.
            let response = answer.await;
            postrust_server::otel::flush().await;
            response
        })
    }
}

/// Whether [`run`] flushes after each invocation: the `otel` feature is
/// built in and something is exporting. For a binary that serves the
/// router itself, through `lambda_http::run`, and wants the same: wrap the
/// router in [`FlushLayer`] when this is true.
#[cfg(feature = "otel")]
pub fn flush_after_each_invocation() -> bool {
    postrust_server::otel::exporting()
}
