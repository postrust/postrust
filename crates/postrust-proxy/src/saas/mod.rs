//! SaaS Domain Management Module
//!
//! Provides multi-tenant domain ownership validation and reverse proxy routing.
//!
//! ## Features
//!
//! - Domain ownership verification (DNS TXT and HTTP challenge)
//! - Multi-tenant API key authentication
//! - Dynamic route management per verified domain
//! - Manual certificate upload support
//!
//! ## Automatic SSL via ACME
//!
//! A domain with `ssl_provider = "acme"` that passes verification is left
//! `ssl_status = 'pending'`, and [`ssl::run`] -- a background worker -- places
//! the order, answers the HTTP-01 challenge out of
//! `/.well-known/acme-challenge/{token}`, and stores the certificate. Failures
//! back off and record `ssl_error`; `POST /domains/{id}/ssl/retry` requeues one.
//!
//! Issuance is a worker rather than an endpoint because an ACME order takes
//! several round trips and a challenge fetch, and retrying under a rate limit
//! is the normal failure mode. Requires the `acme` feature, which is on by
//! default.
//!
//! ## Prefer DNS verification
//!
//! HTTP verification asks the claimant to serve content at a path on the
//! domain -- but once that domain points at this proxy, this proxy is what
//! serves that path. Passing shows the domain resolves here, not that the
//! claimant controls it. DNS verification does show control, and is the
//! default.

pub mod api_keys;
pub mod auth;
pub(crate) mod db;
pub mod handlers;
pub mod manager;
#[cfg(feature = "acme")]
pub mod ssl;
pub mod types;
pub mod verification;
pub mod wellknown_host;

pub use api_keys::ApiKeyService;
pub use auth::{AuthContext, AuthType, SaasAuthLayer};
pub use manager::DomainManager;
pub use types::*;
pub use verification::DomainVerificationService;
