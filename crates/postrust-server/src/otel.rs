//! OpenTelemetry: the request's spans exported over OTLP, and metrics -- over
//! OTLP, and as Prometheus text at `GET /metrics` on the admin port.
//!
//! Compiled in with the `otel` feature, which the published images are built
//! with, and inactive until configured. Nothing here instruments anything of
//! its own: the spans are the ones [`crate::telemetry`] and the engine already
//! open, turned into OpenTelemetry spans by `tracing-opentelemetry`, and the
//! metrics are read from what the server already knows -- the pool, the token
//! cache -- or taken at the few points the request path already passes.
//!
//! # Activation
//!
//! Each half switches on separately, by the standard variables:
//!
//! - **Traces** are exported when `OTEL_EXPORTER_OTLP_ENDPOINT` or
//!   `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT` is set, unless
//!   `OTEL_TRACES_EXPORTER=none`.
//! - **Metrics** are recorded when they have somewhere to go: OTLP, by
//!   `OTEL_EXPORTER_OTLP_ENDPOINT` or `OTEL_EXPORTER_OTLP_METRICS_ENDPOINT`
//!   (unless `OTEL_METRICS_EXPORTER=none`), or the admin port's `/metrics`,
//!   whenever `PGRST_ADMIN_SERVER_PORT` is set.
//! - `OTEL_SDK_DISABLED=true` turns off both OTLP exporters whatever else
//!   says; `/metrics` stays, since it is not an SDK exporter but an endpoint
//!   the operator asked for by opening the admin port.
//!
//! The rest of the standard variables are the SDK's to read, and it does:
//! `OTEL_SERVICE_NAME` (here `postrust` when unset, rather than the SDK's
//! `unknown_service`), `OTEL_RESOURCE_ATTRIBUTES`, `OTEL_TRACES_SAMPLER` and
//! its argument, `OTEL_EXPORTER_OTLP_HEADERS` and the per-signal variants,
//! `OTEL_EXPORTER_OTLP_TIMEOUT`, `OTEL_BSP_*` and `OTEL_METRIC_EXPORT_INTERVAL`.
//!
//! # What it costs when inactive
//!
//! The point of building the images with this is that turning it on is a
//! configuration change. That only works if leaving it off costs nothing, so
//! every hook on the request path is a check of a flag set once at start-up:
//!
//! - no tracer provider, so no OpenTelemetry layer in the subscriber, which
//!   is built exactly as it is without the feature;
//! - no meter provider, so the request layer reads one empty `OnceLock` and
//!   takes no timestamp it did not already take;
//! - no observer for the engine, so it does not read the clock around
//!   `begin()` either.
//!
//! # Why the exporter's HTTP client is our own
//!
//! opentelemetry-otlp's clients are reqwest 0.13 or a hyper client of its
//! own; the workspace has reqwest 0.12, so either would have been a second
//! HTTP stack in the binary. [`ReqwestClient`] is reqwest 0.12 behind the
//! exporter's `HttpClient` trait, and runs each export on the tokio runtime
//! the server started in -- the SDK's batch processors run on threads of
//! their own, with no reactor for reqwest to use.

use crate::state::AppState;
use opentelemetry::metrics::{Histogram, Meter, MeterProvider as _, UpDownCounter};
use opentelemetry::trace::{
    SpanContext, TraceContextExt as _, TraceFlags, TraceState, TracerProvider as _,
};
use opentelemetry::KeyValue;
use opentelemetry_otlp::WithHttpConfig as _;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::trace::{IdGenerator, RandomIdGenerator, SdkTracerProvider};
use opentelemetry_sdk::Resource;
use postrust_core::trace::TraceContext;
use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};
use tracing::Span;
use tracing_opentelemetry::OpenTelemetrySpanExt as _;
use tracing_subscriber::filter::filter_fn;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::Layer;

/// The name of every metric this server records, in one place.
///
/// OpenTelemetry's semantic conventions where there is one -- HTTP server
/// and database client metrics -- and `postrust.*` for what is Postrust's
/// own. A Prometheus scrape sees them as the exporter renders them: dots
/// become underscores, the unit is appended (`_seconds`), and a counter ends
/// in `_total`. docs/observability.md lists both spellings.
pub mod names {
    /// Histogram, seconds: how long each request took, by
    /// `http.request.method`, `http.route` and `http.response.status_code`.
    pub const HTTP_SERVER_REQUEST_DURATION: &str = "http.server.request.duration";
    /// Up-down counter: requests being served right now, by
    /// `http.request.method`.
    pub const HTTP_SERVER_ACTIVE_REQUESTS: &str = "http.server.active_requests";
    /// Up-down counter: the pool's connections, by `db.client.connection.state`
    /// (`idle` or `used`).
    pub const DB_CLIENT_CONNECTION_COUNT: &str = "db.client.connection.count";
    /// Up-down counter: the most connections the pool will open
    /// (`PGRST_DB_POOL`).
    pub const DB_CLIENT_CONNECTION_MAX: &str = "db.client.connection.max";
    /// Histogram, seconds: how long a request waited for its transaction to
    /// begin -- a connection from the pool, and `BEGIN`.
    pub const DB_CLIENT_CONNECTION_WAIT_TIME: &str = "db.client.connection.wait_time";
    /// Counter: token cache lookups, by `outcome` (`hit` or `miss`). A request
    /// with no token is neither, and is not counted.
    pub const JWT_CACHE_LOOKUPS: &str = "postrust.jwt.cache.lookups";
    /// Counter: schema cache reloads after `NOTIFY`, by `outcome` (`success`
    /// or `failure`).
    pub const SCHEMA_CACHE_RELOADS: &str = "postrust.schema_cache.reloads";
    /// Up-down counter: GraphQL subscriptions open right now.
    pub const GRAPHQL_SUBSCRIPTIONS_ACTIVE: &str = "postrust.graphql.subscriptions.active";

    /// Counter: JWKS fetches from `PGRST_JWT_JWKS_URL`, by `outcome`
    /// (`success` or `failure`). Absent without a JWKS URL.
    pub const JWT_JWKS_REFRESHES: &str = "postrust.jwt.jwks.refreshes";
    /// Gauge, seconds: how long ago the key set in use was last confirmed by
    /// a successful fetch. Absent until one has been.
    pub const JWT_JWKS_KEY_SET_AGE: &str = "postrust.jwt.jwks.key_set.age";

    /// The instrumentation scope every metric and span is recorded under.
    pub const SCOPE: &str = "postrust";
}

// ---------------------------------------------------------------------------
// Activation, from the standard variables.
// ---------------------------------------------------------------------------

fn env_set(name: &str) -> bool {
    std::env::var_os(name).is_some_and(|value| !value.is_empty())
}

fn env_is(name: &str, expected: &str) -> bool {
    std::env::var(name).is_ok_and(|value| value.trim().eq_ignore_ascii_case(expected))
}

/// Whether OTLP export of one signal is configured: an endpoint for it, the
/// SDK not disabled, and that signal's exporter not set to `none`.
fn otlp_configured(signal_endpoint: &str, signal_exporter: &str) -> bool {
    !env_is("OTEL_SDK_DISABLED", "true")
        && !env_is(signal_exporter, "none")
        && (env_set("OTEL_EXPORTER_OTLP_ENDPOINT") || env_set(signal_endpoint))
}

/// Whether traces will be exported, by the environment alone.
pub fn traces_configured() -> bool {
    otlp_configured("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT", "OTEL_TRACES_EXPORTER")
}

/// Whether metrics will be exported over OTLP, by the environment alone.
pub fn metrics_otlp_configured() -> bool {
    otlp_configured(
        "OTEL_EXPORTER_OTLP_METRICS_ENDPOINT",
        "OTEL_METRICS_EXPORTER",
    )
}

/// Only HTTP/protobuf is built in. A protocol asked for that is not it is
/// said once, rather than silently sending protobuf to a gRPC port.
fn warn_on_protocol() {
    for var in [
        "OTEL_EXPORTER_OTLP_PROTOCOL",
        "OTEL_EXPORTER_OTLP_TRACES_PROTOCOL",
        "OTEL_EXPORTER_OTLP_METRICS_PROTOCOL",
    ] {
        if let Ok(value) = std::env::var(var) {
            if value.trim() != "http/protobuf" {
                tracing::warn!(
                    "{var}={value} is not supported: Postrust exports OTLP over \
                     http/protobuf only, usually on port 4318"
                );
            }
        }
    }
}

/// The resource every span and metric is reported under: the SDK's reading
/// of `OTEL_SERVICE_NAME` and `OTEL_RESOURCE_ATTRIBUTES`, with `postrust` as
/// the service name where neither gives one, and the version.
pub fn resource() -> Resource {
    let detected = Resource::builder().build();
    let unnamed = detected
        .get(&opentelemetry::Key::new("service.name"))
        .is_none_or(|name| name.as_str().starts_with("unknown_service"));
    let builder = Resource::builder()
        .with_attribute(KeyValue::new("service.version", env!("CARGO_PKG_VERSION")));
    match unnamed {
        true => builder.with_service_name("postrust").build(),
        false => builder.build(),
    }
}

// ---------------------------------------------------------------------------
// Traces.
// ---------------------------------------------------------------------------

/// The tracer provider, once traces are exporting.
static TRACER_PROVIDER: OnceLock<SdkTracerProvider> = OnceLock::new();

/// Set once an OpenTelemetry layer has been made, and read on every request:
/// the request span is joined to its trace and named only when something
/// will export it.
static TRACING: AtomicBool = AtomicBool::new(false);

/// The OpenTelemetry layer for a subscriber, if traces are configured: a
/// tracer provider exporting over OTLP, behind a filter of its own.
///
/// Call it once, as the subscriber is built, from within the tokio runtime
/// the server runs on -- exports run there. `None` when traces are not
/// configured, or the exporter cannot be built (which is logged once the
/// subscriber exists; see [`log_startup`]).
pub fn layer<S>() -> Option<impl Layer<S> + Send + Sync + 'static>
where
    S: tracing::Subscriber + for<'span> LookupSpan<'span> + Send + Sync,
{
    if !traces_configured() {
        return None;
    }
    let client = match ReqwestClient::from_env() {
        Ok(client) => client,
        Err(e) => {
            startup_warning(format!("OpenTelemetry traces are off: {e}"));
            return None;
        }
    };
    let exporter = match opentelemetry_otlp::SpanExporter::builder()
        .with_http()
        .with_http_client(client)
        .build()
    {
        Ok(exporter) => exporter,
        Err(e) => {
            startup_warning(format!("OpenTelemetry traces are off: {e}"));
            return None;
        }
    };
    let provider = tracer_provider_builder()
        .with_batch_exporter(exporter)
        .build();
    opentelemetry::global::set_tracer_provider(provider.clone());
    let layer = layer_with(&provider);
    let _ = TRACER_PROVIDER.set(provider);
    Some(layer)
}

/// A tracer provider builder set up as [`layer`] sets one up -- this
/// server's resource, and the id generator that keeps the trace id the logs
/// carry the one that is exported -- with no exporter yet. For a binary that
/// exports elsewhere, and for tests.
pub fn tracer_provider_builder() -> opentelemetry_sdk::trace::TracerProviderBuilder {
    SdkTracerProvider::builder()
        .with_resource(resource())
        .with_id_generator(RequestTraceIds::default())
}

/// The OpenTelemetry layer over a tracer provider of the caller's, behind the
/// same filter [`layer`] uses. Marks tracing active, so the request span is
/// joined to its caller's trace from here on.
///
/// The provider should come from [`tracer_provider_builder`]: with another
/// id generator, a request that starts its own trace is exported under a
/// trace id that is not the one its log lines carry.
pub fn layer_with<S>(provider: &SdkTracerProvider) -> impl Layer<S> + Send + Sync + 'static
where
    S: tracing::Subscriber + for<'span> LookupSpan<'span> + Send + Sync,
{
    TRACING.store(true, Ordering::Release);
    tracing_opentelemetry::layer()
        .with_tracer(provider.tracer(names::SCOPE))
        // Each of these is an attribute on every span, or a clock read on
        // every enter and exit, for something a trace backend already shows:
        // which thread, which line, how long a span sat idle between polls.
        .with_threads(false)
        .with_location(false)
        .with_target(false)
        .with_tracked_inactivity(false)
        .with_filter(export_filter())
}

/// What the OpenTelemetry layer sees, independent of the log filter.
///
/// A filter of its own so the log is not made noisier by exporting: the
/// request's `auth`, `plan`, `db.*` and `render` spans are at debug, and
/// are exported while the log stays at `PGRST_LOG_LEVEL`.
///
/// - **Spans** at debug and above, except from the HTTP, database and
///   telemetry crates beneath the server, whose own spans (where they have
///   any) describe their internals rather than the request.
/// - **Events** only from Postrust and only at warn and above, as span events.
///   The debug lines carry SQL text, and sqlx logs a slow statement with its
///   SQL at warn: exporting either would send the statement to the trace
///   backend when `PGRST_OTEL_DB_STATEMENT` says not to.
///
/// Decided per call site from metadata alone, so the answer is cached and a
/// span or event the exporter does not want costs what it did before.
fn export_filter<S>() -> impl tracing_subscriber::layer::Filter<S> + Send + Sync + 'static {
    const BENEATH: &[&str] = &[
        "opentelemetry",
        "tracing_opentelemetry",
        "h2",
        "hyper",
        "reqwest",
        "sqlx",
        "tower",
        "tokio",
        "rustls",
    ];
    // A crate and its siblings by name: `opentelemetry` covers
    // `opentelemetry_sdk` and `opentelemetry-otlp`, `hyper` covers
    // `hyper_util`.
    filter_fn(|meta| {
        let target = meta.target();
        if meta.is_span() {
            *meta.level() <= tracing::Level::DEBUG
                && !BENEATH.iter().any(|crate_name| {
                    target.starts_with(crate_name)
                        && matches!(
                            target.as_bytes().get(crate_name.len()),
                            None | Some(b':' | b'_' | b'-')
                        )
                })
        } else {
            *meta.level() <= tracing::Level::WARN && target.starts_with("postrust")
        }
    })
}

thread_local! {
    /// The trace id the next root span on this thread is to have. Set by
    /// [`join_trace`] for the length of one call; see [`RequestTraceIds`].
    static NEXT_TRACE_ID: Cell<Option<[u8; 16]>> = const { Cell::new(None) };
}

/// Ids for the SDK, where a request's own trace id comes from the request.
///
/// A request that arrives without a `traceparent` already has a trace id by
/// the time its span exists: [`crate::telemetry`] made one, logs it as
/// `trace_id`, and sends it to the database as `request.trace_id`. Left to
/// itself the SDK would start the exported span under a second, unrelated
/// id, and the logs, the database and the trace backend would each name a
/// different trace. The SDK offers no way to hand a root span its trace id,
/// so [`join_trace`] leaves it here, on the thread, for exactly the call that
/// starts the span, and this takes it. Everything else -- every span id, and
/// a root span nobody left an id for -- is the SDK's random generator.
#[derive(Debug, Default)]
pub struct RequestTraceIds(RandomIdGenerator);

impl IdGenerator for RequestTraceIds {
    fn new_trace_id(&self) -> opentelemetry::trace::TraceId {
        match NEXT_TRACE_ID.with(Cell::take) {
            Some(bytes) => opentelemetry::trace::TraceId::from_bytes(bytes),
            None => self.0.new_trace_id(),
        }
    }

    fn new_span_id(&self) -> opentelemetry::trace::SpanId {
        self.0.new_span_id()
    }
}

/// Put the request span in the trace `trace` names, for export.
///
/// A caller's `traceparent` makes its span the parent, as a remote span
/// context -- the exported span joins the caller's trace, and the sampling
/// decision the caller made in `trace-flags` is the parent-based sampler's to
/// honour. Without one the span starts a trace of its own, under the trace id
/// already chosen for it; see [`RequestTraceIds`].
///
/// Does nothing unless an OpenTelemetry layer exists.
pub(crate) fn join_trace(span: &Span, trace: &TraceContext) {
    if !TRACING.load(Ordering::Acquire) || span.is_disabled() {
        return;
    }
    match trace.parent_id {
        Some(parent) => {
            let state = trace
                .tracestate
                .as_ref()
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<TraceState>().ok())
                .unwrap_or_default();
            let remote = SpanContext::new(
                opentelemetry::trace::TraceId::from_bytes(trace.trace_id.to_bytes()),
                opentelemetry::trace::SpanId::from_bytes(parent.to_bytes()),
                TraceFlags::new(trace.flags),
                true,
                state,
            );
            let _ = span.set_parent(opentelemetry::Context::new().with_remote_span_context(remote));
        }
        None => {
            // `context` starts the span, here and now, on this thread -- so
            // the id left for it is taken by it and by nothing else. Cleared
            // after in case the span was not started after all.
            NEXT_TRACE_ID.with(|next| next.set(Some(trace.trace_id.to_bytes())));
            let _ = span.context();
            NEXT_TRACE_ID.with(|next| next.set(None));
        }
    }
}

/// Name the request span as the HTTP conventions name a server span:
/// `{method} {route}`, or the method alone where no route matched. Recorded
/// as `otel.name`, which is what the exporter names the span by; the
/// `tracing` span stays `request`.
pub(crate) fn name_request_span(span: &Span, method: &http::Method, route: Option<&str>) {
    if !TRACING.load(Ordering::Acquire) || span.is_disabled() {
        return;
    }
    let method = known_method(method);
    match route {
        Some(route) => span.record("otel.name", format!("{method} {route}")),
        None => span.record("otel.name", method),
    };
}

/// The method as the HTTP conventions spell it: one of the standard nine,
/// or `_OTHER`, so a client sending made-up methods cannot mint a metric
/// series or span name for each.
fn known_method(method: &http::Method) -> &'static str {
    match *method {
        http::Method::GET => "GET",
        http::Method::POST => "POST",
        http::Method::PUT => "PUT",
        http::Method::PATCH => "PATCH",
        http::Method::DELETE => "DELETE",
        http::Method::HEAD => "HEAD",
        http::Method::OPTIONS => "OPTIONS",
        http::Method::CONNECT => "CONNECT",
        http::Method::TRACE => "TRACE",
        _ => "_OTHER",
    }
}

// ---------------------------------------------------------------------------
// Metrics.
// ---------------------------------------------------------------------------

/// What the request path records into, once metrics are on.
struct Instruments {
    request_duration: Histogram<f64>,
    active_requests: UpDownCounter<i64>,
    transaction_wait: Histogram<f64>,
    schema_reloads: opentelemetry::metrics::Counter<u64>,
}

static METER_PROVIDER: OnceLock<SdkMeterProvider> = OnceLock::new();
static INSTRUMENTS: OnceLock<Instruments> = OnceLock::new();
static PROMETHEUS: OnceLock<prometheus::Registry> = OnceLock::new();

/// The state the observed metrics are read from: the pool, the token cache.
/// Weak, so that metrics never keep an application alive, and replaced by
/// each [`observe`], so a process that builds several (a test suite) reports
/// the latest.
static OBSERVED: Mutex<Option<Weak<AppState>>> = Mutex::new(None);

/// HTTP server durations, in seconds: the boundaries the semantic
/// conventions advise for `http.server.request.duration`.
const DURATION_BOUNDARIES: &[f64] = &[
    0.005, 0.01, 0.025, 0.05, 0.075, 0.1, 0.25, 0.5, 0.75, 1.0, 2.5, 5.0, 7.5, 10.0,
];

/// Start recording metrics, if they have somewhere to go: OTLP when it is
/// configured, and the admin port's `/metrics` when `prometheus` says the
/// admin port is open.
///
/// Once per process; a second call changes nothing. Returns whether metrics
/// are being recorded. Call it within the tokio runtime, as for [`layer`].
pub fn init_metrics(prometheus: bool) -> bool {
    if METER_PROVIDER.get().is_some() {
        return true;
    }
    let otlp = metrics_otlp_configured();
    if !otlp && !prometheus {
        return false;
    }

    let mut builder = SdkMeterProvider::builder().with_resource(resource());
    if otlp {
        match ReqwestClient::from_env().and_then(|client| {
            opentelemetry_otlp::MetricExporter::builder()
                .with_http()
                .with_http_client(client)
                .build()
                .map_err(|e| e.to_string())
        }) {
            Ok(exporter) => {
                builder = builder.with_reader(
                    opentelemetry_sdk::metrics::PeriodicReader::builder(exporter).build(),
                );
            }
            Err(e) => tracing::warn!("OpenTelemetry metrics are not exported over OTLP: {e}"),
        }
    }
    if prometheus {
        let registry = prometheus::Registry::new();
        // No `otel_scope_*` labels on every series: there is one scope,
        // `postrust`, and repeating it on each line says nothing.
        match opentelemetry_prometheus::exporter()
            .with_registry(registry.clone())
            .scope_info_enabled(false)
            .build()
        {
            Ok(exporter) => {
                builder = builder.with_reader(exporter);
                let _ = PROMETHEUS.set(registry);
            }
            Err(e) => tracing::warn!("/metrics is unavailable: {e}"),
        }
    }

    let provider = builder.build();
    let meter = provider.meter(names::SCOPE);
    if INSTRUMENTS.set(instruments(&meter)).is_err() {
        return true;
    }
    register_observers(&meter);
    let _ = postrust_engine::observe::set_observer(EngineObserver);
    opentelemetry::global::set_meter_provider(provider.clone());
    let _ = METER_PROVIDER.set(provider);
    true
}

/// Whether metrics are being recorded.
pub fn metrics_enabled() -> bool {
    INSTRUMENTS.get().is_some()
}

fn instruments(meter: &Meter) -> Instruments {
    Instruments {
        request_duration: meter
            .f64_histogram(names::HTTP_SERVER_REQUEST_DURATION)
            .with_unit("s")
            .with_description("Duration of HTTP server requests.")
            .with_boundaries(DURATION_BOUNDARIES.to_vec())
            .build(),
        active_requests: meter
            .i64_up_down_counter(names::HTTP_SERVER_ACTIVE_REQUESTS)
            .with_unit("{request}")
            .with_description("Number of active HTTP server requests.")
            .build(),
        transaction_wait: meter
            .f64_histogram(names::DB_CLIENT_CONNECTION_WAIT_TIME)
            .with_unit("s")
            .with_description(
                "The time it took to obtain a connection from the pool and begin a transaction.",
            )
            .with_boundaries(vec![
                0.0001, 0.0005, 0.001, 0.0025, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5,
                5.0, 10.0,
            ])
            .build(),
        schema_reloads: meter
            .u64_counter(names::SCHEMA_CACHE_RELOADS)
            .with_description("Schema cache reloads requested by NOTIFY.")
            .build(),
    }
}

/// The pool's name on its metrics, as the conventions require one.
const POOL_NAME: &str = "postrust";

/// The metrics read when they are collected rather than recorded as they
/// happen: the pool, the token cache, the JWKS, open subscriptions.
///
/// Each callback reads [`OBSERVED`] when a scrape or an export asks, and
/// reports nothing while there is no application to read.
fn register_observers(meter: &Meter) {
    meter
        .i64_observable_up_down_counter(names::DB_CLIENT_CONNECTION_COUNT)
        .with_unit("{connection}")
        .with_description("The number of connections that are currently in the state described.")
        .with_callback(|observer| {
            if let Some(state) = observed() {
                let open = i64::from(state.pool.size());
                let idle = state.pool.num_idle() as i64;
                let pool = KeyValue::new("db.client.connection.pool.name", POOL_NAME);
                observer.observe(
                    idle,
                    &[
                        pool.clone(),
                        KeyValue::new("db.client.connection.state", "idle"),
                    ],
                );
                observer.observe(
                    (open - idle).max(0),
                    &[pool, KeyValue::new("db.client.connection.state", "used")],
                );
            }
        })
        .build();
    meter
        .i64_observable_up_down_counter(names::DB_CLIENT_CONNECTION_MAX)
        .with_unit("{connection}")
        .with_description("The maximum number of open connections allowed.")
        .with_callback(|observer| {
            if let Some(state) = observed() {
                observer.observe(
                    i64::from(state.pool.options().get_max_connections()),
                    &[KeyValue::new("db.client.connection.pool.name", POOL_NAME)],
                );
            }
        })
        .build();
    meter
        .u64_observable_counter(names::JWT_CACHE_LOOKUPS)
        .with_unit("{lookup}")
        .with_description("Token cache lookups, by whether the cache answered.")
        .with_callback(|observer| {
            if let Some(stats) = observed()
                .as_deref()
                .and_then(|state| state.jwt_cache.as_ref())
                .map(postrust_auth::JwtCache::stats)
            {
                observer.observe(stats.hits, &[KeyValue::new("outcome", "hit")]);
                observer.observe(stats.misses, &[KeyValue::new("outcome", "miss")]);
            }
        })
        .build();
    meter
        .u64_observable_counter(names::JWT_JWKS_REFRESHES)
        .with_unit("{fetch}")
        .with_description("JWKS fetches from PGRST_JWT_JWKS_URL, by outcome.")
        .with_callback(|observer| {
            if let Some(status) = jwks_status() {
                observer.observe(status.refreshes, &[KeyValue::new("outcome", "success")]);
                observer.observe(status.failures, &[KeyValue::new("outcome", "failure")]);
            }
        })
        .build();
    meter
        .f64_observable_gauge(names::JWT_JWKS_KEY_SET_AGE)
        .with_unit("s")
        .with_description("How long ago the JWKS in use was last fetched.")
        .with_callback(|observer| {
            if let Some(age) = jwks_status().and_then(|status| status.age) {
                observer.observe(age.as_secs_f64(), &[]);
            }
        })
        .build();
    #[cfg(feature = "graphql")]
    meter
        .i64_observable_up_down_counter(names::GRAPHQL_SUBSCRIPTIONS_ACTIVE)
        .with_unit("{subscription}")
        .with_description("GraphQL subscriptions open right now.")
        .with_callback(|observer| {
            observer.observe(postrust_graphql::subscription::active_subscriptions(), &[]);
        })
        .build();
}

fn observed() -> Option<Arc<AppState>> {
    OBSERVED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .and_then(Weak::upgrade)
}

/// The JWKS refresher's state, where `PGRST_JWT_JWKS_URL` is set.
fn jwks_status() -> Option<postrust_auth::jwks::JwksStatus> {
    observed()?
        .jwt_config
        .jwks
        .as_ref()
        .map(postrust_auth::Jwks::status)
}

/// Read the pool and token cache metrics from `state`. Called by `build`.
pub(crate) fn observe(state: &Arc<AppState>) {
    *OBSERVED.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::downgrade(state));
}

/// Count a schema cache reload. Called by the `NOTIFY` reloader.
pub(crate) fn schema_reloaded(succeeded: bool) {
    if let Some(instruments) = INSTRUMENTS.get() {
        let outcome = match succeeded {
            true => "success",
            false => "failure",
        };
        instruments
            .schema_reloads
            .add(1, &[KeyValue::new("outcome", outcome)]);
    }
}

/// What the engine measures, recorded here.
struct EngineObserver;

impl postrust_engine::observe::Observer for EngineObserver {
    fn transaction_begun(&self, waited: Duration) {
        if let Some(instruments) = INSTRUMENTS.get() {
            instruments.transaction_wait.record(
                waited.as_secs_f64(),
                &[KeyValue::new("db.client.connection.pool.name", POOL_NAME)],
            );
        }
    }
}

/// One request, as the HTTP server metrics see it: counted as active from
/// [`start`](Self::start) until dropped, and its duration recorded by
/// [`finish`](Self::finish).
///
/// Empty, and free, when metrics are off.
pub(crate) struct RequestMeter(Option<MeteredRequest>);

struct MeteredRequest {
    instruments: &'static Instruments,
    method: &'static str,
    route: Option<Arc<str>>,
}

impl RequestMeter {
    pub(crate) fn start(method: &http::Method, route: Option<&str>) -> Self {
        let Some(instruments) = INSTRUMENTS.get() else {
            return Self(None);
        };
        let method = known_method(method);
        instruments
            .active_requests
            .add(1, &[KeyValue::new("http.request.method", method)]);
        Self(Some(MeteredRequest {
            instruments,
            method,
            route: route.map(Arc::from),
        }))
    }

    pub(crate) fn finish(&self, status: http::StatusCode, started: Instant) {
        let Some(request) = &self.0 else {
            return;
        };
        let mut attributes = Vec::with_capacity(3);
        attributes.push(KeyValue::new("http.request.method", request.method));
        // Unset where nothing matched, as on the span: a 404 for every path
        // a scanner tries would otherwise be a series each.
        if let Some(route) = &request.route {
            attributes.push(KeyValue::new("http.route", route.clone()));
        }
        attributes.push(KeyValue::new(
            "http.response.status_code",
            i64::from(status.as_u16()),
        ));
        request
            .instruments
            .request_duration
            .record(started.elapsed().as_secs_f64(), &attributes);
    }
}

impl Drop for RequestMeter {
    fn drop(&mut self) {
        if let Some(request) = &self.0 {
            request
                .instruments
                .active_requests
                .add(-1, &[KeyValue::new("http.request.method", request.method)]);
        }
    }
}

/// The metrics as Prometheus text, or `None` when `/metrics` is not enabled.
pub fn prometheus_text() -> Option<String> {
    let registry = PROMETHEUS.get()?;
    let encoder = prometheus::TextEncoder::new();
    match encoder.encode_to_string(&registry.gather()) {
        Ok(text) => Some(text),
        Err(e) => {
            tracing::error!("Could not render /metrics: {e}");
            Some(String::new())
        }
    }
}

/// `GET /metrics`, on the admin router.
pub(crate) async fn metrics_handler() -> axum::response::Response {
    use axum::response::IntoResponse;
    match prometheus_text() {
        Some(text) => (
            [(
                http::header::CONTENT_TYPE,
                // The exposition format's own content type, version and all:
                // Prometheus picks its parser by it.
                "text/plain; version=0.0.4; charset=utf-8",
            )],
            text,
        )
            .into_response(),
        None => (
            http::StatusCode::NOT_FOUND,
            "metrics are not enabled: call postrust_server::otel::init_metrics(true)\n",
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// Flushing and shutting down.
// ---------------------------------------------------------------------------

/// Export everything recorded so far, and wait for it.
///
/// Blocks the calling thread until the exporters have answered or timed out,
/// so call it from a blocking context; [`flush`] does that for async code.
pub fn force_flush() {
    if let Some(provider) = TRACER_PROVIDER.get() {
        if let Err(e) = provider.force_flush() {
            tracing::warn!("Could not flush spans: {e}");
        }
    }
    if let Some(provider) = METER_PROVIDER.get() {
        if let Err(e) = provider.force_flush() {
            tracing::warn!("Could not flush metrics: {e}");
        }
    }
}

/// Whether anything is exported that [`force_flush`] would send.
pub fn exporting() -> bool {
    TRACER_PROVIDER.get().is_some() || METER_PROVIDER.get().is_some()
}

/// [`force_flush`], from async code: on a blocking thread, so the export --
/// which runs on this same runtime -- is not waiting for the thread that is
/// waiting for it. That matters on a current-thread runtime, where it would
/// never finish.
pub async fn flush() {
    if !exporting() {
        return;
    }
    if let Err(e) = tokio::task::spawn_blocking(force_flush).await {
        tracing::warn!("Could not flush telemetry: {e}");
    }
}

/// Flush and stop the exporters, at the end of the process.
pub async fn shutdown() {
    if !exporting() {
        return;
    }
    let _ = tokio::task::spawn_blocking(|| {
        if let Some(provider) = TRACER_PROVIDER.get() {
            let _ = provider.shutdown();
        }
        if let Some(provider) = METER_PROVIDER.get() {
            let _ = provider.shutdown();
        }
    })
    .await;
}

// ---------------------------------------------------------------------------
// Start-up warnings, said once there is a subscriber to say them to.
// ---------------------------------------------------------------------------

static STARTUP_WARNINGS: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn startup_warning(message: String) {
    STARTUP_WARNINGS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(message);
}

/// Log what went wrong while [`layer`] ran, which is before any subscriber
/// existed to log it to. Call once the subscriber is installed.
pub fn log_startup() {
    warn_on_protocol();
    for message in std::mem::take(&mut *STARTUP_WARNINGS.lock().unwrap_or_else(|e| e.into_inner()))
    {
        tracing::warn!("{message}");
    }
    if TRACER_PROVIDER.get().is_some() {
        tracing::info!("OpenTelemetry: exporting traces over OTLP");
    }
}

// ---------------------------------------------------------------------------
// The exporter's HTTP client.
// ---------------------------------------------------------------------------

/// reqwest 0.12 as the OTLP exporter's HTTP client. See the module
/// documentation for why it is not the exporter's own.
#[derive(Debug, Clone)]
pub struct ReqwestClient {
    client: reqwest::Client,
    runtime: tokio::runtime::Handle,
}

impl ReqwestClient {
    /// A client for the current tokio runtime, timing out as
    /// `OTEL_EXPORTER_OTLP_TIMEOUT` says (milliseconds; ten seconds unset).
    /// The exporter applies that timeout to its own clients and leaves one
    /// supplied to it to enforce its own.
    pub fn from_env() -> Result<Self, String> {
        let runtime = tokio::runtime::Handle::try_current()
            .map_err(|_| "the exporter must be set up within the tokio runtime".to_string())?;
        let timeout = std::env::var("OTEL_EXPORTER_OTLP_TIMEOUT")
            .ok()
            .and_then(|value| value.trim().parse::<u64>().ok())
            .map(Duration::from_millis)
            .unwrap_or(Duration::from_secs(10));
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| format!("could not build the HTTP client: {e}"))?;
        Ok(Self { client, runtime })
    }
}

#[async_trait::async_trait]
impl opentelemetry_http::HttpClient for ReqwestClient {
    async fn send_bytes(
        &self,
        request: http::Request<bytes::Bytes>,
    ) -> Result<http::Response<bytes::Bytes>, opentelemetry_http::HttpError> {
        let client = self.client.clone();
        let request = reqwest::Request::try_from(request)?;
        // Spawned rather than awaited here: the SDK calls this from a thread
        // of its own, where there is no reactor for reqwest's connection to
        // be driven by. The runtime drives it, and the join handle can be
        // waited on from anywhere.
        self.runtime
            .spawn(async move {
                let response = client.execute(request).await?;
                let mut answer = http::Response::builder().status(response.status());
                if let Some(headers) = answer.headers_mut() {
                    *headers = response.headers().clone();
                }
                let body = response.bytes().await?;
                Ok::<_, opentelemetry_http::HttpError>(answer.body(body)?)
            })
            .await?
    }
}
