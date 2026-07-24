//! Upstream API handlers.

use crate::admin::api::ApiResponse;
use crate::saas::auth::Auth;
use crate::saas::handlers::{error_response, SaasState};
use crate::saas::types::{CreateBackendRequest, CreateUpstreamRequest, UpdateUpstreamRequest};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use uuid::Uuid;

/// List upstreams for the authenticated tenant.
pub async fn list_upstreams(State(state): State<SaasState>, Auth(auth): Auth) -> impl IntoResponse {
    match state.domain_manager.list_upstreams(auth.tenant_id).await {
        Ok(upstreams) => Json(ApiResponse::success(upstreams)).into_response(),
        Err(e) => error_response(e).into_response(),
    }
}

/// Create a new upstream.
pub async fn create_upstream(
    State(state): State<SaasState>,
    Auth(auth): Auth,
    Json(req): Json<CreateUpstreamRequest>,
) -> impl IntoResponse {
    if !auth.can_write("upstreams") {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiResponse::<()>::error("Insufficient permissions")),
        )
            .into_response();
    }

    match state
        .domain_manager
        .create_upstream(auth.tenant_id, req)
        .await
    {
        Ok(upstream) => (StatusCode::CREATED, Json(ApiResponse::success(upstream))).into_response(),
        Err(e) => error_response(e).into_response(),
    }
}

/// Get an upstream by ID.
pub async fn get_upstream(
    State(state): State<SaasState>,
    Auth(auth): Auth,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match state.domain_manager.get_upstream(id, auth.tenant_id).await {
        Ok(Some(upstream)) => Json(ApiResponse::success(upstream)).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::<()>::error("Upstream not found")),
        )
            .into_response(),
        Err(e) => error_response(e).into_response(),
    }
}

/// Update an upstream.
pub async fn update_upstream(
    State(state): State<SaasState>,
    Auth(auth): Auth,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateUpstreamRequest>,
) -> impl IntoResponse {
    if !auth.can_write("upstreams") {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiResponse::<()>::error("Insufficient permissions")),
        )
            .into_response();
    }

    match state
        .domain_manager
        .update_upstream(id, auth.tenant_id, req)
        .await
    {
        Ok(Some(upstream)) => Json(ApiResponse::success(upstream)).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::<()>::error("Upstream not found")),
        )
            .into_response(),
        Err(e) => error_response(e).into_response(),
    }
}

/// Delete an upstream.
pub async fn delete_upstream(
    State(state): State<SaasState>,
    Auth(auth): Auth,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    if !auth.can_write("upstreams") {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiResponse::<()>::error("Insufficient permissions")),
        )
            .into_response();
    }

    match state
        .domain_manager
        .delete_upstream(id, auth.tenant_id)
        .await
    {
        Ok(true) => Json(ApiResponse::success(())).into_response(),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::<()>::error("Upstream not found")),
        )
            .into_response(),
        Err(e) => error_response(e).into_response(),
    }
}

/// Add a backend to an upstream.
pub async fn add_backend(
    State(state): State<SaasState>,
    Auth(auth): Auth,
    Path(upstream_id): Path<Uuid>,
    Json(req): Json<CreateBackendRequest>,
) -> impl IntoResponse {
    if !auth.can_write("upstreams") {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiResponse::<()>::error("Insufficient permissions")),
        )
            .into_response();
    }

    match state
        .domain_manager
        .add_backend(upstream_id, auth.tenant_id, req)
        .await
    {
        Ok(backend) => (StatusCode::CREATED, Json(ApiResponse::success(backend))).into_response(),
        Err(e) => error_response(e).into_response(),
    }
}

/// Remove a backend from an upstream.
pub async fn remove_backend(
    State(state): State<SaasState>,
    Auth(auth): Auth,
    Path((upstream_id, backend_id)): Path<(Uuid, Uuid)>,
) -> impl IntoResponse {
    if !auth.can_write("upstreams") {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiResponse::<()>::error("Insufficient permissions")),
        )
            .into_response();
    }

    match state
        .domain_manager
        .remove_backend(backend_id, upstream_id, auth.tenant_id)
        .await
    {
        Ok(true) => Json(ApiResponse::success(())).into_response(),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::<()>::error("Backend not found")),
        )
            .into_response(),
        Err(e) => error_response(e).into_response(),
    }
}
