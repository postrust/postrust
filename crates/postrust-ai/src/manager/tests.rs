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
    // Tests run side by side: managers that start real servers must not pick
    // from the same ports, or two of them take the first free one before
    // either child has bound it.
    static NEXT_RANGE: std::sync::atomic::AtomicU16 = std::sync::atomic::AtomicU16::new(0);
    let low = 30_000 + NEXT_RANGE.fetch_add(1, Ordering::Relaxed) * 50;
    let resources = ResourceConfig {
        port_range: (low, low + 49),
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

// ---- Phase 2 rules ---------------------------------------------------------

/// A server that can park: it answers `/health`, `/sleep` and `/wake_up` with
/// 200, as vLLM does with `--enable-sleep-mode`. The manager treats a
/// non-attached vLLM spec as parkable, and parking is a POST that only has to
/// succeed, so a few lines of Python stand in for the engine.
const SLEEPY_SERVER: &str = "
import http.server, sys
class H(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200); self.send_header('Content-Length', '2'); self.end_headers()
        self.wfile.write(b'ok')
    do_POST = do_GET
    def log_message(self, *args): pass
http.server.ThreadingHTTPServer(('127.0.0.1', int(sys.argv[1])), H).serve_forever()
";

fn parkable(id: &str, mem_mb: u64, gpu_mb: u64) -> ModelSpec {
    let mut spec = ModelSpec::new(
        id,
        ModelKind::Text,
        Flavor::Vllm,
        RuntimeSpec::Process {
            command: "python3".into(),
            args: vec!["-c".into(), SLEEPY_SERVER.into(), "{port}".into()],
            env: Default::default(),
            workdir: None,
        },
    );
    spec.memory_mb = mem_mb;
    spec.gpu_mb = gpu_mb;
    spec.load_timeout_secs = 20;
    assert!(spec.can_park());
    spec
}

/// `(model, state)` of every instance, sorted.
fn states(m: &ModelManager) -> Vec<(String, InstanceState)> {
    let mut out: Vec<_> = m
        .instances()
        .into_iter()
        .map(|i| (i.model_id, i.state))
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn state_of(m: &ModelManager, model: &str) -> Vec<InstanceState> {
    states(m)
        .into_iter()
        .filter(|(id, _)| id == model)
        .map(|(_, s)| s)
        .collect()
}

/// Poll until `done`, failing after ten seconds.
async fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

/// Make every instance of `model` look like it was last used `ago` ago.
fn backdate(m: &ModelManager, model: &str, ago: Duration) {
    let at = Instant::now().checked_sub(ago).expect("monotonic clock");
    let mut state = m.inner.state.lock();
    for i in state.instances.values_mut().filter(|i| i.spec.id == model) {
        i.last_used = at;
    }
}

fn demand(model: &str, top_priority: Priority) -> ModelDemand {
    ModelDemand {
        model_id: model.into(),
        top_priority,
        waiting: 1,
    }
}

/// One parkable model `v` (600 MiB VRAM) is idle on a 1000 MiB GPU; then `b`
/// (600 MiB VRAM, `b_mem` host memory) is asked for. Returns the manager once
/// `b` is ready.
async fn make_b_displace_v(mem_budget: u64, b_mem: u64) -> ModelManager {
    let url = stub_server().await;
    let m = manager(
        &[parkable("v", 100, 600), model("b", &url, b_mem, 600)],
        mem_budget,
        &[1000],
    )
    .await;
    drop(m.acquire("v", opts(Priority::Normal)).await.unwrap());
    let lease = m.acquire("b", opts(Priority::Normal)).await.unwrap();
    assert_eq!(lease.spec().id, "b");
    drop(lease);
    m
}

#[tokio::test]
async fn parks_when_only_vram_is_short_and_host_memory_can_take_the_weights() {
    let m = make_b_displace_v(1000, 100).await;
    assert_eq!(state_of(&m, "v"), vec![InstanceState::Parked]);
    let v = m
        .instances()
        .into_iter()
        .find(|i| i.model_id == "v")
        .unwrap();
    assert_eq!(v.reservation.gpu, None, "the GPU was given up");
    assert_eq!(
        v.reservation.mem_mb, 700,
        "the weights moved to host memory"
    );
}

#[tokio::test]
async fn stops_instead_of_parking_when_host_memory_cannot_hold_the_weights() {
    // Parking v needs 600 MiB of host memory; only 550 are free.
    let m = make_b_displace_v(650, 100).await;
    assert!(state_of(&m, "v").is_empty(), "v was stopped, not parked");
}

#[tokio::test]
async fn stops_instead_of_parking_when_the_parked_weights_would_starve_the_requester() {
    // Parking v leaves 300 MiB of host memory, and b needs 400.
    let m = make_b_displace_v(1000, 400).await;
    assert!(state_of(&m, "v").is_empty(), "v was stopped, not parked");
}

#[tokio::test]
async fn a_parked_instance_wakes_when_its_model_is_asked_for_again() {
    let m = make_b_displace_v(1000, 100).await;
    assert_eq!(state_of(&m, "v"), vec![InstanceState::Parked]);
    let parked_id = m
        .instances()
        .into_iter()
        .find(|i| i.model_id == "v")
        .unwrap()
        .id;

    // b is idle and cannot park (attached): it makes way, and v wakes.
    let lease = m.acquire("v", opts(Priority::Normal)).await.unwrap();
    assert_eq!(state_of(&m, "v"), vec![InstanceState::Ready]);
    assert_eq!(
        lease.instance_id(),
        parked_id,
        "the parked instance was woken, not stopped and reloaded"
    );
    assert!(state_of(&m, "b").is_empty());
    drop(lease);
}

#[tokio::test]
async fn min_warm_protects_the_last_ready_instance() {
    let url = stub_server().await;
    let mut warm = model("a", &url, 600, 0);
    warm.min_warm = 1;
    let m = manager(&[warm, model("b", &url, 600, 0)], 1000, &[]).await;

    drop(m.acquire("a", opts(Priority::Normal)).await.unwrap());
    let mut o = opts(Priority::Critical);
    o.wait = Duration::from_millis(500);
    let error = m.acquire("b", o).await.unwrap_err();
    assert!(matches!(error, AiError::LoadTimeout { .. }), "{error}");
    assert_eq!(state_of(&m, "a"), vec![InstanceState::Ready]);
}

#[tokio::test]
async fn min_warm_holds_when_one_pass_needs_several_victims() {
    let url = stub_server().await;
    let mut warm = model("a", &url, 300, 0);
    warm.min_warm = 1;
    warm.max_replicas = 2;
    let m = manager(&[warm, model("big", &url, 1000, 0)], 1000, &[]).await;

    // Two idle replicas of a. Evicting both would take the budget back, but
    // one of them has to stay.
    let first = m.acquire("a", opts(Priority::Normal)).await.unwrap();
    let second = m.acquire("a", opts(Priority::Normal)).await.unwrap();
    assert_ne!(first.instance_id(), second.instance_id());
    drop((first, second));

    let mut o = opts(Priority::Critical);
    o.wait = Duration::from_millis(500);
    let error = m.acquire("big", o).await.unwrap_err();
    assert!(matches!(error, AiError::LoadTimeout { .. }), "{error}");
    assert_eq!(
        state_of(&m, "a"),
        vec![InstanceState::Ready, InstanceState::Ready],
        "nothing was evicted for a request that could not be satisfied anyway"
    );
}

#[tokio::test]
async fn min_warm_allows_one_replica_to_go_when_one_is_enough() {
    let url = stub_server().await;
    let mut warm = model("a", &url, 300, 0);
    warm.min_warm = 1;
    warm.max_replicas = 2;
    let m = manager(&[warm, model("b", &url, 700, 0)], 1000, &[]).await;

    let first = m.acquire("a", opts(Priority::Normal)).await.unwrap();
    let second = m.acquire("a", opts(Priority::Normal)).await.unwrap();
    drop((first, second));

    // b needs 700 and 400 are free: one replica goes, the other stays warm.
    assert!(m.acquire("b", opts(Priority::Normal)).await.is_ok());
    assert_eq!(state_of(&m, "a"), vec![InstanceState::Ready]);
}

#[tokio::test]
async fn min_warm_does_not_protect_a_parked_instance() {
    let url = stub_server().await;
    let mut v = parkable("v", 100, 300);
    v.min_warm = 1;
    v.max_replicas = 2;
    let mut pinned = model("b", &url, 100, 300);
    pinned.pinned = true;
    let m = manager(&[v, pinned, model("d", &url, 600, 0)], 1000, &[700]).await;

    // Two idle replicas of v fill 600 of the 700 MiB of VRAM.
    let first = m.acquire("v", opts(Priority::Normal)).await.unwrap();
    let second = m.acquire("v", opts(Priority::Normal)).await.unwrap();
    drop((first, second));
    // b needs 300: one v is parked, the other stays ready for min_warm.
    drop(m.acquire("b", opts(Priority::Normal)).await.unwrap());
    let mut v_states = state_of(&m, "v");
    v_states.sort_by_key(|s| format!("{s:?}"));
    assert_eq!(v_states, vec![InstanceState::Parked, InstanceState::Ready]);

    // d needs host memory, and the parked replica holds 400 MiB of it.
    assert!(m.acquire("d", opts(Priority::Normal)).await.is_ok());
    assert_eq!(state_of(&m, "v"), vec![InstanceState::Ready]);
}

#[tokio::test]
async fn waiting_demand_protects_an_idle_instance_from_a_lower_priority_requester() {
    // (what is waiting for a, who asks for b, whether a goes)
    let cases = [
        (Priority::High, Priority::Normal, false),
        (Priority::High, Priority::High, true),
        (Priority::Normal, Priority::High, true),
        (Priority::Low, Priority::Normal, true),
    ];
    for (waiting, requester, evicts) in cases {
        let url = stub_server().await;
        let m = manager(
            &[model("a", &url, 600, 0), model("b", &url, 600, 0)],
            1000,
            &[],
        )
        .await;
        drop(m.acquire("a", opts(Priority::Normal)).await.unwrap());
        m.set_demand(vec![demand("a", waiting)]);

        let mut o = opts(requester);
        o.wait = Duration::from_millis(500);
        let got = m.acquire("b", o).await;
        if evicts {
            assert!(
                got.is_ok(),
                "{requester:?} may take a from {waiting:?} demand"
            );
            assert!(state_of(&m, "a").is_empty());
        } else {
            assert!(
                matches!(got, Err(AiError::LoadTimeout { .. })),
                "{requester:?} must not take a from {waiting:?} demand"
            );
            assert_eq!(state_of(&m, "a"), vec![InstanceState::Ready]);
        }
    }
}

#[tokio::test]
async fn the_lowest_value_instance_goes_first_not_the_least_recently_used() {
    let url = stub_server().await;
    let m = manager(
        &[
            model("a", &url, 400, 0),
            model("b", &url, 400, 0),
            model("c", &url, 400, 0),
        ],
        1000,
        &[],
    )
    .await;
    drop(m.acquire("a", opts(Priority::Normal)).await.unwrap());
    drop(m.acquire("b", opts(Priority::Normal)).await.unwrap());
    {
        // a is the older, but took two minutes to load; b reloads in a
        // second. Value: a 120 - 2 = 118, b 1 - 0 = 1, so b is the cheaper
        // loss. Plain LRU would take a.
        let mut state = m.inner.state.lock();
        for i in state.instances.values_mut() {
            if i.spec.id == "a" {
                i.last_used = Instant::now() - Duration::from_secs(120);
                i.load_duration = Duration::from_secs(120);
            } else {
                i.load_duration = Duration::from_secs(1);
            }
        }
    }

    // c needs 400 and 200 are free: one victim.
    drop(m.acquire("c", opts(Priority::Normal)).await.unwrap());
    let models: Vec<String> = states(&m).into_iter().map(|(id, _)| id).collect();
    assert_eq!(models, vec!["a".to_string(), "c".to_string()]);
}

#[tokio::test]
async fn maintain_stops_an_idle_instance_that_cannot_park() {
    let url = stub_server().await;
    let mut a = model("a", &url, 100, 0);
    a.idle_timeout_secs = 60;
    let mut keep = model("keep", &url, 100, 0);
    keep.idle_timeout_secs = 3600;
    let m = manager(&[a, keep], 1000, &[]).await;
    drop(m.acquire("a", opts(Priority::Normal)).await.unwrap());
    drop(m.acquire("keep", opts(Priority::Normal)).await.unwrap());

    // Not idle for long enough yet: timeouts are a minute, and nothing here
    // waits for real time; instances are back-dated instead.
    m.maintain().await;
    assert_eq!(state_of(&m, "a"), vec![InstanceState::Ready]);

    backdate(&m, "a", Duration::from_secs(90));
    // Queued work for a keeps it, whatever its idle time.
    m.set_demand(vec![demand("a", Priority::Low)]);
    m.maintain().await;
    assert_eq!(state_of(&m, "a"), vec![InstanceState::Ready]);

    m.set_demand(vec![]);
    m.maintain().await;
    wait_until("a to stop", || state_of(&m, "a").is_empty()).await;
    assert_eq!(state_of(&m, "keep"), vec![InstanceState::Ready]);
}

#[tokio::test]
async fn maintain_parks_an_idle_parkable_instance_then_stops_it_after_the_park_timeout() {
    let mut v = parkable("v", 100, 600);
    v.idle_timeout_secs = 60;
    v.park_timeout_secs = 60;
    let m = manager(&[v], 1000, &[1000]).await;
    drop(m.acquire("v", opts(Priority::Normal)).await.unwrap());

    backdate(&m, "v", Duration::from_secs(90));
    m.maintain().await;
    wait_until("v to park", || {
        state_of(&m, "v") == vec![InstanceState::Parked]
    })
    .await;

    // Parking counts as use, so the park timeout runs from the park.
    m.maintain().await;
    assert_eq!(state_of(&m, "v"), vec![InstanceState::Parked]);
    backdate(&m, "v", Duration::from_secs(90));
    m.maintain().await;
    wait_until("v to stop", || state_of(&m, "v").is_empty()).await;
    assert_eq!(
        m.resources().gpus[0].reserved_mb,
        0,
        "everything given back"
    );
    assert_eq!(m.resources().mem_reserved_mb, 0);
}

#[tokio::test]
async fn demand_preloads_an_unloaded_model_when_it_fits() {
    let url = stub_server().await;
    let m = manager(&[model("x", &url, 600, 0)], 1000, &[]).await;
    m.set_demand(vec![demand("x", Priority::High)]);
    m.maintain().await;
    wait_until("x to preload", || {
        state_of(&m, "x") == vec![InstanceState::Ready]
    })
    .await;
}

#[tokio::test]
async fn demand_preload_uses_free_capacity_only() {
    let url = stub_server().await;
    let m = manager(
        &[model("a", &url, 600, 0), model("x", &url, 600, 0)],
        1000,
        &[],
    )
    .await;
    drop(m.acquire("a", opts(Priority::Normal)).await.unwrap());
    m.set_demand(vec![demand("x", Priority::Critical)]);
    m.maintain().await;

    // A preload that would evict does so at once, on its first pass; give it
    // far longer than that to prove it did not.
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(
        state_of(&m, "a"),
        vec![InstanceState::Ready],
        "a was left alone"
    );
    assert!(state_of(&m, "x").is_empty());

    // The same preload, still waiting, takes the room once it exists.
    m.unload("a").unwrap();
    wait_until("x to load into the freed room", || {
        state_of(&m, "x") == vec![InstanceState::Ready]
    })
    .await;
}

#[tokio::test]
async fn several_small_victims_on_one_gpu_make_room_there() {
    // GPU 0 holds three 1000 MiB models (1000 free), GPU 1 one busy 2000 MiB
    // model (2000 free). A 3500 MiB load is closest to fitting on GPU 1, so
    // no single victim on GPU 0 lowers the shortfall; all three together
    // make room there.
    let url = stub_server().await;
    let m = manager(
        &[
            model("x", &url, 10, 1000),
            model("y", &url, 10, 1000),
            model("z", &url, 10, 1000),
            model("w", &url, 10, 2000),
            model("big", &url, 10, 3500),
        ],
        10_000,
        &[4000, 4000],
    )
    .await;
    for id in ["x", "y", "z"] {
        drop(m.acquire(id, opts(Priority::Normal)).await.unwrap());
    }
    let busy = m.acquire("w", opts(Priority::Normal)).await.unwrap();
    let gpu_of = |id: &str| {
        m.instances()
            .into_iter()
            .find(|i| i.model_id == id)
            .and_then(|i| i.reservation.gpu)
            .map(|(g, _)| g)
    };
    assert_eq!(
        ["x", "y", "z", "w"].map(gpu_of),
        [Some(0), Some(0), Some(0), Some(1)],
        "the layout the test relies on"
    );

    let big = m.acquire("big", opts(Priority::Normal)).await.unwrap();
    assert_eq!(gpu_of("big"), Some(0));
    let mut left: Vec<String> = m.instances().into_iter().map(|i| i.model_id).collect();
    left.sort();
    assert_eq!(left, vec!["big".to_string(), "w".to_string()]);
    drop((big, busy));
}
