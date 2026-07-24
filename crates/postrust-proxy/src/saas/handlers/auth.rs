//! Authentication and API key management handlers.

use crate::admin::api::ApiResponse;
use crate::saas::auth::Auth;
use crate::saas::handlers::{error_response, SaasState};
use crate::saas::types::CreateApiKeyRequest;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Serialize;
use uuid::Uuid;

/// Create a new API key.
pub async fn create_api_key(
    State(state): State<SaasState>,
    Auth(auth): Auth,
    Json(req): Json<CreateApiKeyRequest>,
) -> impl IntoResponse {
    match state
        .api_key_service
        .create_api_key(auth.tenant_id, req)
        .await
    {
        Ok(api_key) => (StatusCode::CREATED, Json(ApiResponse::success(api_key))).into_response(),
        Err(e) => error_response(e).into_response(),
    }
}

/// List API keys for the authenticated tenant.
pub async fn list_api_keys(State(state): State<SaasState>, Auth(auth): Auth) -> impl IntoResponse {
    match state.api_key_service.list_api_keys(auth.tenant_id).await {
        Ok(keys) => Json(ApiResponse::success(keys)).into_response(),
        Err(e) => error_response(e).into_response(),
    }
}

/// Revoke (delete) an API key.
pub async fn revoke_api_key(
    State(state): State<SaasState>,
    Auth(auth): Auth,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match state
        .api_key_service
        .revoke_api_key(id, auth.tenant_id)
        .await
    {
        Ok(true) => Json(ApiResponse::success(())).into_response(),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::<()>::error("API key not found")),
        )
            .into_response(),
        Err(e) => error_response(e).into_response(),
    }
}

/// Current tenant info response.
#[derive(Serialize)]
pub struct CurrentTenantResponse {
    pub tenant_id: Uuid,
    pub auth_type: String,
    pub scopes: Vec<String>,
}

/// Get current tenant info.
pub async fn get_current_tenant(Auth(auth): Auth) -> impl IntoResponse {
    let auth_type = match &auth.auth_type {
        crate::saas::auth::AuthType::Jwt { .. } => "jwt",
        crate::saas::auth::AuthType::ApiKey { .. } => "api_key",
    };

    Json(ApiResponse::success(CurrentTenantResponse {
        tenant_id: auth.tenant_id,
        auth_type: auth_type.to_string(),
        scopes: auth.scopes,
    }))
}

/// Get tenant usage statistics.
pub async fn get_tenant_usage(
    State(state): State<SaasState>,
    Auth(auth): Auth,
) -> impl IntoResponse {
    match state.domain_manager.get_tenant_usage(auth.tenant_id).await {
        Ok(usage) => Json(ApiResponse::success(usage)).into_response(),
        Err(e) => error_response(e).into_response(),
    }
}
