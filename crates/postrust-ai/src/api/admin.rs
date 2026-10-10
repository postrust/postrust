//! Models, queues, sessions and node state.

use axum::extract::{Path, State};
use axum::{Extension, Json};
use serde_json::{json, Value};

use super::{require_admin, ApiResult, AppState, Caller};
use crate::cache::CachedFile;
use crate::error::AiError;
use crate::nodes::NodeInfo;
use crate::queue::Priority;
use crate::session::SessionLock;

pub(super) async fn pause_queue(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path(name): Path<String>,
) -> ApiResult<Json<Value>> {
    require_admin(&caller)?;
    state.store.set_queue_paused(&name, true).await?;
    Ok(Json(json!({ "queue": name, "paused": true })))
}

pub(super) async fn resume_queue(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path(name): Path<String>,
) -> ApiResult<Json<Value>> {
    require_admin(&caller)?;
    state.store.set_queue_paused(&name, false).await?;
    state.scheduler.wake();
    Ok(Json(json!({ "queue": name, "paused": false })))
}

pub(super) async fn models(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let instances = state.manager.instances();
    let models: Vec<Value> = state
        .manager
        .specs()
        .iter()
        .map(|spec| {
            let mine: Vec<_> = instances.iter().filter(|i| i.model_id == spec.id).collect();
            json!({ "spec": spec, "instances": mine })
        })
        .collect();
    Ok(Json(
        json!({ "node": state.config.node.id, "models": models }),
    ))
}

pub(super) async fn load_model(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    require_admin(&caller)?;
    state.manager.spec(&id)?;
    // An operator asking for a load may unload idle models for it.
    state
        .manager
        .preload_in_background(&id, Priority::High, true);
    Ok(Json(json!({ "model": id, "loading": true })))
}

pub(super) async fn unload_model(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    require_admin(&caller)?;
    let (stopped, busy) = state.manager.unload(&id)?;
    Ok(Json(
        json!({ "model": id, "stopping": stopped, "busy": busy }),
    ))
}

pub(super) async fn invalidate_results(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    require_admin(&caller)?;
    let removed = state.results.invalidate_model(&id).await?;
    Ok(Json(json!({ "model": id, "removed": removed })))
}

pub(super) async fn sessions(State(state): State<AppState>) -> ApiResult<Json<Vec<SessionLock>>> {
    Ok(Json(state.sessions.list().await?))
}

pub(super) async fn release_session(
    State(state): State<AppState>,
    Path((model, session)): Path<(String, String)>,
) -> ApiResult<Json<Value>> {
    if !state.sessions.release(&session, &model).await? {
        return Err(AiError::NotFound(format!("session '{session}' for '{model}'")).into());
    }
    Ok(Json(json!({ "released": true })))
}

pub(super) async fn nodes(State(state): State<AppState>) -> ApiResult<Json<Vec<NodeInfo>>> {
    Ok(Json(state.nodes.list().await?))
}

pub(super) async fn resources(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    Ok(Json(json!({
        "node": state.config.node.id,
        "pool": state.manager.resources(),
        "running": state.scheduler.running_counts(),
    })))
}

pub(super) async fn cache_files(State(state): State<AppState>) -> ApiResult<Json<Vec<CachedFile>>> {
    Ok(Json(state.files.list().await?))
}
