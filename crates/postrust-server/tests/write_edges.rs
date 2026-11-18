//! Edges of the write path where Postrust once differed from PostgREST: an
//! empty form body, the status of an upsert that created nothing (on views and
//! partitioned tables, where `xmax` is unavailable), `Range` on a non-GET, and
//! `max-affected` with `handling=strict` on a function returning one value.
//!
//! `DATABASE_URL` must name a superuser on a database with
//! `scripts/init-db.sql` loaded (for `web_anon`). Run with:
//!
//! ```text
//! cargo test --package postrust-server --test write_edges -- --ignored
//! ```

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::any;
use axum::Router;
use postrust_core::{AppConfig, SchemaCache};
use postrust_server::{handle_request, AppState};
use sqlx::postgres::PgPoolOptions;
use sqlx::AssertSqlSafe;
use sqlx::PgPool;
use std::sync::Arc;
use tower::ServiceExt;

/// Each test's own schema, so tests can run at the same time.
fn schema(test: &str) -> String {
    format!("write_edges_{test}")
}

fn database_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/postrust_test".to_string())
}

async fn pool() -> PgPool {
    PgPoolOptions::new()
        .max_connections(2)
        .connect(&database_url())
        .await
        .expect("failed to connect to test database")
}

/// A fresh schema with a table, a view over it, a partitioned table, and the
/// functions the tests call, all open to `web_anon`.
async fn setup(pool: &PgPool, test: &str) {
    let schema = schema(test);
    for statement in [
        "DROP SCHEMA IF EXISTS {schema} CASCADE",
        "CREATE SCHEMA {schema}",
        "GRANT USAGE ON SCHEMA {schema} TO web_anon",
        "CREATE TABLE {schema}.items (id int PRIMARY KEY, v text)",
        "CREATE VIEW {schema}.items_v AS SELECT id, v FROM {schema}.items",
        "CREATE TABLE {schema}.parts (id int, y int, v text, PRIMARY KEY (id, y)) PARTITION BY RANGE (y)",
        "CREATE TABLE {schema}.parts_1 PARTITION OF {schema}.parts FOR VALUES FROM (0) TO (10)",
        "CREATE TABLE {schema}.parts_2 PARTITION OF {schema}.parts FOR VALUES FROM (10) TO (20)",
        "CREATE TABLE {schema}.defaults (id serial PRIMARY KEY, note text DEFAULT 'x')",
        "CREATE FUNCTION {schema}.nothing() RETURNS void LANGUAGE sql AS $$ SELECT $$",
        "CREATE FUNCTION {schema}.numbers() RETURNS SETOF int LANGUAGE sql AS $$ SELECT generate_series(1, 3) $$",
        "GRANT ALL ON ALL TABLES IN SCHEMA {schema} TO web_anon",
        "GRANT ALL ON ALL SEQUENCES IN SCHEMA {schema} TO web_anon",
        "GRANT EXECUTE ON ALL FUNCTIONS IN SCHEMA {schema} TO web_anon",
    ] {
        sqlx::query(AssertSqlSafe(statement.replace("{schema}", &schema)))
            .execute(pool)
            .await
            .unwrap();
    }
}

async fn app(pool: PgPool, test: &str) -> Router {
    let mut config = AppConfig::default();
    config.db_uri = database_url();
    config.db_schemas = vec![schema(test)];
    config.db_anon_role = Some("web_anon".to_string());
    let schema_cache = SchemaCache::load(&pool, &config.db_schemas)
        .await
        .expect("schema cache");
    let state = Arc::new({
        let mut state =
            AppState::new(pool, schema_cache, config).expect("the test's JWT secret parses");
        // Uncached, so a test sees each request validated as it was written.
        state.jwt_cache = None;
        state
    });
    let api: Router<Arc<AppState>> = Router::new()
        .route("/", any(handle_request))
        .route("/{*path}", any(handle_request));
    // `Allow` on an OPTIONS is added by this layer, as in the server.
    Router::new()
        .nest("/api", api)
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            postrust_server::app::options_allow,
        ))
        .with_state(state)
}

struct Answer {
    status: StatusCode,
    body: String,
}

async fn send(
    app: &Router,
    method: &str,
    uri: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> Answer {
    let mut builder = Request::builder().method(method).uri(uri);
    if !headers
        .iter()
        .any(|(k, _)| k.eq_ignore_ascii_case("content-type"))
    {
        builder = builder.header("Content-Type", "application/json");
    }
    for (k, v) in headers {
        builder = builder.header(*k, *v);
    }
    let response = app
        .clone()
        .oneshot(builder.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    Answer {
        status,
        body: String::from_utf8_lossy(&body).into_owned(),
    }
}

const MERGE: &[(&str, &str)] = &[(
    "Prefer",
    "resolution=merge-duplicates, return=representation",
)];
const PUT: &[(&str, &str)] = &[("Prefer", "return=representation")];

/// A merge that created nothing is a 200 and one that created anything is a
/// 201, on a view and on a partitioned table, where `xmax` cannot say.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn post_merge_duplicates_status_follows_what_was_created() {
    let pool = pool().await;
    setup(&pool, "post").await;
    let app = app(pool.clone(), "post").await;

    for (name, first, again, mixed, empty) in [
        (
            "items_v",
            r#"[{"id":1,"v":"a"}]"#,
            r#"[{"id":1,"v":"b"}]"#,
            r#"[{"id":1,"v":"c"},{"id":2,"v":"d"}]"#,
            "[]",
        ),
        (
            "parts",
            r#"[{"id":1,"y":1,"v":"a"}]"#,
            r#"[{"id":1,"y":1,"v":"b"}]"#,
            r#"[{"id":1,"y":1,"v":"c"},{"id":2,"y":11,"v":"d"}]"#,
            "[]",
        ),
        (
            "items",
            r#"[{"id":11,"v":"a"}]"#,
            r#"[{"id":11,"v":"b"}]"#,
            r#"[{"id":11,"v":"c"},{"id":12,"v":"d"}]"#,
            "[]",
        ),
    ] {
        let uri = match name {
            // A view has no key of its own to conflict on.
            "items_v" => format!("/api/{name}?on_conflict=id"),
            _ => format!("/api/{name}"),
        };
        let created = send(&app, "POST", &uri, MERGE, first).await;
        assert_eq!(
            created.status,
            StatusCode::CREATED,
            "{name}: {}",
            created.body
        );
        let merged = send(&app, "POST", &uri, MERGE, again).await;
        assert_eq!(merged.status, StatusCode::OK, "{name}: {}", merged.body);
        let both = send(&app, "POST", &uri, MERGE, mixed).await;
        assert_eq!(both.status, StatusCode::CREATED, "{name}: {}", both.body);
        let none = send(&app, "POST", &uri, MERGE, empty).await;
        assert_eq!(none.status, StatusCode::OK, "{name}: {}", none.body);
    }
}

/// A view with no key still answers 200 for a merge that wrote nothing.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn an_empty_merge_on_a_keyless_view_is_200() {
    let pool = pool().await;
    setup(&pool, "empty").await;
    let app = app(pool.clone(), "empty").await;
    let none = send(&app, "POST", "/api/items_v", MERGE, "[]").await;
    assert_eq!(none.status, StatusCode::OK, "{}", none.body);
}

/// A `PUT` is 201 for a row it created and 200 for one it replaced.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn put_status_follows_what_was_created() {
    let pool = pool().await;
    setup(&pool, "put").await;
    let app = app(pool.clone(), "put").await;

    for (name, filter, body) in [
        ("parts", "id=eq.1&y=eq.1", r#"{"id":1,"y":1,"v":"a"}"#),
        ("items", "id=eq.1", r#"{"id":1,"v":"a"}"#),
    ] {
        let uri = format!("/api/{name}?{filter}");
        let created = send(&app, "PUT", &uri, PUT, body).await;
        assert_eq!(
            created.status,
            StatusCode::CREATED,
            "{name}: {}",
            created.body
        );
        let replaced = send(&app, "PUT", &uri, PUT, body).await;
        assert_eq!(replaced.status, StatusCode::OK, "{name}: {}", replaced.body);
        // `Range` is for GET alone: here it is no reason to refuse.
        let ranged = send(
            &app,
            "PUT",
            &uri,
            &[("Prefer", "return=representation"), ("Range", "1-1")],
            body,
        )
        .await;
        assert_eq!(ranged.status, StatusCode::OK, "{name}: {}", ranged.body);
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_range_header_is_read_for_get_only() {
    let pool = pool().await;
    setup(&pool, "range").await;
    let app = app(pool.clone(), "range").await;

    // Not a range at all for a POST: neither 400 nor 416.
    let posted = send(
        &app,
        "POST",
        "/api/items",
        &[("Range", "5-1")],
        r#"{"id":1,"v":"a"}"#,
    )
    .await;
    assert_eq!(posted.status, StatusCode::CREATED, "{}", posted.body);
    let got = send(&app, "GET", "/api/items", &[("Range", "5-1")], "").await;
    assert!(got.status.is_client_error(), "{}: {}", got.status, got.body);
}

/// An empty urlencoded body is `{}`: a row of defaults.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn an_empty_urlencoded_body_inserts_defaults() {
    let pool = pool().await;
    setup(&pool, "form").await;
    let app = app(pool.clone(), "form").await;

    let form = [("Content-Type", "application/x-www-form-urlencoded")];
    let created = send(&app, "POST", "/api/defaults", &form, "").await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
    // The other media types still refuse an empty body.
    let json = send(&app, "POST", "/api/defaults", &[], "").await;
    assert_eq!(json.status, StatusCode::BAD_REQUEST, "{}", json.body);
    assert!(json.body.contains("PGRST102"), "{}", json.body);
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn max_affected_strict_on_a_function_that_returns_one_value() {
    let pool = pool().await;
    setup(&pool, "affected").await;
    let app = app(pool.clone(), "affected").await;

    let strict = [("Prefer", "handling=strict, max-affected=20")];
    let refused = send(&app, "POST", "/api/rpc/nothing", &strict, "{}").await;
    assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{}", refused.body);
    assert!(refused.body.contains("PGRST128"), "{}", refused.body);
    assert!(
        refused
            .body
            .contains("Function must return SETOF or TABLE when max-affected preference is used with handling=strict"),
        "{}",
        refused.body
    );

    // Not strict: no refusal. A set: nothing to refuse.
    let lenient = send(
        &app,
        "POST",
        "/api/rpc/nothing",
        &[("Prefer", "max-affected=20")],
        "{}",
    )
    .await;
    assert_eq!(lenient.status, StatusCode::NO_CONTENT, "{}", lenient.body);
    let set = send(&app, "POST", "/api/rpc/numbers", &strict, "{}").await;
    assert_eq!(set.status, StatusCode::OK, "{}", set.body);
}
