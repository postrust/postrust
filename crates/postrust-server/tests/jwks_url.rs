//! `PGRST_JWT_JWKS_URL`, end to end: an identity provider served by axum on
//! a loopback port, the application `postrust_server::build` makes, and RLS
//! policies that read the token's `sub`.
//!
//! The provider signs with the keys in crates/postrust-auth/tests/fixtures,
//! generated once for the tests of `KeySet`. What the provider answers is
//! changed between requests to stand for a rotation, a withdrawal or an
//! outage.
//!
//! `DATABASE_URL` must name a role that can become `web_anon` and `web_user`,
//! on a database with `scripts/init-db.sql` and `scripts/test-fixtures.sql`
//! loaded (user 1 owns the draft post 5, user 2 the draft post 10):
//!
//! ```text
//! cargo test --package postrust-server --test jwks_url -- --ignored
//! ```

use axum::body::Body;
use axum::extract::State;
use axum::http::{Request, StatusCode};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use base64::Engine;
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use postrust_core::AppConfig;
use postrust_server::{build, App, AppState, Builder, Host, User};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tower::ServiceExt;

const SECRET: &str = "reallyreallyreallyreallyverysafe";

/// A fixture's private key, and its public JWK under `kid`.
fn fixture(name: &str, kid: &str) -> (EncodingKey, Value) {
    let all: Value =
        serde_json::from_str(include_str!("../../postrust-auth/tests/fixtures/keys.json")).unwrap();
    let der = base64::engine::general_purpose::STANDARD
        .decode(all[name]["private_der"].as_str().unwrap())
        .unwrap();
    let mut jwk = all[name]["jwk"].clone();
    jwk["kid"] = json!(kid);
    jwk["alg"] = json!("RS256");
    (EncodingKey::from_rsa_der(&der), jwk)
}

fn jwks(keys: &[(&str, &str)]) -> String {
    let keys: Vec<Value> = keys
        .iter()
        .map(|(name, kid)| fixture(name, kid).1)
        .collect();
    json!({ "keys": keys }).to_string()
}

/// A token for user `sub`, signed by fixture `name` and naming `kid`.
fn token(name: &str, kid: &str, sub: &str) -> String {
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(kid.to_string());
    let claims = json!({"role": "web_user", "sub": sub, "exp": 4_000_000_000u64});
    encode(&header, &claims, &fixture(name, kid).0).unwrap()
}

/// What the provider answers: a JWKS, or 503 when `body` is `None`.
#[derive(Default)]
struct Answer {
    body: Option<String>,
    cache_control: Option<String>,
    hits: usize,
}

#[derive(Clone, Default)]
struct Idp(Arc<Mutex<Answer>>);

impl Idp {
    fn serve(&self, body: &str) {
        self.0.lock().unwrap().body = Some(body.to_string());
    }
    fn go_down(&self) {
        self.0.lock().unwrap().body = None;
    }
    fn hits(&self) -> usize {
        self.0.lock().unwrap().hits
    }
}

async fn answer(State(idp): State<Idp>) -> axum::response::Response {
    let mut answer = idp.0.lock().unwrap();
    answer.hits += 1;
    match &answer.body {
        Some(body) => {
            let mut response = body.clone().into_response();
            if let Some(cache_control) = &answer.cache_control {
                response
                    .headers_mut()
                    .insert("cache-control", cache_control.parse().unwrap());
            }
            response
        }
        None => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}

/// Serve `idp` on `listener`.
fn spawn_idp(idp: &Idp, listener: tokio::net::TcpListener) {
    let router = Router::new()
        .route("/.well-known/jwks.json", get(answer))
        .with_state(idp.clone());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
}

/// A provider on a port of its own, and its JWKS URL.
async fn idp(body: Option<&str>) -> (Idp, String) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!(
        "http://{}/.well-known/jwks.json",
        listener.local_addr().unwrap()
    );
    let idp = Idp::default();
    if let Some(body) = body {
        idp.serve(body);
    }
    spawn_idp(&idp, listener);
    (idp, url)
}

fn config(jwks_url: &str) -> AppConfig {
    let mut config = AppConfig::default();
    config.db_uri = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/postrust_test".into());
    config.db_schemas = vec!["public".to_string()];
    config.db_anon_role = Some("web_anon".to_string());
    config.jwt_jwks_url = Some(jwks_url.to_string());
    config
}

async fn app(config: AppConfig) -> App {
    build(config, Host::SERVER).await.expect("build")
}

async fn get_json(app: &App, path: &str, token: Option<&str>) -> (StatusCode, Value) {
    let mut request = Request::get(path);
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    let response = app
        .router
        .clone()
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
}

/// The drafts the token's user may read: their own, by RLS on `sub`.
async fn drafts(app: &App, token: &str) -> (StatusCode, Value) {
    get_json(
        app,
        "/api/posts?status=eq.draft&select=id&order=id",
        Some(token),
    )
    .await
}

#[tokio::test]
#[ignore = "needs PostgreSQL"]
async fn the_keys_are_fetched_at_start_and_rls_sees_the_tokens_sub() {
    let (idp, url) = idp(Some(&jwks(&[("rsa_a", "a")]))).await;
    let app = app(config(&url)).await;
    assert_eq!(idp.hits(), 1, "fetched before serving");

    assert_eq!(
        drafts(&app, &token("rsa_a", "a", "1")).await,
        (StatusCode::OK, json!([{"id": 5}]))
    );
    assert_eq!(
        drafts(&app, &token("rsa_a", "a", "2")).await,
        (StatusCode::OK, json!([{"id": 10}]))
    );
    // Signed by a key the provider does not publish, under its kid.
    let (status, body) = drafts(&app, &token("rsa_b", "a", "1")).await;
    assert_eq!(
        (status, body["code"].as_str()),
        (StatusCode::UNAUTHORIZED, Some("PGRST301"))
    );

    let (status, ready) = get_json(&app, "/_/ready", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(ready["jwks"], json!(true));
    assert_eq!(idp.hits(), 1, "known kids fetch nothing");
}

#[tokio::test]
#[ignore = "needs PostgreSQL"]
async fn a_rotated_in_key_is_fetched_when_first_seen_and_unknown_kids_are_rate_limited() {
    let (idp, url) = idp(Some(&jwks(&[("rsa_a", "a")]))).await;
    let app = app(config(&url)).await;

    idp.serve(&jwks(&[("rsa_a", "a"), ("rsa_b", "b")]));
    assert_eq!(
        drafts(&app, &token("rsa_b", "b", "1")).await,
        (StatusCode::OK, json!([{"id": 5}]))
    );
    assert_eq!(idp.hits(), 2);

    // Kids nobody publishes, again and again: the fetch for `b` was this
    // 30 s's one, so they are refused with PGRST301 without asking again.
    for kid in ["x", "y", "z"] {
        let (status, body) = drafts(&app, &token("rsa_b", kid, "1")).await;
        assert_eq!(
            (status, body["code"].as_str()),
            (StatusCode::UNAUTHORIZED, Some("PGRST301"))
        );
    }
    assert_eq!(idp.hits(), 2);
}

#[tokio::test]
#[ignore = "needs PostgreSQL"]
async fn a_provider_down_at_boot_fails_closed_and_the_server_recovers_when_it_is_up() {
    // A port nothing listens on: connection refused.
    let reserved = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = reserved.local_addr().unwrap();
    drop(reserved);
    let url = format!("http://{address}/.well-known/jwks.json");

    let app = app(config(&url)).await;
    let token = token("rsa_a", "a", "1");
    let (status, body) = drafts(&app, &token).await;
    assert_eq!(
        (status, body["code"].as_str()),
        (StatusCode::UNAUTHORIZED, Some("PGRST301"))
    );
    // Anonymous requests are still served: only tokens are refused.
    assert_eq!(
        get_json(&app, "/api/posts?select=id&limit=1", None).await.0,
        StatusCode::OK
    );
    let (status, ready) = get_json(&app, "/_/ready", None).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        (
            ready["ready"].clone(),
            ready["database"].clone(),
            ready["jwks"].clone()
        ),
        (json!(false), json!(true), json!(false))
    );
    assert!(
        ready["message"]
            .as_str()
            .unwrap()
            .contains("PGRST_JWT_JWKS_URL"),
        "{ready}"
    );

    // The provider comes up; the background task's retry finds it.
    let idp = Idp::default();
    idp.serve(&jwks(&[("rsa_a", "a")]));
    spawn_idp(&idp, tokio::net::TcpListener::bind(address).await.unwrap());
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    while get_json(&app, "/_/ready", None).await.0 != StatusCode::OK {
        assert!(tokio::time::Instant::now() < deadline, "never became ready");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    assert_eq!(
        drafts(&app, &token).await,
        (StatusCode::OK, json!([{"id": 5}]))
    );
}

#[tokio::test]
#[ignore = "needs PostgreSQL"]
async fn a_provider_down_later_leaves_the_last_good_keys_in_force() {
    let (idp, url) = idp(Some(&jwks(&[("rsa_a", "a")]))).await;
    let app = app(config(&url)).await;

    idp.go_down();
    // An unknown kid makes the server ask, and the ask fails.
    assert_eq!(
        drafts(&app, &token("rsa_b", "b", "1")).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(idp.hits(), 2);
    let status = app.state.jwt_config.jwks.as_ref().unwrap().status();
    assert_eq!((status.loaded, status.consecutive_failures), (true, 1));

    assert_eq!(
        drafts(&app, &token("rsa_a", "a", "1")).await,
        (StatusCode::OK, json!([{"id": 5}]))
    );
    assert_eq!(get_json(&app, "/_/ready", None).await.0, StatusCode::OK);
}

#[tokio::test]
#[ignore = "needs PostgreSQL"]
async fn a_withdrawn_key_revokes_its_cached_tokens_at_once() {
    let (idp, url) = idp(Some(&jwks(&[("rsa_a", "a"), ("rsa_b", "b")]))).await;
    let app = app(config(&url)).await;
    assert!(
        app.state.jwt_cache.is_some(),
        "the token cache is on by default"
    );

    let by_b = token("rsa_b", "b", "1");
    assert_eq!(drafts(&app, &by_b).await.0, StatusCode::OK);
    assert_eq!(
        drafts(&app, &by_b).await.0,
        StatusCode::OK,
        "now from the cache"
    );

    // The provider withdraws b. The next fetch -- here, the one an unknown
    // kid causes -- drops it, and the cached verification with it.
    idp.serve(&jwks(&[("rsa_a", "a")]));
    assert_eq!(
        drafts(&app, &token("rsa_a", "new", "1")).await.0,
        StatusCode::UNAUTHORIZED
    );
    let (status, body) = drafts(&app, &by_b).await;
    assert_eq!(
        (status, body["code"].as_str()),
        (StatusCode::UNAUTHORIZED, Some("PGRST301"))
    );
}

#[tokio::test]
#[ignore = "needs PostgreSQL"]
async fn cache_control_max_age_sets_when_the_keys_are_fetched_again() {
    let (idp, url) = idp(Some(&jwks(&[("rsa_a", "a")]))).await;
    idp.0.lock().unwrap().cache_control = Some("public, max-age=900".into());
    let mut config = config(&url);
    config.jwt_jwks_refresh = 60;
    let app = app(config).await;

    let status = app.state.jwt_config.jwks.as_ref().unwrap().status();
    assert_eq!(status.refresh_interval, Duration::from_secs(900));
    assert!(status.next_refresh_in > Duration::from_secs(890));
}

#[tokio::test]
#[ignore = "needs PostgreSQL"]
async fn with_a_secret_too_both_kinds_of_token_are_accepted() {
    let (_idp, url) = idp(Some(&jwks(&[("rsa_a", "a")]))).await;
    let mut config = config(&url);
    config.jwt_secret = Some(SECRET.to_string());
    let app = app(config).await;

    let service = encode(
        &Header::default(),
        &json!({"role": "web_user", "sub": "2"}),
        &EncodingKey::from_secret(SECRET.as_bytes()),
    )
    .unwrap();
    assert_eq!(
        drafts(&app, &service).await,
        (StatusCode::OK, json!([{"id": 10}]))
    );
    assert_eq!(
        drafts(&app, &token("rsa_a", "a", "1")).await,
        (StatusCode::OK, json!([{"id": 5}]))
    );
}

/// The `sub` a route of your own is called by, as `User` has it.
async fn whose(user: User) -> axum::Json<Value> {
    axum::Json(json!({ "sub": user.claim("sub") }))
}

#[tokio::test]
#[ignore = "needs PostgreSQL"]
async fn a_route_of_your_own_fetches_a_rotated_in_key_too() {
    let (idp, url) = idp(Some(&jwks(&[("rsa_a", "a")]))).await;
    let routes: Router<Arc<AppState>> = Router::new().route("/whose", get(whose));
    let app = Builder::new(config(&url), Host::SERVER)
        .nest("/mine", routes)
        .build()
        .await
        .expect("build");

    // `User` authenticates as REST does, so a key first seen here is
    // fetched here, rather than refused until REST happens to fetch it.
    idp.serve(&jwks(&[("rsa_a", "a"), ("rsa_b", "b")]));
    assert_eq!(
        get_json(&app, "/mine/whose", Some(&token("rsa_b", "b", "1"))).await,
        (StatusCode::OK, json!({ "sub": "1" }))
    );
    assert_eq!(idp.hits(), 2);
}

#[cfg(feature = "graphql")]
#[tokio::test]
#[ignore = "needs PostgreSQL"]
async fn graphql_fetches_a_rotated_in_key_too() {
    let (idp, url) = idp(Some(&jwks(&[("rsa_a", "a")]))).await;
    let app = app(config(&url)).await;
    idp.serve(&jwks(&[("rsa_a", "a"), ("rsa_b", "b")]));

    let response = app
        .router
        .clone()
        .oneshot(
            Request::post("/v1/graphql")
                .header("content-type", "application/json")
                .header(
                    "authorization",
                    format!("Bearer {}", token("rsa_b", "b", "1")),
                )
                .body(Body::from(json!({"query": "{ __typename }"}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(body.get("errors").is_none(), "{body}");
    assert_eq!(idp.hits(), 2);
}

#[tokio::test]
async fn a_jwks_url_over_plain_http_to_another_host_does_not_start() {
    let mut config = AppConfig::default();
    config.jwt_jwks_url = Some("http://idp.example/.well-known/jwks.json".into());
    let e = build(config, Host::SERVER).await.err().expect("refused");
    assert!(e.to_string().contains("PGRST_JWT_JWKS_URL"), "{e}");
}
