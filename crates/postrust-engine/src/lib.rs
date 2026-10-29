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
mod state;

pub use handler::{allow, error_response, handle};
pub use state::State;

use postrust_auth::JwtConfig;
use postrust_core::AppConfig;

/// The JWT configuration an [`AppConfig`] describes.
///
/// Every host builds its [`State`] from one `AppConfig`; this is how each of
/// them derives the JWT half, so the server, Lambda and the Worker cannot read
/// the same settings differently.
pub fn jwt_config(config: &AppConfig) -> JwtConfig {
    let mut jwt = JwtConfig::default();
    jwt.secret = config.jwt_secret.clone();
    jwt.secret_is_base64 = config.jwt_secret_is_base64;
    jwt.audience = config.jwt_aud.clone();
    jwt.role_claim_key = config.jwt_role_claim_key.clone();
    jwt.anon_role = config.db_anon_role.clone();
    jwt
}
