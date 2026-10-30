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
//! curl localhost:3000/hooks/todos -H "Authorization: Bearer $TOKEN"
//! ```
//!
//! `/hooks/todos` reads a table under a row-level security policy, as the
//! caller. It needs the table: `todos.sql`, beside this crate's manifest, is
//! the SQL to run first, and says which settings and token it expects.

use axum::{
    extract::State,
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use postrust_server::{AppState, Builder, Host, User};
use std::sync::Arc;

/// The routes this binary adds. They take the same state as the API's own:
/// the pool, the schema cache and the configuration.
fn hooks() -> Router<Arc<AppState>> {
    Router::new()
        .route("/echo", post(echo))
        .route("/stats", get(stats))
        .route("/todos", get(todos))
}

/// A webhook receiver's shape: take a JSON body, acknowledge it.
async fn echo(Json(body): Json<serde_json::Value>) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "received": body }))
}

/// Something read straight from the pool.
///
/// The pool connects as the authenticator role, and a query sent on it runs
/// as that role: no row-level security, every grant the authenticator has.
/// Fine for this, which reads nothing of anyone's; see `todos` for a query
/// that should run as its caller.
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

/// The caller's own todos, and how many of them are done.
///
/// `User` authenticates the request as the REST API does -- the same token,
/// the same anonymous role, the same 401 for a token that does not verify --
/// and `begin_as` begins a transaction that runs as them. The policy in
/// `todos.sql` reads the token's `sub` from it, so this sees exactly the rows
/// `GET /api/todos` would show the same caller. A failure is
/// `postrust_server::Error`, answered with PostgREST's status and body: the
/// anonymous role, which has no grant on the table, gets REST's 401.
async fn todos(
    user: User,
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, postrust_server::Error> {
    let mut tx = state.begin_as(&user).await?;
    let (total, done): (i64, i64) =
        sqlx::query_as("SELECT count(*), count(*) FILTER (WHERE done) FROM todos")
            .fetch_one(&mut *tx)
            .await?;
    tx.commit().await?;
    Ok(Json(serde_json::json!({
        "role": user.role(),
        "todos": total,
        "done": done,
    })))
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
