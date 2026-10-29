//! Every request gets a span, a request id and a trace id.
//!
//! The tests without a database drive `RequestTraceLayer` around a stub
//! handler. The one with a database goes through `postrust_server::build` --
//! the router the binary and the Lambda adapter both serve -- and checks the
//! spans a REST request opens beneath its own: `auth`, `plan`, `db.query`,
//! `render`.
//!
//! `DATABASE_URL` must name a superuser on a database with
//! `scripts/init-db.sql` loaded (for `web_anon`). Run with:
//!
//! ```text
//! cargo test --package postrust-server --test request_tracing -- --include-ignored
//! ```

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::any;
use axum::Router;
use postrust_core::trace::TraceContext;
use postrust_server::telemetry::RequestTraceLayer;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use tower::ServiceExt;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::Layer;

/// Held by every test in this file, so none of them runs beside another.
///
/// `tracing` caches, per call site, whether any subscriber is interested, and
/// the cache is process-wide. A test that sends a request with no subscriber
/// on its thread, running while another installs its capturing one, can leave
/// the request span's call site cached as uninteresting just as the second
/// test needs it -- and that test then sees no span at all.
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

const TRACEPARENT: &str = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";

/// A span as it was seen: its name, its parent's name, and every field
/// recorded on it, at creation or after.
#[derive(Clone, Debug, Default)]
struct SeenSpan {
    name: &'static str,
    parent: Option<&'static str>,
    fields: BTreeMap<String, String>,
}

/// Collects every span, in the order they were opened.
#[derive(Clone, Default)]
struct Spans(Arc<Mutex<Vec<(u64, SeenSpan)>>>);

impl Spans {
    fn named(&self, name: &str) -> Vec<SeenSpan> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, s)| s.name == name)
            .map(|(_, s)| s.clone())
            .collect()
    }

    fn take(&self) -> Vec<SeenSpan> {
        std::mem::take(&mut *self.0.lock().unwrap())
            .into_iter()
            .map(|(_, s)| s)
            .collect()
    }
}

struct Fields<'a>(&'a mut BTreeMap<String, String>);

impl Visit for Fields<'_> {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.insert(field.name().to_string(), value.to_string());
    }
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.0
            .insert(field.name().to_string(), format!("{value:?}"));
    }
}

impl<S> Layer<S> for Spans
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let mut seen = SeenSpan {
            name: attrs.metadata().name(),
            parent: ctx
                .span(id)
                .and_then(|s| s.parent())
                .map(|p| p.metadata().name()),
            ..Default::default()
        };
        attrs.record(&mut Fields(&mut seen.fields));
        self.0.lock().unwrap().push((id.into_u64(), seen));
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, _ctx: Context<'_, S>) {
        let mut spans = self.0.lock().unwrap();
        if let Some((_, seen)) = spans.iter_mut().rev().find(|(i, _)| *i == id.into_u64()) {
            values.record(&mut Fields(&mut seen.fields));
        }
    }
}

/// A subscriber that sees every span at debug and above, for as long as the
/// guard lives. The tests run on a current-thread runtime so the request runs
/// on this thread.
fn capture() -> (Spans, tracing::subscriber::DefaultGuard) {
    let spans = Spans::default();
    let subscriber = tracing_subscriber::registry().with(
        spans
            .clone()
            .with_filter(tracing_subscriber::filter::LevelFilter::DEBUG),
    );
    (spans, tracing::subscriber::set_default(subscriber))
}

/// A stub that answers with what the layer put in the extensions.
fn stub() -> Router {
    Router::new()
        .route(
            "/api/{*path}",
            any(|request: Request<Body>| async move {
                let trace = request.extensions().get::<TraceContext>().cloned();
                let status = match request.uri().path() {
                    "/api/broken" => StatusCode::INTERNAL_SERVER_ERROR,
                    _ => StatusCode::OK,
                };
                (
                    status,
                    trace.map(|t| t.trace_id.to_string()).unwrap_or_default(),
                )
            }),
        )
        .layer(RequestTraceLayer)
}

async fn send(app: &Router, request: Request<Body>) -> (StatusCode, http::HeaderMap, String) {
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, headers, String::from_utf8(body.to_vec()).unwrap())
}

fn get(uri: &str) -> http::request::Builder {
    Request::builder().uri(uri)
}

#[tokio::test(flavor = "current_thread")]
async fn a_request_id_is_generated_when_absent() {
    let _serial = SERIAL.lock().await;
    let (_, headers, _) = send(&stub(), get("/api/items").body(Body::empty()).unwrap()).await;
    let id = headers["x-request-id"].to_str().unwrap();
    assert_eq!(id.len(), 36, "{id}");
}

#[tokio::test(flavor = "current_thread")]
async fn a_request_id_is_echoed_when_present() {
    let _serial = SERIAL.lock().await;
    let request = get("/api/items")
        .header("x-request-id", "client-chose-this")
        .body(Body::empty())
        .unwrap();
    let (_, headers, _) = send(&stub(), request).await;
    assert_eq!(headers["x-request-id"], "client-chose-this");
}

#[tokio::test(flavor = "current_thread")]
async fn an_invalid_request_id_is_replaced() {
    let _serial = SERIAL.lock().await;
    let request = get("/api/items")
        .header("x-request-id", "two words")
        .body(Body::empty())
        .unwrap();
    let (_, headers, _) = send(&stub(), request).await;
    let id = headers["x-request-id"].to_str().unwrap();
    assert_ne!(id, "two words");
    assert_eq!(id.len(), 36);
}

#[tokio::test(flavor = "current_thread")]
async fn a_traceparent_names_the_trace() {
    let _serial = SERIAL.lock().await;
    let request = get("/api/items")
        .header("traceparent", TRACEPARENT)
        .body(Body::empty())
        .unwrap();
    let (_, _, trace_id) = send(&stub(), request).await;
    assert_eq!(trace_id, "4bf92f3577b34da6a3ce929d0e0e4736");

    // Version ff is refused, and the request starts a trace of its own.
    let request = get("/api/items")
        .header("traceparent", TRACEPARENT.replacen("00", "ff", 1))
        .body(Body::empty())
        .unwrap();
    let (_, _, trace_id) = send(&stub(), request).await;
    assert_eq!(trace_id.len(), 32);
    assert_ne!(trace_id, "4bf92f3577b34da6a3ce929d0e0e4736");
}

#[tokio::test(flavor = "current_thread")]
async fn the_request_span_follows_the_http_conventions() {
    let _serial = SERIAL.lock().await;
    let (spans, _guard) = capture();
    let request = get("/api/items?id=eq.1")
        .header("traceparent", TRACEPARENT)
        .header("x-request-id", "abc")
        .body(Body::empty())
        .unwrap();
    send(&stub(), request).await;

    let request = spans.named("request");
    assert_eq!(request.len(), 1);
    let fields = &request[0].fields;
    assert_eq!(fields["http.request.method"], "GET");
    assert_eq!(fields["url.path"], "/api/items");
    // The route, not the path.
    assert_eq!(fields["http.route"], "/api/{*path}");
    assert_eq!(fields["http.response.status_code"], "200");
    assert_eq!(fields["otel.kind"], "server");
    assert_eq!(fields["request_id"], "abc");
    assert_eq!(fields["trace_id"], "4bf92f3577b34da6a3ce929d0e0e4736");
    // A success leaves the status unset.
    assert!(!fields.contains_key("otel.status_code"));
    spans.take();

    send(&stub(), get("/api/broken").body(Body::empty()).unwrap()).await;
    let request = spans.named("request");
    assert_eq!(request[0].fields["http.response.status_code"], "500");
    assert_eq!(request[0].fields["otel.status_code"], "ERROR");
    spans.take();

    // Nothing matched: no route is named rather than the path standing in
    // for one. The span and the request id are still there.
    let (status, headers, _) = send(&stub(), get("/nowhere").body(Body::empty()).unwrap()).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(headers.contains_key("x-request-id"));
    let request = spans.named("request");
    assert_eq!(request.len(), 1);
    assert!(!request[0].fields.contains_key("http.route"));
}

// ---------------------------------------------------------------------------
// Through the whole application, with a database.
// ---------------------------------------------------------------------------

const SCHEMA: &str = "request_tracing";

fn database_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/postrust_test".to_string())
}

async fn application(otel_db_statement: bool) -> Router {
    let pool = sqlx::PgPool::connect(&database_url()).await.unwrap();
    for statement in [
        format!("DROP SCHEMA IF EXISTS {SCHEMA} CASCADE"),
        format!("CREATE SCHEMA {SCHEMA}"),
        format!("GRANT USAGE ON SCHEMA {SCHEMA} TO web_anon"),
        format!("CREATE TABLE {SCHEMA}.items (id int PRIMARY KEY)"),
        format!("INSERT INTO {SCHEMA}.items VALUES (1)"),
        format!("GRANT SELECT ON {SCHEMA}.items TO web_anon"),
    ] {
        sqlx::query(&statement).execute(&pool).await.unwrap();
    }

    let mut config = postrust_core::AppConfig::default();
    config.db_uri = database_url();
    config.db_schemas = vec![SCHEMA.to_string()];
    config.db_anon_role = Some("web_anon".to_string());
    config.db_pool_size = 2;
    config.otel_db_statement = otel_db_statement;
    postrust_server::build(config, postrust_server::Host::LAMBDA)
        .await
        .unwrap()
        .router
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires PostgreSQL"]
async fn a_rest_request_opens_its_stages_beneath_the_request_span() {
    let _serial = SERIAL.lock().await;
    let app = application(false).await;
    let (spans, _guard) = capture();

    let request = get("/api/items?id=eq.1")
        .header("traceparent", TRACEPARENT)
        .body(Body::empty())
        .unwrap();
    let (status, headers, body) = send(&app, request).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, r#"[{"id":1}]"#);
    assert!(headers.contains_key("x-request-id"));

    let seen = spans.take();
    let request = seen.iter().find(|s| s.name == "request").unwrap();
    assert_eq!(request.fields["http.route"], "/api/{*path}");
    assert_eq!(request.fields["http.response.status_code"], "200");
    assert_eq!(
        request.fields["trace_id"],
        "4bf92f3577b34da6a3ce929d0e0e4736"
    );

    for stage in ["auth", "plan", "db.query", "render"] {
        let span = seen
            .iter()
            .find(|s| s.name == stage)
            .unwrap_or_else(|| panic!("no {stage} span in {seen:#?}"));
        assert_eq!(span.parent, Some("request"), "{stage}");
    }
    let auth = seen.iter().find(|s| s.name == "auth").unwrap();
    assert_eq!(auth.fields["role"], "web_anon");
    let db = seen.iter().find(|s| s.name == "db.query").unwrap();
    assert_eq!(db.fields["db.system"], "postgresql");
    assert_eq!(db.fields["db.operation.name"], "select");
    assert_eq!(db.fields["db.collection.name"], format!("{SCHEMA}.items"));
    // Off unless asked for.
    assert!(!db.fields.contains_key("db.query.text"), "{db:#?}");

    // GraphQL is served by the same router and gets the same span; what it
    // does inside is its own business for now.
    #[cfg(feature = "graphql")]
    {
        spans.take();
        let request = Request::builder()
            .method("POST")
            .uri("/v1/graphql")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"query":"{ __typename }"}"#))
            .unwrap();
        let (status, headers, body) = send(&app, request).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(headers.contains_key("x-request-id"));
        let request = spans.named("request").pop().unwrap();
        assert!(
            request.fields["http.route"].starts_with("/v1/graphql"),
            "{request:#?}"
        );
    }

    // Asked for, the statement is there -- and the value the client filtered
    // on is not, because it is a parameter.
    let app = application(true).await;
    spans.take();
    let request = get("/api/items?id=eq.424242").body(Body::empty()).unwrap();
    send(&app, request).await;
    let db = spans.named("db.query").pop().unwrap();
    let text = &db.fields["db.query.text"];
    assert!(text.contains("SELECT"), "{text}");
    assert!(!text.contains("424242"), "{text}");

    let pool = sqlx::PgPool::connect(&database_url()).await.unwrap();
    sqlx::query(&format!("DROP SCHEMA {SCHEMA} CASCADE"))
        .execute(&pool)
        .await
        .unwrap();
}
