//! A query the response reports as the client's is not logged as an error.
//!
//! A schema that answers "not found" by raising `PT404` -- or that has a check
//! constraint, or a unique key -- produced `ERROR Query error: ...` for every
//! such request, which buries the failures that are faults. Those refusals are
//! now logged at debug. A query that fails as the server's fault still logs
//! at error -- including the faults answered with a 4xx, such as a missing
//! grant.
//!
//! `DATABASE_URL` must name a superuser on a database with
//! `scripts/init-db.sql` loaded (for `web_anon`). Run with:
//!
//! ```text
//! cargo test --package postrust-server --test query_error_logging -- --ignored
//! ```

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::any;
use axum::Router;
use postrust_core::{AppConfig, SchemaCache};
use postrust_server::{handle_request, AppState};
use sqlx::postgres::PgPoolOptions;
use std::io::Write;
use std::sync::{Arc, Mutex};
use tower::ServiceExt;

const SCHEMA: &str = "query_error_logging";

fn database_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/postrust_test".to_string())
}

/// Everything logged while it is the default subscriber.
#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Captured {
    fn take(&self) -> String {
        String::from_utf8(std::mem::take(&mut *self.0.lock().unwrap())).unwrap()
    }
}

async fn app() -> Router {
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&database_url())
        .await
        .expect("failed to connect to test database");
    for statement in [
        format!("DROP SCHEMA IF EXISTS {SCHEMA} CASCADE"),
        format!("CREATE SCHEMA {SCHEMA}"),
        format!("GRANT USAGE ON SCHEMA {SCHEMA} TO web_anon"),
        format!("CREATE TABLE {SCHEMA}.items (id int PRIMARY KEY, n int NOT NULL CHECK (n >= 0))"),
        format!("INSERT INTO {SCHEMA}.items VALUES (1, 0)"),
        format!("GRANT SELECT, INSERT ON {SCHEMA}.items TO web_anon"),
        // Listed in the schema cache, readable by no one but its owner.
        format!("CREATE TABLE {SCHEMA}.ungranted (id int PRIMARY KEY)"),
        format!(
            "CREATE FUNCTION {SCHEMA}.missing() RETURNS void LANGUAGE plpgsql AS \
             $$ BEGIN RAISE EXCEPTION 'no such thing' USING ERRCODE = 'PT404'; END $$"
        ),
        format!(
            "CREATE FUNCTION {SCHEMA}.broken() RETURNS void LANGUAGE plpgsql AS \
             $$ BEGIN RAISE EXCEPTION 'something broke' USING ERRCODE = 'XX000'; END $$"
        ),
        format!("GRANT EXECUTE ON ALL FUNCTIONS IN SCHEMA {SCHEMA} TO web_anon"),
    ] {
        sqlx::query(&statement).execute(&pool).await.unwrap();
    }

    let mut config = AppConfig::default();
    config.db_uri = database_url();
    config.db_schemas = vec![SCHEMA.to_string()];
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
    Router::new().nest("/api", api).with_state(state)
}

async fn post(app: &Router, uri: &str, body: &str) -> StatusCode {
    let request = Request::builder()
        .method("POST")
        .uri(uri)
        .header("Content-Type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    app.clone().oneshot(request).await.unwrap().status()
}

async fn get(app: &Router, uri: &str) -> StatusCode {
    let request = Request::builder().uri(uri).body(Body::empty()).unwrap();
    app.clone().oneshot(request).await.unwrap().status()
}

/// The query failures logged at error level. Only this handler's own line,
/// so an unrelated error elsewhere does not change the count.
fn error_lines(log: &str) -> Vec<&str> {
    log.lines()
        .filter(|l| l.contains("ERROR") && l.contains("Query error:"))
        .collect()
}

// A current-thread runtime, so the request runs on the thread whose default
// subscriber is the capturing one.
#[tokio::test(flavor = "current_thread")]
#[ignore = "requires PostgreSQL"]
async fn client_errors_log_at_debug_and_faults_at_error() {
    let app = app().await;
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);

    // Three refusals the client earned: PT404, a check constraint, a duplicate key.
    assert_eq!(
        post(&app, "/api/rpc/missing", "{}").await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        post(&app, "/api/items", r#"{"id":2,"n":-1}"#).await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        post(&app, "/api/items", r#"{"id":1,"n":0}"#).await,
        StatusCode::CONFLICT
    );
    let log = captured.take();
    assert!(
        error_lines(&log).is_empty(),
        "client errors logged at error level:\n{log}"
    );
    for expected in [
        "Query refused with 404",
        "no such thing",
        "Query refused with 400",
        "Query refused with 409",
    ] {
        assert!(log.contains(expected), "missing {expected:?} in:\n{log}");
    }

    // A fault is still an error, and so is one answered with a 4xx: a missing
    // grant is the deployment's to fix, not the client's.
    assert_eq!(
        post(&app, "/api/rpc/broken", "{}").await,
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert!(get(&app, "/api/ungranted").await.is_client_error());
    let log = captured.take();
    let errors = error_lines(&log);
    assert_eq!(errors.len(), 2, "{log}");
    assert!(errors[0].contains("something broke"), "{log}");
    assert!(errors[1].contains("permission denied"), "{log}");

    let pool = sqlx::PgPool::connect(&database_url()).await.unwrap();
    sqlx::query(&format!("DROP SCHEMA {SCHEMA} CASCADE"))
        .execute(&pool)
        .await
        .unwrap();
}

/// A transaction that cannot be begun is answered as the server's fault, and
/// the operator is told why.
///
/// The pool's one connection is held for the length of the request, so
/// acquiring another times out. Before, nothing logged this at any level: the
/// query's own logging is never reached when there is no transaction to run
/// it in.
#[tokio::test(flavor = "current_thread")]
#[ignore = "requires PostgreSQL"]
async fn a_pool_that_runs_dry_is_logged_at_error() {
    // A schema of its own: the tests in this file run at once, and the other
    // one drops and recreates its schema as it starts.
    const DRY: &str = "query_error_logging_pool";
    let setup = PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url())
        .await
        .expect("failed to connect to test database");
    for statement in [
        format!("DROP SCHEMA IF EXISTS {DRY} CASCADE"),
        format!("CREATE SCHEMA {DRY}"),
        format!("GRANT USAGE ON SCHEMA {DRY} TO web_anon"),
        format!("CREATE TABLE {DRY}.items (id int PRIMARY KEY)"),
        format!("GRANT SELECT ON {DRY}.items TO web_anon"),
    ] {
        sqlx::query(&statement).execute(&setup).await.unwrap();
    }
    setup.close().await;

    let pool = PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(std::time::Duration::from_millis(300))
        .connect(&database_url())
        .await
        .expect("failed to connect to test database");
    let mut config = AppConfig::default();
    config.db_uri = database_url();
    config.db_schemas = vec![DRY.to_string()];
    config.db_anon_role = Some("web_anon".to_string());
    let schema_cache = SchemaCache::load(&pool, &config.db_schemas)
        .await
        .expect("schema cache");
    let held = pool.acquire().await.expect("the pool's one connection");
    let state =
        Arc::new(AppState::new(pool, schema_cache, config).expect("the test's JWT secret parses"));
    let api: Router<Arc<AppState>> = Router::new()
        .route("/", any(handle_request))
        .route("/{*path}", any(handle_request));
    let app = Router::new().nest("/api", api).with_state(state);

    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);

    assert_eq!(
        get(&app, "/api/items").await,
        StatusCode::INTERNAL_SERVER_ERROR
    );
    let log = captured.take();
    let lines: Vec<&str> = log
        .lines()
        .filter(|l| l.contains("ERROR") && l.contains("Could not begin a transaction"))
        .collect();
    assert_eq!(lines.len(), 1, "{log}");

    drop(held);
    let cleanup = PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url())
        .await
        .expect("failed to connect to test database");
    sqlx::query(&format!("DROP SCHEMA {DRY} CASCADE"))
        .execute(&cleanup)
        .await
        .unwrap();
}
