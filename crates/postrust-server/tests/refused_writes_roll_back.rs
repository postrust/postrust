//! A write the response refuses is rolled back.
//!
//! Some refusals can only be made once the statement has run, because they
//! are about what it did: `Prefer: max-affected` with `handling=strict`, and a
//! request for a single object that touched several rows. The client is told
//! the request failed, so the write must not have happened -- PostgREST rolls
//! it back. Postrust used to commit first and refuse afterwards, which left
//! the rows changed behind the error.
//!
//! `DATABASE_URL` must name a superuser on a database with
//! `scripts/init-db.sql` loaded (for `web_anon`). Run with:
//!
//! ```text
//! cargo test --package postrust-server --test refused_writes_roll_back -- --ignored
//! ```
//!
//! and again with `--features pg-socket`, which serves the same requests
//! through the socket driver a Worker uses: the transaction is that driver's,
//! and so is the rollback.

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

/// Each test's own schema, named after its table. One shared schema made
/// the tests race: two concurrent `CREATE SCHEMA IF NOT EXISTS` can both find
/// it missing, and the second fails on the catalog's unique index.
fn schema(table: &str) -> String {
    format!("refused_writes_{table}")
}

fn database_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/postrust_test".to_string())
}

/// A table of its own per test, five rows of `n = 0`, writable by `web_anon`.
///
/// The schema is dropped and made afresh, so a rerun starts clean whatever a
/// failed run left behind -- and a failure leaves its rows to look at.
async fn table(pool: &PgPool, name: &str) {
    let schema = schema(name);
    for statement in [
        format!("DROP SCHEMA IF EXISTS {schema} CASCADE"),
        format!("CREATE SCHEMA {schema}"),
        format!("GRANT USAGE ON SCHEMA {schema} TO web_anon"),
        format!("CREATE TABLE {schema}.{name} (id int PRIMARY KEY, n int NOT NULL)"),
        format!("INSERT INTO {schema}.{name} SELECT g, 0 FROM generate_series(1, 5) g"),
        format!("GRANT SELECT, UPDATE ON {schema}.{name} TO web_anon"),
    ] {
        sqlx::query(&statement).execute(pool).await.unwrap();
    }
}

/// `statements`, run in order, each with `{schema}` standing for the test's
/// own schema.
async fn run(pool: &PgPool, table: &str, statements: &[&str]) {
    let schema = schema(table);
    for statement in statements {
        sqlx::query(&statement.replace("{schema}", &schema))
            .execute(pool)
            .await
            .unwrap();
    }
}

async fn app(pool: PgPool, table: &str) -> Router {
    router(state(pool, table).await)
}

async fn state(pool: PgPool, table: &str) -> Arc<AppState> {
    state_with(pool, table, |_| {}).await
}

/// `state`, with `configure` given the configuration before it is used.
async fn state_with(
    pool: PgPool,
    table: &str,
    configure: impl FnOnce(&mut AppConfig),
) -> Arc<AppState> {
    let mut config = AppConfig::default();
    // Where a `pg-socket` build connects for each request.
    config.db_uri = database_url();
    configure(&mut config);
    config.db_schemas = vec![schema(table)];
    config.db_anon_role = Some("web_anon".to_string());
    let schema_cache = SchemaCache::load(&pool, &config.db_schemas)
        .await
        .expect("schema cache");
    Arc::new({
        let mut state =
            AppState::new(pool, schema_cache, config).expect("the test's JWT secret parses");
        // Uncached, so a test sees each request validated as it was written.
        state.jwt_cache = None;
        state
    })
}

fn router(state: Arc<AppState>) -> Router {
    let api: Router<Arc<AppState>> = Router::new()
        .route("/", any(handle_request))
        .route("/{*path}", any(handle_request));
    Router::new().nest("/api", api).with_state(state)
}

/// `PATCH` every row of `table` to `n = 1`, with `headers`.
async fn patch_all(app: Router, table: &str, headers: &[(&str, &str)]) -> StatusCode {
    let mut request = Request::builder()
        .method("PATCH")
        .uri(format!("/api/{table}"))
        .header("Content-Type", "application/json");
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    app.oneshot(request.body(Body::from(r#"{"n":1}"#)).unwrap())
        .await
        .unwrap()
        .status()
}

/// Send one request, and return its status and its body.
async fn send(
    app: Router,
    method: &str,
    uri: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> (StatusCode, String) {
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header("Content-Type", "application/json");
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let response = app
        .oneshot(request.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&body).into_owned())
}

async fn changed(pool: &PgPool, table: &str) -> i64 {
    let schema = schema(table);
    sqlx::query_scalar(&format!(
        "SELECT count(*) FROM {schema}.{table} WHERE n = 1"
    ))
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn pool() -> PgPool {
    PgPoolOptions::new()
        .max_connections(2)
        .connect(&database_url())
        .await
        .expect("failed to connect to test database")
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn more_rows_than_max_affected_allows_are_not_written() {
    let pool = pool().await;
    table(&pool, "max_affected").await;
    let app = app(pool.clone(), "max_affected").await;

    let status = patch_all(
        app.clone(),
        "max_affected",
        &[("Prefer", "max-affected=1, handling=strict")],
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        changed(&pool, "max_affected").await,
        0,
        "a refused write was committed"
    );

    // Within the limit, the same request writes.
    let status = patch_all(
        app,
        "max_affected",
        &[("Prefer", "max-affected=5, handling=strict")],
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(changed(&pool, "max_affected").await, 5);
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn several_rows_where_one_object_was_asked_for_are_not_written() {
    let pool = pool().await;
    table(&pool, "singular").await;
    let app = app(pool.clone(), "singular").await;

    let status = patch_all(
        app,
        "singular",
        &[("Accept", "application/vnd.pgrst.object+json")],
    )
    .await;
    assert_eq!(status, StatusCode::NOT_ACCEPTABLE);
    assert_eq!(
        changed(&pool, "singular").await,
        0,
        "a refused write was committed"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_function_answering_no_row_or_several_where_one_was_asked_for_is_undone() {
    let pool = pool().await;
    table(&pool, "rpc_singular").await;
    run(
        &pool,
        "rpc_singular",
        &[
            "CREATE FUNCTION {schema}.touch_none() RETURNS SETOF {schema}.rpc_singular \
             LANGUAGE plpgsql AS $$ BEGIN UPDATE {schema}.rpc_singular SET n = 1; RETURN; END $$",
            "CREATE FUNCTION {schema}.touch_all() RETURNS SETOF {schema}.rpc_singular \
             LANGUAGE sql AS $$ UPDATE {schema}.rpc_singular SET n = 1 RETURNING * $$",
        ],
    )
    .await;
    let app = app(pool.clone(), "rpc_singular").await;

    for function in ["touch_none", "touch_all"] {
        let (status, body) = send(
            app.clone(),
            "POST",
            &format!("/api/rpc/{function}"),
            &[("Accept", "application/vnd.pgrst.object+json")],
            "{}",
        )
        .await;
        assert_eq!(status, StatusCode::NOT_ACCEPTABLE, "{function}: {body}");
        assert_eq!(
            changed(&pool, "rpc_singular").await,
            0,
            "{function}: a refused call's writes were committed"
        );
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_put_whose_body_names_another_row_is_undone() {
    let pool = pool().await;
    table(&pool, "put_mismatch").await;
    // The body names another row, so the statement inserts nothing -- but a
    // statement-level trigger runs all the same, and its write is what a
    // refusal must take back with it.
    run(
        &pool,
        "put_mismatch",
        &[
            "CREATE TABLE {schema}.audit (at timestamptz NOT NULL DEFAULT now())",
            "GRANT INSERT ON {schema}.audit TO web_anon",
            "GRANT INSERT ON {schema}.put_mismatch TO web_anon",
            "CREATE FUNCTION {schema}.audit() RETURNS trigger LANGUAGE plpgsql AS \
             $$ BEGIN INSERT INTO {schema}.audit DEFAULT VALUES; RETURN NULL; END $$",
            "CREATE TRIGGER audit BEFORE INSERT ON {schema}.put_mismatch \
             FOR EACH STATEMENT EXECUTE FUNCTION {schema}.audit()",
        ],
    )
    .await;
    let app = app(pool.clone(), "put_mismatch").await;

    let (status, body) = send(
        app,
        "PUT",
        "/api/put_mismatch?id=eq.1",
        &[],
        r#"{"id":2,"n":1}"#,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body.contains("PGRST115"), "{body}");
    let audited: i64 = sqlx::query_scalar("SELECT count(*) FROM refused_writes_put_mismatch.audit")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(audited, 0, "a refused PUT's trigger write was committed");
    assert_eq!(changed(&pool, "put_mismatch").await, 0);
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_function_setting_an_invalid_status_is_undone() {
    let pool = pool().await;
    table(&pool, "bad_status").await;
    run(
        &pool,
        "bad_status",
        &[
            "CREATE FUNCTION {schema}.touch() RETURNS void LANGUAGE plpgsql AS $$ BEGIN \
           UPDATE {schema}.bad_status SET n = 1; \
           PERFORM set_config('response.status', 'not a status', true); END $$",
        ],
    )
    .await;
    let app = app(pool.clone(), "bad_status").await;

    let (status, body) = send(app, "POST", "/api/rpc/touch", &[], "{}").await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
    assert!(body.contains("PGRST112"), "{body}");
    assert_eq!(
        changed(&pool, "bad_status").await,
        0,
        "a refused call's writes were committed"
    );
}

/// A header HTTP cannot carry is refused before the commit. It used to be
/// found only when the response was built, after the function's writes were
/// committed, and the client got an empty 200.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_function_setting_an_unsendable_header_is_undone() {
    let pool = pool().await;
    table(&pool, "bad_header").await;
    run(
        &pool,
        "bad_header",
        &[
            "CREATE FUNCTION {schema}.touch() RETURNS void LANGUAGE plpgsql AS $$ BEGIN \
           UPDATE {schema}.bad_header SET n = 1; \
           PERFORM set_config('response.headers', \
             json_build_array(json_build_object('X-Split', E'one\\ntwo'))::text, true); END $$",
        ],
    )
    .await;
    let app = app(pool.clone(), "bad_header").await;

    let (status, body) = send(app, "POST", "/api/rpc/touch", &[], "{}").await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
    assert!(body.contains("PGRST111"), "{body}");
    assert_eq!(
        changed(&pool, "bad_header").await,
        0,
        "a refused call's writes were committed"
    );
}

/// A deferred constraint is checked at COMMIT, so that is where it fails --
/// and it is the database refusing the write, reported as such, not the
/// connection pool being unavailable.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_deferred_constraint_failing_at_commit_is_a_conflict() {
    let pool = pool().await;
    table(&pool, "deferred").await;
    run(
        &pool,
        "deferred",
        &[
            "CREATE TABLE {schema}.child (id int PRIMARY KEY, parent int NOT NULL \
             REFERENCES {schema}.deferred (id) DEFERRABLE INITIALLY DEFERRED)",
            "GRANT SELECT, INSERT ON {schema}.child TO web_anon",
        ],
    )
    .await;
    // Compatibility mode, which passes PostgreSQL's SQLSTATE through: it is
    // the deferred violation's own code the client should see.
    let app =
        router(state_with(pool.clone(), "deferred", |config| config.compat_mode = true).await);

    let (status, body) = send(
        app,
        "POST",
        "/api/child",
        &[("Prefer", "return=representation")],
        r#"{"id":1,"parent":99}"#,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(body.contains("23503"), "{body}");
    let written: i64 = sqlx::query_scalar("SELECT count(*) FROM refused_writes_deferred.child")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(written, 0);
}

/// The control: a write that is what the client asked for still commits.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_single_row_write_asked_for_as_one_object_commits() {
    let pool = pool().await;
    table(&pool, "singular_ok").await;
    let app = app(pool.clone(), "singular_ok").await;

    let (status, body) = send(
        app,
        "PATCH",
        "/api/singular_ok?id=eq.1",
        &[
            ("Accept", "application/vnd.pgrst.object+json"),
            ("Prefer", "return=representation"),
        ],
        r#"{"n":1}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&body).unwrap(),
        serde_json::json!({"id": 1, "n": 1})
    );
    assert_eq!(changed(&pool, "singular_ok").await, 1);
}

/// A schema reload arriving while a request runs does not stall the server.
///
/// The request used to hold the schema cache's read lock from planning to the
/// end of execution, and take it again on the way. tokio's `RwLock` puts a
/// reader behind any writer already waiting, so a reload queued between the
/// two waited for the first guard while the request waited behind the reload
/// for the second -- forever, with every later request queued behind both.
///
/// Made deterministic with a proxy in front of the database that holds the
/// request's connection: the request is then between the two, the reload is
/// queued while it waits, and only then is the connection let through. The
/// reload's own connection is passed straight through.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_schema_reload_during_a_request_does_not_deadlock() {
    use std::time::Duration;

    let direct = pool().await;
    table(&direct, "reload").await;
    let proxy = proxy::Proxy::start(
        proxy::host_and_port(&database_url()),
        proxy::Behaviour::HoldTheFirst,
    )
    .await;
    let through_proxy = proxy.url(&database_url());

    // Both ways a request reaches the database go through the proxy: the pool
    // by default, and the URI with `pg-socket`. Whichever the request uses
    // first is the connection held.
    let state = state_with(direct, "reload", |config| {
        config.db_uri = through_proxy.clone()
    })
    .await;
    let mut state = Arc::try_unwrap(state).ok().expect("one owner");
    state.pool = PgPoolOptions::new()
        .max_connections(2)
        .connect_lazy(&through_proxy)
        .unwrap();
    let state = Arc::new(state);
    let app = router(state.clone());

    let request = tokio::spawn(send(app, "GET", "/api/reload", &[], ""));

    // Until the request is waiting on its connection.
    let connecting = async {
        while proxy.accepted() == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    };
    tokio::time::timeout(Duration::from_secs(10), connecting)
        .await
        .expect("the request never asked for a connection");

    // What a `NOTIFY pgrst` does. It reads the catalogs first and only then
    // asks for the write lock, so wait until it is either finished or queued
    // for the lock. Queued is visible from outside, since a waiting writer
    // stops a new reader from getting in.
    let reload = tokio::spawn({
        let state = state.clone();
        async move { state.reload_schema().await }
    });
    let queued_or_done = async {
        while !reload.is_finished() && state.schema_cache.try_read().is_ok() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    };
    tokio::time::timeout(Duration::from_secs(10), queued_or_done)
        .await
        .expect("the schema reload never got as far as the lock");

    proxy.open();

    let (status, body) = tokio::time::timeout(Duration::from_secs(10), request)
        .await
        .expect("the request deadlocked against the schema reload")
        .unwrap();
    assert_eq!(status, StatusCode::OK, "{body}");
    tokio::time::timeout(Duration::from_secs(10), reload)
        .await
        .expect("the schema reload never finished")
        .unwrap()
        .unwrap();
}
