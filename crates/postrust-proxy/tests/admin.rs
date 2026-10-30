//! The admin API and UI routers, driven without a database.
//!
//! The pool points at nothing that listens. With `database_config` off nothing
//! touches it; with it on, every write fails -- which is exactly the case the
//! error-text test needs.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use postrust_proxy::admin::{admin_router, admin_ui_router};
use postrust_proxy::{ProxyConfig, ProxyState};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tower::ServiceExt;

async fn app(database_config: bool) -> axum::Router {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .acquire_timeout(Duration::from_millis(500))
        .connect_lazy("postgres://nobody:nothing@127.0.0.1:1/none")
        .unwrap();
    let mut config = ProxyConfig::default();
    config.server.database_config = database_config;
    let state = Arc::new(ProxyState::new(pool, config).await.unwrap());
    axum::Router::new()
        .nest("/api", admin_router())
        .nest("/ui", admin_ui_router())
        .with_state(state)
}

async fn send(
    app: &axum::Router,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> (StatusCode, String) {
    let builder = Request::builder().method(method).uri(path);
    let request = match body {
        Some(body) => builder
            .header("content-type", "application/json")
            .body(Body::from(body.to_string())),
        None => builder.body(Body::empty()),
    }
    .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

const PAYLOAD: &str = r#"<script>alert("x")</script>"#;

#[tokio::test]
async fn the_admin_ui_escapes_what_the_api_stored() {
    let app = app(false).await;

    let (status, body) = send(
        &app,
        "POST",
        "/api/upstreams",
        Some(json!({
            "name": PAYLOAD,
            "backends": [{ "address": format!("{PAYLOAD}:80"), "scheme": "http" }],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    let (status, body) = send(
        &app,
        "POST",
        "/api/routes",
        Some(json!({
            "name": PAYLOAD,
            "host": PAYLOAD,
            "path": format!("/{PAYLOAD}"),
            "upstream": PAYLOAD,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    for page in ["/ui/routes", "/ui/upstreams", "/ui/health"] {
        let (status, html) = send(&app, "GET", page, None).await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            !html.contains("<script>"),
            "{page} rendered markup unescaped"
        );
        assert!(
            html.contains("&lt;script&gt;alert(&quot;x&quot;)&lt;/script&gt;"),
            "{page} did not show the escaped value"
        );
    }
}

#[tokio::test]
async fn a_persistence_failure_does_not_return_the_database_error() {
    let app = app(true).await;
    let (status, body) = send(
        &app,
        "POST",
        "/api/routes",
        Some(json!({ "name": "r", "upstream": "u" })),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    let body: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(body["code"], "persistence_failed");
    let text = body.to_string();
    for leak in ["pool", "timed out", "Database error", "127.0.0.1"] {
        assert!(!text.contains(leak), "leaked {leak:?}: {text}");
    }
}

#[tokio::test]
async fn an_unknown_backend_scheme_is_refused_before_it_is_applied() {
    let app = app(false).await;
    let (status, body) = send(
        &app,
        "POST",
        "/api/upstreams",
        Some(json!({ "name": "u", "backends": [{ "address": "a:80", "scheme": "gopher" }] })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    let (_, list) = send(&app, "GET", "/api/upstreams", None).await;
    let list: Value = serde_json::from_str(&list).unwrap();
    assert_eq!(
        list["data"],
        json!([]),
        "the refused upstream was applied anyway"
    );
}
