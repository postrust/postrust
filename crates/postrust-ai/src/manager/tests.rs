//! Manager behaviour against stub model servers.
//!
//! The stubs are attached generic servers (an axum app on a random port), so
//! loading is a health check and stopping is bookkeeping. That is enough to
//! exercise the decisions -- what loads, what is evicted, what is preempted --
//! without a real model.

use super::*;
use crate::config::{CacheConfig, GpuConfig};
use crate::model::{Flavor, ModelKind, RuntimeSpec};

async fn stub_server() -> String {
    let app = axum::Router::new().route("/health", axum::routing::get(|| async { "ok" }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

fn model(id: &str, base_url: &str, mem_mb: u64, gpu_mb: u64) -> ModelSpec {
    let mut spec = ModelSpec::new(
        id,
        ModelKind::Text,
        Flavor::Generic,
        RuntimeSpec::Attached {
            base_url: base_url.into(),
        },
    );
    spec.memory_mb = mem_mb;
    spec.gpu_mb = gpu_mb;
    spec.load_timeout_secs = 5;
    spec
}

async fn manager(models: &[ModelSpec], mem_mb: u64, gpus: &[u64]) -> ModelManager {
    let resources = ResourceConfig {
        memory_mb: Some(mem_mb),
        gpus: Some(
            gpus.iter()
                .enumerate()
                .map(|(i, mb)| GpuConfig::new(i as u32, *mb))
                .collect(),
        ),
        respect_probe: false,
        ..Default::default()
    };
    let cache = CacheConfig {
        dir: std::env::temp_dir().join(format!("postrust-ai-test-{}", Uuid::new_v4())),
        ..Default::default()
    };
    let files = Arc::new(
        FileCache::open(&cache, reqwest::Client::new())
            .await
            .unwrap(),
    );
    ModelManager::new(
        "test".into(),
        models,
        resources,
        Probe::default(),
        reqwest::Client::new(),
        files,
        None,
    )
}

fn opts(priority: Priority) -> AcquireOptions {
    AcquireOptions::new(priority, Duration::from_secs(5))
}

#[tokio::test]
async fn loads_lazily_and_reuses_the_instance() {
    let url = stub_server().await;
    let m = manager(&[model("a", &url, 100, 0)], 1000, &[]).await;
    assert!(
        m.instances().is_empty(),
        "nothing loads before it is asked for"
    );

    let first = m.acquire("a", opts(Priority::Normal)).await.unwrap();
    let id = first.instance_id();
    drop(first);
    let second = m.acquire("a", opts(Priority::Normal)).await.unwrap();
    assert_eq!(second.instance_id(), id);
    assert_eq!(m.instances().len(), 1);
}

#[tokio::test]
async fn evicts_an_idle_model_to_make_room() {
    let url = stub_server().await;
    let m = manager(
        &[model("a", &url, 600, 0), model("b", &url, 600, 0)],
        1000,
        &[],
    )
    .await;

    drop(m.acquire("a", opts(Priority::Normal)).await.unwrap());
    let b = m.acquire("b", opts(Priority::Normal)).await.unwrap();
    assert_eq!(b.spec().id, "b");
    let models: Vec<String> = m.instances().iter().map(|i| i.model_id.clone()).collect();
    assert_eq!(models, vec!["b".to_string()]);
}

#[tokio::test]
async fn never_evicts_a_busy_or_pinned_model() {
    let url = stub_server().await;
    let mut pinned = model("p", &url, 600, 0);
    pinned.pinned = true;
    let m = manager(&[pinned, model("b", &url, 600, 0)], 1000, &[]).await;

    drop(m.acquire("p", opts(Priority::Normal)).await.unwrap());
    let mut o = opts(Priority::Critical);
    o.wait = Duration::from_millis(500);
    let error = m.acquire("b", o).await.unwrap_err();
    assert!(matches!(error, AiError::LoadTimeout { .. }), "{error}");
}

#[tokio::test]
async fn superlow_never_evicts() {
    let url = stub_server().await;
    let m = manager(
        &[model("a", &url, 600, 0), model("b", &url, 600, 0)],
        1000,
        &[],
    )
    .await;

    drop(m.acquire("a", opts(Priority::Normal)).await.unwrap());
    assert!(!m.has_spare_capacity("b"));
    let mut o = opts(Priority::Superlow);
    o.wait = Duration::from_millis(500);
    assert!(m.acquire("b", o).await.is_err());
    assert_eq!(m.instances()[0].model_id, "a", "a was left alone");
}

#[tokio::test]
async fn preempts_superlow_holders_for_resources() {
    let url = stub_server().await;
    let m = manager(
        &[model("a", &url, 600, 0), model("b", &url, 600, 0)],
        1000,
        &[],
    )
    .await;

    let mut o = opts(Priority::Superlow);
    o.job_id = Some(Uuid::new_v4());
    let background = m.acquire("a", o).await.unwrap();
    let preempted = background.preempted().clone();

    // A normal request for b cannot fit while a is busy...
    let m2 = m.clone();
    let wanted = tokio::spawn(async move { m2.acquire("b", opts(Priority::Normal)).await });
    // ...so the superlow holder is asked to give a up,
    tokio::time::timeout(Duration::from_secs(3), preempted.cancelled())
        .await
        .expect("superlow lease was preempted");
    // and once it does, b loads.
    drop(background);
    let lease = wanted.await.unwrap().unwrap();
    assert_eq!(lease.spec().id, "b");
}

#[tokio::test]
async fn preempts_a_superlow_slot_at_the_replica_limit() {
    let url = stub_server().await;
    let m = manager(&[model("a", &url, 100, 0)], 1000, &[]).await;

    let background = m.acquire("a", opts(Priority::Superlow)).await.unwrap();
    let preempted = background.preempted().clone();
    let m2 = m.clone();
    let wanted = tokio::spawn(async move { m2.acquire("a", opts(Priority::High)).await });
    tokio::time::timeout(Duration::from_secs(3), preempted.cancelled())
        .await
        .expect("superlow lease was preempted for its slot");
    drop(background);
    assert!(wanted.await.unwrap().is_ok());
}

#[tokio::test]
async fn sessions_protect_an_instance_from_normal_eviction() {
    let url = stub_server().await;
    let m = manager(
        &[model("a", &url, 600, 0), model("b", &url, 600, 0)],
        1000,
        &[],
    )
    .await;

    let lease = m.acquire("a", opts(Priority::Normal)).await.unwrap();
    assert!(m.bind_session(lease.instance_id()));
    drop(lease);

    let mut o = opts(Priority::High);
    o.wait = Duration::from_millis(500);
    assert!(
        m.acquire("b", o).await.is_err(),
        "high may not break a session lock"
    );
    // Critical may.
    assert!(m.acquire("b", opts(Priority::Critical)).await.is_ok());
}

#[tokio::test]
async fn a_model_bigger_than_the_node_fails_at_once() {
    let url = stub_server().await;
    let m = manager(&[model("huge", &url, 10, 80_000)], 1000, &[24_000]).await;
    let started = Instant::now();
    let error = m
        .acquire("huge", opts(Priority::Critical))
        .await
        .unwrap_err();
    assert!(
        matches!(error, AiError::InsufficientResources { .. }),
        "{error}"
    );
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[tokio::test]
async fn a_failed_load_fails_waiters_fast() {
    // Nothing listens on port 9: the health check never passes, and the
    // attached "server" never exits, so this is the timeout path.
    let mut spec = model("dead", "http://127.0.0.1:9", 10, 0);
    spec.load_timeout_secs = 1;
    let m = manager(&[spec], 1000, &[]).await;
    let error = m.acquire("dead", opts(Priority::Normal)).await.unwrap_err();
    assert!(matches!(error, AiError::LoadFailed { .. }), "{error}");
}

#[tokio::test]
async fn new_sessions_spread_across_replicas() {
    let url = stub_server().await;
    let mut spec = model("a", &url, 100, 0);
    spec.max_replicas = 2;
    spec.max_concurrency = 4;
    let m = manager(&[spec], 1000, &[]).await;

    let mut o = opts(Priority::High);
    o.for_new_session = true;
    o.max_sessions = Some(1);
    let first = m.acquire("a", o.clone()).await.unwrap();
    m.bind_session(first.instance_id());
    let second = m.acquire("a", o).await.unwrap();
    assert_ne!(
        first.instance_id(),
        second.instance_id(),
        "a full instance was passed over"
    );
}
