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
//! ## Not yet implemented
//!
//! **Automatic SSL provisioning via ACME.** A domain can be configured with
//! `ssl_provider = "acme"` and the schema tracks an `ssl_status`, but nothing
//! here has ever talked to a CA: no order is placed, and
//! `/.well-known/acme-challenge/{token}` has no authorization to serve.
//! Verifying such a domain leaves it `pending` rather than claiming to be
//! provisioning. Use `manual` and upload a certificate until this lands.
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
pub mod db;
pub mod handlers;
pub mod manager;
pub mod types;
pub mod verification;
pub mod wellknown_host;

pub use api_keys::ApiKeyService;
pub use auth::{AuthContext, AuthType, SaasAuthLayer};
pub use manager::DomainManager;
pub use types::*;
pub use verification::DomainVerificationService;
