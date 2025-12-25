//! Postrust Server library.
//!
//! This crate provides the HTTP server implementation for Postrust.
//!
//! ## Features
//!
//! - `admin-ui` - Enables the admin UI with OpenAPI documentation,
//!   Swagger UI, Scalar, and GraphQL Playground at `/admin`.

pub mod app;
pub mod state;

#[cfg(feature = "admin-ui")]
pub mod admin;

pub use app::handle_request;
pub use state::AppState;

#[cfg(feature = "admin-ui")]
pub use admin::admin_router;
