//! Postrust Lambda library.
//!
//! What the `postrust-lambda` binary is made of, for a Lambda binary of your
//! own: one that serves routes of yours beside the API, added with
//! [`postrust_server::Builder`].
//!
//! ```no_run
//! use axum::{routing::post, Router};
//! use postrust_lambda::postrust_server::{AppState, Builder, Host};
//! use std::sync::Arc;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), postrust_lambda::Error> {
//!     postrust_lambda::init_tracing();
//!     let hooks: Router<Arc<AppState>> = Router::new().route("/stripe", post(|| async { "ok" }));
//!     let app = Builder::new(postrust_lambda::config_from_env()?, Host::LAMBDA)
//!         .nest("/hooks", hooks)
//!         .build()
//!         .await?;
//!     postrust_lambda::run(app).await
//! }
//! ```
//!
//! Build the application before calling [`run`], in the init phase rather
//! than on the first request: the connection and the schema cache are paid
//! for once per sandbox, and a database that cannot be reached fails the init
//! -- which Lambda reports and retries -- instead of failing every invocation.

#![warn(missing_docs)]

pub use postrust_auth;
pub use postrust_core;
pub use postrust_response;
pub use postrust_server;
pub use postrust_sql;

/// The error a Lambda handler returns, as the runtime takes it.
pub use lambda_http::Error;

/// Log as the server does: `RUST_LOG` if set, otherwise `PGRST_LOG_LEVEL`.
///
/// No timestamps, which CloudWatch adds to every line itself. Installs the
/// global subscriber, so call it once, first.
pub fn init_tracing() {
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
}

/// The configuration from the environment, sized for a Lambda sandbox.
///
/// A sandbox answers one invocation at a time, so the server's default pool
/// holds connections nothing will use -- and every concurrent sandbox holds
/// its own, which is how a burst of Lambdas exhausts `max_connections`. The
/// pool is two connections unless `PGRST_DB_POOL` or `PGRST_DB_POOL_SIZE`
/// says otherwise; two rather than one because a GraphQL mutation holds a
/// transaction while its resolvers read.
pub fn config_from_env() -> Result<postrust_core::AppConfig, Error> {
    let mut config = postrust_core::AppConfig::try_from_env()?;
    if ["PGRST_DB_POOL", "PGRST_DB_POOL_SIZE"]
        .iter()
        .all(|var| std::env::var_os(var).is_none())
    {
        config.db_pool_size = 2;
    }
    Ok(config)
}

/// Serve `app` to the Lambda runtime until the sandbox is shut down.
pub async fn run(app: postrust_server::App) -> Result<(), Error> {
    lambda_http::run(app.router).await
}
