//! Postrust AWS Lambda handler.
//!
//! Serves the application the `postrust` binary serves -- the same router,
//! built by `postrust_server::build` -- to the Lambda runtime instead of a
//! socket. REST, GraphQL, JWT authentication, role switching and the claims a
//! row-level security policy reads all take the server's path, because there
//! is no other.
//!
//! This adapter used to carry a request path of its own. It dropped every
//! bound parameter, so any filter or write failed, and it never switched role,
//! so a function connected as a superuser served every row to everyone.
//!
//! Everything here is in the library, so a Lambda binary with routes of its
//! own is this file with a `Builder` in place of `build`; see `lib.rs`.

use postrust_lambda::Error;
use tracing::info;

#[tokio::main]
async fn main() -> Result<(), Error> {
    postrust_lambda::init_tracing();
    let config = postrust_lambda::config_from_env()?;

    // Built here, in the init phase, rather than on the first request; the
    // library's documentation says why.
    info!("Starting Postrust Lambda handler");
    let app = postrust_server::build(config, postrust_server::Host::LAMBDA).await?;

    postrust_lambda::run(app).await
}
