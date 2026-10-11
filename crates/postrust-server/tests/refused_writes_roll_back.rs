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

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::any;
use axum::Router;
use postrust_core::{AppConfig, SchemaCache};
use postrust_server::{handle_request, AppState};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::sync::Arc;
use tokio::sync::RwLock;
use tower::ServiceExt;

const SCHEMA: &str = "refused_writes";

fn database_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/postrust_test".to_string())
}

/// A table of its own per test, five rows of `n = 0`, writable by `web_anon`.
async fn table(pool: &PgPool, name: &str) {
    for statement in [
        format!("CREATE SCHEMA IF NOT EXISTS {SCHEMA}"),
        format!("GRANT USAGE ON SCHEMA {SCHEMA} TO web_anon"),
        format!("DROP TABLE IF EXISTS {SCHEMA}.{name}"),
        format!("CREATE TABLE {SCHEMA}.{name} (id int PRIMARY KEY, n int NOT NULL)"),
        format!("INSERT INTO {SCHEMA}.{name} SELECT g, 0 FROM generate_series(1, 5) g"),
        format!("GRANT SELECT, UPDATE ON {SCHEMA}.{name} TO web_anon"),
    ] {
        sqlx::query(&statement).execute(pool).await.unwrap();
    }
}

async fn app(pool: PgPool) -> Router {
    let mut config = AppConfig::default();
    config.db_schemas = vec![SCHEMA.to_string()];
    config.db_anon_role = Some("web_anon".to_string());
    let schema_cache = SchemaCache::load(&pool, &config.db_schemas)
        .await
        .expect("schema cache");
    let jwt_config = postrust_auth::JwtConfig {
        secret: config.jwt_secret.clone(),
        secret_is_base64: config.jwt_secret_is_base64,
        audience: config.jwt_aud.clone(),
        role_claim_key: config.jwt_role_claim_key.clone(),
        anon_role: config.db_anon_role.clone(),
    };
    let state = Arc::new(AppState {
        pool,
        schema_cache: RwLock::new(schema_cache),
        config,
        jwt_config,
        jwt_cache: None,
    });
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

async fn changed(pool: &PgPool, table: &str) -> i64 {
    sqlx::query_scalar(&format!(
        "SELECT count(*) FROM {SCHEMA}.{table} WHERE n = 1"
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
    let app = app(pool.clone()).await;

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
    let app = app(pool.clone()).await;

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
