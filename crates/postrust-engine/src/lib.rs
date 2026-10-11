//! The REST request pipeline of [Postrust](https://postrust.org/): an HTTP
//! request in, the response the PostgREST-dialect API gives it out.
//!
//! No web framework is involved. The server (`postrust-server`) wraps this in
//! axum, and the Lambda adapter reaches it through the server; a host that
//! cannot run axum -- a Cloudflare Worker -- calls it directly. See issue #50.
//!
//! ```text
//! let response = postrust_engine::handle(&state, request).await;
//! ```

#![warn(missing_docs)]
#![allow(clippy::result_large_err)]

mod handler;
pub mod state;

pub use handler::{allow, error_response, handle};
pub use state::AppState;
