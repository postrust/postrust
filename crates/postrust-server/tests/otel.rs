//! OpenTelemetry export, with the `otel` feature: the spans a request opens,
//! as an exporter receives them, and the metrics `/metrics` renders.
//!
//! Spans go to an in-memory exporter rather than a collector, through the
//! same layer and id generator the server uses (`otel::layer_with`,
//! `otel::tracer_provider_builder`). The tests without a database drive
//! `RequestTraceLayer` around a stub; the ones with a database go through
//! `postrust_server::build`.
//!
//! `DATABASE_URL` must name a superuser on a database with
//! `scripts/init-db.sql` loaded (for `web_anon`). Run with:
//!
//! ```text
//! cargo test -p postrust-server --features otel --test otel -- --include-ignored
//! ```

#![cfg(feature = "otel")]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::any;
use axum::Router;
use opentelemetry::trace::{SpanId, TraceId};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider, SpanData};
use postrust_core::trace::TraceContext;
use postrust_server::otel;
use postrust_server::telemetry::RequestTraceLayer;
use sqlx::AssertSqlSafe;
use std::sync::Arc;
use tower::ServiceExt;
use tracing_subscriber::layer::SubscriberExt;

/// Held by every test here, as in tests/request_tracing.rs: `tracing` caches
/// per call site whether anybody is listening, process-wide, and a test
/// sending a request with no subscriber beside one installing its own can
/// leave a call site cached as uninteresting.
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

const TRACEPARENT: &str = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";
const CALLER_TRACE: &str = "4bf92f3577b34da6a3ce929d0e0e4736";
const CALLER_SPAN: &str = "00f067aa0ba902b7";

/// Spans exported to memory, for as long as the guard lives. The tests run
/// on a current-thread runtime, so the request runs on this thread and the
/// thread's default subscriber sees it.
fn export() -> (
    InMemorySpanExporter,
    SdkTracerProvider,
    tracing::subscriber::DefaultGuard,
) {
    let exporter = InMemorySpanExporter::default();
    let provider = otel::tracer_provider_builder()
        .with_simple_exporter(exporter.clone())
        .build();
    let subscriber = tracing_subscriber::registry().with(otel::layer_with(&provider));
    let guard = tracing::subscriber::set_default(subscriber);
    (exporter, provider, guard)
}

fn finished(exporter: &InMemorySpanExporter) -> Vec<SpanData> {
    exporter.get_finished_spans().unwrap()
}

fn attribute(span: &SpanData, key: &str) -> Option<String> {
    span.attributes
        .iter()
        .find(|kv| kv.key.as_str() == key)
        .map(|kv| kv.value.as_str().into_owned())
}

/// The one span whose `tracing` name was `name`. The request span is exported
/// as `otel.name` says, so it is found by its server kind instead.
fn named<'a>(spans: &'a [SpanData], name: &str) -> &'a SpanData {
    let found: Vec<&SpanData> = spans.iter().filter(|s| s.name == name).collect();
    assert_eq!(found.len(), 1, "one {name} span in {:#?}", names(spans));
    found[0]
}

fn request_span(spans: &[SpanData]) -> &SpanData {
    spans
        .iter()
        .find(|s| s.span_kind == opentelemetry::trace::SpanKind::Server)
        .unwrap_or_else(|| panic!("no server span in {:#?}", names(spans)))
}

fn names(spans: &[SpanData]) -> Vec<String> {
    spans.iter().map(|s| s.name.to_string()).collect()
}

/// A stub that answers with the trace id the layer put in the extensions.
fn stub() -> Router {
    Router::new()
        .route(
            "/api/{*path}",
            any(|request: Request<Body>| async move {
                request
                    .extensions()
                    .get::<TraceContext>()
                    .map(|t| t.trace_id.to_string())
                    .unwrap_or_default()
            }),
        )
        .layer(RequestTraceLayer)
}

async fn send(app: &Router, request: Request<Body>) -> (StatusCode, String) {
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8(body.to_vec()).unwrap())
}

fn get(uri: &str) -> http::request::Builder {
    Request::builder().uri(uri)
}

#[tokio::test(flavor = "current_thread")]
async fn the_exported_span_joins_the_callers_trace() {
    let _serial = SERIAL.lock().await;
    let (exporter, _provider, _guard) = export();

    let request = get("/api/items")
        .header("traceparent", TRACEPARENT)
        .body(Body::empty())
        .unwrap();
    let (_, logged) = send(&stub(), request).await;
    assert_eq!(logged, CALLER_TRACE);

    let spans = finished(&exporter);
    let request = request_span(&spans);
    assert_eq!(request.span_context.trace_id().to_string(), CALLER_TRACE);
    assert_eq!(request.parent_span_id.to_string(), CALLER_SPAN);
    assert!(request.parent_span_is_remote);
    // `{method} {route}`, by `otel.name`; the HTTP fields beside it.
    assert_eq!(request.name, "GET /api/{*path}");
    assert_eq!(attribute(request, "http.route").unwrap(), "/api/{*path}");
    assert_eq!(attribute(request, "http.request.method").unwrap(), "GET");
    assert_eq!(
        attribute(request, "http.response.status_code").unwrap(),
        "200"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn a_trace_started_here_is_exported_under_the_id_the_logs_carry() {
    let _serial = SERIAL.lock().await;
    let (exporter, _provider, _guard) = export();

    let (_, logged) = send(&stub(), get("/api/items").body(Body::empty()).unwrap()).await;
    let spans = finished(&exporter);
    let request = request_span(&spans);
    assert_eq!(request.span_context.trace_id().to_string(), logged);
    assert_eq!(request.parent_span_id, SpanId::INVALID);
    assert_eq!(request.name, "GET /api/{*path}");

    // And the next request starts a different one.
    exporter.reset();
    let (_, second) = send(&stub(), get("/api/items").body(Body::empty()).unwrap()).await;
    assert_ne!(second, logged);
    let spans = finished(&exporter);
    assert_eq!(
        request_span(&spans).span_context.trace_id().to_string(),
        second
    );
}

#[tokio::test(flavor = "current_thread")]
async fn a_caller_that_is_not_sampling_is_not_exported() {
    let _serial = SERIAL.lock().await;
    let (exporter, _provider, _guard) = export();
    let request = get("/api/items")
        .header("traceparent", TRACEPARENT.replace("-01", "-00"))
        .body(Body::empty())
        .unwrap();
    let (_, logged) = send(&stub(), request).await;
    // Logged under the caller's trace all the same.
    assert_eq!(logged, CALLER_TRACE);
    assert!(finished(&exporter).is_empty());
}

#[tokio::test(flavor = "current_thread")]
async fn a_request_that_matched_no_route_is_named_by_its_method() {
    let _serial = SERIAL.lock().await;
    let (exporter, _provider, _guard) = export();
    let (status, _) = send(&stub(), get("/nowhere").body(Body::empty()).unwrap()).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let spans = finished(&exporter);
    let request = request_span(&spans);
    assert_eq!(request.name, "GET");
    assert!(attribute(request, "http.route").is_none());
}

// ---------------------------------------------------------------------------
// Through the whole application, with a database.
// ---------------------------------------------------------------------------

const SCHEMA: &str = "otel_export";
const SECRET: &str = "reallyreallyreallyreallyverysafe";

fn database_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/postrust_test".to_string())
}

async fn application(host: postrust_server::Host) -> postrust_server::App {
    let pool = sqlx::PgPool::connect(&database_url()).await.unwrap();
    for statement in [
        format!("DROP SCHEMA IF EXISTS {SCHEMA} CASCADE"),
        format!("CREATE SCHEMA {SCHEMA}"),
        format!("GRANT USAGE ON SCHEMA {SCHEMA} TO web_anon"),
        format!("CREATE TABLE {SCHEMA}.projects (id int PRIMARY KEY, name text)"),
        format!(
            "CREATE TABLE {SCHEMA}.tasks (id int PRIMARY KEY, \
             project_id int REFERENCES {SCHEMA}.projects (id))"
        ),
        format!("INSERT INTO {SCHEMA}.projects VALUES (1, 'one')"),
        format!("INSERT INTO {SCHEMA}.tasks VALUES (1, 1), (2, 1)"),
        format!("GRANT SELECT, INSERT ON ALL TABLES IN SCHEMA {SCHEMA} TO web_anon"),
    ] {
        sqlx::query(AssertSqlSafe(statement))
            .execute(&pool)
            .await
            .unwrap();
    }

    let mut config = postrust_core::AppConfig::default();
    config.db_uri = database_url();
    config.db_schemas = vec![SCHEMA.to_string()];
    config.db_anon_role = Some("web_anon".to_string());
    // Room beside the connections a server host's listeners hold: the
    // `NOTIFY` reloader's, and with GraphQL the subscription broker's.
    config.db_pool_size = 4;
    config.jwt_secret = Some(SECRET.to_string());
    config.db_channel_enabled = true;
    postrust_server::build(config, host).await.unwrap()
}

async fn drop_schema() {
    let pool = sqlx::PgPool::connect(&database_url()).await.unwrap();
    sqlx::query(AssertSqlSafe(format!(
        "DROP SCHEMA IF EXISTS {SCHEMA} CASCADE"
    )))
    .execute(&pool)
    .await
    .unwrap();
}

fn token() -> String {
    let exp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 600;
    jsonwebtoken::encode(
        &jsonwebtoken::Header::default(),
        &serde_json::json!({"role": "web_anon", "exp": exp}),
        &jsonwebtoken::EncodingKey::from_secret(SECRET.as_bytes()),
    )
    .unwrap()
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires PostgreSQL"]
async fn a_rest_requests_stages_are_exported_beneath_it() {
    let _serial = SERIAL.lock().await;
    let app = application(postrust_server::Host::LAMBDA).await;
    let (exporter, _provider, _guard) = export();

    // A count, so the read does not commit with its query: a `db.count`
    // beneath `db.query`, and a `db.commit` after it.
    let request = get("/api/tasks?select=id,projects(name)")
        .header("traceparent", TRACEPARENT)
        .header("prefer", "count=exact")
        .body(Body::empty())
        .unwrap();
    let (status, body) = send(&app.router, request).await;
    assert!(status.is_success(), "{status} {body}");
    assert!(body.contains(r#""projects":{"name":"one"}"#), "{body}");

    let spans = finished(&exporter);
    let request = request_span(&spans);
    let trace = TraceId::from_hex(CALLER_TRACE).unwrap();
    assert_eq!(request.name, "GET /api/{*path}");
    for span in &spans {
        assert_eq!(span.span_context.trace_id(), trace, "{}", span.name);
    }

    let id = request.span_context.span_id();
    for stage in ["auth", "plan", "db.query", "db.commit", "render"] {
        assert_eq!(named(&spans, stage).parent_span_id, id, "{stage}");
    }
    let query = named(&spans, "db.query");
    assert_eq!(attribute(query, "db.system").unwrap(), "postgresql");
    assert_eq!(attribute(query, "db.operation.name").unwrap(), "select");
    assert_eq!(
        attribute(query, "db.collection.name").unwrap(),
        format!("{SCHEMA}.tasks")
    );
    let count = named(&spans, "db.count");
    assert_eq!(count.parent_span_id, query.span_context.span_id());
    assert_eq!(attribute(count, "db.operation.name").unwrap(), "count");
    // Off unless `PGRST_OTEL_DB_STATEMENT` asks for it.
    assert!(attribute(count, "db.query.text").is_none());
    assert_eq!(
        attribute(named(&spans, "db.commit"), "db.operation.name").unwrap(),
        "COMMIT"
    );

    // A write commits after it renders, in a span of its own beneath the
    // request.
    exporter.reset();
    let request = Request::builder()
        .method("POST")
        .uri("/api/tasks")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"id": 3, "project_id": 1}"#))
        .unwrap();
    let (status, body) = send(&app.router, request).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let spans = finished(&exporter);
    let request = request_span(&spans);
    assert_eq!(request.name, "POST /api/{*path}");
    assert_eq!(
        named(&spans, "db.commit").parent_span_id,
        request.span_context.span_id()
    );
    assert_eq!(
        attribute(named(&spans, "db.query"), "db.operation.name").unwrap(),
        "insert"
    );

    drop_schema().await;
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires PostgreSQL"]
async fn the_token_cache_says_whether_it_answered() {
    let _serial = SERIAL.lock().await;
    let app = application(postrust_server::Host::LAMBDA).await;
    let (exporter, _provider, _guard) = export();
    let authorization = format!("Bearer {}", token());

    let mut outcomes = Vec::new();
    for _ in 0..2 {
        let request = get("/api/projects")
            .header("authorization", &authorization)
            .body(Body::empty())
            .unwrap();
        let (status, body) = send(&app.router, request).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let spans = finished(&exporter);
        outcomes.push(attribute(named(&spans, "auth"), "jwt.cache").unwrap());
        assert_eq!(
            attribute(named(&spans, "auth"), "role").unwrap(),
            "web_anon"
        );
        exporter.reset();
    }
    assert_eq!(outcomes, ["miss", "hit"]);

    // And no token at all looks nothing up.
    send(
        &app.router,
        get("/api/projects").body(Body::empty()).unwrap(),
    )
    .await;
    let spans = finished(&exporter);
    assert_eq!(
        attribute(named(&spans, "auth"), "jwt.cache").unwrap(),
        "bypassed"
    );

    let stats = app.state.jwt_cache.as_ref().unwrap().stats();
    assert_eq!((stats.hits, stats.misses), (1, 1));
    drop_schema().await;
}

/// `GET /metrics` on the admin router, as Prometheus text.
async fn scrape(state: &Arc<postrust_server::AppState>) -> String {
    let admin = postrust_server::custom::admin_router().with_state(state.clone());
    let (status, text) = send(&admin, get("/metrics").body(Body::empty()).unwrap()).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    text
}

/// The value of the one series whose line starts with `series`.
fn value(text: &str, series: &str) -> Option<f64> {
    text.lines()
        .find(|line| line.starts_with(series))
        .and_then(|line| line.rsplit(' ').next())
        .and_then(|value| value.parse().ok())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires PostgreSQL"]
async fn metrics_are_served_on_the_admin_router_and_not_on_the_api() {
    let _serial = SERIAL.lock().await;
    assert!(otel::init_metrics(true));
    // A server host, for the `NOTIFY` reloader the reload metric counts.
    let app = application(postrust_server::Host::SERVER).await;
    let authorization = format!("Bearer {}", token());

    for _ in 0..3 {
        let request = get("/api/projects")
            .header("authorization", &authorization)
            .body(Body::empty())
            .unwrap();
        let (status, body) = send(&app.router, request).await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }

    // Not on the API's port: the API router has no such route.
    let (status, _) = send(&app.router, get("/metrics").body(Body::empty()).unwrap()).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = send(&app.router, get("/_/metrics").body(Body::empty()).unwrap()).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // A schema reload, which the reloader counts once it has swapped the
    // cache in.
    let pool = sqlx::PgPool::connect(&database_url()).await.unwrap();
    let mut text = String::new();
    for _ in 0..50 {
        sqlx::query("NOTIFY pgrst, 'reload schema'")
            .execute(&pool)
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        text = scrape(&app.state).await;
        if text.contains("postrust_schema_cache_reloads_total") {
            break;
        }
    }

    let route = r#"http_route="/api/{*path}""#;
    let ok = r#"http_response_status_code="200""#;
    let count = text
        .lines()
        .find(|line| {
            line.starts_with("http_server_request_duration_seconds_count")
                && line.contains(route)
                && line.contains(ok)
        })
        .unwrap_or_else(|| panic!("no request duration for the API in\n{text}"));
    assert!(count.contains(r#"http_request_method="GET""#), "{count}");
    assert!(
        count.rsplit(' ').next().unwrap().parse::<f64>().unwrap() >= 3.0,
        "{count}"
    );
    assert!(text.contains("http_server_request_duration_seconds_bucket{"));
    // Every request seen has finished.
    assert_eq!(
        value(
            &text,
            r#"http_server_active_requests{http_request_method="GET"}"#
        ),
        Some(0.0),
        "{text}"
    );

    // The pool: four at most, as configured, and some open.
    assert_eq!(
        value(
            &text,
            r#"db_client_connection_max{db_client_connection_pool_name="postrust"}"#
        ),
        Some(4.0),
        "{text}"
    );
    assert!(
        text.contains(r#"db_client_connection_state="idle""#),
        "{text}"
    );
    assert!(
        text.contains(r#"db_client_connection_state="used""#),
        "{text}"
    );
    assert!(
        value(
            &text,
            r#"db_client_connection_wait_time_seconds_count{db_client_connection_pool_name="postrust"}"#
        )
        .unwrap_or_default()
            >= 3.0,
        "{text}"
    );

    // One token, three requests: one miss, then hits.
    assert_eq!(
        value(&text, r#"postrust_jwt_cache_lookups_total{outcome="miss"}"#),
        Some(1.0),
        "{text}"
    );
    assert_eq!(
        value(&text, r#"postrust_jwt_cache_lookups_total{outcome="hit"}"#),
        Some(2.0),
        "{text}"
    );

    assert!(
        value(
            &text,
            r#"postrust_schema_cache_reloads_total{outcome="success"}"#
        )
        .unwrap_or_default()
            >= 1.0,
        "{text}"
    );
    #[cfg(feature = "graphql")]
    assert_eq!(
        value(&text, "postrust_graphql_subscriptions_active"),
        Some(0.0),
        "{text}"
    );

    drop_schema().await;
}

/// A JWKS of one fixture key, served on a port of its own, and its URL.
async fn identity_provider() -> String {
    let keys: serde_json::Value =
        serde_json::from_str(include_str!("../../postrust-auth/tests/fixtures/keys.json")).unwrap();
    let mut jwk = keys["rsa_a"]["jwk"].clone();
    jwk["kid"] = serde_json::json!("a");
    jwk["alg"] = serde_json::json!("RS256");
    let body = serde_json::json!({ "keys": [jwk] }).to_string();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!(
        "http://{}/.well-known/jwks.json",
        listener.local_addr().unwrap()
    );
    let router = Router::new().route(
        "/.well-known/jwks.json",
        axum::routing::get(move || async move { body }),
    );
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    url
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires PostgreSQL"]
async fn the_jwks_refresher_is_measured() {
    let _serial = SERIAL.lock().await;
    assert!(otel::init_metrics(true));

    let mut config = postrust_core::AppConfig::default();
    config.db_uri = database_url();
    config.db_schemas = vec!["public".to_string()];
    config.db_anon_role = Some("web_anon".to_string());
    config.db_pool_size = 2;
    config.jwt_jwks_url = Some(identity_provider().await);
    let app = postrust_server::build(config, postrust_server::Host::LAMBDA)
        .await
        .unwrap();

    // Fetched once at start-up, and not failed.
    let text = scrape(&app.state).await;
    assert_eq!(
        value(
            &text,
            r#"postrust_jwt_jwks_refreshes_total{outcome="success"}"#
        ),
        Some(1.0),
        "{text}"
    );
    assert_eq!(
        value(
            &text,
            r#"postrust_jwt_jwks_refreshes_total{outcome="failure"}"#
        ),
        Some(0.0),
        "{text}"
    );
    let age = value(&text, "postrust_jwt_jwks_key_set_age_seconds").expect(&text);
    assert!((0.0..60.0).contains(&age), "{text}");
}
