//! Custom routes extension for Postrust.
//!
//! Add your custom API endpoints here that don't go through the
//! standard PostgREST-style query processing.

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::{get, post},
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
        // Add your custom routes here:
        // .route("/webhooks/stripe", post(handle_stripe_webhook))
        // .route("/email/send", post(send_email))
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
    // Check database connectivity
    match sqlx::query("SELECT 1").execute(&state.pool).await {
        Ok(_) => (
            StatusCode::OK,
            Json(ReadinessResponse {
                ready: true,
                database: true,
                message: None,
            }),
        ),
        Err(e) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ReadinessResponse {
                ready: false,
                database: false,
                message: Some(e.to_string()),
            }),
        ),
    }
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

    (StatusCode::OK, Json(serde_json::json!({ "received": true })))
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
