//! `PGRST_DB_ERRORS_VERBATIM`: a database error answered in its own words
//! outside compatibility mode.
//!
//! A function that refuses with `RAISE EXCEPTION '...' USING ERRCODE =
//! 'P0001'` wrote that message for the person using the app. With the
//! SQLSTATE listed, the client gets it -- code, message, detail and hint --
//! from REST and from a route given to `Builder::nest` alike, while a
//! SQLSTATE that is not listed is still `"Database error"`. The RPC response
//! keeps its default shape: none of the rest of compatibility mode comes
//! with it.
//!
//! `DATABASE_URL` must name a superuser on a database with
//! `scripts/init-db.sql` loaded (for `web_anon`). Run with:
//!
//! ```text
//! cargo test --package postrust-server --test db_errors_verbatim -- --ignored
//! ```

use axum::body::Body;
use axum::extract::State;
use axum::http::{Request, StatusCode};
use axum::routing::get;
use axum::Router;
use postrust_core::AppConfig;
use postrust_server::{AppState, Builder, Host};
use serde_json::{json, Value};
use sqlx::postgres::PgPoolOptions;
use sqlx::AssertSqlSafe;
use std::sync::Arc;
use tower::ServiceExt;

fn database_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/postrust_test".to_string())
}

/// A schema of the test's own -- two tests remaking one race on the catalog
/// -- holding a function that refuses on purpose, one that fails on a missing
/// table, and one that answers.
async fn schema(name: &str) {
    let schema = format!("db_errors_verbatim_{name}");
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url())
        .await
        .unwrap();
    for statement in [
        format!("DROP SCHEMA IF EXISTS {schema} CASCADE"),
        format!("CREATE SCHEMA {schema}"),
        format!("GRANT USAGE ON SCHEMA {schema} TO web_anon"),
        format!(
            "CREATE FUNCTION {schema}.refuse() RETURNS int LANGUAGE plpgsql AS $$ \
             BEGIN RAISE EXCEPTION 'the covered area is required' \
             USING ERRCODE = 'P0001', DETAIL = 'step 2', HINT = 'fill it in'; END $$"
        ),
        format!(
            "CREATE FUNCTION {schema}.broken() RETURNS int LANGUAGE plpgsql AS $$ \
             BEGIN PERFORM * FROM {schema}.no_such_table; RETURN 1; END $$"
        ),
        format!("CREATE FUNCTION {schema}.answer() RETURNS int LANGUAGE sql AS $$ SELECT 42 $$"),
        format!("GRANT EXECUTE ON ALL FUNCTIONS IN SCHEMA {schema} TO web_anon"),
    ] {
        sqlx::query(AssertSqlSafe(statement))
            .execute(&pool)
            .await
            .unwrap();
    }
}

/// A router whose `GET /refuse` runs the refusing function itself and lets
/// `?` turn its error into a `postrust_server::Error`.
fn hooks(schema: String) -> Router<Arc<AppState>> {
    Router::new().route(
        "/refuse",
        get(move |State(state): State<Arc<AppState>>| async move {
            let n: i32 = sqlx::query_scalar(AssertSqlSafe(format!("SELECT {schema}.refuse()")))
                .fetch_one(&state.pool)
                .await?;
            Ok::<_, postrust_server::Error>(n.to_string())
        }),
    )
}

async fn router(name: &str, verbatim: &[&str]) -> Router {
    let schema = format!("db_errors_verbatim_{name}");
    let mut config = AppConfig::default();
    config.db_uri = database_url();
    config.db_schemas = vec![schema.clone()];
    config.db_anon_role = Some("web_anon".to_string());
    config.db_pool_size = 2;
    config.db_errors_verbatim = verbatim.iter().map(|s| s.to_string()).collect();
    Builder::new(config, Host::SERVER)
        .nest("/hooks", hooks(schema))
        .build()
        .await
        .unwrap()
        .router
}

async fn send(router: &Router, method: &str, uri: &str) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(method)
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

fn refusal() -> Value {
    json!({
        "code": "P0001",
        "message": "the covered area is required",
        "details": "step 2",
        "hint": "fill it in",
    })
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn a_listed_sqlstate_is_answered_in_its_own_words() {
    schema("listed").await;
    let router = router("listed", &["P0001"]).await;

    let (status, body) = send(&router, "POST", "/api/rpc/refuse").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body, refusal());

    // A route of the caller's own answers the same way.
    let (status, body) = send(&router, "GET", "/hooks/refuse").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body, refusal());

    // A fault that is not listed still says nothing about the schema.
    let (status, body) = send(&router, "POST", "/api/rpc/broken").await;
    assert_eq!(body["message"], "Database error", "{status} {body}");

    // And nothing else of compatibility mode comes with it: the RPC
    // response keeps its default shape, and the root serves nothing.
    let (status, body) = send(&router, "POST", "/api/rpc/answer").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!([{"answer": 42}]));
    let request = Request::post("/rpc/answer").body(Body::empty()).unwrap();
    let status = router.clone().oneshot(request).await.unwrap().status();
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn an_unlisted_sqlstate_stays_sanitised() {
    schema("unlisted").await;
    for verbatim in [&[][..], &["PT*"][..]] {
        let router = router("unlisted", verbatim).await;
        for (method, uri) in [("POST", "/api/rpc/refuse"), ("GET", "/hooks/refuse")] {
            let (status, body) = send(&router, method, uri).await;
            assert_eq!(
                status,
                StatusCode::BAD_REQUEST,
                "{verbatim:?} {uri}: {body}"
            );
            assert_eq!(body["message"], "Database error", "{verbatim:?} {uri}");
            assert_ne!(body["code"], "P0001", "{verbatim:?} {uri}");
        }
    }
}
