//! `PGRST_DB_ERROR_STATUS`, and `P0002` as a 404 outside compatibility mode.
//!
//! A function that raises `no_data_found` (`P0002`) is reporting that it
//! found nothing, which PostgREST answers 500 and Postrust answers 404 unless
//! compatibility mode is on. A mapping of the operator's own wins in either
//! mode, and a route given to `Builder::nest` answers as REST does.
//!
//! `DATABASE_URL` must name a superuser on a database with
//! `scripts/init-db.sql` loaded (for `web_anon`). Run with:
//!
//! ```text
//! cargo test --package postrust-server --test db_error_status -- --ignored
//! ```

use axum::body::Body;
use axum::extract::State;
use axum::http::{Request, StatusCode};
use axum::routing::get;
use axum::Router;
use postrust_core::AppConfig;
use postrust_server::{AppState, Builder, Host};
use sqlx::postgres::PgPoolOptions;
use sqlx::AssertSqlSafe;
use std::sync::Arc;
use tower::ServiceExt;

fn database_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/postrust_test".to_string())
}

/// A schema of the test's own -- two tests remaking one race on the catalog
/// -- holding functions that raise `P0002`, `PT402` and a bare `P0001`.
async fn schema(name: &str) {
    let schema = format!("db_error_status_{name}");
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
            "CREATE FUNCTION {schema}.lookup() RETURNS int LANGUAGE plpgsql AS $$ \
             DECLARE n int; \
             BEGIN SELECT 1 INTO STRICT n WHERE false; RETURN n; END $$"
        ),
        format!(
            "CREATE FUNCTION {schema}.pay() RETURNS int LANGUAGE plpgsql AS $$ \
             BEGIN RAISE EXCEPTION 'pay up' USING ERRCODE = 'PT402'; END $$"
        ),
        format!(
            "CREATE FUNCTION {schema}.refuse() RETURNS int LANGUAGE plpgsql AS $$ \
             BEGIN RAISE EXCEPTION 'no' USING ERRCODE = 'P0001'; END $$"
        ),
        format!("GRANT EXECUTE ON ALL FUNCTIONS IN SCHEMA {schema} TO web_anon"),
    ] {
        sqlx::query(AssertSqlSafe(statement))
            .execute(&pool)
            .await
            .unwrap();
    }
}

/// A router whose `GET /<function>` runs that function itself and lets `?`
/// turn its error into a `postrust_server::Error`.
fn hooks(schema: String) -> Router<Arc<AppState>> {
    Router::new().route(
        "/{function}",
        get(
            move |State(state): State<Arc<AppState>>,
                  axum::extract::Path(function): axum::extract::Path<String>| async move {
                let n: i32 =
                    sqlx::query_scalar(AssertSqlSafe(format!("SELECT {schema}.{function}()")))
                        .fetch_one(&state.pool)
                        .await?;
                Ok::<_, postrust_server::Error>(n.to_string())
            },
        ),
    )
}

async fn router(name: &str, compat_mode: bool, mapping: &[(&str, u16)]) -> Router {
    let schema = format!("db_error_status_{name}");
    let mut config = AppConfig::default();
    config.db_uri = database_url();
    config.db_schemas = vec![schema.clone()];
    config.db_anon_role = Some("web_anon".to_string());
    config.db_pool_size = 2;
    config.compat_mode = compat_mode;
    config.db_error_status = mapping
        .iter()
        .map(|(pattern, status)| (pattern.to_string(), *status))
        .collect();
    Builder::new(config, Host::SERVER)
        .nest("/hooks", hooks(schema))
        .build()
        .await
        .unwrap()
        .router
}

async fn status(router: &Router, method: &str, uri: &str) -> StatusCode {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from("{}"))
        .unwrap();
    router.clone().oneshot(request).await.unwrap().status()
}

/// The status of one function by REST and by the route that runs it itself,
/// which have to agree.
async fn both(router: &Router, function: &str) -> (StatusCode, StatusCode) {
    let rest = status(router, "POST", &format!("/api/rpc/{function}")).await;
    let route = status(router, "GET", &format!("/hooks/{function}")).await;
    (rest, route)
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn no_data_found_is_a_404_by_default() {
    schema("default").await;
    let router = router("default", false, &[]).await;

    assert_eq!(
        both(&router, "lookup").await,
        (StatusCode::NOT_FOUND, StatusCode::NOT_FOUND)
    );
    // Nothing else moved.
    assert_eq!(
        both(&router, "refuse").await,
        (StatusCode::BAD_REQUEST, StatusCode::BAD_REQUEST)
    );
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn compat_mode_keeps_postgrests_500() {
    schema("compat").await;
    let router = router("compat", true, &[]).await;

    assert_eq!(
        both(&router, "lookup").await,
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            StatusCode::INTERNAL_SERVER_ERROR
        )
    );
    // In compatibility mode REST is also served at the root.
    assert_eq!(
        status(&router, "POST", "/rpc/lookup").await,
        StatusCode::INTERNAL_SERVER_ERROR
    );
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn a_configured_status_wins_in_both_modes() {
    schema("mapped").await;
    for compat_mode in [false, true] {
        let router = router(
            "mapped",
            compat_mode,
            &[("P0002", 410), ("P0*", 422), ("PT402", 418)],
        )
        .await;

        // The whole SQLSTATE over the prefix, for REST and for the route.
        assert_eq!(
            both(&router, "lookup").await,
            (StatusCode::GONE, StatusCode::GONE),
            "compat_mode={compat_mode}"
        );
        assert_eq!(
            both(&router, "refuse").await,
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                StatusCode::UNPROCESSABLE_ENTITY
            ),
            "compat_mode={compat_mode}"
        );
        // What the schema raised with `PT<nnn>` is not remapped.
        assert_eq!(
            both(&router, "pay").await,
            (StatusCode::PAYMENT_REQUIRED, StatusCode::PAYMENT_REQUIRED),
            "compat_mode={compat_mode}"
        );
    }

    // `P0002:500` puts PostgREST's answer back without compatibility mode.
    let router = router("mapped", false, &[("P0002", 500)]).await;
    assert_eq!(
        both(&router, "lookup").await,
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            StatusCode::INTERNAL_SERVER_ERROR
        )
    );
}
