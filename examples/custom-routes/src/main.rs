//! Postrust's API with routes of your own beside it.
//!
//! Serves everything the `postrust` binary serves -- REST at `/api`, GraphQL
//! if the feature is on, the health checks -- and adds `/hooks`, configured
//! from the same `PGRST_*` environment variables. Run it with:
//!
//! ```text
//! PGRST_DB_URI=postgres://... cargo run --manifest-path examples/custom-routes/Cargo.toml
//! curl localhost:3000/hooks/stats
//! curl -X POST localhost:3000/hooks/echo -d '{"hello":"world"}'
//! ```

use axum::{
    extract::State,
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use postrust_server::{AppState, Builder, Host};
use std::sync::Arc;

/// The routes this binary adds. They take the same state as the API's own:
/// the pool, the schema cache and the configuration.
fn hooks() -> Router<Arc<AppState>> {
    Router::new()
        .route("/echo", post(echo))
        .route("/stats", get(stats))
}

/// A webhook receiver's shape: take a JSON body, acknowledge it.
async fn echo(Json(body): Json<serde_json::Value>) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "received": body }))
}

/// Something read straight from the database.
///
/// The pool connects as the authenticator role and this query does not
/// switch role, so row-level security does not apply to it. A helper for
/// running as the caller's role is planned; until then, keep these routes to
/// what the authenticator may see.
async fn stats(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let tables = state.schema_cache().await.tables.len();
    let now: String = sqlx::query_scalar("SELECT now()::text")
        .fetch_one(&state.pool)
        .await
        .map_err(|e| (StatusCode::SERVICE_UNAVAILABLE, e.to_string()))?;
    Ok(Json(serde_json::json!({ "tables": tables, "now": now })))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string()),
        ))
        .init();

    let config = postrust_core::AppConfig::try_from_env()?;
    let addr = format!("{}:{}", config.server_host, config.server_port);

    let app = Builder::new(config, Host::SERVER)
        .nest("/hooks", hooks())
        .build()
        .await?;

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("Listening on http://{}", addr);
    axum::serve(listener, app.router).await?;
    Ok(())
}
