//! The built-in routes under `/_`: health and readiness.
//!
//! Routes of your own do not belong here. A binary of yours nests them with
//! [`crate::Builder::nest`], at a prefix of its own or beneath `/_`, without
//! forking this crate; see docs/custom-routes.md. The handlers at the bottom of
//! this file are sketches of what such routes look like.

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::get,
    Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::state::AppState;

/// Build the custom routes router.
pub fn custom_router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/health", get(health_check))
        .route("/ready", get(readiness_check))
}

/// Build the router served on `admin_server_port`.
///
/// Liveness and readiness only. The point of a separate port is that it can be
/// reachable from an orchestrator without the API being reachable from the same
/// place, so nothing that reads or writes data belongs here.
///
/// `/live` and `/ready` are the names PostgREST's admin server uses; `/health`
/// is the same answer as `/live` under the name the main API already serves.
///
/// With the `otel` feature, `/metrics` too: the server's metrics as
/// Prometheus text. Here and never on the API's port, for the same reason as
/// the rest -- what the pool and the token cache are doing is the operator's
/// business, not every client's.
pub fn admin_router() -> Router<Arc<AppState>> {
    let router = Router::new()
        .route("/live", get(health_check))
        .route("/health", get(health_check))
        .route("/ready", get(readiness_check));
    #[cfg(feature = "otel")]
    let router = router.route("/metrics", get(crate::otel::metrics_handler));
    router
}

// =============================================================================
// Health & Readiness
// =============================================================================

async fn health_check() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

async fn readiness_check(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    // Keys from `PGRST_JWT_JWKS_URL` that have never loaded mean every token
    // they would verify is refused: a replica in that state should not be
    // sent traffic, though it is alive and will recover by itself.
    let jwks = state.jwt_config.jwks.as_ref().map(|jwks| jwks.status());
    let jwks_loaded = jwks.as_ref().map(|status| status.loaded);
    let jwks_problem = jwks.filter(|status| !status.loaded).map(|status| {
        format!(
            "the keys at PGRST_JWT_JWKS_URL have not loaded: {}",
            status.last_error.as_deref().unwrap_or("not fetched yet")
        )
    });

    // Check database connectivity
    let (database, message) = match sqlx::query("SELECT 1").execute(&state.pool).await {
        Ok(_) => (true, jwks_problem),
        Err(e) => (false, Some(e.to_string())),
    };
    let ready = database && jwks_loaded != Some(false);
    (
        if ready {
            StatusCode::OK
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        },
        Json(ReadinessResponse {
            ready,
            database,
            jwks: jwks_loaded,
            message,
        }),
    )
}

// =============================================================================
// Response Types
// =============================================================================

#[derive(Serialize)]
struct HealthResponse {
    status: String,
    version: String,
}

#[derive(Serialize)]
struct ReadinessResponse {
    ready: bool,
    database: bool,
    /// Whether the keys at `PGRST_JWT_JWKS_URL` have loaded; absent when it
    /// is not set.
    #[serde(skip_serializing_if = "Option::is_none")]
    jwks: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

// =============================================================================
// Example: Webhook Handler
// =============================================================================

#[derive(Deserialize)]
#[allow(dead_code)]
struct WebhookPayload {
    event_type: String,
    data: serde_json::Value,
}

#[allow(dead_code)]
async fn handle_webhook(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<WebhookPayload>,
) -> impl IntoResponse {
    tracing::info!("Received webhook: {}", payload.event_type);

    // Process webhook with database access
    let _pool = &state.pool;

    // Your custom logic here...

    (
        StatusCode::OK,
        Json(serde_json::json!({ "received": true })),
    )
}

// =============================================================================
// Example: Custom RPC that bypasses PostgREST
// =============================================================================

#[derive(Deserialize)]
#[allow(dead_code)]
struct CustomRpcRequest {
    action: String,
    params: serde_json::Value,
}

#[allow(dead_code)]
async fn custom_rpc(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CustomRpcRequest>,
) -> impl IntoResponse {
    // Direct database access for custom operations
    let result = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM users")
        .fetch_one(&state.pool)
        .await;

    match result {
        Ok(count) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "action": req.action,
                "result": count
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": e.to_string()
            })),
        ),
    }
}
