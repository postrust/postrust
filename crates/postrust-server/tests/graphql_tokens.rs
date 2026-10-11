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
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tower::ServiceExt;

const SECRET: &str = "reallyreallyreallyreallyverysafe";

fn config(secret: Option<&str>) -> AppConfig {
    let mut config = AppConfig::default();
    config.db_uri = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/postrust_test".into());
    config.db_schemas = vec!["public".to_string()];
    config.db_anon_role = Some("web_anon".to_string());
    config.jwt_secret = secret.map(str::to_string);
    config
}

async fn router(secret: Option<&str>) -> Router {
    build(config(secret), Host::LAMBDA)
        .await
        .expect("build")
        .router
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
    let headers: Vec<(&str, String)> = authorization
        .into_iter()
        .map(|value| ("authorization", value))
        .collect();
    post(app, &headers, json!({"query": "{ __typename }"})).await
}

async fn post(app: &Router, headers: &[(&str, String)], body: Value) -> Value {
    let mut request = Request::post("/v1/graphql").header("content-type", "application/json");
    for (name, value) in headers {
        request = request.header(*name, value);
    }
    let response = app
        .clone()
        .oneshot(request.body(Body::from(body.to_string())).unwrap())
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

/// A batch is several operations under one header, and each is refused.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_batch_under_a_bad_token_is_refused_in_every_part() {
    let app = router(Some(SECRET)).await;
    let answered = post(
        &app,
        &[("authorization", "Bearer nope".to_string())],
        json!([{"query": "{ __typename }"}, {"query": "{ __typename }"}]),
    )
    .await;
    let parts = answered.as_array().expect("an array");
    assert_eq!(parts.len(), 2, "{answered}");
    assert!(parts.iter().all(refused_as_invalid_jwt), "{answered}");
}

/// The admin secret settles who is asking before the token is looked at, as
/// it does in Hasura: a bad token beside the right secret is answered.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_admin_secret_wins_over_a_bad_token() {
    let mut config = config(Some(SECRET));
    config.hasura_admin_secret = Some("s3cret".to_string());
    let app = build(config, Host::LAMBDA).await.expect("build").router;

    let answered = post(
        &app,
        &[
            ("authorization", "Bearer nope".to_string()),
            ("x-hasura-admin-secret", "s3cret".to_string()),
        ],
        json!({"query": "{ __typename }"}),
    )
    .await;
    assert_eq!(answered["data"]["__typename"], "query_root", "{answered}");

    let answered = ask(&app, Some("Bearer nope".to_string())).await;
    assert!(refused_as_invalid_jwt(&answered), "{answered}");
}

/// A token that verifies and names no role, on a server with no anonymous
/// role, is refused -- REST answers it 401 -- rather than served as a role
/// nobody chose. A request with no token at all is unchanged.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_verified_token_naming_no_role_is_refused_without_an_anonymous_role() {
    let mut config = config(Some(SECRET));
    config.db_anon_role = None;
    let app = build(config, Host::LAMBDA).await.expect("build").router;

    let roleless = token(SECRET, json!({"sub": "someone", "exp": 4_000_000_000_u64}));
    let answered = ask(&app, Some(format!("Bearer {roleless}"))).await;
    assert_eq!(
        answered.pointer("/errors/0/extensions/code"),
        Some(&json!("jwt-missing-role-claims")),
        "{answered}"
    );
    assert_eq!(
        answered.pointer("/errors/0/message"),
        Some(&json!("JWT claim does not contain role")),
        "{answered}"
    );
    assert!(
        answered.get("data").is_none_or(Value::is_null),
        "{answered}"
    );

    let answered = ask(&app, None).await;
    assert_eq!(answered["data"]["__typename"], "query_root", "{answered}");

    // With an anonymous role, a token naming none is that role, as in REST.
    let app = router(Some(SECRET)).await;
    let answered = ask(&app, Some(format!("Bearer {roleless}"))).await;
    assert_eq!(answered["data"]["__typename"], "query_root", "{answered}");
}

/// Over a WebSocket the token arrives in `connection_init`, and a bad one is
/// refused there. Spoken by hand -- the handshake and a few frames -- rather
/// than through a client library this crate does not otherwise need.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_bad_token_is_refused_at_connection_init() {
    let app = build(config(Some(SECRET)), Host::SERVER)
        .await
        .expect("build")
        .router;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await });

    let valid = token(
        SECRET,
        json!({"role": "web_user", "exp": 4_000_000_000_u64}),
    );
    let answered = connection_init(address, &format!("Bearer {valid}")).await;
    assert_eq!(answered["type"], "connection_ack", "{answered}");

    let answered = connection_init(address, "Bearer nope").await;
    assert_eq!(answered["type"], "connection_error", "{answered}");
    let message = answered["payload"]["message"].as_str().unwrap_or_default();
    assert!(message.starts_with("Could not verify JWT"), "{answered}");
}

/// Open a `graphql-ws` connection, send `connection_init` carrying the
/// header, and return the server's first message.
async fn connection_init(address: std::net::SocketAddr, authorization: &str) -> Value {
    let mut stream = TcpStream::connect(address).await.unwrap();
    let handshake = format!(
        "GET /v1/graphql/ws HTTP/1.1\r\nHost: {address}\r\nUpgrade: websocket\r\n\
         Connection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
         Sec-WebSocket-Version: 13\r\nSec-WebSocket-Protocol: graphql-ws\r\n\r\n"
    );
    stream.write_all(handshake.as_bytes()).await.unwrap();

    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        head.push(stream.read_u8().await.unwrap());
    }
    let head = String::from_utf8_lossy(&head);
    assert!(head.starts_with("HTTP/1.1 101"), "{head}");

    // A client's frames are masked; a zero mask leaves the payload as it is.
    let payload =
        json!({"type": "connection_init", "payload": {"headers": {"Authorization": authorization}}})
            .to_string();
    let mut frame = vec![0x81];
    match payload.len() {
        n if n < 126 => frame.push(0x80 | n as u8),
        n => {
            frame.push(0x80 | 126);
            frame.extend_from_slice(&(n as u16).to_be_bytes());
        }
    }
    frame.extend_from_slice(&[0; 4]);
    frame.extend_from_slice(payload.as_bytes());
    stream.write_all(&frame).await.unwrap();

    // The first text frame back. A server's frames are not masked.
    loop {
        let opcode = stream.read_u8().await.unwrap() & 0x0f;
        let length = match stream.read_u8().await.unwrap() & 0x7f {
            126 => u64::from(stream.read_u16().await.unwrap()),
            127 => stream.read_u64().await.unwrap(),
            n => u64::from(n),
        };
        let mut body = vec![0; length as usize];
        stream.read_exact(&mut body).await.unwrap();
        if opcode == 0x1 {
            return serde_json::from_slice(&body).unwrap();
        }
        assert_ne!(opcode, 0x8, "closed: {}", String::from_utf8_lossy(&body));
    }
}
