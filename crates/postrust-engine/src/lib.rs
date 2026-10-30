//! The REST request pipeline of [Postrust](https://postrust.org/): an HTTP
//! request in, the response the PostgREST-dialect API gives it out.
//!
//! No web framework is involved. The server (`postrust-server`) wraps this in
//! axum, and the Lambda adapter reaches it through the server. It is meant to
//! be shared with a host that cannot run axum -- a Cloudflare Worker -- too,
//! once the database interface is driver-neutral; it does not build for wasm32
//! yet, since it still talks to PostgreSQL through sqlx. See issue #50.
//!
//! ```text
//! let response = postrust_engine::handle(&state, request).await;
//! ```

#![warn(missing_docs)]
#![allow(clippy::result_large_err)]

mod handler;
pub mod observe;
mod state;

pub use handler::{allow, authenticate, authorization, error_response, handle};
pub use state::State;

use postrust_auth::jwks::{Fetch, JwksBuilder};
use postrust_auth::{Jwks, JwtConfig, JwtKeyError, JwtKeys};
use postrust_core::AppConfig;

/// The JWT configuration an [`AppConfig`] describes.
///
/// Every host builds its [`State`] from one `AppConfig`; this is how each of
/// them derives the JWT half, so the server, Lambda and the Worker cannot read
/// the same settings differently.
///
/// Fails when `PGRST_JWT_SECRET` is a JWK or JWKS that does not parse, or is
/// not the base64 `PGRST_JWT_SECRET_IS_BASE64` says it is. That is a
/// configuration error, for a host to report before it serves anything.
///
/// Keys from `PGRST_JWT_JWKS_URL` are not here: fetching them takes an HTTP
/// client, which is the host's. A host that has one attaches them with
/// [`jwks_builder`] and [`JwtConfig::with_jwks`].
pub fn jwt_config(config: &AppConfig) -> Result<JwtConfig, JwtKeyError> {
    let keys = JwtKeys::from_config(config.jwt_secret.as_deref(), config.jwt_secret_is_base64)?;
    Ok(jwt_config_with_keys(config, keys))
}

/// As [`jwt_config`], with keys the host has already parsed.
///
/// For a host that keeps parsed keys longer than one configuration lives: a
/// Worker reads its configuration on every request, and decodes a JWKS only
/// when the secret changes.
pub fn jwt_config_with_keys(config: &AppConfig, keys: Option<JwtKeys>) -> JwtConfig {
    let mut jwt = JwtConfig::default();
    jwt.keys = keys;
    jwt.audience = config.jwt_aud.clone();
    jwt.issuer = config.jwt_iss.clone();
    jwt.role_claim_key = config.jwt_role_claim_key.clone();
    jwt.anon_role = config.db_anon_role.clone();
    jwt
}

/// The JWKS `PGRST_JWT_JWKS_URL` names, to be fetched through `fetch`, or
/// `None` when it is not set.
///
/// `keys` are the ones parsed from `PGRST_JWT_SECRET`, which stay accepted
/// beside the fetched ones. Every host builds its [`Jwks`] here, so that the
/// URL, the refresh interval and how the two sources combine are read the
/// same way everywhere; the host adds what only it has (a store, a task).
pub fn jwks_builder(
    config: &AppConfig,
    keys: Option<&JwtKeys>,
    fetch: impl Fetch + 'static,
) -> Option<JwksBuilder> {
    let url = config.jwt_jwks_url.as_deref()?;
    Some(
        Jwks::builder(url, fetch)
            .fixed(keys)
            .refresh(std::time::Duration::from_secs(config.jwt_jwks_refresh)),
    )
}
