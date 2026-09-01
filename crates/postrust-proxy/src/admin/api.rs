//! REST API endpoints for proxy management.

use crate::config::{Backend, HealthCheckConfig, LoadBalanceStrategy, Route, RouteMatch, Upstream};
use crate::ProxyState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post, put},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

/// API response wrapper.
#[derive(Serialize)]
pub struct ApiResponse<T> {
    /// Whether the request did what was asked. Read this, not the presence of
    /// `data`.
    pub success: bool,
    /// The payload, on success.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
    /// What went wrong, on failure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl<T: Serialize> ApiResponse<T> {
    /// A successful response carrying `data`.
    pub fn success(data: T) -> Self {
        Self {
            success: true,
            data: Some(data),
            error: None,
        }
    }

    /// A failure response carrying `message` and no data.
    ///
    /// Returns `ApiResponse<()>` regardless of `T`: there is no payload to
    /// type, and requiring the caller to name one at every error site would be
    /// noise.
    pub fn error(message: impl Into<String>) -> ApiResponse<()> {
        ApiResponse {
            success: false,
            data: None,
            error: Some(message.into()),
        }
    }
}

/// A 500 for a persistence failure.
///
/// The distinction from a 404 matters: "the database refused this" and "there
/// is no such route" are different answers, and returning success for either
/// is what this module used to do.
fn persistence_failed(error: crate::error::ProxyError) -> axum::response::Response {
    tracing::error!(%error, "admin API could not persist a configuration change");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ApiResponse::<()>::error(format!(
            "could not persist the change: {error}"
        ))),
    )
        .into_response()
}

/// Whether edits go to the database as well as to the running config.
///
/// `server.database_config` defaults to true. Setting it false is an explicit
/// "this proxy is configured from a file", and edits then last as long as the
/// process -- which is what was asked for, rather than a silent loss.
async fn persists(state: &ProxyState) -> bool {
    state.config.read().await.server.database_config
}

/// Create the admin API router.
/// Build the admin API router.
///
/// **No authentication of its own.** Mount it behind something that
/// authenticates, or on an address only operators can reach.
pub fn admin_router() -> Router<Arc<ProxyState>> {
    Router::new()
        // Routes
        .route("/routes", get(list_routes))
        .route("/routes", post(create_route))
        .route("/routes/{id}", get(get_route))
        .route("/routes/{id}", put(update_route))
        .route("/routes/{id}", delete(delete_route))
        // Upstreams
        .route("/upstreams", get(list_upstreams))
        .route("/upstreams", post(create_upstream))
        .route("/upstreams/{id}", get(get_upstream))
        .route("/upstreams/{id}", put(update_upstream))
        .route("/upstreams/{id}", delete(delete_upstream))
        // Backends
        .route("/upstreams/{id}/backends", get(list_backends))
        .route("/upstreams/{id}/backends", post(add_backend))
        .route(
            "/upstreams/{id}/backends/{backend_id}",
            delete(remove_backend),
        )
        // Health
        .route("/health", get(health_status))
        .route("/health/{backend_id}", get(backend_health))
        // Config
        // Stats
        .route("/stats", get(get_stats))
}

// Route handlers

async fn list_routes(State(state): State<Arc<ProxyState>>) -> impl IntoResponse {
    let config = state.config.read().await;
    let routes = config.routes.clone();
    Json(ApiResponse::success(routes))
}

async fn get_route(
    State(state): State<Arc<ProxyState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let config = state.config.read().await;
    match config.routes.iter().find(|r| r.id == Some(id)) {
        Some(route) => Json(ApiResponse::success(route)).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::<()>::error("Route not found")),
        )
            .into_response(),
    }
}

/// Body of `POST /routes` and `PUT /routes/{id}`.
#[derive(Deserialize)]
pub struct CreateRouteRequest {
    /// Route name, unique in the config.
    pub name: String,
    /// Host to match. Omit for any.
    pub host: Option<String>,
    /// Path to match, as a prefix.
    pub path: Option<String>,
    /// Name of the upstream to forward to.
    pub upstream: String,
    /// Whether to remove the matched prefix. Defaults to false.
    pub strip_path: Option<bool>,
    /// Higher is matched first. Defaults to 100.
    pub priority: Option<i32>,
    /// Headers to add to the forwarded request.
    pub add_headers: Option<HashMap<String, String>>,
    /// Headers to remove from the forwarded request.
    pub remove_headers: Option<Vec<String>>,
}

async fn create_route(
    State(state): State<Arc<ProxyState>>,
    Json(req): Json<CreateRouteRequest>,
) -> impl IntoResponse {
    let route = Route {
        id: Some(Uuid::new_v4()),
        name: req.name,
        description: None,
        match_: RouteMatch {
            host: req.host,
            path: req.path,
            path_type: Default::default(),
            headers: HashMap::new(),
            methods: None,
        },
        priority: req.priority.unwrap_or(100),
        upstream: req.upstream,
        strip_path: req.strip_path.unwrap_or(false),
        add_headers: req.add_headers.unwrap_or_default(),
        remove_headers: req.remove_headers.unwrap_or_default(),
        rate_limit: None,
        timeout_secs: 30,
        retry_count: 0,
        enabled: true,
    };

    if persists(&state).await {
        // Write through before touching the running config: a create that
        // answers 201 and is gone after a restart is worse than one that
        // fails now and says why.
        if let Err(error) = crate::config::save_route(&state.pool, &route).await {
            return persistence_failed(error);
        }
    }

    let mut config = state.config.write().await;
    config.routes.push(route.clone());

    (StatusCode::CREATED, Json(ApiResponse::success(route))).into_response()
}

async fn update_route(
    State(state): State<Arc<ProxyState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<CreateRouteRequest>,
) -> impl IntoResponse {
    // Build the updated route from a snapshot rather than mutating in place,
    // so the running config is only touched once the database has accepted it.
    let Some(mut updated) = state
        .config
        .read()
        .await
        .routes
        .iter()
        .find(|r| r.id == Some(id))
        .cloned()
    else {
        return (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::<()>::error("Route not found")),
        )
            .into_response();
    };

    updated.name = req.name;
    updated.match_.host = req.host;
    updated.match_.path = req.path;
    updated.upstream = req.upstream;
    updated.strip_path = req.strip_path.unwrap_or(false);
    updated.priority = req.priority.unwrap_or(100);
    if let Some(add_headers) = req.add_headers {
        updated.add_headers = add_headers;
    }
    if let Some(remove_headers) = req.remove_headers {
        updated.remove_headers = remove_headers;
    }

    if persists(&state).await {
        if let Err(error) = crate::config::save_route(&state.pool, &updated).await {
            return persistence_failed(error);
        }
    }

    let mut config = state.config.write().await;
    match config.routes.iter_mut().find(|r| r.id == Some(id)) {
        Some(route) => {
            *route = updated.clone();
            Json(ApiResponse::success(updated)).into_response()
        }
        // Deleted while we were writing. The database now holds it and the
        // running config does not; say so rather than reporting success.
        None => (
            StatusCode::CONFLICT,
            Json(ApiResponse::<()>::error(
                "Route was removed while the update was being saved",
            )),
        )
            .into_response(),
    }
}

async fn delete_route(
    State(state): State<Arc<ProxyState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    if !state
        .config
        .read()
        .await
        .routes
        .iter()
        .any(|r| r.id == Some(id))
    {
        return (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::<()>::error("Route not found")),
        )
            .into_response();
    }

    if persists(&state).await {
        // A `false` here is not an error: a route that came from a TOML config
        // has no row to delete. The 404 above already established that the
        // route existed in the running config, which is what was asked about.
        if let Err(error) = crate::config::delete_route(&state.pool, id).await {
            return persistence_failed(error);
        }
    }

    state
        .config
        .write()
        .await
        .routes
        .retain(|r| r.id != Some(id));

    Json(ApiResponse::success(())).into_response()
}

// Upstream handlers

async fn list_upstreams(State(state): State<Arc<ProxyState>>) -> impl IntoResponse {
    let config = state.config.read().await;
    let upstreams = config.upstreams.clone();
    Json(ApiResponse::success(upstreams))
}

async fn get_upstream(
    State(state): State<Arc<ProxyState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let config = state.config.read().await;
    match config.upstreams.iter().find(|u| u.id == Some(id)) {
        Some(upstream) => Json(ApiResponse::success(upstream)).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::<()>::error("Upstream not found")),
        )
            .into_response(),
    }
}

/// Body of `POST /upstreams` and `PUT /upstreams/{id}`.
#[derive(Deserialize)]
pub struct CreateUpstreamRequest {
    /// Upstream name. Routes reference it.
    pub name: String,
    /// How requests are spread across the backends. Defaults to round-robin.
    pub lb_strategy: Option<LoadBalanceStrategy>,
    /// Backends to create with it.
    ///
    /// Only read on create. `update_upstream` ignores this field, because the
    /// type cannot express a backend's id -- applying it would re-create every
    /// backend under a fresh id, and an empty list sent by a caller that meant
    /// only to rename would remove them all.
    pub backends: Vec<BackendRequest>,
}

/// A backend, inside an upstream request or `POST /upstreams/{id}/backends`.
#[derive(Deserialize)]
pub struct BackendRequest {
    /// `host:port`.
    pub address: String,
    /// Relative share of traffic under a weighted strategy.
    pub weight: Option<u32>,
    /// `http` or `https`. Defaults to `http`.
    pub scheme: Option<String>,
}

async fn create_upstream(
    State(state): State<Arc<ProxyState>>,
    Json(req): Json<CreateUpstreamRequest>,
) -> impl IntoResponse {
    let backends: Vec<Backend> = req
        .backends
        .into_iter()
        .map(|b| Backend {
            id: Some(Uuid::new_v4()),
            address: b.address,
            weight: b.weight.unwrap_or(100),
            scheme: b.scheme.unwrap_or_else(|| "http".to_string()),
            enabled: true,
            http_version: Default::default(),
        })
        .collect();

    let upstream = Upstream {
        id: Some(Uuid::new_v4()),
        name: req.name,
        description: None,
        lb_strategy: req.lb_strategy.unwrap_or_default(),
        backends,
        health_check: HealthCheckConfig::default(),
        enabled: true,
    };

    if persists(&state).await {
        if let Err(error) = crate::config::save_upstream(&state.pool, &upstream).await {
            return persistence_failed(error);
        }
    }

    let mut config = state.config.write().await;
    config.upstreams.push(upstream.clone());

    (StatusCode::CREATED, Json(ApiResponse::success(upstream))).into_response()
}

async fn update_upstream(
    State(state): State<Arc<ProxyState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<CreateUpstreamRequest>,
) -> impl IntoResponse {
    let Some(mut updated) = state
        .config
        .read()
        .await
        .upstreams
        .iter()
        .find(|u| u.id == Some(id))
        .cloned()
    else {
        return (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::<()>::error("Upstream not found")),
        )
            .into_response();
    };

    updated.name = req.name;
    updated.lb_strategy = req.lb_strategy.unwrap_or_default();
    // `req.backends` is deliberately not applied. CreateUpstreamRequest cannot
    // express a backend's id, so treating its list as the new set would
    // re-create every backend under a fresh id on each update, and an empty
    // list -- which is what a caller sends when it only means to rename --
    // would silently remove every backend. Use the backend endpoints instead.

    if persists(&state).await {
        if let Err(error) = crate::config::save_upstream(&state.pool, &updated).await {
            return persistence_failed(error);
        }
    }

    let mut config = state.config.write().await;
    match config.upstreams.iter_mut().find(|u| u.id == Some(id)) {
        Some(upstream) => {
            *upstream = updated.clone();
            Json(ApiResponse::success(updated)).into_response()
        }
        None => (
            StatusCode::CONFLICT,
            Json(ApiResponse::<()>::error(
                "Upstream was removed while the update was being saved",
            )),
        )
            .into_response(),
    }
}

async fn delete_upstream(
    State(state): State<Arc<ProxyState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    if !state
        .config
        .read()
        .await
        .upstreams
        .iter()
        .any(|u| u.id == Some(id))
    {
        return (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::<()>::error("Upstream not found")),
        )
            .into_response();
    }

    if persists(&state).await {
        if let Err(error) = crate::config::delete_upstream(&state.pool, id).await {
            return persistence_failed(error);
        }
    }

    state
        .config
        .write()
        .await
        .upstreams
        .retain(|u| u.id != Some(id));

    Json(ApiResponse::success(())).into_response()
}

// Backend handlers

async fn list_backends(
    State(state): State<Arc<ProxyState>>,
    Path(upstream_id): Path<Uuid>,
) -> impl IntoResponse {
    let config = state.config.read().await;
    match config.upstreams.iter().find(|u| u.id == Some(upstream_id)) {
        Some(upstream) => {
            let backends = upstream.backends.clone();
            Json(ApiResponse::success(backends)).into_response()
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::<()>::error("Upstream not found")),
        )
            .into_response(),
    }
}

async fn add_backend(
    State(state): State<Arc<ProxyState>>,
    Path(upstream_id): Path<Uuid>,
    Json(req): Json<BackendRequest>,
) -> impl IntoResponse {
    let Some(mut updated) = state
        .config
        .read()
        .await
        .upstreams
        .iter()
        .find(|u| u.id == Some(upstream_id))
        .cloned()
    else {
        return (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::<()>::error("Upstream not found")),
        )
            .into_response();
    };

    let backend = Backend {
        id: Some(Uuid::new_v4()),
        address: req.address,
        weight: req.weight.unwrap_or(100),
        scheme: req.scheme.unwrap_or_else(|| "http".to_string()),
        enabled: true,
        http_version: Default::default(),
    };
    updated.backends.push(backend.clone());

    if persists(&state).await {
        // `save_upstream` writes the whole backend set in one transaction, so
        // the upstream is never briefly left without backends.
        if let Err(error) = crate::config::save_upstream(&state.pool, &updated).await {
            return persistence_failed(error);
        }
    }

    let mut config = state.config.write().await;
    match config
        .upstreams
        .iter_mut()
        .find(|u| u.id == Some(upstream_id))
    {
        Some(upstream) => {
            *upstream = updated;
            (StatusCode::CREATED, Json(ApiResponse::success(backend))).into_response()
        }
        None => (
            StatusCode::CONFLICT,
            Json(ApiResponse::<()>::error(
                "Upstream was removed while the backend was being saved",
            )),
        )
            .into_response(),
    }
}

async fn remove_backend(
    State(state): State<Arc<ProxyState>>,
    Path((upstream_id, backend_id)): Path<(Uuid, Uuid)>,
) -> impl IntoResponse {
    let Some(mut updated) = state
        .config
        .read()
        .await
        .upstreams
        .iter()
        .find(|u| u.id == Some(upstream_id))
        .cloned()
    else {
        return (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::<()>::error("Upstream not found")),
        )
            .into_response();
    };

    let before = updated.backends.len();
    updated.backends.retain(|b| b.id != Some(backend_id));
    if updated.backends.len() == before {
        return (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::<()>::error("Backend not found")),
        )
            .into_response();
    }

    if persists(&state).await {
        if let Err(error) = crate::config::save_upstream(&state.pool, &updated).await {
            return persistence_failed(error);
        }
    }

    let mut config = state.config.write().await;
    match config
        .upstreams
        .iter_mut()
        .find(|u| u.id == Some(upstream_id))
    {
        Some(upstream) => {
            *upstream = updated;
            Json(ApiResponse::success(())).into_response()
        }
        None => (
            StatusCode::CONFLICT,
            Json(ApiResponse::<()>::error(
                "Upstream was removed while the backend was being deleted",
            )),
        )
            .into_response(),
    }
}

// Health handlers

/// Backend health across the whole running config.
#[derive(Serialize)]
pub struct HealthSummary {
    /// Backends across every upstream.
    pub total_backends: usize,
    /// How many are currently passing their health check.
    pub healthy_backends: usize,
    /// How many are not. An unknown backend counts as healthy, so this is
    /// backends known to be failing rather than backends not known to be
    /// passing.
    pub unhealthy_backends: usize,
}

async fn health_status(State(state): State<Arc<ProxyState>>) -> impl IntoResponse {
    let config = state.config.read().await;

    let mut total = 0;
    let mut healthy = 0;

    for upstream in &config.upstreams {
        for backend in &upstream.backends {
            total += 1;
            if let Some(id) = backend.id {
                if state.health_checker.is_healthy(id) {
                    healthy += 1;
                }
            }
        }
    }

    Json(ApiResponse::success(HealthSummary {
        total_backends: total,
        healthy_backends: healthy,
        unhealthy_backends: total - healthy,
    }))
}

async fn backend_health(
    State(state): State<Arc<ProxyState>>,
    Path(backend_id): Path<Uuid>,
) -> impl IntoResponse {
    match state.health_checker.get_health(backend_id) {
        Some(health) => Json(ApiResponse::success(health)).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::<()>::error("Backend not found")),
        )
            .into_response(),
    }
}

// Stats handlers

/// How much configuration the proxy is currently holding.
#[derive(Serialize)]
pub struct ProxyStats {
    /// Routes in the running config, enabled or not.
    pub routes_count: usize,
    /// Upstreams in the running config.
    pub upstreams_count: usize,
    /// Backends across all of them.
    pub backends_count: usize,
}

async fn get_stats(State(state): State<Arc<ProxyState>>) -> impl IntoResponse {
    let config = state.config.read().await;

    let backends_count: usize = config.upstreams.iter().map(|u| u.backends.len()).sum();

    Json(ApiResponse::success(ProxyStats {
        routes_count: config.routes.len(),
        upstreams_count: config.upstreams.len(),
        backends_count,
    }))
}
