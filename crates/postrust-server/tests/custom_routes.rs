//! Routes of the caller's own, added with `Builder::nest`.
//!
//! A nested router is served the way the API is: inside the same CORS policy
//! and `OPTIONS` handling, with the same state. In compatibility mode it is an
//! explicit route, matched before the REST surface the root falls back to. A
//! prefix the server already serves is refused by `build` with an error,
//! before anything is connected to.
//!
//! The refusals need no database. The rest need `DATABASE_URL` to name a
//! superuser on a database with `scripts/init-db.sql` loaded (for
//! `web_anon`). Run with:
//!
//! ```text
//! cargo test --package postrust-server --test custom_routes -- --include-ignored
//! ```

use axum::body::Body;
use axum::extract::State;
use axum::http::{Request, StatusCode};
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use postrust_core::AppConfig;
use postrust_server::{AppState, Builder, Host};
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use tower::ServiceExt;

fn database_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/postrust_test".to_string())
}

fn config(schema: &str) -> AppConfig {
    let mut config = AppConfig::default();
    config.db_uri = database_url();
    config.db_schemas = vec![schema.to_string()];
    config.db_anon_role = Some("web_anon".to_string());
    config.db_pool_size = 2;
    config
}

/// A router answering `GET /ping` with its own name, and `GET /` with the
/// database's answer to a query, to show the state reaches it.
fn hooks(name: &'static str) -> Router<Arc<AppState>> {
    Router::new()
        .route("/ping", get(move || async move { name }))
        .route(
            "/",
            get(|State(state): State<Arc<AppState>>| async move {
                let n: i32 = sqlx::query_scalar("SELECT 41 + 1")
                    .fetch_one(&state.pool)
                    .await
                    .unwrap();
                n.to_string()
            }),
        )
}

async fn send(router: &Router, request: Request<Body>) -> Response {
    router.clone().oneshot(request).await.unwrap()
}

async fn text(response: Response) -> String {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}

fn get_from(path: &str) -> Request<Body> {
    Request::get(path)
        .header("origin", "https://example.com")
        .body(Body::empty())
        .unwrap()
}

/// Built with a database that is never reached: the prefixes are checked
/// first, so these fail on the prefix and not on the connection.
async fn refusal(builder: Builder) -> String {
    match builder.build().await {
        Ok(_) => panic!("built"),
        Err(e) => e.to_string(),
    }
}

fn unreachable_config() -> AppConfig {
    let mut config = AppConfig::default();
    config.db_uri = "postgres://nobody@127.0.0.1:1/none".to_string();
    config
}

#[tokio::test]
async fn a_prefix_the_server_serves_is_an_error() {
    for prefix in ["/api", "/v1/graphql", "/admin", "/healthz", "/_", "/"] {
        let refused =
            refusal(Builder::new(unreachable_config(), Host::SERVER).nest(prefix, Router::new()))
                .await;
        assert!(
            refused.contains("cannot nest routes"),
            "{prefix}: {refused}"
        );
    }
}

#[tokio::test]
async fn the_same_prefix_twice_is_an_error() {
    let refused = refusal(
        Builder::new(unreachable_config(), Host::SERVER)
            .nest("/hooks", hooks("a"))
            .nest("hooks/", hooks("b")),
    )
    .await;
    assert!(refused.contains("nested earlier"), "{refused}");
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn a_nested_route_is_served_with_the_apis_layers() {
    let app = Builder::new(config("public"), Host::SERVER)
        .nest("/hooks", hooks("hooks"))
        .nest("/_/jobs", hooks("jobs"))
        .build()
        .await
        .unwrap();
    let router = app.router;

    let response = send(&router, get_from("/hooks/ping")).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()["access-control-allow-origin"],
        "*",
        "the CORS layer wraps the nested route"
    );
    assert_eq!(text(response).await, "hooks");

    // The state is the API's.
    let response = send(&router, get_from("/hooks")).await;
    assert_eq!(text(response).await, "42");

    // Beneath `/_`, beside the built-in routes there, which still answer.
    let response = send(&router, get_from("/_/jobs/ping")).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(text(response).await, "jobs");
    let response = send(&router, get_from("/_/health")).await;
    assert_eq!(response.status(), StatusCode::OK);

    // A preflight is answered by the CORS layer, as it is for the API.
    let response = send(
        &router,
        Request::builder()
            .method("OPTIONS")
            .uri("/hooks/ping")
            .header("origin", "https://example.com")
            .header("access-control-request-method", "POST")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert!(response.status().is_success(), "{}", response.status());
    assert!(response
        .headers()
        .contains_key("access-control-allow-methods"));
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn in_compatibility_mode_a_nested_route_comes_before_the_root_fallback() {
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url())
        .await
        .unwrap();
    let schema = "custom_routes_compat";
    for statement in [
        format!("DROP SCHEMA IF EXISTS {schema} CASCADE"),
        format!("CREATE SCHEMA {schema}"),
        format!("GRANT USAGE ON SCHEMA {schema} TO web_anon"),
        // A table with the prefix's name, which the root would otherwise
        // serve at `/hooks`.
        format!("CREATE TABLE {schema}.hooks (id int PRIMARY KEY)"),
        format!("INSERT INTO {schema}.hooks VALUES (7)"),
        format!("GRANT SELECT ON {schema}.hooks TO web_anon"),
        format!("CREATE TABLE {schema}.other (id int PRIMARY KEY)"),
        format!("GRANT SELECT ON {schema}.other TO web_anon"),
    ] {
        sqlx::query(&statement).execute(&pool).await.unwrap();
    }

    let mut config = config(schema);
    config.compat_mode = true;
    let app = Builder::new(config, Host::SERVER)
        .nest("/hooks", hooks("hooks"))
        .build()
        .await
        .unwrap();
    let router = app.router;

    let response = send(&router, get_from("/hooks")).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(text(response).await, "42", "the nested route answers");

    // The table is still served under `/api`, and every other path still
    // falls back to the REST surface at the root.
    let response = send(&router, get_from("/api/hooks")).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(text(response).await, r#"[{"id":7}]"#);
    let response = send(&router, get_from("/other")).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(text(response).await, "[]");

    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&pool)
        .await
        .unwrap();
}

/// For a layer that appends `name` to `x-wrapped` on the way out, so a
/// response says which layers it passed through and in what order.
fn wrapped_by(
    name: &'static str,
) -> impl Fn(Response) -> std::future::Ready<Response> + Clone + Send + Sync + 'static {
    move |mut response: Response| {
        let seen = match response.headers().get("x-wrapped") {
            Some(v) => format!("{},{name}", v.to_str().unwrap()),
            None => name.to_string(),
        };
        response
            .headers_mut()
            .insert("x-wrapped", seen.parse().unwrap());
        std::future::ready(response)
    }
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn a_layer_wraps_everything_the_server_serves() {
    let app = Builder::new(config("public"), Host::SERVER)
        .nest("/hooks", hooks("hooks"))
        .layer(axum::middleware::map_response(wrapped_by("inner")))
        .layer(axum::middleware::map_response(wrapped_by("outer")))
        .build()
        .await
        .unwrap();
    let router = app.router;

    // The caller's route, the API, a path nothing serves, and a preflight the
    // CORS layer answers without calling what it wraps.
    let preflight = Request::options("/api/order_items")
        .header("origin", "https://example.com")
        .header("access-control-request-method", "GET")
        .body(Body::empty())
        .unwrap();
    for request in [
        get_from("/hooks/ping"),
        get_from("/api/order_items"),
        get_from("/nothing/here"),
        preflight,
    ] {
        let path = request.uri().to_string();
        let response = send(&router, request).await;
        // The one given last is the outermost, so it sees the response last.
        assert_eq!(
            response
                .headers()
                .get("x-wrapped")
                .map(|v| v.to_str().unwrap()),
            Some("inner,outer"),
            "{path}"
        );
    }
}
