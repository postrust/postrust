//! `PGRST_RPC_UNWRAP`: PostgREST's RPC response shape outside compatibility
//! mode.
//!
//! A scalar function answers with the bare value, a set-returning one with a
//! plain array, while REST stays under `/api` and a database error is still
//! `"Database error"`.
//!
//! `DATABASE_URL` must name a superuser on a database with
//! `scripts/init-db.sql` loaded (for `web_anon`). Run with:
//!
//! ```text
//! cargo test --package postrust-server --test rpc_unwrap -- --ignored
//! ```

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use postrust_core::AppConfig;
use postrust_server::{Builder, Host};
use serde_json::{json, Value};
use sqlx::postgres::PgPoolOptions;
use sqlx::AssertSqlSafe;
use tower::ServiceExt;

fn database_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/postrust_test".to_string())
}

async fn schema(name: &str) {
    let schema = format!("rpc_unwrap_{name}");
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url())
        .await
        .unwrap();
    for statement in [
        format!("DROP SCHEMA IF EXISTS {schema} CASCADE"),
        format!("CREATE SCHEMA {schema}"),
        format!("GRANT USAGE ON SCHEMA {schema} TO web_anon"),
        format!("CREATE FUNCTION {schema}.answer() RETURNS int LANGUAGE sql AS $$ SELECT 42 $$"),
        format!(
            "CREATE FUNCTION {schema}.numbers() RETURNS SETOF int LANGUAGE sql AS \
             $$ SELECT generate_series(1, 3) $$"
        ),
        format!(
            "CREATE FUNCTION {schema}.broken() RETURNS int LANGUAGE plpgsql AS $$ \
             BEGIN PERFORM * FROM {schema}.no_such_table; RETURN 1; END $$"
        ),
        format!("GRANT EXECUTE ON ALL FUNCTIONS IN SCHEMA {schema} TO web_anon"),
    ] {
        sqlx::query(AssertSqlSafe(statement))
            .execute(&pool)
            .await
            .unwrap();
    }
}

async fn router(name: &str, rpc_unwrap: bool) -> Router {
    let mut config = AppConfig::default();
    config.db_uri = database_url();
    config.db_schemas = vec![format!("rpc_unwrap_{name}")];
    config.db_anon_role = Some("web_anon".to_string());
    config.db_pool_size = 2;
    config.compat_mode = false;
    config.rpc_unwrap = rpc_unwrap;
    Builder::new(config, Host::SERVER)
        .build()
        .await
        .unwrap()
        .router
}

async fn send(router: &Router, uri: &str) -> (StatusCode, Value) {
    let request = Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from("{}"))
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, serde_json::from_slice(&body).unwrap())
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn rpc_results_are_unwrapped_without_compat_mode() {
    schema("on").await;
    let router = router("on", true).await;

    let (status, body) = send(&router, "/api/rpc/answer").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, json!(42));

    let (status, body) = send(&router, "/api/rpc/numbers").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, json!([1, 2, 3]));

    // The rest of compatibility mode stays off: nothing at the root, and a
    // database error says nothing about the schema.
    let request = Request::post("/rpc/answer").body(Body::empty()).unwrap();
    let status = router.clone().oneshot(request).await.unwrap().status();
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (_, body) = send(&router, "/api/rpc/broken").await;
    assert_eq!(body["message"], "Database error", "{body}");
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn rpc_results_keep_their_default_shape_when_off() {
    schema("off").await;
    let router = router("off", false).await;

    let (_, body) = send(&router, "/api/rpc/answer").await;
    assert_eq!(body, json!([{"answer": 42}]));
    let (_, body) = send(&router, "/api/rpc/numbers").await;
    assert_eq!(
        body,
        json!([{"numbers": 1}, {"numbers": 2}, {"numbers": 3}])
    );
}
