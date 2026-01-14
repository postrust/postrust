//! SaaS Domain Management Module
//!
//! Provides multi-tenant domain ownership validation and reverse proxy routing.
//!
//! ## Features
//!
//! - Domain ownership verification (DNS TXT and HTTP challenge)
//! - Multi-tenant API key authentication
//! - Dynamic route management per verified domain
//! - Automatic SSL certificate provisioning via ACME
//! - Manual certificate upload support

pub mod types;
pub mod db;
pub mod api_keys;
pub mod auth;
pub mod verification;
pub mod manager;
pub mod handlers;

pub use types::*;
pub use manager::DomainManager;
pub use verification::DomainVerificationService;
pub use api_keys::ApiKeyService;
pub use auth::{AuthContext, AuthType, SaasAuthLayer};
