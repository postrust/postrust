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

use lambda_http::{run, Error};
use tracing::info;

#[tokio::main]
async fn main() -> Result<(), Error> {
    // As the server reads it: `RUST_LOG` if set, otherwise `PGRST_LOG_LEVEL`.
    // No timestamps, which CloudWatch adds to every line itself.
    let filter = std::env::var("RUST_LOG").unwrap_or_else(|_| {
        let level = std::env::var("PGRST_LOG_LEVEL")
            .ok()
            .and_then(|v| postrust_core::LogLevel::parse_config(&v))
            .unwrap_or_else(|| postrust_core::AppConfig::default().log_level);
        format!("postrust={}", level.to_tracing())
    });
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new(filter))
        .with_target(false)
        .without_time()
        .init();

    let mut config = postrust_core::AppConfig::try_from_env()?;

    // A sandbox answers one invocation at a time, so the server's default pool
    // holds connections nothing will use -- and every concurrent sandbox holds
    // its own, which is how a burst of Lambdas exhausts `max_connections`.
    // Two rather than one: a GraphQL mutation holds a transaction while its
    // resolvers read.
    if ["PGRST_DB_POOL", "PGRST_DB_POOL_SIZE"]
        .iter()
        .all(|var| std::env::var_os(var).is_none())
    {
        config.db_pool_size = 2;
    }

    // Built here, in the init phase, rather than on the first request: the
    // connection and the schema cache are paid for once per sandbox, and a
    // database that cannot be reached fails the init -- which Lambda reports
    // and retries -- instead of failing every invocation.
    info!("Starting Postrust Lambda handler");
    let app = postrust_server::build(config, postrust_server::Host::LAMBDA).await?;

    run(app.router).await
}
