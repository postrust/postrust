//! Helpers shared by the database-backed integration tests: a pool, unique
//! names, a stub model server, and a node in miniature (a scheduler, executor
//! and manager over one stub model). `queue_pg.rs` predates this module and
//! keeps its own copies.
//!
//! Every test namespaces its rows with [`unique`], so they run in parallel
//! with each other and never assume an empty database.

#![allow(dead_code)]

use std::sync::Arc;
use std::time::Duration;

use postrust_ai::config::{CacheConfig, ResourceConfig, SchedulerConfig};
use postrust_ai::executor::Executor;
use postrust_ai::manager::ModelManager;
use postrust_ai::metrics::Metrics;
use postrust_ai::queue::{Job, JobStatus, JobStore, PgJobStore, QueueSpec};
use postrust_ai::resources::Probe;
use postrust_ai::scheduler::Scheduler;
use postrust_ai::workflow::WorkflowEngine;
use postrust_ai::{Flavor, ModelKind, ModelSpec, RuntimeSpec};
use serde_json::{json, Value};
use sqlx::PgPool;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// A migrated pool and job store on `DATABASE_URL`.
pub async fn store() -> (PgPool, PgJobStore) {
    let url = std::env::var("DATABASE_URL")
        .expect("DATABASE_URL must be set for the database-backed tests");
    let pool = PgPool::connect(&url)
        .await
        .expect("could not connect to DATABASE_URL");
    let store = PgJobStore::new(pool.clone());
    store.migrate().await.expect("migration");
    (pool, store)
}

/// A name no other test uses.
pub fn unique(prefix: &str) -> String {
    format!("{prefix}-{}", Uuid::new_v4().simple())
}

/// A model server. `/echo` returns its body after `body.delay_ms` (if any),
/// `/run` returns `{"n": body.n}` after `delay`, and `/bad` answers 400, which
/// the executor treats as a permanent failure.
pub async fn stub_server(delay: Duration) -> String {
    let app = axum::Router::new()
        .route("/health", axum::routing::get(|| async { "ok" }))
        .route(
            "/echo",
            axum::routing::post(|axum::Json(body): axum::Json<Value>| async move {
                if let Some(ms) = body["delay_ms"].as_u64() {
                    tokio::time::sleep(Duration::from_millis(ms)).await;
                }
                axum::Json(body)
            }),
        )
        .route(
            "/run",
            axum::routing::post(move |axum::Json(body): axum::Json<Value>| async move {
                tokio::time::sleep(delay).await;
                axum::Json(json!({ "n": body["n"].as_i64().unwrap_or(-1) }))
            }),
        )
        .route(
            "/bad",
            axum::routing::post(|| async { (axum::http::StatusCode::BAD_REQUEST, "no") }),
        );
    serve(app).await
}

/// Serve `app` on an ephemeral port; returns its base URL.
pub async fn serve(app: axum::Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

/// An attached model on a stub server.
pub fn stub_model(id: &str, url: &str, max_concurrency: usize) -> ModelSpec {
    let mut spec = ModelSpec::new(
        id,
        ModelKind::Text,
        Flavor::Generic,
        RuntimeSpec::Attached {
            base_url: url.into(),
        },
    );
    spec.memory_mb = 100;
    spec.max_concurrency = max_concurrency;
    spec.load_timeout_secs = 5;
    spec
}

/// A manager over `models` with a generous memory budget and no GPUs.
pub async fn manager(node: &str, models: &[ModelSpec]) -> ModelManager {
    manager_with(node, models, None, |_| {}).await
}

/// [`manager`], recording model events to `events` and with `tune` applied
/// to the resource config (GPUs, a tighter memory budget).
pub async fn manager_with(
    node: &str,
    models: &[ModelSpec],
    events: Option<PgPool>,
    tune: impl FnOnce(&mut ResourceConfig),
) -> ModelManager {
    let mut resources = ResourceConfig::default();
    resources.memory_mb = Some(10_000);
    resources.gpus = Some(vec![]);
    resources.respect_probe = false;
    // Managers in one test binary run side by side and would both pick the
    // first free port of the default range before either child binds it.
    static NEXT_RANGE: std::sync::atomic::AtomicU16 = std::sync::atomic::AtomicU16::new(0);
    let low = 20_000 + NEXT_RANGE.fetch_add(1, std::sync::atomic::Ordering::Relaxed) * 50;
    resources.port_range = (low, low + 49);
    tune(&mut resources);
    let mut cache = CacheConfig::default();
    cache.dir = std::env::temp_dir().join(unique("postrust-ai-e2e"));
    let files = Arc::new(
        postrust_ai::cache::FileCache::open(&cache, reqwest::Client::new())
            .await
            .unwrap(),
    );
    ModelManager::new(
        node.to_string(),
        models,
        resources,
        Probe::default(),
        reqwest::Client::new(),
        files,
        events,
    )
}

/// A scheduler running over a stub model, with a workflow engine attached.
pub struct Harness {
    pub pool: PgPool,
    pub store: Arc<dyn JobStore>,
    pub engine: WorkflowEngine,
    pub shutdown: CancellationToken,
}

pub async fn harness(queues: Vec<QueueSpec>, models: Vec<ModelSpec>) -> Harness {
    let (pool, pg) = store().await;
    let store: Arc<dyn JobStore> = Arc::new(pg);
    for queue in &queues {
        store.upsert_queue(queue).await.unwrap();
    }

    let node = unique("node");
    let known = models.iter().map(|m| m.id.clone()).collect();
    let manager = manager(&node, &models).await;
    let executor = Arc::new(Executor::new(
        node.clone(),
        store.clone(),
        manager.clone(),
        None,
        reqwest::Client::new(),
        Duration::from_secs(30),
        Duration::from_secs(1),
        Arc::new(Metrics::default()),
    ));
    let engine = WorkflowEngine::new(pool.clone(), store.clone(), known);
    let mut config = SchedulerConfig::default();
    config.poll_secs = 1;
    config.lease_secs = 30;
    config.heartbeat_secs = 1;
    let scheduler = Arc::new(Scheduler::new(
        node,
        store.clone(),
        manager,
        executor,
        Some(engine.clone()),
        queues,
        config,
    ));
    let shutdown = CancellationToken::new();
    tokio::spawn(scheduler.run(shutdown.clone()));
    Harness {
        pool,
        store,
        engine,
        shutdown,
    }
}

/// Poll a job until it has `want` status.
pub async fn wait_for(
    store: &Arc<dyn JobStore>,
    id: Uuid,
    want: JobStatus,
    within: Duration,
) -> Job {
    let deadline = tokio::time::Instant::now() + within;
    loop {
        let job = store.get(id).await.unwrap().unwrap();
        if job.status == want {
            return job;
        }
        if tokio::time::Instant::now() > deadline {
            panic!("job {id} is {:?}, wanted {want:?}: {job:?}", job.status);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Bind a listener first, so its address can go into a config before the
/// server that uses it exists.
pub async fn reserve() -> (tokio::net::TcpListener, String) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    (listener, url)
}
