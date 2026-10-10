//! Postrust Server library.
//!
//! This crate provides the HTTP server implementation for Postrust.
//!
//! ## Features
//!
//! - `graphql` - Serves the GraphQL API at `/v1/graphql`, with Hasura's
//!   contract, subscriptions and Apollo Federation.
//! - `admin-ui` - Implies `graphql`, and enables the admin UI with OpenAPI documentation,
//!   Swagger UI, Scalar, and GraphQL Playground at `/admin`.

#![warn(missing_docs)]
// Every fallible function in this crate returns `postrust_core::Error`, which
// is a wide enum by design: a PostgREST error carries its code, message,
// details and hint. Boxing it to satisfy the lint would put an allocation on
// the error path of every request to save a return slot on the success path.
// `postrust-core` takes the same position at its own crate root; this was
// being restated a dozen times a file instead.
#![allow(clippy::result_large_err)]

pub mod app;
pub mod build;
pub mod custom;
pub mod lenient_uri;
pub mod state;

#[cfg(feature = "admin-ui")]
pub mod admin;

pub use app::handle_request;
pub use build::{build, App, Host};
pub use state::AppState;

#[cfg(feature = "admin-ui")]
pub use admin::admin_router;
