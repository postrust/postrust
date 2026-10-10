//! Running one claimed job: the result cache, getting a model instance,
//! heartbeats, the handler, and what happens on pause, cancel or failure.
//!
//! # The built-in handler
//!
//! [`ModelHttpHandler`] calls the model server over HTTP. A job's input is
//! one request or a list of them:
//!
//! ```json
//! { "request":  { "path": "/v1/chat/completions", "body": { "messages": [...] } } }
//! { "requests": [ { "body": ... }, { "body": ... } ] }
//! { "request":  { "multipart": { "file_url": "https://.../a.wav",
//!                                "fields": { "response_format": "json" } } } }
//! ```
//!
//! `path` defaults to the flavor's inference path, `method` to POST. For vLLM
//! and Ollama, `body.model` is filled in with the served model name if absent.
//!
//! A list is the unit of checkpointing: after each request the results so far
//! are saved as `{"next": i, "results": [...]}`, and a paused job resumes from
//! `next`. Split long work -- a video's segments, an audio file's chunks, a
//! document's pages -- into a list, and pausing costs at most one request.
//!
//! # Pausing
//!
//! A pause comes from the manager (a superlow job's instance is wanted) or
//! from the API (`pause_requested`, seen on heartbeat or by notification).
//! What happens depends on the queue:
//!
//! - `checkpoint`: the handler is told and gets the queue's grace period to
//!   stop at a checkpoint by itself. If it has not, it is cancelled, and the
//!   job resumes from the last checkpoint it saved.
//! - `requeue`: the handler is cancelled at once and progress is discarded.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine;
use parking_lot::Mutex;
use serde_json::{json, Value};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::cache::ResultCache;
use crate::error::{AiError, AiResult};
use crate::manager::{AcquireOptions, Lease, ModelManager};
use crate::metrics::Metrics;
use crate::model::{Flavor, ModelSpec};
use crate::queue::{Heartbeat, Job, JobStore, PauseMode, PauseReason, QueueSpec, Yield};

/// Why a job is being asked to stop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    /// Make way for higher-priority work.
    Preempted,
    /// Paused through the API.
    ManualPause,
    /// Cancelled through the API.
    Cancel,
}

/// What a handler sees.
pub struct JobContext {
    job: Job,
    lease: Lease,
    http: reqwest::Client,
    store: Arc<dyn JobStore>,
    node: String,
    pause: CancellationToken,
    last_checkpoint: Mutex<Option<Value>>,
}

impl JobContext {
    /// The job.
    pub fn job(&self) -> &Job {
        &self.job
    }
    /// The model.
    pub fn spec(&self) -> &ModelSpec {
        self.lease.spec()
    }
    /// The instance's base URL.
    pub fn base_url(&self) -> &str {
        self.lease.base_url()
    }
    /// A client for calling it.
    pub fn http(&self) -> &reqwest::Client {
        &self.http
    }
    /// Progress saved by an earlier run of this job, if any.
    pub fn checkpoint(&self) -> Option<&Value> {
        self.job.checkpoint.as_ref()
    }
    /// Whether the job has been asked to pause. Check between units of work.
    pub fn pause_requested(&self) -> bool {
        self.pause.is_cancelled()
    }
    /// Resolves when the job is asked to pause.
    pub async fn paused(&self) {
        self.pause.cancelled().await
    }
    /// Save progress. A paused job resumes from the last one saved.
    pub async fn save_checkpoint(&self, checkpoint: Value) -> AiResult<()> {
        if !self
            .store
            .save_checkpoint(self.job.id, &self.node, &checkpoint)
            .await?
        {
            return Err(AiError::Conflict(
                "job is no longer held by this node".into(),
            ));
        }
        *self.last_checkpoint.lock() = Some(checkpoint);
        Ok(())
    }
}

/// How a handler finished.
#[derive(Debug)]
pub enum HandlerOutcome {
    /// Done; this is the output.
    Completed(Value),
    /// Stopped at a checkpoint because a pause was asked for.
    Paused,
}

/// How a handler failed.
#[derive(Debug)]
pub struct JobFailure {
    /// What went wrong.
    pub message: String,
    /// Whether trying again might help.
    pub retryable: bool,
}

impl JobFailure {
    /// A failure worth retrying.
    pub fn retryable(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            retryable: true,
        }
    }
    /// A failure that will happen again.
    pub fn permanent(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            retryable: false,
        }
    }
}

impl From<AiError> for JobFailure {
    fn from(error: AiError) -> Self {
        let retryable = !matches!(
            error,
            AiError::BadRequest(_) | AiError::UnknownModel(_) | AiError::NotFound(_)
        );
        Self {
            message: error.to_string(),
            retryable,
        }
    }
}

/// What a job does once it has an instance.
#[async_trait]
pub trait JobHandler: Send + Sync {
    /// Run the job. Long handlers should check
    /// [`JobContext::pause_requested`] between units of work, save a
    /// checkpoint, and return [`HandlerOutcome::Paused`].
    async fn run(&self, ctx: &JobContext) -> Result<HandlerOutcome, JobFailure>;
}

/// Runs claimed jobs.
pub struct Executor {
    node: String,
    store: Arc<dyn JobStore>,
    manager: ModelManager,
    results: Option<ResultCache>,
    http: reqwest::Client,
    handlers: HashMap<String, Arc<dyn JobHandler>>,
    default_handler: Arc<dyn JobHandler>,
    lease: Duration,
    heartbeat: Duration,
    /// Running jobs: a poke makes the heartbeat check control flags now, and
    /// the token preempts the job from this node (a queue slot is wanted).
    running: Mutex<HashMap<Uuid, (Arc<Notify>, CancellationToken)>>,
    metrics: Arc<Metrics>,
}

impl Executor {
    /// An executor using the built-in HTTP handler for every queue.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        node: String,
        store: Arc<dyn JobStore>,
        manager: ModelManager,
        results: Option<ResultCache>,
        http: reqwest::Client,
        lease: Duration,
        heartbeat: Duration,
        metrics: Arc<Metrics>,
    ) -> Self {
        Self {
            node,
            store,
            manager,
            results,
            http,
            handlers: HashMap::new(),
            default_handler: Arc::new(ModelHttpHandler),
            lease,
            heartbeat,
            running: Mutex::new(HashMap::new()),
            metrics,
        }
    }

    /// Use `handler` for jobs on `queue`.
    pub fn with_handler(mut self, queue: impl Into<String>, handler: Arc<dyn JobHandler>) -> Self {
        self.handlers.insert(queue.into(), handler);
        self
    }

    /// Have a running job check its control flags now rather than at its next
    /// heartbeat.
    pub fn poke(&self, job_id: Uuid) {
        if let Some((notify, _)) = self.running.lock().get(&job_id) {
            notify.notify_one();
        }
    }

    /// Preempt a job running on this node. Returns whether it was running.
    pub fn preempt(&self, job_id: Uuid) -> bool {
        match self.running.lock().get(&job_id) {
            Some((_, token)) => {
                token.cancel();
                true
            }
            None => false,
        }
    }

    /// Run one claimed job to an outcome and record it.
    pub async fn run(&self, job: Job, queue: &QueueSpec) {
        let id = job.id;
        let poke = Arc::new(Notify::new());
        let preempt = CancellationToken::new();
        self.running
            .lock()
            .insert(id, (poke.clone(), preempt.clone()));
        let started = std::time::Instant::now();
        let result = self.run_inner(job, queue, poke, preempt).await;
        self.running.lock().remove(&id);
        self.metrics.observe_job(&queue.name, started.elapsed());
        if let Err(error) = result {
            // Only storage errors get here. The lease will expire and the
            // reaper will requeue the job, so nothing is lost.
            tracing::error!(job = %id, %error, "could not record job outcome");
        }
    }

    async fn run_inner(
        &self,
        job: Job,
        queue: &QueueSpec,
        poke: Arc<Notify>,
        local_preempt: CancellationToken,
    ) -> AiResult<()> {
        let spec = match self.manager.spec(&job.model_id) {
            Ok(spec) => spec,
            Err(error) => {
                self.store
                    .fail(job.id, &self.node, &error.to_string(), false)
                    .await?;
                return Ok(());
            }
        };

        // Layer 5: a cached result means no model at all.
        let cache_key = (spec.result_cache || queue.result_cache)
            .then(|| ResultCache::key(&spec.id, &spec.version, &job.input, &job.params));
        if let (Some(cache), Some(key)) = (&self.results, &cache_key) {
            if let Some(output) = cache.get(key).await? {
                self.metrics.result_cache_hit();
                self.store
                    .complete(job.id, &self.node, &output, true)
                    .await?;
                tracing::debug!(job = %job.id, "result cache hit");
                return Ok(());
            }
            self.metrics.result_cache_miss();
        }

        // Signals: the heartbeat loop turns stored flags into these.
        let stop = CancellationToken::new();
        let stop_reason: Arc<Mutex<Option<StopReason>>> = Arc::new(Mutex::new(None));
        let lost = CancellationToken::new();
        let heartbeat = self.spawn_heartbeat(
            job.id,
            poke,
            stop.clone(),
            stop_reason.clone(),
            lost.clone(),
        );
        forward_preemption(
            local_preempt,
            stop.clone(),
            stop_reason.clone(),
            heartbeat.clone(),
        );

        // Getting an instance can mean waiting for a load; a pause or cancel
        // during that wait is honoured too.
        let mut opts = AcquireOptions::new(
            job.priority_class,
            Duration::from_secs(spec.load_timeout_secs + 60),
        );
        opts.job_id = Some(job.id);
        let lease = tokio::select! {
            lease = self.manager.acquire(&job.model_id, opts) => lease,
            _ = stop.cancelled() => {
                heartbeat.cancel();
                let reason = stop_reason.lock().unwrap_or(StopReason::ManualPause);
                return self.set_aside(&job, queue, reason).await;
            }
            _ = lost.cancelled() => {
                heartbeat.cancel();
                return Ok(());
            }
        };
        let lease = match lease {
            Ok(lease) => lease,
            Err(error) => {
                heartbeat.cancel();
                if job.priority_class.is_superlow() {
                    // Capacity was taken between claim and load. Superlow
                    // waits for spare capacity rather than burning attempts.
                    self.store
                        .yield_job(
                            job.id,
                            &self.node,
                            Yield::keep(PauseReason::Preempted, None),
                        )
                        .await?;
                } else {
                    let failure = JobFailure::from(error);
                    self.store
                        .fail(job.id, &self.node, &failure.message, failure.retryable)
                        .await?;
                }
                return Ok(());
            }
        };

        // The manager preempting the lease is a pause like any other.
        forward_preemption(
            lease.preempted().clone(),
            stop.clone(),
            stop_reason.clone(),
            heartbeat.clone(),
        );

        let ctx = JobContext {
            job: job.clone(),
            lease,
            http: self.http.clone(),
            store: self.store.clone(),
            node: self.node.clone(),
            pause: stop.clone(),
            last_checkpoint: Mutex::new(None),
        };
        let handler = self
            .handlers
            .get(&job.queue)
            .cloned()
            .unwrap_or_else(|| self.default_handler.clone());

        let outcome = {
            let run = handler.run(&ctx);
            tokio::pin!(run);
            tokio::select! {
                outcome = &mut run => Some(outcome),
                _ = lost.cancelled() => None,
                _ = stop.cancelled() => {
                    let reason = stop_reason.lock().unwrap_or(StopReason::ManualPause);
                    let cooperative = reason != StopReason::Cancel
                        && queue.pause_mode == PauseMode::Checkpoint;
                    if cooperative {
                        // Grace period: let it reach a checkpoint.
                        tokio::select! {
                            outcome = &mut run => Some(outcome),
                            _ = tokio::time::sleep(queue.pause_grace()) => {
                                tracing::info!(job = %job.id, "job did not pause within grace; cancelling");
                                Some(Ok(HandlerOutcome::Paused))
                            }
                            _ = lost.cancelled() => None,
                        }
                    } else {
                        Some(Ok(HandlerOutcome::Paused))
                    }
                }
            }
        };
        heartbeat.cancel();
        let Some(outcome) = outcome else {
            tracing::warn!(job = %job.id, "lost the job's lease; abandoning it");
            return Ok(());
        };
        drop(ctx.lease);

        match outcome {
            Ok(HandlerOutcome::Completed(output)) => {
                if let (Some(cache), Some(key)) = (&self.results, &cache_key) {
                    if let Err(error) = cache.put(key, &spec.id, &output).await {
                        tracing::warn!(%error, "could not store result in cache");
                    }
                }
                self.store
                    .complete(job.id, &self.node, &output, false)
                    .await?;
            }
            Ok(HandlerOutcome::Paused) => {
                let reason = stop_reason.lock().unwrap_or(StopReason::ManualPause);
                let last = ctx.last_checkpoint.lock().take();
                self.set_aside_with(&job, queue, reason, last).await?;
            }
            Err(failure) => {
                // A failure caused by our own preemption (the instance went
                // away under the request) is still a pause, not an attempt.
                let stopped = *stop_reason.lock();
                if let Some(reason) = stopped {
                    let last = ctx.last_checkpoint.lock().take();
                    return self.set_aside_with(&job, queue, reason, last).await;
                }
                tracing::warn!(job = %job.id, error = %failure.message, retryable = failure.retryable, "job failed");
                self.store
                    .fail(job.id, &self.node, &failure.message, failure.retryable)
                    .await?;
            }
        }
        Ok(())
    }

    async fn set_aside(&self, job: &Job, queue: &QueueSpec, reason: StopReason) -> AiResult<()> {
        self.set_aside_with(job, queue, reason, None).await
    }

    async fn set_aside_with(
        &self,
        job: &Job,
        queue: &QueueSpec,
        reason: StopReason,
        checkpoint: Option<Value>,
    ) -> AiResult<()> {
        let pause_reason = match reason {
            StopReason::Cancel => {
                self.store.finish_cancelled(job.id, &self.node).await?;
                return Ok(());
            }
            StopReason::Preempted => {
                self.metrics.preemption();
                PauseReason::Preempted
            }
            StopReason::ManualPause => PauseReason::Manual,
        };
        let how = match queue.pause_mode {
            PauseMode::Checkpoint => Yield::keep(pause_reason, checkpoint),
            PauseMode::Requeue => Yield::discard(pause_reason),
        };
        tracing::info!(job = %job.id, reason = pause_reason.as_str(), mode = queue.pause_mode.as_str(), "job paused");
        self.store.yield_job(job.id, &self.node, how).await?;
        Ok(())
    }

    /// Heartbeat until cancelled, translating stored flags into signals.
    fn spawn_heartbeat(
        &self,
        id: Uuid,
        poke: Arc<Notify>,
        stop: CancellationToken,
        stop_reason: Arc<Mutex<Option<StopReason>>>,
        lost: CancellationToken,
    ) -> CancellationToken {
        let done = CancellationToken::new();
        let store = self.store.clone();
        let node = self.node.clone();
        let lease = self.lease;
        let interval = self.heartbeat;
        let finished = done.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = finished.cancelled() => return,
                    _ = tokio::time::sleep(interval) => {}
                    _ = poke.notified() => {}
                }
                match store.heartbeat(id, &node, lease).await {
                    Ok(Heartbeat::Continue) => {}
                    Ok(Heartbeat::PauseRequested) => {
                        stop_reason.lock().get_or_insert(StopReason::ManualPause);
                        stop.cancel();
                    }
                    Ok(Heartbeat::CancelRequested) => {
                        // A cancel overrides a pause already under way.
                        *stop_reason.lock() = Some(StopReason::Cancel);
                        stop.cancel();
                    }
                    Ok(Heartbeat::Lost) => {
                        lost.cancel();
                        return;
                    }
                    Err(error) => {
                        // Keep trying: the lease has slack for a missed beat
                        // or two (heartbeat_secs is at most half the lease).
                        tracing::warn!(job = %id, %error, "heartbeat failed");
                    }
                }
            }
        });
        done
    }
}

/// Turn a preemption token into a stop, until `done`.
fn forward_preemption(
    preempted: CancellationToken,
    stop: CancellationToken,
    stop_reason: Arc<Mutex<Option<StopReason>>>,
    done: CancellationToken,
) {
    tokio::spawn(async move {
        tokio::select! {
            _ = preempted.cancelled() => {
                stop_reason.lock().get_or_insert(StopReason::Preempted);
                stop.cancel();
            }
            _ = done.cancelled() => {}
        }
    });
}

// ============================================================================
// Built-in handler
// ============================================================================

/// Calls the model server over HTTP. See the module docs for the input shape.
pub struct ModelHttpHandler;

#[async_trait]
impl JobHandler for ModelHttpHandler {
    async fn run(&self, ctx: &JobContext) -> Result<HandlerOutcome, JobFailure> {
        let input = &ctx.job().input;
        if let Some(list) = input.get("requests") {
            let Some(requests) = list.as_array() else {
                return Err(JobFailure::permanent("'requests' must be an array"));
            };
            return run_list(ctx, requests).await;
        }
        let request = input.get("request").unwrap_or(input);
        let output = call(ctx, request).await?;
        Ok(HandlerOutcome::Completed(output))
    }
}

async fn run_list(ctx: &JobContext, requests: &[Value]) -> Result<HandlerOutcome, JobFailure> {
    let (mut next, mut results) = match ctx.checkpoint() {
        Some(cp) => (
            cp.get("next").and_then(Value::as_u64).unwrap_or(0) as usize,
            cp.get("results")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
        ),
        None => (0, Vec::new()),
    };
    if next > requests.len() || results.len() != next {
        // A checkpoint that does not match the input (edited job?) is not
        // something to resume from.
        tracing::warn!(job = %ctx.job().id, "checkpoint does not match input; starting over");
        next = 0;
        results.clear();
    }
    if next > 0 {
        tracing::info!(job = %ctx.job().id, from = next, of = requests.len(), "resuming from checkpoint");
    }
    while next < requests.len() {
        if ctx.pause_requested() {
            return Ok(HandlerOutcome::Paused);
        }
        let output = call(ctx, &requests[next]).await?;
        results.push(output);
        next += 1;
        if next < requests.len() {
            ctx.save_checkpoint(json!({ "next": next, "results": results }))
                .await
                .map_err(JobFailure::from)?;
        }
    }
    Ok(HandlerOutcome::Completed(json!({ "results": results })))
}

/// One request to the model server.
pub async fn call(ctx: &JobContext, request: &Value) -> Result<Value, JobFailure> {
    let spec = ctx.spec();
    let path = request
        .get("path")
        .and_then(Value::as_str)
        .unwrap_or_else(|| spec.flavor.default_invoke_path(spec.kind));
    if !path.starts_with('/') {
        return Err(JobFailure::permanent(format!(
            "path '{path}' must start with '/'"
        )));
    }
    let method = request
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("POST")
        .parse::<reqwest::Method>()
        .map_err(|_| JobFailure::permanent("unknown method"))?;
    let url = format!("{}{}", ctx.base_url(), path);
    let mut builder = ctx.http().request(method, &url);
    if let Some(headers) = request.get("headers").and_then(Value::as_object) {
        for (name, value) in headers {
            if let Some(value) = value.as_str() {
                builder = builder.header(name, value);
            }
        }
    }

    if let Some(multipart) = request.get("multipart") {
        builder = builder.multipart(build_multipart(ctx, multipart).await?);
    } else if let Some(body) = request.get("body") {
        builder = builder.json(&with_model_name(spec, body));
    }

    let response = builder
        .send()
        .await
        .map_err(|e| JobFailure::retryable(format!("request to model server failed: {e}")))?;
    let status = response.status();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let bytes = response
        .bytes()
        .await
        .map_err(|e| JobFailure::retryable(format!("reading model response: {e}")))?;

    if !status.is_success() {
        let text = String::from_utf8_lossy(&bytes);
        let message = format!("model server returned {status}: {}", truncate(&text, 2000));
        // 4xx means the request is wrong and will stay wrong; except timeouts
        // and rate limits, which pass.
        let permanent = status.is_client_error()
            && status != reqwest::StatusCode::REQUEST_TIMEOUT
            && status != reqwest::StatusCode::TOO_MANY_REQUESTS;
        return Err(if permanent {
            JobFailure::permanent(message)
        } else {
            JobFailure::retryable(message)
        });
    }
    Ok(decode_body(&content_type, &bytes))
}

/// Fill in `model` for servers that route by it.
fn with_model_name(spec: &ModelSpec, body: &Value) -> Value {
    let mut body = body.clone();
    if matches!(spec.flavor, Flavor::Vllm | Flavor::Ollama) {
        if let Some(obj) = body.as_object_mut() {
            obj.entry("model")
                .or_insert_with(|| Value::String(spec.served_name().to_string()));
        }
    }
    body
}

async fn build_multipart(
    ctx: &JobContext,
    spec: &Value,
) -> Result<reqwest::multipart::Form, JobFailure> {
    let file_url = spec
        .get("file_url")
        .and_then(Value::as_str)
        .ok_or_else(|| JobFailure::permanent("multipart.file_url is required"))?;
    // Only remote files: a job submitter must not be able to make this node
    // upload its own local files to a model server.
    if !(file_url.starts_with("https://") || file_url.starts_with("http://")) {
        return Err(JobFailure::permanent("multipart.file_url must be http(s)"));
    }
    let response = ctx
        .http()
        .get(file_url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| JobFailure::retryable(format!("fetching {file_url}: {e}")))?;
    let bytes = response
        .bytes()
        .await
        .map_err(|e| JobFailure::retryable(format!("fetching {file_url}: {e}")))?;
    let file_name = file_url
        .split(['?', '#'])
        .next()
        .and_then(|u| u.rsplit('/').next())
        .filter(|n| !n.is_empty())
        .unwrap_or("input")
        .to_string();
    let field = spec
        .get("file_field")
        .and_then(Value::as_str)
        .unwrap_or("file")
        .to_string();
    let mut form = reqwest::multipart::Form::new().part(
        field,
        reqwest::multipart::Part::bytes(bytes.to_vec()).file_name(file_name),
    );
    if let Some(fields) = spec.get("fields").and_then(Value::as_object) {
        for (name, value) in fields {
            let text = match value {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            form = form.text(name.clone(), text);
        }
    }
    Ok(form)
}

fn decode_body(content_type: &str, bytes: &[u8]) -> Value {
    if content_type.contains("json") || content_type.is_empty() {
        if let Ok(value) = serde_json::from_slice(bytes) {
            return value;
        }
    }
    match std::str::from_utf8(bytes) {
        Ok(text) if content_type.starts_with("text/") || content_type.is_empty() => {
            json!({ "text": text })
        }
        _ => json!({
            "content_type": content_type,
            "base64": base64::engine::general_purpose::STANDARD.encode(bytes),
        }),
    }
}

fn truncate(text: &str, max: usize) -> &str {
    match text.char_indices().nth(max) {
        Some((i, _)) => &text[..i],
        None => text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ModelKind, RuntimeSpec};

    #[test]
    fn model_name_is_filled_for_openai_servers_only() {
        let mut spec = ModelSpec::new(
            "llama",
            ModelKind::Text,
            Flavor::Ollama,
            RuntimeSpec::Attached {
                base_url: "http://x".into(),
            },
        );
        spec.served_name = Some("llama3.1:8b".into());
        let body = with_model_name(&spec, &json!({"messages": []}));
        assert_eq!(body["model"], "llama3.1:8b");
        // An explicit model wins.
        let body = with_model_name(&spec, &json!({"model": "other"}));
        assert_eq!(body["model"], "other");
        spec.flavor = Flavor::Generic;
        assert!(with_model_name(&spec, &json!({})).get("model").is_none());
    }

    #[test]
    fn bodies_decode_by_content_type() {
        assert_eq!(
            decode_body("application/json", b"{\"a\":1}"),
            json!({"a": 1})
        );
        assert_eq!(
            decode_body("text/plain", b"hello"),
            json!({"text": "hello"})
        );
        let audio = decode_body("audio/wav", &[0, 159, 146, 150]);
        assert_eq!(audio["content_type"], "audio/wav");
        assert!(audio["base64"].is_string());
    }

    #[test]
    fn failures_classify() {
        assert!(!JobFailure::from(AiError::BadRequest("x".into())).retryable);
        assert!(JobFailure::from(AiError::Upstream("x".into())).retryable);
    }
}
