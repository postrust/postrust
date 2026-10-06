//! `ai.model_events`: what the manager writes as instances load, park, stop,
//! crash and outgrow their specs. Needs `DATABASE_URL`; run with `--ignored`.

mod common;

use std::time::{Duration, Instant};

use common::{manager_with, store, stub_model, stub_server, unique};
use postrust_ai::config::GpuConfig;
use postrust_ai::manager::AcquireOptions;
use postrust_ai::queue::Priority;
use postrust_ai::{Flavor, ModelKind, ModelSpec, RuntimeSpec};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

/// A server answering `/health`, `/sleep` and `/wake_up`, which also exits
/// when the file named by its second argument appears, and, if given a third,
/// holds that many MiB of resident memory.
const SERVER: &str = "
import http.server, os, sys, threading, time
class H(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200); self.send_header('Content-Length', '2'); self.end_headers()
        self.wfile.write(b'ok')
    do_POST = do_GET
    def log_message(self, *args): pass
def watch():
    while not os.path.exists(sys.argv[2]):
        time.sleep(0.05)
    os._exit(3)
threading.Thread(target=watch, daemon=True).start()
# Written to, so the pages are resident (zero pages from calloc are not).
ballast = b'x' * (int(sys.argv[3]) << 20) if len(sys.argv) > 3 else b''
http.server.ThreadingHTTPServer(('127.0.0.1', int(sys.argv[1])), H).serve_forever()
";

fn python_model(
    id: &str,
    flavor: Flavor,
    exit_flag: &std::path::Path,
    ballast_mb: Option<u64>,
) -> ModelSpec {
    let mut args = vec![
        "-c".to_string(),
        SERVER.to_string(),
        "{port}".to_string(),
        exit_flag.display().to_string(),
    ];
    args.extend(ballast_mb.map(|mb| mb.to_string()));
    let mut spec = ModelSpec::new(
        id,
        ModelKind::Text,
        flavor,
        RuntimeSpec::Process {
            command: "python3".into(),
            args,
            env: Default::default(),
            workdir: None,
        },
    );
    spec.load_timeout_secs = 20;
    spec
}

fn opts() -> AcquireOptions {
    AcquireOptions::new(Priority::Normal, Duration::from_secs(30))
}

/// The events of `model` on `node`, oldest first.
async fn events(pool: &PgPool, node: &str, model: &str) -> Vec<(String, Option<Uuid>, Value)> {
    sqlx::query_as(
        "SELECT event, instance_id, detail FROM ai.model_events
         WHERE node_id = $1 AND model_id = $2 ORDER BY id",
    )
    .bind(node)
    .bind(model)
    .fetch_all(pool)
    .await
    .unwrap()
}

/// Events are written by spawned tasks: poll until `event` is there.
async fn wait_for_event(
    pool: &PgPool,
    node: &str,
    model: &str,
    event: &str,
) -> (String, Option<Uuid>, Value) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let all = events(pool, node, model).await;
        if let Some(row) = all.iter().find(|(e, _, _)| e == event) {
            return row.clone();
        }
        assert!(
            Instant::now() < deadline,
            "no '{event}' event for {model}; have {:?}",
            all.iter().map(|(e, _, _)| e).collect::<Vec<_>>()
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn kinds(pool: &PgPool, node: &str, model: &str) -> Vec<String> {
    events(pool, node, model)
        .await
        .into_iter()
        .map(|(e, _, _)| e)
        .collect()
}

fn flag() -> std::path::PathBuf {
    std::env::temp_dir().join(unique("postrust-ai-exit"))
}

#[tokio::test]
#[ignore = "needs DATABASE_URL"]
async fn a_load_and_an_unload_are_recorded() {
    let (pool, _store) = store().await;
    let node = unique("node");
    let url = stub_server(Duration::ZERO).await;
    let model = unique("loaded");
    let m = manager_with(
        &node,
        &[stub_model(&model, &url, 1)],
        Some(pool.clone()),
        |_| {},
    )
    .await;

    let lease = m.acquire(&model, opts()).await.unwrap();
    let instance = lease.instance_id();
    let started = wait_for_event(&pool, &node, &model, "load_started").await;
    let ready = wait_for_event(&pool, &node, &model, "ready").await;
    assert_eq!(started.1, Some(instance));
    assert_eq!(ready.1, Some(instance));
    assert!(ready.2["load_secs"].is_number(), "{}", ready.2);
    assert_eq!(started.2["reservation"]["mem_mb"], 100);

    drop(lease);
    assert_eq!(m.unload(&model).unwrap(), (1, 0));
    let stopped = wait_for_event(&pool, &node, &model, "stopped").await;
    assert_eq!(stopped.1, Some(instance));
    assert_eq!(
        kinds(&pool, &node, &model).await,
        ["load_started", "ready", "stopped"]
    );
}

#[tokio::test]
#[ignore = "needs DATABASE_URL"]
async fn a_park_a_wake_and_an_eviction_are_recorded() {
    let (pool, _store) = store().await;
    let node = unique("node");
    let url = stub_server(Duration::ZERO).await;
    let (v, b) = (unique("parkable"), unique("displacer"));
    let flag = flag();
    let mut parkable = python_model(&v, Flavor::Vllm, &flag, None);
    parkable.memory_mb = 100;
    parkable.gpu_mb = 600;
    let mut other = stub_model(&b, &url, 1);
    other.gpu_mb = 600;
    let m = manager_with(&node, &[parkable, other], Some(pool.clone()), |r| {
        r.gpus = Some(vec![GpuConfig::new(0, 1000)]);
    })
    .await;

    drop(m.acquire(&v, opts()).await.unwrap());
    // b does not fit beside v on the GPU, and v can park: it does.
    drop(m.acquire(&b, opts()).await.unwrap());
    let parked = wait_for_event(&pool, &node, &v, "parked").await;
    assert!(parked.1.is_some());

    // Asking for v again evicts the idle b, which cannot park, and wakes v.
    drop(m.acquire(&v, opts()).await.unwrap());
    let woken = wait_for_event(&pool, &node, &v, "woken").await;
    assert_eq!(woken.1, parked.1, "the same instance");
    let evicted = wait_for_event(&pool, &node, &b, "evicted").await;
    assert!(evicted.1.is_some());
    m.shutdown().await;
}

#[tokio::test]
#[ignore = "needs DATABASE_URL"]
async fn a_crash_is_recorded() {
    let (pool, _store) = store().await;
    let node = unique("node");
    let model = unique("crashy");
    let flag = flag();
    let m = manager_with(
        &node,
        &[python_model(&model, Flavor::Generic, &flag, None)],
        Some(pool.clone()),
        |_| {},
    )
    .await;

    let lease = m.acquire(&model, opts()).await.unwrap();
    let instance = lease.instance_id();
    drop(lease);
    // Ask the server to die; maintain() is what notices.
    std::fs::write(&flag, "").unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let crashed = loop {
        m.maintain().await;
        let all = events(&pool, &node, &model).await;
        if let Some(row) = all.iter().find(|(e, _, _)| e == "crashed") {
            break row.clone();
        }
        assert!(Instant::now() < deadline, "no crashed event: {all:?}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    assert_eq!(crashed.1, Some(instance));
    assert!(
        crashed.2["exit"].as_str().unwrap().contains("exited"),
        "{}",
        crashed.2
    );
    assert!(m.instances().is_empty());
    let _ = std::fs::remove_file(flag);
}

#[tokio::test]
#[ignore = "needs DATABASE_URL"]
async fn using_more_than_declared_is_recorded_once() {
    let (pool, _store) = store().await;
    let node = unique("node");
    let model = unique("greedy");
    let flag = flag();
    // Declares 10 MiB and holds more than 300.
    let mut spec = python_model(&model, Flavor::Generic, &flag, Some(300));
    spec.memory_mb = 10;
    let m = manager_with(&node, &[spec], Some(pool.clone()), |_| {}).await;
    let _lease = m.acquire(&model, opts()).await.unwrap();

    m.maintain().await;
    let row = wait_for_event(&pool, &node, &model, "over_budget").await;
    assert_eq!(row.2["declared_mem_mb"], 10);
    assert!(
        row.2["measured_mem_mb"].as_u64().unwrap() >= 300,
        "{}",
        row.2
    );
    let info = m.instances().remove(0);
    assert!(info.measured_mem_mb.unwrap() >= 300);

    // Further passes report nothing new.
    m.maintain().await;
    m.maintain().await;
    // Events are written by spawned tasks, so look again after a pause that
    // is long compared to an insert.
    tokio::time::sleep(Duration::from_millis(500)).await;
    let n = kinds(&pool, &node, &model)
        .await
        .iter()
        .filter(|e| *e == "over_budget")
        .count();
    assert_eq!(n, 1);
    m.shutdown().await;
}
