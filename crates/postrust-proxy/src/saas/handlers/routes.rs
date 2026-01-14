//! Domain route API handlers.

use crate::admin::api::ApiResponse;
use crate::saas::auth::Auth;
use crate::saas::handlers::{error_response, SaasState};
use crate::saas::types::{CreateDomainRouteRequest, UpdateDomainRouteRequest};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use uuid::Uuid;

/// List routes for a domain.
pub async fn list_routes(
    State(state): State<SaasState>,
    Auth(auth): Auth,
    Path(domain_id): Path<Uuid>,
) -> impl IntoResponse {
    match state
        .domain_manager
        .list_routes_for_domain(domain_id, auth.tenant_id)
        .await
    {
        Ok(routes) => Json(ApiResponse::success(routes)).into_response(),
        Err(e) => error_response(e).into_response(),
    }
}

/// Create a new route for a domain.
pub async fn create_route(
    State(state): State<SaasState>,
    Auth(auth): Auth,
    Path(domain_id): Path<Uuid>,
    Json(req): Json<CreateDomainRouteRequest>,
) -> impl IntoResponse {
    if !auth.can_write("routes") {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiResponse::<()>::error("Insufficient permissions")),
        )
            .into_response();
    }

    match state
        .domain_manager
        .create_route(domain_id, auth.tenant_id, req)
        .await
    {
        Ok(route) => (StatusCode::CREATED, Json(ApiResponse::success(route))).into_response(),
        Err(e) => error_response(e).into_response(),
    }
}

/// Get a route by ID.
pub async fn get_route(
    State(state): State<SaasState>,
    Auth(auth): Auth,
    Path((domain_id, id)): Path<(Uuid, Uuid)>,
) -> impl IntoResponse {
    match state.domain_manager.get_route(id, auth.tenant_id).await {
        Ok(Some(route)) => {
            // Verify route belongs to the specified domain
            if route.domain_id != domain_id {
                return (
                    StatusCode::NOT_FOUND,
                    Json(ApiResponse::<()>::error("Route not found")),
                )
                    .into_response();
            }
            Json(ApiResponse::success(route)).into_response()
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::<()>::error("Route not found")),
        )
            .into_response(),
        Err(e) => error_response(e).into_response(),
    }
}

/// Update a route.
pub async fn update_route(
    State(state): State<SaasState>,
    Auth(auth): Auth,
    Path((domain_id, id)): Path<(Uuid, Uuid)>,
    Json(req): Json<UpdateDomainRouteRequest>,
) -> impl IntoResponse {
    if !auth.can_write("routes") {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiResponse::<()>::error("Insufficient permissions")),
        )
            .into_response();
    }

    // First check if route exists and belongs to the domain
    match state.domain_manager.get_route(id, auth.tenant_id).await {
        Ok(Some(route)) => {
            if route.domain_id != domain_id {
                return (
                    StatusCode::NOT_FOUND,
                    Json(ApiResponse::<()>::error("Route not found")),
                )
                    .into_response();
            }
        }
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(ApiResponse::<()>::error("Route not found")),
            )
                .into_response();
        }
        Err(e) => return error_response(e).into_response(),
    }

    match state
        .domain_manager
        .update_route(id, auth.tenant_id, req)
        .await
    {
        Ok(Some(route)) => Json(ApiResponse::success(route)).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::<()>::error("Route not found")),
        )
            .into_response(),
        Err(e) => error_response(e).into_response(),
    }
}

/// Delete a route.
pub async fn delete_route(
    State(state): State<SaasState>,
    Auth(auth): Auth,
    Path((domain_id, id)): Path<(Uuid, Uuid)>,
) -> impl IntoResponse {
    if !auth.can_write("routes") {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiResponse::<()>::error("Insufficient permissions")),
        )
            .into_response();
    }

    // First check if route exists and belongs to the domain
    match state.domain_manager.get_route(id, auth.tenant_id).await {
        Ok(Some(route)) => {
            if route.domain_id != domain_id {
                return (
                    StatusCode::NOT_FOUND,
                    Json(ApiResponse::<()>::error("Route not found")),
                )
                    .into_response();
            }
        }
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(ApiResponse::<()>::error("Route not found")),
            )
                .into_response();
        }
        Err(e) => return error_response(e).into_response(),
    }

    match state.domain_manager.delete_route(id, auth.tenant_id).await {
        Ok(true) => Json(ApiResponse::success(())).into_response(),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::<()>::error("Route not found")),
        )
            .into_response(),
        Err(e) => error_response(e).into_response(),
    }
}
