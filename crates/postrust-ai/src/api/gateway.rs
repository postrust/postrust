//! Synchronous inference: `ANY /v1/infer/{model}/{*path}` is forwarded to
//! `{instance}/{path}`, streaming both ways.
//!
//! Where it goes:
//!
//! 1. With a session header and a live lock on another node: forwarded to
//!    that node (once -- a forwarded request is always served where it lands).
//! 2. With a lock on this node: its instance, waiting for a slot on it. If the
//!    instance is gone, a new one is bound and `x-session-rebound: true` set.
//! 3. With a session header and no lock: the least-locked instance with room
//!    for another session, and a new lock on it.
//! 4. Without a session: a local ready instance; else a peer with the model
//!    ready; else a lazy load here.
//!
//! A load that takes longer than `gateway.load_wait_secs` answers 503 with
//! `Retry-After`; the load carries on, so the retry finds it ready.
//!
//! The request's priority comes from `x-priority` (default `high`), and
//! decides what the request may unload to make room.

use std::time::Duration;

use axum::body::Body;
use axum::extract::{Path, Request, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue};
use axum::response::Response;
use futures::StreamExt;

use super::{ApiError, ApiResult, AppState};
use crate::error::AiError;
use crate::manager::{AcquireOptions, Lease};
use crate::queue::Priority;
use crate::session::SessionLock;

/// Set on requests one node forwards to another; such requests are never
/// forwarded again.
pub const FORWARDED: &str = "x-postrust-ai-forwarded";

/// Hop-by-hop headers, which belong to one connection and are not forwarded.
const HOP_BY_HOP: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    "host",
    "content-length",
];

pub(super) async fn infer(
    State(state): State<AppState>,
    Path((model, path)): Path<(String, String)>,
    request: Request,
) -> ApiResult<Response> {
    state.metrics.gateway_request();
    state.manager.spec(&model)?;
    let headers = request.headers().clone();
    let forwarded = headers.contains_key(FORWARDED);
    let priority = headers
        .get("x-priority")
        .and_then(|v| v.to_str().ok())
        .map(str::parse::<Priority>)
        .transpose()?
        .unwrap_or(Priority::High);
    let session = headers
        .get(state.config.sessions.header.as_str())
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.is_empty() && s.len() <= 256)
        .map(String::from);
    let wait = Duration::from_secs(state.config.gateway.load_wait_secs);
    let query = request
        .uri()
        .query()
        .map(|q| format!("?{q}"))
        .unwrap_or_default();

    let mut rebound = false;
    let mut lock: Option<SessionLock> = None;

    let lease = if let Some(session_id) = &session {
        let existing = state.sessions.lookup(session_id, &model).await?;
        match existing {
            Some(held) if held.node_id != state.sessions.node_id() => {
                if !forwarded {
                    if let Some(url) = state.nodes.url_of(&held.node_id).await? {
                        state.metrics.gateway_forwarded();
                        return forward_to_peer(&state, &url, &model, &path, &query, request).await;
                    }
                }
                // The owner is down (or this is already a forward): take the
                // session over here.
                rebound = true;
                let lease =
                    acquire(&state, &model, new_session_opts(&state, priority, wait)).await?;
                lock =
                    Some(bind(&state, session_id, &model, &lease, Some(held.instance_id)).await?);
                lease
            }
            Some(held) => {
                let mut opts = AcquireOptions::new(priority, wait);
                opts.instance = Some(held.instance_id);
                match state.manager.acquire(&model, opts).await {
                    Ok(lease) => {
                        lock = Some(held);
                        lease
                    }
                    Err(AiError::NotFound(_)) => {
                        // The instance is gone: crashed, or unloaded for
                        // critical work. Bind anew and say so.
                        rebound = true;
                        state.metrics.session_rebind();
                        let lease =
                            acquire(&state, &model, new_session_opts(&state, priority, wait))
                                .await?;
                        lock = Some(
                            bind(&state, session_id, &model, &lease, Some(held.instance_id))
                                .await?,
                        );
                        lease
                    }
                    Err(error) => return Err(unavailable(&state, error)),
                }
            }
            None => {
                let lease =
                    acquire(&state, &model, new_session_opts(&state, priority, wait)).await?;
                let won = bind(&state, session_id, &model, &lease, None).await?;
                if won.node_id != state.sessions.node_id() {
                    // Another node bound it first; that node serves it.
                    drop(lease);
                    if !forwarded {
                        if let Some(url) = state.nodes.url_of(&won.node_id).await? {
                            state.metrics.gateway_forwarded();
                            return forward_to_peer(&state, &url, &model, &path, &query, request)
                                .await;
                        }
                    }
                    return Err(AiError::Conflict("session is held by another node".into()).into());
                }
                if won.instance_id != lease.instance_id() {
                    // Same node, raced to a different instance: use that one.
                    drop(lease);
                    let mut opts = AcquireOptions::new(priority, wait);
                    opts.instance = Some(won.instance_id);
                    let lease = state
                        .manager
                        .acquire(&model, opts)
                        .await
                        .map_err(|e| unavailable(&state, e))?;
                    lock = Some(won);
                    lease
                } else {
                    lock = Some(won);
                    lease
                }
            }
        }
    } else {
        let ready_here = state.manager.ready_models().contains(&model);
        if !ready_here && !forwarded && state.config.gateway.forward_to_ready_nodes {
            if let Some((_, url)) = state.nodes.peers_with(&model).await?.into_iter().next() {
                state.metrics.gateway_forwarded();
                return forward_to_peer(&state, &url, &model, &path, &query, request).await;
            }
        }
        if !ready_here {
            state.metrics.gateway_load_wait();
        }
        acquire(&state, &model, AcquireOptions::new(priority, wait)).await?
    };

    let target = format!("{}/{}{}", lease.base_url(), path, query);
    let timeout = Duration::from_secs(state.config.gateway.request_timeout_secs);
    let mut response = send(&state, &target, request, timeout, lease).await?;

    let h = response.headers_mut();
    if let Ok(v) = HeaderValue::from_str(&state.config.node.id) {
        h.insert("x-postrust-ai-node", v);
    }
    if let Some(lock) = &lock {
        if let Ok(v) = HeaderValue::from_str(&lock.instance_id.to_string()) {
            h.insert("x-postrust-ai-instance", v);
        }
        if rebound {
            h.insert("x-session-rebound", HeaderValue::from_static("true"));
        }
        if let Err(error) = state.sessions.touch(lock).await {
            tracing::warn!(%error, "could not extend session lock");
        }
    }
    Ok(response)
}

fn new_session_opts(state: &AppState, priority: Priority, wait: Duration) -> AcquireOptions {
    let mut opts = AcquireOptions::new(priority, wait);
    opts.for_new_session = true;
    opts.max_sessions = Some(state.config.sessions.max_per_instance);
    opts
}

async fn acquire(state: &AppState, model: &str, opts: AcquireOptions) -> ApiResult<Lease> {
    state
        .manager
        .acquire(model, opts)
        .await
        .map_err(|e| unavailable(state, e))
}

fn unavailable(state: &AppState, error: AiError) -> ApiError {
    if matches!(
        error,
        AiError::LoadTimeout { .. } | AiError::InsufficientResources { .. }
    ) {
        state.metrics.gateway_unavailable();
    }
    error.into()
}

async fn bind(
    state: &AppState,
    session: &str,
    model: &str,
    lease: &Lease,
    replacing: Option<uuid::Uuid>,
) -> ApiResult<SessionLock> {
    Ok(state
        .sessions
        .bind(session, model, lease.instance_id(), replacing)
        .await?)
}

async fn forward_to_peer(
    state: &AppState,
    base: &str,
    model: &str,
    path: &str,
    query: &str,
    mut request: Request,
) -> ApiResult<Response> {
    request.headers_mut().insert(
        HeaderName::from_static(FORWARDED),
        HeaderValue::from_static("1"),
    );
    let target = format!(
        "{}/v1/infer/{model}/{path}{query}",
        base.trim_end_matches('/')
    );
    let timeout = Duration::from_secs(state.config.gateway.request_timeout_secs);
    send(state, &target, request, timeout, ()).await
}

/// Forward `request` to `target`, streaming. `guard` is held until the
/// response body has been fully sent: for a lease, that is what keeps the
/// instance counted as busy for the whole stream.
async fn send<G: Send + 'static>(
    state: &AppState,
    target: &str,
    request: Request,
    timeout: Duration,
    guard: G,
) -> ApiResult<Response> {
    let method = reqwest::Method::from_bytes(request.method().as_str().as_bytes())
        .map_err(|_| AiError::BadRequest("unsupported method".into()))?;
    let headers = filter_headers(request.headers());
    let body = reqwest::Body::wrap_stream(request.into_body().into_data_stream());

    let upstream = state
        .forward_http
        .request(method, target)
        .headers(headers)
        .body(body)
        .timeout(timeout)
        .send()
        .await
        .map_err(|e| AiError::Upstream(format!("{target}: {e}")))?;

    let mut builder = Response::builder().status(upstream.status().as_u16());
    for (name, value) in upstream.headers() {
        if !HOP_BY_HOP.contains(&name.as_str()) {
            builder = builder.header(name.as_str(), value.as_bytes());
        }
    }
    let stream = upstream.bytes_stream().map(move |chunk| {
        let _held = &guard;
        chunk.map_err(std::io::Error::other)
    });
    builder
        .body(Body::from_stream(stream))
        .map_err(|e| AiError::Upstream(e.to_string()).into())
}

fn filter_headers(headers: &HeaderMap) -> reqwest::header::HeaderMap {
    let mut out = reqwest::header::HeaderMap::new();
    for (name, value) in headers {
        if HOP_BY_HOP.contains(&name.as_str()) {
            continue;
        }
        if let (Ok(n), Ok(v)) = (
            reqwest::header::HeaderName::from_bytes(name.as_str().as_bytes()),
            reqwest::header::HeaderValue::from_bytes(value.as_bytes()),
        ) {
            out.append(n, v);
        }
    }
    out
}
