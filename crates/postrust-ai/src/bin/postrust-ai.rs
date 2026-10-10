//! The postrust-ai node.
//!
//! ```text
//! postrust-ai [config.toml]
//! ```
//!
//! The config path defaults to `$POSTRUST_AI_CONFIG`, then `postrust-ai.toml`.
//! `DATABASE_URL`, `POSTRUST_AI_LISTEN`, `POSTRUST_AI_NODE_ID`,
//! `POSTRUST_AI_ADVERTISE_URL`, `POSTRUST_AI_JWT_SECRET` and
//! `POSTRUST_AI_CACHE_DIR` override the file.

use std::path::PathBuf;

use postrust_ai::app::Node;
use postrust_ai::AiConfig;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    if let Err(error) = run().await {
        tracing::error!(%error, "postrust-ai failed");
        std::process::exit(1);
    }
}

async fn run() -> postrust_ai::AiResult<()> {
    let path = std::env::args()
        .nth(1)
        .or_else(|| std::env::var("POSTRUST_AI_CONFIG").ok())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("postrust-ai.toml"));
    let config = if path.exists() {
        AiConfig::load(&path)?
    } else {
        tracing::warn!(path = %path.display(), "no config file; using defaults and environment");
        AiConfig::from_toml("")?
    };

    let node = Node::build(config, Vec::new()).await?;
    let shutdown = node.shutdown_token();
    tokio::spawn(async move {
        wait_for_signal().await;
        shutdown.cancel();
    });
    node.serve().await
}

async fn wait_for_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut term = signal(SignalKind::terminate()).expect("SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
