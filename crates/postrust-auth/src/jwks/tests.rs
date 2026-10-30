//! A [`Jwks`] against an identity provider made of a hash map: what it
//! fetches, when, and what it does with the answer. The real HTTP clients,
//! and the limits only they can enforce, are tested where they live (the
//! server's `jwks` module).

use super::*;
use crate::{validate_token, JwtCache, JwtConfig, JwtError};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use jsonwebtoken::{encode, EncodingKey, Header};
use serde_json::{json, Value};
use std::collections::HashMap;

const URL: &str = "https://idp.example/.well-known/jwks.json";

/// A fixture's private key, and its public JWK with `kid` set.
fn fixture(name: &str, kid: &str) -> (EncodingKey, Value) {
    let all: Value = serde_json::from_str(include_str!("../../tests/fixtures/keys.json")).unwrap();
    let entry = &all[name];
    let der = STANDARD
        .decode(entry["private_der"].as_str().unwrap())
        .unwrap();
    let key = match entry["jwk"]["kty"].as_str().unwrap() {
        "RSA" => EncodingKey::from_rsa_der(&der),
        "EC" => EncodingKey::from_ec_der(&der),
        _ => EncodingKey::from_ed_der(&der),
    };
    let mut jwk = entry["jwk"].clone();
    jwk["kid"] = json!(kid);
    (key, jwk)
}

fn set_of(keys: &[(&str, &str)]) -> String {
    let keys: Vec<Value> = keys
        .iter()
        .map(|(name, kid)| fixture(name, kid).1)
        .collect();
    json!({ "keys": keys }).to_string()
}

fn signed(name: &str, kid: Option<&str>) -> String {
    let mut header = Header::new(Algorithm::RS256);
    header.kid = kid.map(str::to_string);
    let (key, _) = fixture(name, kid.unwrap_or(""));
    encode(&header, &json!({"role": "web_user", "sub": "alice"}), &key).unwrap()
}

/// An identity provider: a response per URL, and a log of what was asked.
#[derive(Clone, Default)]
struct Idp {
    state: Arc<Mutex<IdpState>>,
}

#[derive(Default)]
struct IdpState {
    routes: HashMap<String, Result<FetchResponse, FetchError>>,
    calls: Vec<String>,
    /// How long each answer takes, so that requests can overlap.
    delay: Option<std::time::Duration>,
}

impl Idp {
    fn serving(body: &str) -> Self {
        let idp = Self::default();
        idp.serve(body);
        idp
    }

    fn serve(&self, body: &str) {
        self.answer(URL, Ok(FetchResponse::new(200, body.as_bytes().to_vec())));
    }

    fn answer(&self, url: &str, response: Result<FetchResponse, FetchError>) {
        self.state
            .lock()
            .unwrap()
            .routes
            .insert(url.to_string(), response);
    }

    fn go_down(&self) {
        self.answer(URL, Err(FetchError::new("connection refused")));
    }

    fn calls(&self) -> usize {
        self.state.lock().unwrap().calls.len()
    }
}

#[async_trait::async_trait]
impl Fetch for Idp {
    async fn get(&self, url: &str) -> Result<FetchResponse, FetchError> {
        let delay = {
            let mut state = self.state.lock().unwrap();
            state.calls.push(url.to_string());
            state.delay
        };
        if let Some(delay) = delay {
            tokio::time::sleep(delay).await;
        }
        self.state
            .lock()
            .unwrap()
            .routes
            .get(url)
            .cloned()
            .unwrap_or_else(|| Ok(FetchResponse::new(404, Vec::new())))
    }
}

fn jwks(idp: &Idp) -> Jwks {
    Jwks::builder(URL, idp.clone()).build().unwrap()
}

fn config(jwks: &Jwks) -> JwtConfig {
    JwtConfig::default().with_jwks(jwks.clone())
}

/// Verify as a host does: the async step, then the synchronous check.
async fn verify(token: &str, config: &JwtConfig) -> Result<String, JwtError> {
    let header = format!("Bearer {token}");
    crate::ensure_keys(Some(&header), config).await;
    validate_token(token, config).map(|auth| auth.role)
}

#[tokio::test]
async fn the_set_is_fetched_and_tokens_it_signed_are_accepted() {
    let idp = Idp::serving(&set_of(&[("rsa_a", "a")]));
    let jwks = jwks(&idp);
    jwks.refresh().await.unwrap();

    let config = config(&jwks);
    assert_eq!(
        verify(&signed("rsa_a", Some("a")), &config).await.unwrap(),
        "web_user"
    );
    // A token with no kid is tried against every key, as from a secret.
    assert!(verify(&signed("rsa_a", None), &config).await.is_ok());
    assert!(matches!(
        verify(&signed("rsa_b", Some("a")), &config).await,
        Err(JwtError::InvalidSignature)
    ));

    let status = jwks.status();
    assert!(status.loaded);
    assert_eq!((status.keys, status.refreshes, status.failures), (1, 1, 0));
    assert_eq!(status.refresh_interval, DEFAULT_REFRESH);
    assert_eq!(idp.calls(), 1, "a known kid fetches nothing");
}

#[tokio::test]
async fn never_loaded_refuses_every_token_and_recovers_when_the_idp_does() {
    let idp = Idp::default();
    idp.go_down();
    let jwks = jwks(&idp);
    assert!(jwks.refresh().await.is_err());
    let config = config(&jwks);

    let token = signed("rsa_a", Some("a"));
    // Fail closed, with the answer a token whose key is unknown gets: 401,
    // not the 500 of a server configured with no key at all.
    assert!(matches!(
        verify(&token, &config).await,
        Err(JwtError::NoSuitableKey)
    ));
    let status = jwks.status();
    assert!(!status.loaded);
    assert_eq!(status.consecutive_failures, 2, "the token asked again");
    assert_eq!(status.last_error.as_deref(), Some("connection refused"));

    idp.serve(&set_of(&[("rsa_a", "a")]));
    // The schedule's retry, which a host with a background task makes.
    jwks.refresh().await.unwrap();
    assert!(verify(&token, &config).await.is_ok());
    assert_eq!(jwks.status().consecutive_failures, 0);
}

#[tokio::test]
async fn a_token_arriving_before_any_fetch_fetches() {
    // A host with no background task: the first request loads the keys.
    let idp = Idp::serving(&set_of(&[("rsa_a", "a")]));
    let config = config(&jwks(&idp));
    assert!(verify(&signed("rsa_a", Some("a")), &config).await.is_ok());
    assert_eq!(idp.calls(), 1);
}

#[tokio::test]
async fn an_unknown_kid_refreshes_and_a_rotated_in_key_is_accepted() {
    let idp = Idp::serving(&set_of(&[("rsa_a", "a")]));
    let jwks = jwks(&idp);
    jwks.refresh().await.unwrap();
    let config = config(&jwks);

    // The provider rotates a key in and starts signing with it.
    idp.serve(&set_of(&[("rsa_a", "a"), ("rsa_b", "b")]));
    assert!(verify(&signed("rsa_b", Some("b")), &config).await.is_ok());
    assert_eq!(idp.calls(), 2);
    assert_eq!(jwks.status().changes, 2);
}

#[tokio::test]
async fn unknown_kids_fetch_at_most_once_per_interval() {
    let idp = Idp::serving(&set_of(&[("rsa_a", "a")]));
    let jwks = jwks(&idp);
    jwks.refresh().await.unwrap();
    let config = config(&jwks);

    for kid in ["x", "y", "z"] {
        assert!(matches!(
            verify(&signed("rsa_b", Some(kid)), &config).await,
            Err(JwtError::NoSuitableKey)
        ));
    }
    assert_eq!(idp.calls(), 2, "one fetch for three unknown kids in 30 s");

    // A key that was rotated in during the window is still found once the
    // window has passed.
    let quick = Jwks::builder(URL, idp.clone())
        .unknown_kid_interval(Duration::ZERO)
        .build()
        .unwrap();
    quick.refresh().await.unwrap();
    let config = JwtConfig::default().with_jwks(quick);
    let before = idp.calls();
    for kid in ["x", "y"] {
        let _ = verify(&signed("rsa_b", Some(kid)), &config).await;
    }
    assert_eq!(idp.calls(), before + 2);
}

#[tokio::test]
async fn requests_waiting_on_one_fetch_share_its_outcome() {
    let idp = Idp::serving(&set_of(&[("rsa_a", "a")]));
    let jwks = jwks(&idp);
    jwks.refresh().await.unwrap();
    let config = config(&jwks);

    idp.serve(&set_of(&[("rsa_a", "a"), ("rsa_b", "b")]));
    idp.state.lock().unwrap().delay = Some(std::time::Duration::from_millis(50));
    let token = signed("rsa_b", Some("b"));
    let outcomes = futures::future::join_all((0..8).map(|_| verify(&token, &config))).await;
    assert!(outcomes.iter().all(Result::is_ok), "{outcomes:?}");
    assert_eq!(idp.calls(), 2, "eight requests, one fetch");
}

#[tokio::test]
async fn a_failed_refresh_keeps_the_last_good_set() {
    let idp = Idp::serving(&set_of(&[("rsa_a", "a")]));
    let jwks = jwks(&idp);
    jwks.refresh().await.unwrap();
    let config = config(&jwks);

    for failure in [
        Err(FetchError::new("timed out")),
        Ok(FetchResponse::new(503, b"busy".to_vec())),
        Ok(FetchResponse::new(
            200,
            b"<html>maintenance</html>".to_vec(),
        )),
        Ok(FetchResponse::new(200, br#"{"keys": []}"#.to_vec())),
    ] {
        idp.answer(URL, failure);
        assert!(jwks.refresh().await.is_err());
        assert!(verify(&signed("rsa_a", Some("a")), &config).await.is_ok());
    }
    let status = jwks.status();
    assert!(status.loaded);
    assert_eq!((status.failures, status.consecutive_failures), (4, 4));
    assert!(status.next_refresh_in <= MAX_RETRY);
}

#[tokio::test]
async fn a_withdrawn_key_revokes_the_tokens_cached_under_it() {
    let idp = Idp::serving(&set_of(&[("rsa_a", "a"), ("rsa_b", "b")]));
    let jwks = jwks(&idp);
    jwks.refresh().await.unwrap();
    let config = config(&jwks);
    let cache = JwtCache::new(true, 3600).unwrap();

    let by_b = format!("Bearer {}", signed("rsa_b", Some("b")));
    let by_a = format!("Bearer {}", signed("rsa_a", Some("a")));
    assert!(cache.authenticate(Some(&by_b), &config).is_ok());
    assert!(cache.authenticate(Some(&by_a), &config).is_ok());

    idp.serve(&set_of(&[("rsa_a", "a")]));
    jwks.refresh().await.unwrap();
    assert!(matches!(
        cache.authenticate(Some(&by_b), &config),
        Err(JwtError::NoSuitableKey)
    ));
    assert!(cache.authenticate(Some(&by_a), &config).is_ok());
}

#[tokio::test]
async fn the_same_set_fetched_again_changes_nothing() {
    let idp = Idp::serving(&set_of(&[("rsa_a", "a")]));
    let jwks = jwks(&idp);
    jwks.refresh().await.unwrap();
    let generation = jwks.keys().generation();
    jwks.refresh().await.unwrap();
    assert_eq!(
        jwks.keys().generation(),
        generation,
        "the token cache is kept"
    );
    assert_eq!(jwks.status().changes, 1);
    assert_eq!(jwks.status().refreshes, 2);
}

#[tokio::test]
async fn cache_control_max_age_sets_the_interval_within_bounds() {
    let idp = Idp::default();
    let jwks = Jwks::builder(URL, idp.clone())
        .refresh(Duration::from_secs(42))
        .build()
        .unwrap();
    let body = set_of(&[("rsa_a", "a")]);
    for (header, interval) in [
        (None, 42),
        (Some("public, max-age=120"), 120),
        (Some("max-age=5, stale-while-revalidate=15"), 60),
        (Some("no-store"), 60),
        (Some("max-age=999999999"), 86_400),
        (Some("max-age=soon"), 42),
    ] {
        idp.answer(
            URL,
            Ok(FetchResponse::new(200, body.as_bytes().to_vec())
                .with_cache_control(header.map(str::to_string))),
        );
        jwks.refresh().await.unwrap();
        let status = jwks.status();
        assert_eq!(
            status.refresh_interval,
            Duration::from_secs(interval),
            "{header:?}"
        );
        assert!(status.next_refresh_in <= Duration::from_secs(interval));
        assert!(status.next_refresh_in > Duration::from_secs(interval - 2));
    }
}

#[tokio::test]
async fn a_due_set_is_refreshed_by_a_request_when_nothing_else_will() {
    let idp = Idp::serving(&set_of(&[("rsa_a", "a")]));
    let jwks = Jwks::builder(URL, idp.clone())
        .refresh(Duration::from_secs(1))
        .build()
        .unwrap();
    jwks.refresh().await.unwrap();
    let config = config(&jwks);
    let token = signed("rsa_a", Some("a"));

    assert!(verify(&token, &config).await.is_ok());
    assert_eq!(idp.calls(), 1, "not due yet");
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    assert!(verify(&token, &config).await.is_ok());
    assert_eq!(idp.calls(), 2, "due, and no background task");
}

#[tokio::test]
async fn with_a_secret_both_kinds_of_token_are_accepted() {
    let secret = "reallyreallyreallyreallyverysafe";
    let fixed = JwtKeys::from_config(Some(secret), false).unwrap();
    let idp = Idp::default();
    idp.go_down();
    let jwks = Jwks::builder(URL, idp.clone())
        .fixed(fixed.as_ref())
        .build()
        .unwrap();
    let config = config(&jwks);
    let service = encode(
        &Header::default(),
        &json!({"role": "service_role"}),
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .unwrap();

    // The secret's tokens do not wait for the provider.
    assert_eq!(verify(&service, &config).await.unwrap(), "service_role");
    assert!(verify(&signed("rsa_a", Some("a")), &config).await.is_err());

    idp.serve(&set_of(&[("rsa_a", "a")]));
    jwks.refresh().await.unwrap();
    assert!(verify(&service, &config).await.is_ok());
    assert!(verify(&signed("rsa_a", Some("a")), &config).await.is_ok());
}

#[tokio::test]
async fn redirects_are_followed_on_the_same_host_only() {
    let idp = Idp::default();
    let body = set_of(&[("rsa_a", "a")]);
    let moved =
        |to: &str| Ok(FetchResponse::new(301, Vec::new()).with_location(Some(to.to_string())));
    idp.answer(URL, moved("/keys"));
    idp.answer(
        "https://idp.example/keys",
        Ok(FetchResponse::new(200, body.as_bytes().to_vec())),
    );
    let jwks = jwks(&idp);
    jwks.refresh().await.unwrap();

    for (to, why) in [
        ("https://evil.example/keys", "another host"),
        ("http://idp.example/keys", "plain http"),
    ] {
        idp.answer(URL, moved(to));
        let e = jwks.refresh().await.unwrap_err().to_string();
        assert!(
            e.contains(if why == "plain http" {
                "plain HTTP"
            } else {
                why
            }),
            "{e}"
        );
    }

    idp.answer(URL, moved(URL));
    let e = jwks.refresh().await.unwrap_err().to_string();
    assert!(e.contains("redirects"), "{e}");
    assert_eq!(idp.calls(), 2 + 2 + MAX_REDIRECTS + 1);
}

#[test]
fn only_https_or_loopback_urls_are_accepted() {
    for ok in [
        URL,
        "http://localhost:9999/jwks",
        "http://127.0.0.1/jwks",
        "http://127.8.9.10/jwks",
        "http://[::1]:8080/jwks",
    ] {
        assert!(check_url(ok).is_ok(), "{ok}");
    }
    for refused in [
        "http://idp.example/jwks",
        "http://127.0.0.1.nip.io/jwks",
        "http://127.0.0.1@idp.example/jwks",
        "http://10.0.0.1/jwks",
        "ftp://idp.example/jwks",
        "/.well-known/jwks.json",
        "",
    ] {
        assert!(check_url(refused).is_err(), "{refused}");
    }
}

#[tokio::test]
async fn an_oversized_body_is_refused_even_if_the_client_read_it() {
    let idp = Idp::default();
    idp.answer(
        URL,
        Ok(FetchResponse::new(200, vec![b' '; MAX_JWKS_BYTES + 1])),
    );
    let e = jwks(&idp).refresh().await.unwrap_err().to_string();
    assert!(e.contains("larger"), "{e}");
}

/// A [`JwksStore`] made of a cell, as KV is to a Worker.
#[derive(Clone, Default)]
struct Store {
    held: Arc<Mutex<Option<StoredJwks>>>,
}

#[async_trait::async_trait]
impl JwksStore for Store {
    async fn load(&self, _url: &str) -> Option<StoredJwks> {
        self.held.lock().unwrap().clone()
    }
    async fn save(&self, _url: &str, jwks: &StoredJwks, _ttl: Duration) {
        *self.held.lock().unwrap() = Some(jwks.clone());
    }
}

#[tokio::test]
async fn a_cold_start_reads_a_fresh_stored_copy_instead_of_fetching() {
    let idp = Idp::serving(&set_of(&[("rsa_a", "a")]));
    let store = Store::default();
    let warm = Jwks::builder(URL, idp.clone())
        .store(store.clone())
        .build()
        .unwrap();
    warm.refresh().await.unwrap();
    assert!(store.held.lock().unwrap().is_some(), "a fetch is stored");

    let cold = Jwks::builder(URL, idp.clone())
        .store(store.clone())
        .build()
        .unwrap();
    let config = config(&cold);
    assert!(verify(&signed("rsa_a", Some("a")), &config).await.is_ok());
    assert_eq!(idp.calls(), 1, "the cold one read the store");
    assert!(cold.status().loaded);

    // An unknown kid is a question for the provider, not for the store.
    idp.serve(&set_of(&[("rsa_a", "a"), ("rsa_b", "b")]));
    assert!(verify(&signed("rsa_b", Some("b")), &config).await.is_ok());
    assert_eq!(idp.calls(), 2);

    // A stale copy is not used.
    store.held.lock().unwrap().as_mut().unwrap().fresh_until = unix_now() - 1;
    let late = Jwks::builder(URL, idp.clone())
        .store(store)
        .build()
        .unwrap();
    assert!(verify(
        &signed("rsa_a", Some("a")),
        &JwtConfig::default().with_jwks(late)
    )
    .await
    .is_ok());
    assert_eq!(idp.calls(), 3);
}
