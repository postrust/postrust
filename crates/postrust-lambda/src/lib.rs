//! Postrust Lambda library.
//!
//! Re-exports of the crates the handler is built from. The handler itself
//! serves `postrust_server::build`; see `main.rs`.

#![warn(missing_docs)]

pub use postrust_auth;
pub use postrust_core;
pub use postrust_response;
pub use postrust_server;
pub use postrust_sql;
