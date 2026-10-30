//! Authentication and API key management handlers.

use crate::admin::api::ApiResponse;
use crate::saas::auth::Auth;
use crate::saas::handlers::{error_response, require_scope, SaasState};
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
///
/// Requires `api_keys:write`, and the new key's scopes must all be scopes the
/// caller holds: a key cannot mint a key more powerful than itself, and only a
/// `*` credential can mint a `*` key. Without both rules any key that could
/// reach this endpoint was, in effect, a `*` key.
pub async fn create_api_key(
    State(state): State<SaasState>,
    Auth(auth): Auth,
    Json(req): Json<CreateApiKeyRequest>,
) -> impl IntoResponse {
    if let Err(refusal) = require_scope(&auth, "api_keys:write") {
        return refusal;
    }
    // A misspelt scope is a 400 for everyone, not a 403 for most callers.
    if let Err(e) = crate::saas::auth::validate_scopes(&req.scopes) {
        return error_response(e).into_response();
    }
    if let Some(scope) = req.scopes.iter().find(|scope| !auth.grants(scope)) {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiResponse::<()>::error_with_code(
                "insufficient_scope",
                format!("A key cannot be given the {scope} scope by a credential that lacks it"),
            )),
        )
            .into_response();
    }

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
    if let Err(refusal) = require_scope(&auth, "api_keys:read") {
        return refusal;
    }

    match state.api_key_service.list_api_keys(auth.tenant_id).await {
        Ok(keys) => Json(ApiResponse::success(keys)).into_response(),
        Err(e) => error_response(e).into_response(),
    }
}

/// Revoke (delete) an API key.
///
/// Requires `api_keys:write`, and the key being revoked must hold nothing the
/// caller does not: otherwise a narrow key that can manage keys could revoke
/// the tenant's `*` key and lock its owner out.
pub async fn revoke_api_key(
    State(state): State<SaasState>,
    Auth(auth): Auth,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    if let Err(refusal) = require_scope(&auth, "api_keys:write") {
        return refusal;
    }

    match state.api_key_service.get_api_key(id, auth.tenant_id).await {
        Ok(Some(target)) => {
            if let Some(scope) = target.scopes.iter().find(|scope| !auth.grants(scope)) {
                return (
                    StatusCode::FORBIDDEN,
                    Json(ApiResponse::<()>::error_with_code(
                        "insufficient_scope",
                        format!(
                            "This key holds the {scope} scope, which the caller lacks, \
                             and cannot be revoked by it"
                        ),
                    )),
                )
                    .into_response();
            }
        }
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(ApiResponse::<()>::error_with_code(
                    "not_found",
                    "API key not found",
                )),
            )
                .into_response()
        }
        Err(e) => return error_response(e).into_response(),
    }

    match state
        .api_key_service
        .revoke_api_key(id, auth.tenant_id)
        .await
    {
        Ok(true) => Json(ApiResponse::success(())).into_response(),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::<()>::error_with_code(
                "not_found",
                "API key not found",
            )),
        )
            .into_response(),
        Err(e) => error_response(e).into_response(),
    }
}

/// Current tenant info response.
#[derive(Serialize)]
pub struct CurrentTenantResponse {
    /// The tenant the caller is acting as.
    pub tenant_id: Uuid,
    /// How they authenticated -- `jwt` or `api_key`.
    pub auth_type: String,
    /// What this credential may do.
    pub scopes: Vec<String>,
}

/// Get current tenant info. Requires `tenant:read`.
pub async fn get_current_tenant(Auth(auth): Auth) -> impl IntoResponse {
    if let Err(refusal) = require_scope(&auth, "tenant:read") {
        return refusal;
    }

    let auth_type = match &auth.auth_type {
        crate::saas::auth::AuthType::Jwt { .. } => "jwt",
        crate::saas::auth::AuthType::ApiKey { .. } => "api_key",
    };

    Json(ApiResponse::success(CurrentTenantResponse {
        tenant_id: auth.tenant_id,
        auth_type: auth_type.to_string(),
        scopes: auth.scopes,
    }))
    .into_response()
}

/// Get tenant usage statistics. Requires `tenant:read`.
pub async fn get_tenant_usage(
    State(state): State<SaasState>,
    Auth(auth): Auth,
) -> impl IntoResponse {
    if let Err(refusal) = require_scope(&auth, "tenant:read") {
        return refusal;
    }

    match state.domain_manager.get_tenant_usage(auth.tenant_id).await {
        Ok(usage) => Json(ApiResponse::success(usage)).into_response(),
        Err(e) => error_response(e).into_response(),
    }
}
