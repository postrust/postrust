//! Postrust Server library.
//!
//! This crate provides the HTTP server implementation for Postrust.

pub mod app;
pub mod state;

pub use app::handle_request;
pub use state::AppState;
