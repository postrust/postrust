//! Jobs, queues and workflows.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use super::{ApiResult, AppState};
use crate::error::AiError;
use crate::queue::{Job, JobFilter, NewJob, Priority, QueueStats};
use crate::workflow::{WorkflowDef, WorkflowRun};

pub(super) async fn submit(
    State(state): State<AppState>,
    Json(job): Json<NewJob>,
) -> ApiResult<(StatusCode, Json<Job>)> {
    // Every node carries the whole model registry (resources decide what it
    // actually loads), so an unknown model here is a typo, not a model some
    // other node has. Refusing it beats a job that waits forever.
    state.manager.spec(&job.model_id)?;
    if state.config.queue(&job.queue).is_none() {
        return Err(AiError::UnknownQueue(job.queue).into());
    }
    let job = state.store.enqueue(job).await?;
    state.scheduler.wake();
    Ok((StatusCode::CREATED, Json(job)))
}

pub(super) async fn list(
    State(state): State<AppState>,
    Query(filter): Query<JobFilter>,
) -> ApiResult<Json<Vec<Job>>> {
    Ok(Json(state.store.list(&filter).await?))
}

pub(super) async fn get(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Job>> {
    state
        .store
        .get(id)
        .await?
        .map(Json)
        .ok_or_else(|| AiError::NotFound(format!("job {id}")).into())
}

pub(super) async fn pause(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Job>> {
    Ok(Json(state.store.request_pause(id).await?))
}

pub(super) async fn resume(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Job>> {
    let job = state.store.resume(id).await?;
    state.scheduler.wake();
    Ok(Json(job))
}

pub(super) async fn cancel(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Job>> {
    Ok(Json(state.store.request_cancel(id).await?))
}

pub(super) async fn queues(State(state): State<AppState>) -> ApiResult<Json<Vec<QueueStats>>> {
    Ok(Json(state.store.queue_stats().await?))
}

pub(super) async fn put_workflow(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Json(def): Json<WorkflowDef>,
) -> ApiResult<Json<Value>> {
    let version = state.workflows.put(&name, &def).await?;
    Ok(Json(json!({ "name": name, "version": version })))
}

pub(super) async fn get_workflow(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> ApiResult<Json<Value>> {
    let (def, version) = state
        .workflows
        .get(&name)
        .await?
        .ok_or_else(|| AiError::NotFound(format!("workflow '{name}'")))?;
    Ok(Json(
        json!({ "name": name, "version": version, "definition": def }),
    ))
}

#[derive(Deserialize)]
pub(super) struct StartRun {
    input: Value,
    #[serde(default)]
    queue: Option<String>,
    #[serde(default)]
    priority: Option<Priority>,
}

pub(super) async fn start_run(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Json(body): Json<StartRun>,
) -> ApiResult<(StatusCode, Json<WorkflowRun>)> {
    let queue = body
        .queue
        .unwrap_or_else(|| state.config.queues[0].name.clone());
    if state.config.queue(&queue).is_none() {
        return Err(AiError::UnknownQueue(queue).into());
    }
    let run = state
        .workflows
        .start(&name, body.input, &queue, body.priority.unwrap_or_default())
        .await?;
    state.scheduler.wake();
    Ok((StatusCode::CREATED, Json(run)))
}

pub(super) async fn get_run(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<WorkflowRun>> {
    state
        .workflows
        .run(id)
        .await?
        .map(Json)
        .ok_or_else(|| AiError::NotFound(format!("workflow run {id}")).into())
}

pub(super) async fn cancel_run(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<WorkflowRun>> {
    if !state.workflows.cancel(id).await? {
        let run = state
            .workflows
            .run(id)
            .await?
            .ok_or_else(|| AiError::NotFound(format!("workflow run {id}")))?;
        return Err(AiError::Conflict(format!("run {id} is {}", run.status)).into());
    }
    get_run(State(state), Path(id)).await
}
