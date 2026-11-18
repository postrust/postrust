//! A `RAISE sqlstate 'PGRST'` that supplies the whole response body.
//!
//! PostgREST rebuilds the body of a raise as `{code, message, details, hint}`.
//! A schema that replaces an API whose errors look different needs its own
//! bytes, so a `body` key in the DETAIL is sent as written, in default and in
//! compatibility mode alike, and without it nothing changes.
//!
//! `DATABASE_URL` must name a superuser on a database with
//! `scripts/init-db.sql` loaded (for `web_anon`). Run with:
//!
//! ```text
//! cargo test --package postrust-server --test raise_body -- --ignored
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

/// A schema of the test's own -- two tests remaking one race on the catalog
/// -- holding functions that raise `PGRST` in each of the shapes.
async fn schema(name: &str) {
    let schema = format!("raise_body_{name}");
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url())
        .await
        .unwrap();
    let raise = |function: &str, message: &str, detail: &str| {
        format!(
            "CREATE FUNCTION {schema}.{function}() RETURNS int LANGUAGE plpgsql AS $$ \
             BEGIN RAISE EXCEPTION '{message}' USING ERRCODE = 'PGRST', DETAIL = '{detail}'; \
             END $$"
        )
    };
    for statement in [
        format!("DROP SCHEMA IF EXISTS {schema} CASCADE"),
        format!("CREATE SCHEMA {schema}"),
        format!("GRANT USAGE ON SCHEMA {schema} TO web_anon"),
        // The message is free text, there are no headers, and the body is not
        // PostgREST's shape.
        raise(
            "gated",
            "access required for report 7",
            r#"{"status": 403, "body": {"detail": {"code": "access_required", "can_request": true}}}"#,
        ),
        // A body that is a string, and a type of its own for it.
        raise(
            "texty",
            "free text",
            r#"{"status": 418, "headers": {"Content-Type": "text/plain", "X-Kind": "tea"}, "body": "short and stout"}"#,
        ),
        // PostgREST's own shape.
        raise(
            "classic",
            r#"{"code": "123", "message": "Payment Required", "hint": "Upgrade"}"#,
            r#"{"status": 402, "headers": {"X-Powered-By": "Nerd Rage"}}"#,
        ),
        // A body without the status it cannot do without.
        raise("statusless", "free text", r#"{"body": {"a": 1}}"#),
        format!("GRANT EXECUTE ON ALL FUNCTIONS IN SCHEMA {schema} TO web_anon"),
    ] {
        sqlx::query(AssertSqlSafe(statement))
            .execute(&pool)
            .await
            .unwrap();
    }
}

async fn router(name: &str, compat_mode: bool) -> Router {
    let mut config = AppConfig::default();
    config.db_uri = database_url();
    config.db_schemas = vec![format!("raise_body_{name}")];
    config.db_anon_role = Some("web_anon".to_string());
    config.db_pool_size = 2;
    config.compat_mode = compat_mode;
    Builder::new(config, Host::SERVER)
        .build()
        .await
        .unwrap()
        .router
}

struct Answer {
    status: StatusCode,
    content_types: Vec<String>,
    kind: Option<String>,
    body: Vec<u8>,
}

async fn call(router: &Router, function: &str, compat_mode: bool) -> Answer {
    let prefix = if compat_mode { "" } else { "/api" };
    let request = Request::builder()
        .method("POST")
        .uri(format!("{prefix}/rpc/{function}"))
        .header("content-type", "application/json")
        .body(Body::from("{}"))
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    let header = |name: &str| {
        response
            .headers()
            .get_all(name)
            .iter()
            .map(|v| v.to_str().unwrap().to_string())
            .collect::<Vec<_>>()
    };
    Answer {
        status: response.status(),
        content_types: header("content-type"),
        kind: header("x-kind").into_iter().next(),
        body: axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    }
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn the_body_in_the_detail_is_sent_as_written_in_both_modes() {
    schema("body").await;
    for compat_mode in [false, true] {
        let router = router("body", compat_mode).await;

        let gated = call(&router, "gated", compat_mode).await;
        assert_eq!(gated.status, StatusCode::FORBIDDEN, "compat={compat_mode}");
        assert_eq!(gated.content_types, ["application/json"]);
        assert_eq!(
            serde_json::from_slice::<Value>(&gated.body).unwrap(),
            json!({"detail": {"code": "access_required", "can_request": true}})
        );

        // A string body is JSON, and a `Content-Type` the schema raised
        // replaces the default rather than joining it.
        let texty = call(&router, "texty", compat_mode).await;
        assert_eq!(texty.status, StatusCode::IM_A_TEAPOT);
        assert_eq!(texty.content_types, ["text/plain"]);
        assert_eq!(texty.kind.as_deref(), Some("tea"));
        assert_eq!(texty.body, br#""short and stout""#);

        // Without a body, PostgREST's rules: the shape is rebuilt, and the
        // headers follow the default `Content-Type`, as PostgREST sends them.
        let classic = call(&router, "classic", compat_mode).await;
        assert_eq!(classic.status, StatusCode::PAYMENT_REQUIRED);
        assert_eq!(
            serde_json::from_slice::<Value>(&classic.body).unwrap(),
            json!({"code": "123", "message": "Payment Required", "details": null, "hint": "Upgrade"})
        );

        // A body does not excuse a missing status: the schema's fault, 500.
        let statusless = call(&router, "statusless", compat_mode).await;
        assert_eq!(statusless.status, StatusCode::INTERNAL_SERVER_ERROR);
        let body: Value = serde_json::from_slice(&statusless.body).unwrap();
        assert_eq!(body["code"], "PGRST121", "{body}");
    }
}
