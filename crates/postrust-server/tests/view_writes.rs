//! A write to a view that cannot take it is a 405, not a 500.
//!
//! PostgreSQL writes through a view only where the view is simple enough
//! (automatically updatable) or where an `INSTEAD OF` trigger says how. Any
//! other write fails in the database, for every role, with "cannot update
//! view" -- which reached the client as `500 PGRST500 "Database error"`, while
//! an `OPTIONS` on the same view already left the method out of `Allow`.
//! The database's refusal is now answered with the 405 and `Allow` a method
//! the resource does not take earns, and so is every other 405: a function
//! called with a method its volatility does not allow says what it does take
//! too.
//!
//! The database still decides: grants, a grant held per column, and an
//! upsert the view cannot take are its to report, as before.
//!
//! `DATABASE_URL` must name a superuser on a database with
//! `scripts/init-db.sql` loaded (for `web_anon`). Run with:
//!
//! ```text
//! cargo test --package postrust-server --test view_writes -- --ignored
//! ```

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

/// Each test's own schema, so tests can run at the same time.
fn schema(test: &str) -> String {
    format!("view_writes_{test}")
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

/// A fresh schema holding `projects` (three rows) and `owners`, and whatever
/// `statements` add, with `{schema}` standing for the schema.
async fn setup(pool: &PgPool, test: &str, statements: &[&str]) {
    let schema = schema(test);
    let mut all = vec![
        format!("DROP SCHEMA IF EXISTS {schema} CASCADE"),
        format!("CREATE SCHEMA {schema}"),
        format!("GRANT USAGE ON SCHEMA {schema} TO web_anon"),
        format!("CREATE TABLE {schema}.owners (id int PRIMARY KEY, name text NOT NULL)"),
        format!("INSERT INTO {schema}.owners VALUES (1, 'a')"),
        format!(
            "CREATE TABLE {schema}.projects (id int PRIMARY KEY, owner int REFERENCES {schema}.owners, n int NOT NULL)"
        ),
        format!("INSERT INTO {schema}.projects SELECT g, 1, 0 FROM generate_series(1, 3) g"),
    ];
    all.extend(statements.iter().map(|s| s.replace("{schema}", &schema)));
    for statement in all {
        sqlx::query(&statement).execute(pool).await.unwrap();
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
        let mut state = AppState::new(pool, schema_cache, config);
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
    allow: Option<String>,
    body: String,
}

async fn send(app: &Router, method: &str, uri: &str, body: &str) -> Answer {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header("Content-Type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let allow = response
        .headers()
        .get("allow")
        .map(|v| v.to_str().unwrap().to_string());
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    Answer {
        status,
        allow,
        body: String::from_utf8_lossy(&body).into_owned(),
    }
}

async fn count(pool: &PgPool, sql: &str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(pool).await.unwrap()
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn every_write_to_a_join_view_is_a_405_with_allow() {
    let pool = pool().await;
    setup(
        &pool,
        "join",
        &[
            "CREATE VIEW {schema}.project_owners AS \
             SELECT p.id, p.n, o.name FROM {schema}.projects p JOIN {schema}.owners o ON o.id = p.owner",
            // Every grant: the view still cannot take a write.
            "GRANT ALL ON {schema}.project_owners TO web_anon",
        ],
    )
    .await;
    let app = app(pool.clone(), "join").await;

    for (method, uri, body) in [
        (
            "POST",
            "/api/project_owners",
            r#"{"id":9,"n":1,"name":"b"}"#,
        ),
        ("PATCH", "/api/project_owners?id=eq.1", r#"{"n":1}"#),
        ("DELETE", "/api/project_owners?id=eq.1", ""),
        (
            "PUT",
            "/api/project_owners?id=eq.1",
            r#"{"id":1,"n":1,"name":"a"}"#,
        ),
    ] {
        let answer = send(&app, method, uri, body).await;
        assert_eq!(
            answer.status,
            StatusCode::METHOD_NOT_ALLOWED,
            "{method}: {}",
            answer.body
        );
        assert_eq!(
            answer.allow.as_deref(),
            Some("OPTIONS,GET,HEAD"),
            "{method}"
        );
        // A PUT is refused before it gets that far: the view has no primary
        // key for its filters to name. Still a 405, with the same `Allow`.
        let expected = match method {
            "PUT" => "PGRST105",
            _ => "PGRST117",
        };
        assert!(answer.body.contains(expected), "{method}: {}", answer.body);
    }
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM view_writes_join.projects WHERE n <> 0"
        )
        .await,
        0
    );

    // The 405's Allow is the OPTIONS answer, and reading still works.
    let options = send(&app, "OPTIONS", "/api/project_owners", "").await;
    assert_eq!(options.allow.as_deref(), Some("OPTIONS,GET,HEAD"));
    let read = send(&app, "GET", "/api/project_owners?id=eq.1", "").await;
    assert_eq!(read.status, StatusCode::OK, "{}", read.body);
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_view_with_only_an_insert_trigger_takes_only_post() {
    let pool = pool().await;
    setup(
        &pool,
        "trigger",
        &[
            "CREATE VIEW {schema}.project_owners AS \
             SELECT p.id, p.n, o.name FROM {schema}.projects p JOIN {schema}.owners o ON o.id = p.owner",
            "CREATE FUNCTION {schema}.add_project() RETURNS trigger LANGUAGE plpgsql AS \
             $$ BEGIN INSERT INTO {schema}.projects VALUES (NEW.id, 1, NEW.n); RETURN NEW; END $$",
            "CREATE TRIGGER add_project INSTEAD OF INSERT ON {schema}.project_owners \
             FOR EACH ROW EXECUTE FUNCTION {schema}.add_project()",
            "GRANT ALL ON {schema}.project_owners TO web_anon",
            "GRANT INSERT ON {schema}.projects TO web_anon",
        ],
    )
    .await;
    let app = app(pool.clone(), "trigger").await;

    let created = send(
        &app,
        "POST",
        "/api/project_owners",
        r#"{"id":9,"n":5,"name":"a"}"#,
    )
    .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM view_writes_trigger.projects WHERE id = 9"
        )
        .await,
        1
    );

    let patched = send(&app, "PATCH", "/api/project_owners?id=eq.1", r#"{"n":1}"#).await;
    assert_eq!(
        patched.status,
        StatusCode::METHOD_NOT_ALLOWED,
        "{}",
        patched.body
    );
    assert_eq!(patched.allow.as_deref(), Some("OPTIONS,GET,HEAD,POST"));

    // An upsert would have to update, which the view cannot do: PostgreSQL
    // refuses the ON CONFLICT, and that is the client's request to change,
    // not a server fault.
    let upsert = Request::builder()
        .method("POST")
        .uri("/api/project_owners")
        .header("Content-Type", "application/json")
        .header("Prefer", "resolution=merge-duplicates")
        .body(Body::from(r#"{"id":9,"n":6,"name":"a"}"#))
        .unwrap();
    let upserted = app.clone().oneshot(upsert).await.unwrap();
    assert!(upserted.status().is_client_error(), "{}", upserted.status());
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_materialized_view_and_a_volatile_function_say_what_they_take() {
    let pool = pool().await;
    setup(
        &pool,
        "other",
        &[
            "CREATE MATERIALIZED VIEW {schema}.totals AS SELECT 1 AS id, sum(n) AS n FROM {schema}.projects",
            "GRANT ALL ON {schema}.totals TO web_anon",
            "CREATE FUNCTION {schema}.bump() RETURNS int LANGUAGE sql VOLATILE AS \
             'UPDATE {schema}.projects SET n = n + 1 WHERE id = 1 RETURNING n'",
            "GRANT EXECUTE ON FUNCTION {schema}.bump() TO web_anon",
            "GRANT SELECT, UPDATE ON {schema}.projects TO web_anon",
        ],
    )
    .await;
    let app = app(pool.clone(), "other").await;

    let patched = send(&app, "PATCH", "/api/totals?id=eq.1", r#"{"n":1}"#).await;
    assert_eq!(
        patched.status,
        StatusCode::METHOD_NOT_ALLOWED,
        "{}",
        patched.body
    );
    assert_eq!(patched.allow.as_deref(), Some("OPTIONS,GET,HEAD"));

    // Not a view at all: a function that writes, called with GET, which runs
    // read-only. Its 405 had no `Allow` either.
    let got = send(&app, "GET", "/api/rpc/bump", "").await;
    assert_eq!(got.status, StatusCode::METHOD_NOT_ALLOWED, "{}", got.body);
    assert_eq!(got.allow.as_deref(), Some("OPTIONS,POST"));
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn an_auto_updatable_view_is_written_through() {
    let pool = pool().await;
    setup(
        &pool,
        "auto",
        &[
            "CREATE VIEW {schema}.busy_projects AS SELECT id, n FROM {schema}.projects",
            "GRANT SELECT, UPDATE ON {schema}.busy_projects TO web_anon",
        ],
    )
    .await;
    let app = app(pool.clone(), "auto").await;

    let patched = send(&app, "PATCH", "/api/busy_projects?id=eq.1", r#"{"n":7}"#).await;
    assert_eq!(patched.status, StatusCode::NO_CONTENT, "{}", patched.body);
    assert_eq!(
        count(
            &pool,
            "SELECT n::bigint FROM view_writes_auto.projects WHERE id = 1"
        )
        .await,
        7
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn grants_stay_the_database_s_to_judge() {
    let pool = pool().await;
    setup(
        &pool,
        "grants",
        &[
            // UPDATE granted on one column only: no relation-level grant to see.
            "GRANT SELECT ON {schema}.projects TO web_anon",
            "GRANT UPDATE (n) ON {schema}.projects TO web_anon",
            // A view PostgreSQL can write through, with no write granted at all.
            "CREATE VIEW {schema}.ungranted AS SELECT id, n FROM {schema}.projects",
            "GRANT SELECT ON {schema}.ungranted TO web_anon",
        ],
    )
    .await;
    let app = app(pool.clone(), "grants").await;

    // The column grant is enough, and nothing in front of it says otherwise.
    let patched = send(&app, "PATCH", "/api/projects?id=eq.2", r#"{"n":3}"#).await;
    assert_eq!(patched.status, StatusCode::NO_CONTENT, "{}", patched.body);
    assert_eq!(
        count(
            &pool,
            "SELECT n::bigint FROM view_writes_grants.projects WHERE id = 2"
        )
        .await,
        3
    );

    // No grant: the database's permission error (42501, which Postrust
    // answers with 401 for the anonymous role), not a 405.
    let refused = send(&app, "PATCH", "/api/ungranted?id=eq.1", r#"{"n":3}"#).await;
    assert!(
        matches!(
            refused.status,
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
        ),
        "{}: {}",
        refused.status,
        refused.body
    );
    assert_eq!(
        count(
            &pool,
            "SELECT n::bigint FROM view_writes_grants.projects WHERE id = 1"
        )
        .await,
        0
    );
}
