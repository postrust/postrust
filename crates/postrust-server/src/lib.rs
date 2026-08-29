//! Postrust Server library.
//!
//! This crate provides the HTTP server implementation for Postrust.
//!
//! ## Features
//!
//! - `admin-ui` - Enables the admin UI with OpenAPI documentation,
//!   Swagger UI, Scalar, and GraphQL Playground at `/admin`.

// Every fallible function in this crate returns `postrust_core::Error`, which
// is a wide enum by design: a PostgREST error carries its code, message,
// details and hint. Boxing it to satisfy the lint would put an allocation on
// the error path of every request to save a return slot on the success path.
// `postrust-core` takes the same position at its own crate root; this was
// being restated a dozen times a file instead.
#![allow(clippy::result_large_err)]

pub mod app;
pub mod lenient_uri;
pub mod state;

#[cfg(feature = "admin-ui")]
pub mod admin;

pub use app::handle_request;
pub use state::AppState;

#[cfg(feature = "admin-ui")]
pub use admin::admin_router;
