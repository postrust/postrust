//! Well-known endpoint handlers for domain verification.

use crate::saas::handlers::SaasState;
use crate::saas::wellknown_host::host_matches;
use axum::{
    extract::{Path, State},
    http::{header::HOST, HeaderMap, StatusCode},
    response::IntoResponse,
};

/// What an unknown, expired, resolved, or wrong-host token gets.
///
/// The same answer for all of them on purpose: distinguishing "no such token"
/// from "that token is for another domain" would confirm a token's existence
/// to whoever is probing.
const NOT_FOUND: (StatusCode, &str) = (StatusCode::NOT_FOUND, "no such challenge");

/// Serve an HTTP domain-verification challenge.
///
/// Responds at `/.well-known/postrust-verification/{token}` with the content
/// recorded when the challenge was issued, and only for the domain it was
/// issued for.
///
/// This used to compute `postrust-verify={token}` from whatever token was in
/// the path and return it, with no database lookup and no host check -- so
/// every token verified, for every domain, and the check proved nothing. It
/// now answers only for a challenge that is in the database, is not expired,
/// is not already resolved, and whose domain matches the request's `Host`.
///
/// The host check is what keeps a token scoped. A token is a bearer secret for
/// exactly one domain; answering with it on some other host hands that secret
/// to whoever asked for it.
///
/// A caveat worth stating plainly, because no amount of care here removes it:
/// HTTP verification asks the claimant to serve content at a path on the
/// domain, and once that domain's DNS points at this proxy, this proxy is what
/// serves that path. Passing therefore shows the domain resolves here and a
/// challenge was issued for it -- not that the claimant controls the domain.
/// DNS verification does show that, which is why it is the default
/// (`verification_method` defaults to `dns`). Prefer it.
pub async fn handle_verification_challenge(
    State(state): State<SaasState>,
    headers: HeaderMap,
    Path(token): Path<String>,
) -> impl IntoResponse {
    let challenge = match state.domain_manager.find_live_http_challenge(&token).await {
        Ok(Some(challenge)) => challenge,
        Ok(None) => return NOT_FOUND.into_response(),
        Err(error) => {
            // A lookup failure is ours, not the caller's, and must not be
            // reported as "no such challenge" -- that would read as a failed
            // verification rather than an outage.
            tracing::error!(%error, "verification challenge lookup failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "lookup failed").into_response();
        }
    };

    let host = headers.get(HOST).and_then(|value| value.to_str().ok());
    if !host_matches(host, &challenge.domain) {
        tracing::warn!(
            host = host.unwrap_or("<none>"),
            expected = %challenge.domain,
            "verification challenge requested on the wrong host"
        );
        return NOT_FOUND.into_response();
    }

    (StatusCode::OK, challenge.expected_value).into_response()
}

/// Serve an ACME HTTP-01 challenge.
///
/// Responds at `/.well-known/acme-challenge/{token}` with the key
/// authorization the issuer recorded when it placed the order. The CA fetches
/// this over plain HTTP -- it is how it decides the requester controls the
/// domain -- so the route is public and unauthenticated by necessity.
///
/// Answering is safe because the value is not a secret we are being tricked
/// into revealing: it is `{token}.{account_key_thumbprint}`, which we wrote
/// ourselves, for a token the CA has just told us it will ask for. A token
/// nobody stored gets a 404.
///
/// The lookup goes to `proxy_acme_challenges` rather than to memory on purpose:
/// the CA resolves the domain and reaches whichever instance DNS sends it to,
/// which is usually not the one that placed the order.
///
/// Unlike the domain-verification endpoint, this deliberately does **not**
/// match on `Host`. RFC 8555 lets the CA validate from several vantage points
/// and follow redirects, and the token already scopes the response to one
/// order; requiring a `Host` match here would fail validation for a domain
/// served under an alias, and buy nothing -- the value is not sensitive.
///
/// Requires the `acme` feature: without the issuer there is nothing to write a
/// challenge, so the route is not mounted at all rather than answering 404 to
/// every CA that asks.
#[cfg(feature = "acme")]
pub async fn handle_acme_challenge(
    State(state): State<SaasState>,
    Path(token): Path<String>,
) -> impl IntoResponse {
    match crate::tls::find_challenge(&state.pool, &token).await {
        Ok(Some(challenge)) => {
            tracing::debug!(
                domain = %challenge.domain,
                "served an ACME http-01 challenge"
            );
            (StatusCode::OK, challenge.key_authorization).into_response()
        }
        Ok(None) => NOT_FOUND.into_response(),
        Err(error) => {
            tracing::error!(%error, "ACME challenge lookup failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "lookup failed").into_response()
        }
    }
}
