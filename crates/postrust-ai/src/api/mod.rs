//! The HTTP API: jobs, workflows, models, sessions and the synchronous
//! inference gateway.
//!
//! | Method | Path | |
//! |---|---|---|
//! | POST | `/v1/jobs` | submit a job |
//! | GET | `/v1/jobs`, `/v1/jobs/{id}` | list, get |
//! | POST | `/v1/jobs/{id}/pause`, `/resume`, `/cancel` | control |
//! | GET | `/v1/queues` | depth per queue and class |
//! | POST | `/v1/queues/{name}/pause`, `/resume` | admin |
//! | GET | `/v1/models` | specs and instances |
//! | POST | `/v1/models/{id}/load`, `/unload` | admin |
//! | DELETE | `/v1/models/{id}/results` | drop cached results (admin) |
//! | ANY | `/v1/infer/{model}/{*path}` | synchronous inference, lazy load, session lock |
//! | GET | `/v1/sessions` | this node's session locks |
//! | DELETE | `/v1/sessions/{model}/{session}` | release a session lock |
//! | PUT/GET | `/v1/workflows/{name}` | define, read |
//! | POST | `/v1/workflows/{name}/runs` | start a run |
//! | GET | `/v1/workflow-runs/{id}` | run status |
//! | POST | `/v1/workflow-runs/{id}/cancel` | cancel a run |
//! | GET | `/v1/nodes`, `/v1/resources`, `/v1/cache/files` | cluster and node state |
//! | GET | `/healthz`, `/readyz`, `/metrics` | unauthenticated |

mod admin;
mod gateway;
mod jobs;

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::{HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, delete, get, post};
use axum::{Json, Router};
use serde_json::json;

use crate::cache::{FileCache, ResultCache};
use crate::config::AiConfig;
use crate::error::AiError;
use crate::manager::ModelManager;
use crate::metrics::Metrics;
use crate::nodes::Nodes;
use crate::queue::JobStore;
use crate::scheduler::Scheduler;
use crate::session::Sessions;
use crate::workflow::WorkflowEngine;

/// Everything the handlers need.
#[derive(Clone)]
pub struct AppState {
    /// Configuration.
    pub config: Arc<AiConfig>,
    /// Queue.
    pub store: Arc<dyn JobStore>,
    /// Models.
    pub manager: ModelManager,
    /// Session locks.
    pub sessions: Arc<Sessions>,
    /// Peers.
    pub nodes: Nodes,
    /// Workflows.
    pub workflows: WorkflowEngine,
    /// Result cache.
    pub results: ResultCache,
    /// Model file cache.
    pub files: Arc<FileCache>,
    /// Counters.
    pub metrics: Arc<Metrics>,
    /// Worker loop, to wake on submit.
    pub scheduler: Arc<Scheduler>,
    /// Client for forwarding: no overall timeout, since responses stream.
    pub forward_http: reqwest::Client,
    /// Database, for readiness.
    pub pool: sqlx::PgPool,
}

/// Build the router.
pub fn router(state: AppState) -> Router {
    let authed = Router::new()
        .route("/v1/jobs", post(jobs::submit).get(jobs::list))
        .route("/v1/jobs/{id}", get(jobs::get))
        .route("/v1/jobs/{id}/pause", post(jobs::pause))
        .route("/v1/jobs/{id}/resume", post(jobs::resume))
        .route("/v1/jobs/{id}/cancel", post(jobs::cancel))
        .route("/v1/queues", get(jobs::queues))
        .route("/v1/queues/{name}/pause", post(admin::pause_queue))
        .route("/v1/queues/{name}/resume", post(admin::resume_queue))
        .route("/v1/models", get(admin::models))
        .route("/v1/models/{id}/load", post(admin::load_model))
        .route("/v1/models/{id}/unload", post(admin::unload_model))
        .route("/v1/models/{id}/results", delete(admin::invalidate_results))
        .route("/v1/infer/{model}/{*path}", any(gateway::infer))
        .route("/v1/sessions", get(admin::sessions))
        .route(
            "/v1/sessions/{model}/{session}",
            delete(admin::release_session),
        )
        .route(
            "/v1/workflows/{name}",
            get(jobs::get_workflow).put(jobs::put_workflow),
        )
        .route("/v1/workflows/{name}/runs", post(jobs::start_run))
        .route("/v1/workflow-runs/{id}", get(jobs::get_run))
        .route("/v1/workflow-runs/{id}/cancel", post(jobs::cancel_run))
        .route("/v1/nodes", get(admin::nodes))
        .route("/v1/resources", get(admin::resources))
        .route("/v1/cache/files", get(admin::cache_files))
        .layer(middleware::from_fn_with_state(state.clone(), authenticate));

    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/readyz", get(readyz))
        .route("/metrics", get(metrics))
        .merge(authed)
        .with_state(state)
}

async fn readyz(State(state): State<AppState>) -> Response {
    match sqlx::query("SELECT 1").execute(&state.pool).await {
        Ok(_) => "ready".into_response(),
        Err(error) => (
            StatusCode::SERVICE_UNAVAILABLE,
            format!("database: {error}"),
        )
            .into_response(),
    }
}

async fn metrics(State(state): State<AppState>) -> Response {
    let queues = state.store.queue_stats().await.unwrap_or_default();
    let body = state.metrics.render(
        &queues,
        &state.manager.instances(),
        &state.manager.resources(),
    );
    (
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; version=0.0.4",
        )],
        body,
    )
        .into_response()
}

/// The caller, as authentication found them.
#[derive(Debug, Clone)]
pub struct Caller {
    /// Role claim, if authenticated.
    pub role: Option<String>,
    /// Whether they may use admin endpoints.
    pub admin: bool,
}

async fn authenticate(State(state): State<AppState>, mut request: Request, next: Next) -> Response {
    let auth = &state.config.auth;
    let Some(secret) = &auth.jwt_secret else {
        request.extensions_mut().insert(Caller {
            role: None,
            admin: true,
        });
        return next.run(request).await;
    };
    let jwt = postrust_auth::JwtConfig {
        secret: Some(secret.clone()),
        audience: auth.audience.clone(),
        ..Default::default()
    };
    let header = request
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());
    match postrust_auth::authenticate(header, &jwt) {
        Ok(result) => {
            let admin = auth.admin_roles.is_empty() || auth.admin_roles.contains(&result.role);
            request.extensions_mut().insert(Caller {
                role: Some(result.role),
                admin,
            });
            next.run(request).await
        }
        Err(error) => {
            let mut response = (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": error.to_string() })),
            )
                .into_response();
            response
                .headers_mut()
                .insert("www-authenticate", HeaderValue::from_static("Bearer"));
            response
        }
    }
}

/// Reject non-admin callers.
pub(crate) fn require_admin(caller: &Caller) -> Result<(), ApiError> {
    if caller.admin {
        Ok(())
    } else {
        Err(ApiError(
            AiError::Conflict("forbidden".into()),
            Some(StatusCode::FORBIDDEN),
        ))
    }
}

/// An error as an HTTP response.
pub struct ApiError(pub AiError, pub Option<StatusCode>);

impl From<AiError> for ApiError {
    fn from(error: AiError) -> Self {
        ApiError(error, None)
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(error: sqlx::Error) -> Self {
        ApiError(error.into(), None)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let ApiError(error, status) = self;
        let status = status.unwrap_or(match &error {
            AiError::BadRequest(_) | AiError::Config(_) => StatusCode::BAD_REQUEST,
            AiError::NotFound(_) | AiError::UnknownModel(_) | AiError::UnknownQueue(_) => {
                StatusCode::NOT_FOUND
            }
            AiError::Conflict(_) => StatusCode::CONFLICT,
            AiError::LoadTimeout { .. }
            | AiError::InsufficientResources { .. }
            | AiError::Cancelled => StatusCode::SERVICE_UNAVAILABLE,
            AiError::LoadFailed { .. } | AiError::Upstream(_) | AiError::Http(_) => {
                StatusCode::BAD_GATEWAY
            }
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        });
        if status.is_server_error() && status != StatusCode::SERVICE_UNAVAILABLE {
            tracing::error!(%error, "request failed");
        }
        let mut response = (status, Json(json!({ "error": error.to_string() }))).into_response();
        if status == StatusCode::SERVICE_UNAVAILABLE {
            response
                .headers_mut()
                .insert("retry-after", HeaderValue::from_static("10"));
        }
        response
    }
}

/// Handler result.
pub type ApiResult<T> = Result<T, ApiError>;
