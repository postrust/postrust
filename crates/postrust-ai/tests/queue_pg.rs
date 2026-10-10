//! The queue, scheduler and executor against a live PostgreSQL.
//!
//! `#[ignore]`d, like the proxy's database tests, and run by the CI job that
//! has a database:
//!
//!     DATABASE_URL=postgres://postgres:postgres@localhost:5432/postrust_test \
//!         cargo test -p postrust-ai --test queue_pg -- --ignored
//!
//! Every test uses its own queue names, so they run in parallel with each
//! other and with anything else on the same database.

use std::sync::Arc;
use std::time::Duration;

use postrust_ai::config::{CacheConfig, ResourceConfig, SchedulerConfig};
use postrust_ai::executor::Executor;
use postrust_ai::manager::ModelManager;
use postrust_ai::metrics::Metrics;
use postrust_ai::queue::{
    ClaimBands, ClaimRequest, JobStatus, JobStore, NewJob, PauseMode, PauseReason, PgJobStore,
    Priority, QueueSpec, Yield,
};
use postrust_ai::resources::Probe;
use postrust_ai::scheduler::Scheduler;
use postrust_ai::{Flavor, ModelKind, ModelSpec, RuntimeSpec};
use serde_json::{json, Value};
use sqlx::PgPool;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

async fn store() -> (PgPool, PgJobStore) {
    let url = std::env::var("DATABASE_URL")
        .expect("DATABASE_URL must be set for the database-backed tests");
    let pool = PgPool::connect(&url)
        .await
        .expect("could not connect to DATABASE_URL");
    let store = PgJobStore::new(pool.clone());
    store.migrate().await.expect("migration");
    (pool, store)
}

fn unique(prefix: &str) -> String {
    format!("{prefix}-{}", Uuid::new_v4().simple())
}

fn claim(node: &str, queue: &str, bands: ClaimBands) -> ClaimRequest {
    ClaimRequest::new(
        node,
        vec![queue.to_string()],
        bands,
        Duration::from_secs(30),
    )
}

#[tokio::test]
#[ignore]
async fn claims_highest_priority_first_and_keeps_superlow_apart() {
    let (_, store) = store().await;
    let q = unique("prio");
    store.upsert_queue(&QueueSpec::new(&q)).await.unwrap();

    let low = store
        .enqueue(NewJob::new(&q, "m", json!({})).with_priority(Priority::Low))
        .await
        .unwrap();
    let superlow = store
        .enqueue(NewJob::new(&q, "m", json!({})).with_priority(Priority::Superlow))
        .await
        .unwrap();
    let high = store
        .enqueue(NewJob::new(&q, "m", json!({})).with_priority(Priority::High))
        .await
        .unwrap();

    let a = store
        .claim(&claim("n", &q, ClaimBands::Regular))
        .await
        .unwrap()
        .unwrap();
    let b = store
        .claim(&claim("n", &q, ClaimBands::Regular))
        .await
        .unwrap()
        .unwrap();
    assert_eq!((a.id, b.id), (high.id, low.id));
    assert!(
        store
            .claim(&claim("n", &q, ClaimBands::Regular))
            .await
            .unwrap()
            .is_none(),
        "regular never sees superlow"
    );

    let c = store
        .claim(&claim("n", &q, ClaimBands::SuperlowOnly))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(c.id, superlow.id);
    assert_eq!(c.status, JobStatus::Running);
    assert_eq!(c.attempts, 1);
}

#[tokio::test]
#[ignore]
async fn idempotency_key_returns_the_first_job() {
    let (_, store) = store().await;
    let q = unique("idem");
    store.upsert_queue(&QueueSpec::new(&q)).await.unwrap();
    let key = unique("key");
    let a = store
        .enqueue(NewJob::new(&q, "m", json!({"x": 1})).with_idempotency_key(&key))
        .await
        .unwrap();
    let b = store
        .enqueue(NewJob::new(&q, "m", json!({"x": 2})).with_idempotency_key(&key))
        .await
        .unwrap();
    assert_eq!(a.id, b.id);
    assert_eq!(b.input, json!({"x": 1}));
}

#[tokio::test]
#[ignore]
async fn yield_keeps_or_discards_progress_and_is_not_an_attempt() {
    let (_, store) = store().await;
    let q = unique("yield");
    store.upsert_queue(&QueueSpec::new(&q)).await.unwrap();
    let job = store
        .enqueue(NewJob::new(&q, "m", json!({})))
        .await
        .unwrap();

    let running = store
        .claim(&claim("n", &q, ClaimBands::Regular))
        .await
        .unwrap()
        .unwrap();
    store
        .save_checkpoint(running.id, "n", &json!({"next": 3}))
        .await
        .unwrap();
    assert!(store
        .yield_job(job.id, "n", Yield::keep(PauseReason::Preempted, None))
        .await
        .unwrap());

    let paused = store.get(job.id).await.unwrap().unwrap();
    assert_eq!(paused.status, JobStatus::Paused);
    assert_eq!(paused.pause_reason, Some(PauseReason::Preempted));
    assert_eq!(
        paused.checkpoint,
        Some(json!({"next": 3})),
        "None keeps the saved checkpoint"
    );
    assert_eq!(paused.attempts, 0, "a pause is not an attempt");
    assert_eq!(paused.preemptions, 1);

    // Preempted jobs are claimable again by themselves.
    let again = store
        .claim(&claim("n", &q, ClaimBands::Regular))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(again.id, job.id);
    assert_eq!(again.checkpoint, Some(json!({"next": 3})));
    assert!(store
        .yield_job(job.id, "n", Yield::discard(PauseReason::Manual))
        .await
        .unwrap());
    let manual = store.get(job.id).await.unwrap().unwrap();
    assert_eq!(manual.checkpoint, None, "requeue mode discards progress");
    // Manual pauses wait for a resume.
    assert!(store
        .claim(&claim("n", &q, ClaimBands::Regular))
        .await
        .unwrap()
        .is_none());
    store.resume(job.id).await.unwrap();
    assert!(store
        .claim(&claim("n", &q, ClaimBands::Regular))
        .await
        .unwrap()
        .is_some());
}

#[tokio::test]
#[ignore]
async fn writes_from_a_node_that_lost_the_job_are_refused() {
    let (_, store) = store().await;
    let q = unique("lost");
    store.upsert_queue(&QueueSpec::new(&q)).await.unwrap();
    let job = store
        .enqueue(NewJob::new(&q, "m", json!({})))
        .await
        .unwrap();
    store
        .claim(&claim("a", &q, ClaimBands::Regular))
        .await
        .unwrap()
        .unwrap();
    assert!(!store
        .complete(job.id, "b", &json!({}), false)
        .await
        .unwrap());
    assert_eq!(
        store
            .heartbeat(job.id, "b", Duration::from_secs(30))
            .await
            .unwrap(),
        postrust_ai::queue::Heartbeat::Lost
    );
}

#[tokio::test]
#[ignore]
async fn failures_retry_with_backoff_then_die() {
    let (_, store) = store().await;
    let q = unique("retry");
    let mut spec = QueueSpec::new(&q);
    spec.max_attempts = 2;
    spec.retry_backoff_secs = 0;
    store.upsert_queue(&spec).await.unwrap();
    let job = store
        .enqueue(NewJob::new(&q, "m", json!({})))
        .await
        .unwrap();

    store
        .claim(&claim("n", &q, ClaimBands::Regular))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        store.fail(job.id, "n", "boom", true).await.unwrap(),
        Some(JobStatus::Queued)
    );
    store
        .claim(&claim("n", &q, ClaimBands::Regular))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        store.fail(job.id, "n", "boom", true).await.unwrap(),
        Some(JobStatus::Dead)
    );

    let other = store
        .enqueue(NewJob::new(&q, "m", json!({})))
        .await
        .unwrap();
    store
        .claim(&claim("n", &q, ClaimBands::Regular))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        store.fail(other.id, "n", "bad input", false).await.unwrap(),
        Some(JobStatus::Failed)
    );
}

#[tokio::test]
#[ignore]
async fn expired_leases_are_reaped() {
    let (pool, store) = store().await;
    let q = unique("reap");
    store.upsert_queue(&QueueSpec::new(&q)).await.unwrap();
    let job = store
        .enqueue(NewJob::new(&q, "m", json!({})))
        .await
        .unwrap();
    store
        .claim(&claim("n", &q, ClaimBands::Regular))
        .await
        .unwrap()
        .unwrap();
    sqlx::query("UPDATE ai.jobs SET lease_expires_at = now() - interval '1 second' WHERE id = $1")
        .bind(job.id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(store.reap_expired().await.unwrap().contains(&job.id));
    assert_eq!(
        store.get(job.id).await.unwrap().unwrap().status,
        JobStatus::Queued
    );
}

// ============================================================================
// End to end: scheduler + executor + manager on a stub model server
// ============================================================================

/// A model server that takes `delay` per request and counts them by `n`.
async fn slow_server(delay: Duration, seen: Arc<parking_lot::Mutex<Vec<i64>>>) -> String {
    let app = axum::Router::new()
        .route("/health", axum::routing::get(|| async { "ok" }))
        .route(
            "/run",
            axum::routing::post(move |axum::Json(body): axum::Json<Value>| {
                let seen = seen.clone();
                async move {
                    tokio::time::sleep(delay).await;
                    let n = body["n"].as_i64().unwrap_or(-1);
                    seen.lock().push(n);
                    axum::Json(json!({ "n": n }))
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

struct Harness {
    store: Arc<dyn JobStore>,
    shutdown: CancellationToken,
}

async fn harness(queue: QueueSpec, model: ModelSpec) -> Harness {
    let (_, pg) = store().await;
    let store: Arc<dyn JobStore> = Arc::new(pg);
    store.upsert_queue(&queue).await.unwrap();

    let mut resources = ResourceConfig::default();
    resources.memory_mb = Some(1000);
    resources.gpus = Some(vec![]);
    resources.respect_probe = false;
    let mut cache = CacheConfig::default();
    cache.dir = std::env::temp_dir().join(unique("postrust-ai-e2e"));
    let files = Arc::new(
        postrust_ai::cache::FileCache::open(&cache, reqwest::Client::new())
            .await
            .unwrap(),
    );
    let node = unique("node");
    let manager = ModelManager::new(
        node.clone(),
        &[model],
        resources,
        Probe::default(),
        reqwest::Client::new(),
        files,
        None,
    );
    let metrics = Arc::new(Metrics::default());
    let executor = Arc::new(Executor::new(
        node.clone(),
        store.clone(),
        manager.clone(),
        None,
        reqwest::Client::new(),
        Duration::from_secs(30),
        Duration::from_secs(1),
        metrics,
    ));
    let mut config = SchedulerConfig::default();
    config.poll_secs = 1;
    config.lease_secs = 30;
    config.heartbeat_secs = 1;
    let scheduler = Arc::new(Scheduler::new(
        node,
        store.clone(),
        manager,
        executor,
        None,
        vec![queue],
        config,
    ));
    let shutdown = CancellationToken::new();
    tokio::spawn(scheduler.run(shutdown.clone()));
    Harness { store, shutdown }
}

async fn wait_for(
    store: &Arc<dyn JobStore>,
    id: Uuid,
    want: JobStatus,
    within: Duration,
) -> postrust_ai::queue::Job {
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

fn stub_model(id: &str, url: &str) -> ModelSpec {
    let mut spec = ModelSpec::new(
        id,
        ModelKind::Text,
        Flavor::Generic,
        RuntimeSpec::Attached {
            base_url: url.into(),
        },
    );
    spec.memory_mb = 100;
    spec.max_concurrency = 1;
    spec.load_timeout_secs = 5;
    spec
}

fn chunked(n: usize) -> Value {
    json!({ "requests": (0..n).map(|i| json!({ "path": "/run", "body": { "n": i } })).collect::<Vec<_>>() })
}

#[tokio::test]
#[ignore]
async fn superlow_job_pauses_for_high_work_and_resumes_from_its_checkpoint() {
    let seen = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let url = slow_server(Duration::from_millis(300), seen.clone()).await;
    let model_id = unique("model");
    let q = unique("e2e");
    let mut queue = QueueSpec::new(&q);
    queue.concurrency = 2;
    queue.pause_mode = PauseMode::Checkpoint;
    queue.pause_grace_secs = 5;
    let h = harness(queue, stub_model(&model_id, &url)).await;

    // Ten chunks at 300 ms: three seconds of background work on the only slot
    // of the model.
    let background = h
        .store
        .enqueue(NewJob::new(&q, &model_id, chunked(10)).with_priority(Priority::Superlow))
        .await
        .unwrap();
    wait_for(
        &h.store,
        background.id,
        JobStatus::Running,
        Duration::from_secs(5),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(700)).await;

    let urgent = h
        .store
        .enqueue(
            NewJob::new(
                &q,
                &model_id,
                json!({ "request": { "path": "/run", "body": { "n": 100 } } }),
            )
            .with_priority(Priority::High),
        )
        .await
        .unwrap();

    let done = wait_for(
        &h.store,
        urgent.id,
        JobStatus::Completed,
        Duration::from_secs(10),
    )
    .await;
    assert_eq!(done.output, Some(json!({ "n": 100 })));

    let finished = wait_for(
        &h.store,
        background.id,
        JobStatus::Completed,
        Duration::from_secs(20),
    )
    .await;
    assert!(
        finished.preemptions >= 1,
        "the superlow job was preempted: {finished:?}"
    );
    let results = finished.output.unwrap()["results"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(results.len(), 10);
    for (i, r) in results.iter().enumerate() {
        assert_eq!(r["n"], json!(i));
    }

    // The checkpoint held: at most the one in-flight chunk ran twice.
    let calls = seen.lock().clone();
    let background_calls = calls.iter().filter(|n| **n < 100).count();
    assert!(background_calls <= 11, "chunks were repeated: {calls:?}");
    // And the urgent request ran before the background job finished.
    let urgent_at = calls.iter().position(|n| *n == 100).unwrap();
    assert!(urgent_at < calls.len() - 1, "{calls:?}");
    h.shutdown.cancel();
}

#[tokio::test]
#[ignore]
async fn requeue_mode_starts_over() {
    let seen = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let url = slow_server(Duration::from_millis(200), seen.clone()).await;
    let model_id = unique("model");
    let q = unique("e2e-requeue");
    let mut queue = QueueSpec::new(&q);
    queue.concurrency = 2;
    queue.pause_mode = PauseMode::Requeue;
    let h = harness(queue, stub_model(&model_id, &url)).await;

    let background = h
        .store
        .enqueue(NewJob::new(&q, &model_id, chunked(6)).with_priority(Priority::Superlow))
        .await
        .unwrap();
    wait_for(
        &h.store,
        background.id,
        JobStatus::Running,
        Duration::from_secs(5),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let urgent = h
        .store
        .enqueue(
            NewJob::new(
                &q,
                &model_id,
                json!({ "request": { "path": "/run", "body": { "n": 100 } } }),
            )
            .with_priority(Priority::High),
        )
        .await
        .unwrap();
    wait_for(
        &h.store,
        urgent.id,
        JobStatus::Completed,
        Duration::from_secs(10),
    )
    .await;
    let finished = wait_for(
        &h.store,
        background.id,
        JobStatus::Completed,
        Duration::from_secs(20),
    )
    .await;
    assert!(finished.preemptions >= 1);
    // Chunk 0 ran at least twice: progress was thrown away.
    let zeros = seen.lock().iter().filter(|n| **n == 0).count();
    assert!(zeros >= 2, "{:?}", seen.lock());
    h.shutdown.cancel();
}

#[tokio::test]
#[ignore]
async fn manual_pause_and_cancel_reach_a_running_job() {
    let seen = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let url = slow_server(Duration::from_millis(200), seen.clone()).await;
    let model_id = unique("model");
    let q = unique("e2e-manual");
    let h = harness(QueueSpec::new(&q), stub_model(&model_id, &url)).await;

    let job = h
        .store
        .enqueue(NewJob::new(&q, &model_id, chunked(30)))
        .await
        .unwrap();
    wait_for(&h.store, job.id, JobStatus::Running, Duration::from_secs(5)).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    h.store.request_pause(job.id).await.unwrap();
    let paused = wait_for(&h.store, job.id, JobStatus::Paused, Duration::from_secs(5)).await;
    assert_eq!(paused.pause_reason, Some(PauseReason::Manual));
    let next = paused
        .checkpoint
        .as_ref()
        .and_then(|c| c["next"].as_u64())
        .unwrap_or(0);
    assert!(next >= 1, "progress was kept: {paused:?}");

    h.store.resume(job.id).await.unwrap();
    wait_for(&h.store, job.id, JobStatus::Running, Duration::from_secs(5)).await;
    h.store.request_cancel(job.id).await.unwrap();
    wait_for(
        &h.store,
        job.id,
        JobStatus::Cancelled,
        Duration::from_secs(5),
    )
    .await;
    h.shutdown.cancel();
}
