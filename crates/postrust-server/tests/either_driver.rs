//! What the REST API answers is the same whichever driver carries it.
//!
//! The server runs requests on sqlx; built with `pg-socket` it runs them on
//! the socket driver a Cloudflare Worker uses, which pipelines a request's
//! statements. These tests are about the answers that pipelining could
//! change, and are run under both builds:
//!
//! ```text
//! cargo test --package postrust-server --test either_driver -- --ignored
//! cargo test --package postrust-server --features pg-socket --test either_driver -- --ignored
//! ```
//!
//! `DATABASE_URL` must name a superuser on a database with
//! `scripts/init-db.sql` loaded (for `web_anon`).

mod proxy;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::any;
use axum::Router;
use postrust_core::{AppConfig, SchemaCache};
use postrust_server::{handle_request, AppState};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::sync::Arc;
use tower::ServiceExt;

const SCHEMA: &str = "either_driver";

fn database_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/postrust_test".to_string())
}

async fn pool(url: &str) -> PgPool {
    PgPoolOptions::new()
        .max_connections(2)
        .connect_lazy(url)
        .expect("a database URL")
}

/// A table of three rows, and a pre-request function that sets a header.
/// Made under a lock, since the tests here run at the same time, and only
/// when the marker on the schema names an older version of it.
async fn fixture(pool: &PgPool) {
    sqlx::query(&format!(
        "DO $$ BEGIN
           PERFORM pg_advisory_xact_lock(hashtext('{SCHEMA}'));
           IF obj_description(to_regnamespace('{SCHEMA}'), 'pg_namespace')
              IS NOT DISTINCT FROM 'fixture v1' THEN
             RETURN;
           END IF;
           DROP SCHEMA IF EXISTS {SCHEMA} CASCADE;
           CREATE SCHEMA {SCHEMA};
           GRANT USAGE ON SCHEMA {SCHEMA} TO web_anon;
           CREATE TABLE {SCHEMA}.items (id int PRIMARY KEY);
           INSERT INTO {SCHEMA}.items VALUES (1), (2), (3);
           GRANT SELECT ON {SCHEMA}.items TO web_anon;
           CREATE FUNCTION {SCHEMA}.pre() RETURNS void LANGUAGE plpgsql AS $f$ BEGIN
             PERFORM set_config('response.headers',
               '[{{\"Cache-Control\": \"public, max-age=60\"}}]', true);
           END $f$;
           COMMENT ON SCHEMA {SCHEMA} IS 'fixture v1';
         END $$"
    ))
    .execute(pool)
    .await
    .expect("the fixture");
}

/// The server, its requests going to `url`, whichever driver this build uses.
async fn app(url: &str, configure: impl FnOnce(&mut AppConfig)) -> Router {
    let direct = pool(&database_url()).await;
    fixture(&direct).await;

    let mut config = AppConfig::default();
    config.db_uri = url.to_string();
    config.db_schemas = vec![SCHEMA.to_string()];
    config.db_anon_role = Some("web_anon".to_string());
    configure(&mut config);
    let schema_cache = SchemaCache::load(&direct, &config.db_schemas)
        .await
        .expect("schema cache");
    let state = Arc::new({
        let mut state = AppState::new(pool(url).await, schema_cache, config)
            .expect("the test's JWT secret parses");
        // Uncached, so a test sees each request validated as it was written.
        state.jwt_cache = None;
        state
    });
    let api: Router<Arc<AppState>> = Router::new()
        .route("/", any(handle_request))
        .route("/{*path}", any(handle_request));
    Router::new().nest("/api", api).with_state(state)
}

async fn get(
    app: Router,
    uri: &str,
    headers: &[(&str, &str)],
) -> (StatusCode, http::HeaderMap, String) {
    let mut request = Request::builder().uri(uri);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let response = app
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, headers, String::from_utf8_lossy(&body).into_owned())
}

/// A pre-request function can set a header on a plain read, as PostgREST
/// lets it. The settings used to be read back only after a call or a write,
/// so on a read the header was dropped.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_header_set_by_the_pre_request_function_reaches_a_read() {
    let with_pre_request = app(&database_url(), |config| {
        config.db_pre_request = Some(format!("{SCHEMA}.pre"))
    })
    .await;

    // A plain read, which commits with its query, and one with a count to
    // take after it.
    for prefer in [&[][..], &[("Prefer", "count=exact")][..]] {
        let (status, headers, body) = get(with_pre_request.clone(), "/api/items", prefer).await;
        assert!(status.is_success(), "{prefer:?}: {body}");
        assert_eq!(
            headers.get("cache-control").and_then(|v| v.to_str().ok()),
            Some("public, max-age=60"),
            "{prefer:?}: {headers:?}"
        );
    }

    // And without one, no header and the same rows.
    let plain = app(&database_url(), |_| {}).await;
    let (status, headers, body) = get(plain, "/api/items", &[]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(!headers.contains_key("cache-control"), "{headers:?}");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&body).unwrap(),
        serde_json::json!([{"id": 1}, {"id": 2}, {"id": 3}])
    );
}

/// A connection that breaks once it is open is the database being
/// unavailable, not a fault in the query. The socket driver sends `BEGIN`
/// with the first statement, so that is where it finds out; it used to be
/// reported as an internal error there, where sqlx -- which sends `BEGIN`
/// first, on its own -- reported it as the connection's.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_connection_closed_under_a_request_is_the_database_unavailable() {
    let proxy = proxy::Proxy::start(
        proxy::host_and_port(&database_url()),
        proxy::Behaviour::CloseAtTheFirstStatement,
    )
    .await;
    let app = app(&proxy.url(&database_url()), |_| {}).await;

    let (status, _, body) = get(app, "/api/items", &[]).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
    let body: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(body["code"], "PGRST500", "{body}");
    assert_eq!(body["message"], "Service temporarily unavailable", "{body}");
}
