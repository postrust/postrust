//! Keys from `PGRST_JWT_JWKS_URL`, fetched with the Workers Fetch API and
//! shared between isolates through KV.
//!
//! What to fetch, when, and what to make of the answer is decided in
//! `postrust_auth::jwks`, as on the server. This is what only a Worker has:
//!
//! - **The Fetch API**, asked not to follow redirects (`redirect: "manual"`)
//!   so that the shared rule -- same host only -- is the only one, and cut
//!   off after `FETCH_TIMEOUT` with an `AbortController`, the body read as a
//!   stream and abandoned past `MAX_JWKS_BYTES`.
//! - **A copy in KV** (`POSTRUST_SCHEMA_CACHE`, under a key of its own), with
//!   the time until which it is fresh. Isolates come and go far more often
//!   than an identity provider rotates keys; without the copy every cold
//!   isolate would ask the provider again. An isolate that loads its keys,
//!   or finds them due, reads KV first and fetches only when the copy is
//!   missing or stale; every fetch writes it. An unknown `kid` always goes to
//!   the provider, since the copy is exactly what may be out of date. The
//!   copy expires with its freshness (at least KV's 60 seconds), so a stale
//!   one is not left behind.

use crate::settings;
use postrust_auth::jwks::{
    FetchError, FetchResponse, JwksStore, StoredJwks, FETCH_TIMEOUT, MAX_JWKS_BYTES,
};
use std::time::Duration;
use worker::async_trait::async_trait;

/// The Workers Fetch API, as `postrust_auth::jwks::Fetch` asks for it.
pub(crate) struct WorkerFetch;

fn failed(e: worker::Error) -> FetchError {
    FetchError::new(e.to_string())
}

async fn exchange(url: &str, signal: &worker::AbortSignal) -> Result<FetchResponse, FetchError> {
    use futures::StreamExt;

    let headers = worker::Headers::new();
    headers
        .set("accept", "application/jwk-set+json, application/json")
        .map_err(failed)?;
    let mut init = worker::RequestInit::new();
    init.with_redirect(worker::RequestRedirect::Manual)
        .with_headers(headers);
    let request = worker::Request::new_with_init(url, &init).map_err(failed)?;
    let mut response = worker::Fetch::Request(request)
        .send_with_signal(signal)
        .await
        .map_err(failed)?;

    let status = response.status_code();
    let header = |name: &str| response.headers().get(name).ok().flatten();
    let location = header("location");
    let cache_control = header("cache-control");
    let too_large = || {
        FetchError::new(format!(
            "the response is larger than {MAX_JWKS_BYTES} bytes"
        ))
    };

    // Only a 200's body is ever used; the rest are not read at all.
    let mut body = Vec::new();
    if status == 200 {
        if header("content-length")
            .and_then(|length| length.parse::<u64>().ok())
            .is_some_and(|length| length > MAX_JWKS_BYTES as u64)
        {
            return Err(too_large());
        }
        let mut stream = response.stream().map_err(failed)?;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(failed)?;
            if body.len() + chunk.len() > MAX_JWKS_BYTES {
                return Err(too_large());
            }
            body.extend_from_slice(&chunk);
        }
    }
    Ok(FetchResponse::new(status, body)
        .with_location(location)
        .with_cache_control(cache_control))
}

#[async_trait(?Send)]
impl postrust_auth::jwks::Fetch for WorkerFetch {
    async fn get(&self, url: &str) -> Result<FetchResponse, FetchError> {
        let controller = worker::AbortController::default();
        let signal = controller.signal();
        let fetch = std::pin::pin!(exchange(url, &signal));
        let timeout = worker::Delay::from(FETCH_TIMEOUT);
        match futures::future::select(fetch, timeout).await {
            futures::future::Either::Left((outcome, _)) => outcome,
            futures::future::Either::Right(_) => {
                controller.abort();
                Err(FetchError::new(format!(
                    "no answer within {} s",
                    FETCH_TIMEOUT.as_secs()
                )))
            }
        }
    }
}

/// The KV copy of the fetched set.
pub(crate) struct KvJwks(pub(crate) worker::KvStore);

#[async_trait(?Send)]
impl JwksStore for KvJwks {
    async fn load(&self, url: &str) -> Option<StoredJwks> {
        match self.0.get(&settings::jwks_key(url)).text().await {
            Ok(Some(text)) => serde_json::from_str(&text).ok(),
            Ok(None) => None,
            Err(e) => {
                worker::console_warn!("postrust: the JWKS in KV could not be read: {e}");
                None
            }
        }
    }

    async fn save(&self, url: &str, jwks: &StoredJwks, ttl: Duration) {
        let written = match serde_json::to_string(jwks) {
            Ok(json) => match self.0.put(&settings::jwks_key(url), json) {
                Ok(put) => put
                    .expiration_ttl(settings::kv_expiration_secs(ttl.as_secs_f64()))
                    .execute()
                    .await
                    .map_err(|e| e.to_string()),
                Err(e) => Err(e.to_string()),
            },
            Err(e) => Err(e.to_string()),
        };
        if let Err(e) = written {
            worker::console_error!("postrust: could not write the JWKS to KV: {e}");
        }
    }
}
