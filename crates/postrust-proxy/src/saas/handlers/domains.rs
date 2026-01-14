//! Domain API handlers.

use crate::admin::api::ApiResponse;
use crate::saas::auth::Auth;
use crate::saas::handlers::{error_response, SaasState};
use crate::saas::types::CreateDomainRequest;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use uuid::Uuid;

/// List all domains for the authenticated tenant.
pub async fn list_domains(
    State(state): State<SaasState>,
    Auth(auth): Auth,
) -> impl IntoResponse {
    match state.domain_manager.list_domains(auth.tenant_id).await {
        Ok(domains) => Json(ApiResponse::success(domains)).into_response(),
        Err(e) => error_response(e).into_response(),
    }
}

/// Create a new domain.
pub async fn create_domain(
    State(state): State<SaasState>,
    Auth(auth): Auth,
    Json(req): Json<CreateDomainRequest>,
) -> impl IntoResponse {
    if !auth.can_write("domains") {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiResponse::<()>::error("Insufficient permissions")),
        )
            .into_response();
    }

    match state.domain_manager.create_domain(auth.tenant_id, req).await {
        Ok(domain_response) => (StatusCode::CREATED, Json(ApiResponse::success(domain_response))).into_response(),
        Err(e) => error_response(e).into_response(),
    }
}

/// Get a domain by ID.
pub async fn get_domain(
    State(state): State<SaasState>,
    Auth(auth): Auth,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match state.domain_manager.get_domain(id, auth.tenant_id).await {
        Ok(Some(domain)) => Json(ApiResponse::success(domain)).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::<()>::error("Domain not found")),
        )
            .into_response(),
        Err(e) => error_response(e).into_response(),
    }
}

/// Delete a domain.
pub async fn delete_domain(
    State(state): State<SaasState>,
    Auth(auth): Auth,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    if !auth.can_write("domains") {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiResponse::<()>::error("Insufficient permissions")),
        )
            .into_response();
    }

    match state.domain_manager.delete_domain(id, auth.tenant_id).await {
        Ok(true) => Json(ApiResponse::success(())).into_response(),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::<()>::error("Domain not found")),
        )
            .into_response(),
        Err(e) => error_response(e).into_response(),
    }
}

/// Trigger domain verification.
pub async fn verify_domain(
    State(state): State<SaasState>,
    Auth(auth): Auth,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    if !auth.can_write("domains") {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiResponse::<()>::error("Insufficient permissions")),
        )
            .into_response();
    }

    match state.domain_manager.verify_domain(id, auth.tenant_id).await {
        Ok(result) => Json(ApiResponse::success(result)).into_response(),
        Err(e) => error_response(e).into_response(),
    }
}

/// Enable a verified domain.
pub async fn enable_domain(
    State(state): State<SaasState>,
    Auth(auth): Auth,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    if !auth.can_write("domains") {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiResponse::<()>::error("Insufficient permissions")),
        )
            .into_response();
    }

    match state.domain_manager.enable_domain(id, auth.tenant_id).await {
        Ok(true) => Json(ApiResponse::success(())).into_response(),
        Ok(false) => (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse::<()>::error("Could not enable domain. Is it verified?")),
        )
            .into_response(),
        Err(e) => error_response(e).into_response(),
    }
}

/// Disable a domain.
pub async fn disable_domain(
    State(state): State<SaasState>,
    Auth(auth): Auth,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    if !auth.can_write("domains") {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiResponse::<()>::error("Insufficient permissions")),
        )
            .into_response();
    }

    match state.domain_manager.disable_domain(id, auth.tenant_id).await {
        Ok(true) => Json(ApiResponse::success(())).into_response(),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::<()>::error("Domain not found")),
        )
            .into_response(),
        Err(e) => error_response(e).into_response(),
    }
}
