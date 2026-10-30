//! Keys from `PGRST_JWT_JWKS_URL`, fetched with reqwest.
//!
//! What to fetch, when, and what to make of the answer is decided in
//! [`postrust_auth::jwks`], for every host alike. This is the native half:
//! the HTTP client, and the background task that keeps the set fresh.
//!
//! The client follows no redirects -- the shared code follows a redirect only
//! to the same host, and it cannot apply that rule to one the client already
//! followed -- and enforces the two limits only the party reading the socket
//! can: [`FETCH_TIMEOUT`] over the whole exchange, connecting through the
//! last byte of the body, and [`MAX_JWKS_BYTES`], checked against
//! `Content-Length` first and then while the body streams in, so an endpoint
//! that answers with an endless body is cut off at the cap rather than read
//! into memory.

use std::sync::Arc;
use std::time::Duration;

use postrust_auth::jwks::{Fetch, FetchError, FetchResponse, FETCH_TIMEOUT, MAX_JWKS_BYTES};
use postrust_auth::{JwtConfig, JwtKeyError};
use postrust_core::AppConfig;

use crate::{AppState, Host};

/// An HTTP client for fetching a JWKS.
pub(crate) struct ReqwestFetch {
    client: reqwest::Client,
    timeout: Duration,
    max_bytes: usize,
}

impl ReqwestFetch {
    pub(crate) fn new() -> Result<Self, reqwest::Error> {
        Self::with_limits(FETCH_TIMEOUT, MAX_JWKS_BYTES)
    }

    fn with_limits(timeout: Duration, max_bytes: usize) -> Result<Self, reqwest::Error> {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(timeout)
            .user_agent(concat!("postrust/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self {
            client,
            timeout,
            max_bytes,
        })
    }
}

/// What went wrong, with the causes reqwest keeps behind its own summary --
/// "error sending request" alone does not say whether DNS, TLS or the
/// connection failed.
fn describe(e: &reqwest::Error, timeout: Duration) -> String {
    if e.is_timeout() {
        return format!("no answer within {timeout:?}");
    }
    let mut message = e.to_string();
    let mut source = std::error::Error::source(e);
    while let Some(cause) = source {
        message.push_str(": ");
        message.push_str(&cause.to_string());
        source = cause.source();
    }
    message
}

#[async_trait::async_trait]
impl Fetch for ReqwestFetch {
    async fn get(&self, url: &str) -> Result<FetchResponse, FetchError> {
        let too_large = || {
            FetchError::new(format!(
                "the response is larger than {} bytes",
                self.max_bytes
            ))
        };
        let mut response = self
            .client
            .get(url)
            .header(
                reqwest::header::ACCEPT,
                "application/jwk-set+json, application/json",
            )
            .send()
            .await
            .map_err(|e| FetchError::new(describe(&e, self.timeout)))?;
        let header = |name: reqwest::header::HeaderName| {
            response
                .headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string)
        };
        let location = header(reqwest::header::LOCATION);
        let cache_control = header(reqwest::header::CACHE_CONTROL);
        let status = response.status().as_u16();

        if response
            .content_length()
            .is_some_and(|length| length > self.max_bytes as u64)
        {
            return Err(too_large());
        }
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|e| FetchError::new(describe(&e, self.timeout)))?
        {
            if body.len() + chunk.len() > self.max_bytes {
                return Err(too_large());
            }
            body.extend_from_slice(&chunk);
        }
        Ok(FetchResponse::new(status, body)
            .with_location(location)
            .with_cache_control(cache_control))
    }
}

/// `jwt` with keys from `PGRST_JWT_JWKS_URL` attached, when it is set.
pub(crate) fn attach(jwt: JwtConfig, config: &AppConfig) -> Result<JwtConfig, JwtKeyError> {
    if config.jwt_jwks_url.is_none() {
        return Ok(jwt);
    }
    let fetch = ReqwestFetch::new().map_err(|e| {
        JwtKeyError::JwksUrl(format!("no HTTP client: {}", describe(&e, FETCH_TIMEOUT)))
    })?;
    let Some(builder) = postrust_engine::jwks_builder(config, jwt.keys.as_ref(), fetch) else {
        return Ok(jwt);
    };
    let jwks = builder
        .build()
        .map_err(|e| JwtKeyError::JwksUrl(e.to_string()))?;
    Ok(jwt.with_jwks(jwks))
}

/// Fetch the keys before the server answers anything, and keep them fresh
/// where `host` runs background work.
///
/// The first fetch is awaited so that the first request does not wait for
/// it, and so a deployment that cannot reach its identity provider says so in
/// its start-up log. Its failure is not the server's: it starts anyway, not
/// ready (see `/_/ready`), and refuses the provider's tokens until a retry
/// succeeds. On a host with no background work -- Lambda -- the set held
/// lasts as long as the sandbox, and the request that finds it due for a
/// refresh fetches it.
pub(crate) async fn start(state: &Arc<AppState>, host: Host) {
    let Some(jwks) = state.jwt_config.jwks.clone() else {
        return;
    };
    if jwks.refresh().await.is_ok() {
        let status = jwks.status();
        tracing::info!(
            "PGRST_JWT_JWKS_URL: {} key(s) from {}, refreshed every {} s",
            status.keys,
            jwks.url(),
            status.refresh_interval.as_secs()
        );
    }
    if host.background_tasks {
        tokio::spawn(async move { jwks.refresh_forever(tokio::time::sleep).await });
    }
}

#[cfg(test)]
mod tests {
    //! The limits only the client can enforce, against a real socket.

    use super::*;
    use axum::routing::get;
    use axum::Router;

    async fn serve(router: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        format!("http://{address}")
    }

    #[tokio::test]
    async fn status_headers_and_body_come_back_and_redirects_are_not_followed() {
        let base = serve(
            Router::new()
                .route(
                    "/jwks",
                    get(|| async { ([("cache-control", "max-age=600")], "{\"keys\":[]}") }),
                )
                .route(
                    "/moved",
                    get(|| async { axum::response::Redirect::permanent("/jwks") }),
                ),
        )
        .await;
        let fetch = ReqwestFetch::new().unwrap();

        let ok = fetch.get(&format!("{base}/jwks")).await.unwrap();
        assert_eq!(ok.status, 200);
        assert_eq!(ok.cache_control.as_deref(), Some("max-age=600"));
        assert_eq!(ok.body, b"{\"keys\":[]}");

        let moved = fetch.get(&format!("{base}/moved")).await.unwrap();
        assert_eq!(moved.status, 308);
        assert_eq!(moved.location.as_deref(), Some("/jwks"));
    }

    #[tokio::test]
    async fn a_body_over_the_cap_is_refused_whether_or_not_its_length_is_declared() {
        let base = serve(
            Router::new()
                .route("/declared", get(|| async { vec![b' '; 2048] }))
                .route(
                    "/streamed",
                    get(|| async {
                        let chunks = futures::stream::iter(
                            (0..64).map(|_| Ok::<_, std::io::Error>(vec![b' '; 64])),
                        );
                        axum::body::Body::from_stream(chunks)
                    }),
                ),
        )
        .await;
        let fetch = ReqwestFetch::with_limits(FETCH_TIMEOUT, 1024).unwrap();
        for path in ["declared", "streamed"] {
            let e = fetch.get(&format!("{base}/{path}")).await.unwrap_err();
            assert!(e.to_string().contains("larger than 1024"), "{path}: {e}");
        }
        // The real cap, on the server's own client.
        let real = ReqwestFetch::new().unwrap();
        assert_eq!(real.max_bytes, MAX_JWKS_BYTES);
    }

    #[tokio::test]
    async fn an_endpoint_that_does_not_answer_times_out() {
        let base = serve(Router::new().route(
            "/slow",
            get(|| async {
                tokio::time::sleep(Duration::from_secs(30)).await;
                "{}"
            }),
        ))
        .await;
        let fetch = ReqwestFetch::with_limits(Duration::from_millis(200), MAX_JWKS_BYTES).unwrap();
        let started = std::time::Instant::now();
        let e = fetch.get(&format!("{base}/slow")).await.unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(e.to_string().starts_with("no answer within"), "{e}");
    }

    #[test]
    fn a_url_keys_cannot_safely_be_fetched_from_is_a_configuration_error() {
        let mut config = AppConfig::default();
        config.jwt_jwks_url = Some("http://idp.example/jwks".into());
        let e = attach(JwtConfig::default(), &config).unwrap_err();
        assert!(e.to_string().contains("plain HTTP"), "{e}");

        config.jwt_jwks_url = Some("https://idp.example/jwks".into());
        let jwt = attach(JwtConfig::default(), &config).unwrap();
        let jwks = jwt.jwks.expect("attached");
        assert!(!jwks.is_loaded());
        assert_eq!(jwks.status().refresh_interval.as_secs(), 300);
    }
}
