//! The caller of a route of your own, as the REST API would see them.
//!
//! A hand-written route nested beside the API is reached by the same clients
//! with the same tokens, and a table's row-level security policy was written
//! for the role and claims the API runs a request as. [`User`] is the API's
//! answer to "who is this?", taken from a request to your route: the token is
//! checked by the function REST checks it with
//! ([`postrust_engine::authenticate`]), through the same cache, with the same
//! anonymous role and the same refusals. Handed to
//! [`AppState::begin_as`](crate::AppState::begin_as), it is a transaction the
//! policy applies to.

use crate::error::Error;
use crate::state::AppState;
use axum::extract::{FromRequestParts, OptionalFromRequestParts, OriginalUri};
use http::request::Parts;
use indexmap::IndexMap;
use postrust_auth::AuthResult;
use postrust_core::trace::{TraceContext, TraceId};
use std::collections::HashMap;
use std::sync::Arc;

/// Who a request to a route of your own is, authenticated as REST does.
///
/// ```no_run
/// use axum::{extract::State, Json};
/// use postrust_server::{AppState, User};
/// use serde_json::{json, Value};
/// use std::sync::Arc;
///
/// async fn summary(
///     user: User,
///     State(state): State<Arc<AppState>>,
/// ) -> Result<Json<Value>, postrust_server::Error> {
///     let mut tx = state.begin_as(&user).await?;
///     let n: i64 = sqlx::query_scalar("select count(*) from todos")
///         .fetch_one(&mut *tx)
///         .await?;
///     tx.commit().await?;
///     Ok(Json(json!({ "todos": n })))
/// }
/// ```
///
/// As an extractor, `User` reads the `Authorization` header. A request with
/// no token is the anonymous role (`PGRST_DB_ANON_ROLE`), or refused with
/// PostgREST's 401 where there is none; a token that does not verify is
/// refused with the status and body the API gives it -- PGRST301, PGRST303 and
/// the rest, with the `WWW-Authenticate` challenge.
///
/// # `Option<User>`
///
/// `Option<User>` is for a route that does something different for a caller
/// who has not signed in, whatever the server's anonymous role: it is `None`
/// when the request sends no `Authorization` header at all, and `Some` with
/// the token's user when it sends one that verifies. A token that does *not*
/// verify is still refused, not read as `None`: a client whose token has
/// expired should be told so, not quietly answered as somebody else. A route
/// that wants the anonymous role for a request with no token takes `User`
/// itself; see [`User::is_anonymous`].
///
/// # What else it carries
///
/// What REST tells the database about a request besides who sent it, so that
/// a policy or function reading `request.headers`, `request.cookies`,
/// `request.method`, `request.path` or `request.trace_id` sees the same thing
/// in a transaction of yours that it sees in REST's:
///
/// - the method, and the path as the client sent it -- with the prefix your
///   router was nested at, where REST's `request.path` is without `/api`,
///   since that prefix is the API's mount rather than part of the resource;
/// - every header, by its lowercase name, and the cookies;
/// - the trace the request belongs to, from the server's request span.
///
/// Not the schema a `Accept-Profile` header names, nor the time zone a
/// `Prefer: timezone` asks for: those are REST's request syntax, and a route
/// of yours has its own. Its transaction's `search_path` starts at the default
/// schema.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct User {
    auth: AuthResult,
    anonymous: bool,
    method: String,
    path: String,
    headers: IndexMap<String, String>,
    cookies: IndexMap<String, String>,
    trace_id: Option<TraceId>,
}

impl User {
    /// The database role the request runs as: the token's role claim, or the
    /// anonymous role.
    pub fn role(&self) -> &str {
        &self.auth.role
    }

    /// The token's claims, every one of them, as `request.jwt.claims` carries
    /// them. Empty for an anonymous request.
    pub fn claims(&self) -> &HashMap<String, serde_json::Value> {
        &self.auth.claims
    }

    /// One of the token's claims, where it has it.
    pub fn claim(&self, name: &str) -> Option<&serde_json::Value> {
        self.auth.claims.get(name)
    }

    /// Whether the request sent no token, and runs as the anonymous role.
    pub fn is_anonymous(&self) -> bool {
        self.anonymous
    }

    /// The trace the request belongs to, where the server's request span
    /// gave it one.
    pub fn trace_id(&self) -> Option<TraceId> {
        self.trace_id
    }

    /// The transaction-making half of this, for
    /// [`AppState::begin_as_with`](crate::AppState::begin_as_with).
    pub(crate) fn session<'a>(
        &self,
        config: &'a postrust_core::AppConfig,
    ) -> postrust_core::Session<'a> {
        postrust_core::Session::new(config, self.auth.role.clone(), self.auth.claim_settings())
            .request(self.method.clone(), self.path.clone())
            .headers(&self.headers)
            .cookies(&self.cookies)
            .trace_id(self.trace_id.map(|t| t.to_string()))
    }

    /// The user `parts` authenticates as.
    fn from_parts(parts: &Parts, state: &AppState) -> Result<Self, Error> {
        let auth = postrust_engine::authenticate(state, &parts.headers)?;
        // The path the client sent: a nested router sees its own prefix
        // stripped from `parts.uri`, and axum keeps the original beside it.
        let path = parts
            .extensions
            .get::<OriginalUri>()
            .map(|uri| uri.path())
            .unwrap_or_else(|| parts.uri.path())
            .to_string();
        Ok(Self {
            auth,
            anonymous: postrust_engine::authorization(&parts.headers).is_none(),
            method: parts.method.to_string(),
            path,
            headers: request_headers(&parts.headers),
            cookies: request_cookies(&parts.headers),
            trace_id: parts
                .extensions
                .get::<TraceContext>()
                .map(|context| context.trace_id),
        })
    }
}

impl FromRequestParts<Arc<AppState>> for User {
    type Rejection = Error;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        Self::from_parts(parts, state)
    }
}

impl OptionalFromRequestParts<Arc<AppState>> for User {
    type Rejection = Error;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> Result<Option<Self>, Self::Rejection> {
        match postrust_engine::authorization(&parts.headers) {
            None => Ok(None),
            Some(_) => Self::from_parts(parts, state).map(Some),
        }
    }
}

/// Every header, as REST's `request.headers` has them: by name, lowercase as
/// `http` keeps them, and only those whose value is visible ASCII. Of a
/// header sent more than once the last is kept, as REST keeps it.
fn request_headers(headers: &http::HeaderMap) -> IndexMap<String, String> {
    headers
        .iter()
        .filter_map(|(k, v)| v.to_str().ok().map(|v| (k.to_string(), v.to_string())))
        .collect()
}

/// The `Cookie` header's pairs, as REST's `request.cookies` has them.
fn request_cookies(headers: &http::HeaderMap) -> IndexMap<String, String> {
    headers
        .get(http::header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .map(|s| {
            s.split(';')
                .filter_map(|cookie| {
                    let (key, value) = cookie.trim().split_once('=')?;
                    Some((key.to_string(), value.to_string()))
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two maps are written out here, where REST's are private to
    /// `parse_request`. This is what keeps them the same: the request REST
    /// would see, and what each makes of it.
    #[test]
    fn headers_and_cookies_are_read_as_rest_reads_them() {
        let request = http::Request::get("/items")
            .header("accept", "application/json")
            .header("X-Tenant", "acme")
            .header("x-repeated", "one")
            .header("x-repeated", "two")
            .header("cookie", "session=abc; theme=dark;broken; a=b=c")
            .body(Vec::<u8>::new())
            .unwrap();
        let rest = postrust_core::parse_request(&request, "public", &["public".to_string()], None)
            .expect("REST reads the request");

        assert_eq!(request_headers(request.headers()), rest.headers);
        assert_eq!(request_cookies(request.headers()), rest.cookies);
        assert_eq!(rest.cookies.get("a").map(String::as_str), Some("b=c"));
    }
}
