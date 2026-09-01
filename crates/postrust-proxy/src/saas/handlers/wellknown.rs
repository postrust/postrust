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
