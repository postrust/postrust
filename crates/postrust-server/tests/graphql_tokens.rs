//! A token GraphQL cannot verify is refused, not served as the anonymous role.
//!
//! REST has always answered such a token with 401. GraphQL fell back to the
//! anonymous role instead, so a client whose token had expired -- or been
//! signed with the wrong key -- kept getting answers, as someone else, and
//! nothing told it why its rows had gone. Hasura refuses it as `invalid-jwt`,
//! and so does this now.
//!
//! The database must have `scripts/init-db.sql` loaded and `DATABASE_URL` must
//! name a role that can become `web_anon` and `web_user`:
//!
//! ```text
//! cargo test --package postrust-server --features graphql --test graphql_tokens -- --ignored
//! ```

#![cfg(feature = "graphql")]

use axum::body::Body;
use axum::http::Request;
use axum::Router;
use jsonwebtoken::{encode, EncodingKey, Header};
use postrust_core::AppConfig;
use postrust_server::{build, Host};
use serde_json::{json, Value};
use tower::ServiceExt;

const SECRET: &str = "reallyreallyreallyreallyverysafe";

async fn router(secret: Option<&str>) -> Router {
    let mut config = AppConfig::default();
    config.db_uri = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/postrust_test".into());
    config.db_schemas = vec!["public".to_string()];
    config.db_anon_role = Some("web_anon".to_string());
    config.jwt_secret = secret.map(str::to_string);
    build(config, Host::LAMBDA).await.expect("build").router
}

fn token(secret: &str, claims: Value) -> String {
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .unwrap()
}

async fn ask(app: &Router, authorization: Option<String>) -> Value {
    let mut request = Request::post("/v1/graphql").header("content-type", "application/json");
    if let Some(value) = authorization {
        request = request.header("authorization", value);
    }
    let response = app
        .clone()
        .oneshot(
            request
                .body(Body::from(json!({"query": "{ __typename }"}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&body).unwrap()
}

fn refused_as_invalid_jwt(body: &Value) -> bool {
    body.pointer("/errors/0/extensions/code") == Some(&json!("invalid-jwt"))
        && body.get("data").is_none_or(Value::is_null)
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_token_that_does_not_verify_is_refused() {
    let app = router(Some(SECRET)).await;
    let far = 4_000_000_000_u64;

    let answered = ask(&app, None).await;
    assert_eq!(answered["data"]["__typename"], "query_root", "{answered}");

    let valid = token(SECRET, json!({"role": "web_user", "exp": far}));
    let answered = ask(&app, Some(format!("Bearer {valid}"))).await;
    assert_eq!(answered["data"]["__typename"], "query_root", "{answered}");

    for (what, header) in [
        (
            "signed with another key",
            format!(
                "Bearer {}",
                token(
                    "another-secret-another-secret-xx",
                    json!({"role": "web_user", "exp": far})
                )
            ),
        ),
        (
            "expired",
            format!(
                "Bearer {}",
                token(SECRET, json!({"role": "web_user", "exp": 1}))
            ),
        ),
        ("not a JWT", "Bearer nope".to_string()),
        ("not a bearer token", format!("Basic {valid}")),
    ] {
        let answered = ask(&app, Some(header)).await;
        assert!(refused_as_invalid_jwt(&answered), "{what}: {answered}");
    }
}

/// With no secret there is nothing to verify a token against, and the header
/// is ignored, as Hasura ignores it when JWT mode is off. A Federation router
/// forwarding `Authorization` to every subgraph depends on that.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn without_a_secret_the_header_is_ignored() {
    let app = router(None).await;
    let answered = ask(&app, Some("Bearer anything".to_string())).await;
    assert_eq!(answered["data"]["__typename"], "query_root", "{answered}");
}
