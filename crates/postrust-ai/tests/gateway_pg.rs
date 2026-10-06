//! The synchronous gateway, with whole nodes built over a live PostgreSQL and
//! served on local ports, in front of a stub model server.
//!
//!     DATABASE_URL=postgres://postgres:postgres@localhost:5432/postrust_test \
//!         cargo test -p postrust-ai --test gateway_pg -- --ignored

mod common;

use std::convert::Infallible;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::http::StatusCode;
use axum::routing::{any, get};
use bytes::Bytes;
use common::{reserve, serve, stub_model, unique};
use postrust_ai::api::{self, AppState};
use postrust_ai::app::Node;
use postrust_ai::config::AiConfig;
use postrust_ai::ModelSpec;
use serde_json::json;
use sqlx::PgPool;

/// A model server whose health check starts passing after `ready_after`,
/// answers `/ping`, and streams three chunks 400 ms apart at `/stream`.
async fn model_server(ready_after: Duration) -> String {
    let ready_at = Instant::now() + ready_after;
    let app = axum::Router::new()
        .route(
            "/health",
            get(move || async move {
                if Instant::now() >= ready_at {
                    StatusCode::OK
                } else {
                    StatusCode::SERVICE_UNAVAILABLE
                }
            }),
        )
        .route("/ping", any(|| async { "pong" }))
        .route(
            "/stream",
            get(|| async {
                let chunks = futures::stream::unfold(0u32, |i| async move {
                    if i >= 3 {
                        return None;
                    }
                    if i > 0 {
                        tokio::time::sleep(Duration::from_millis(400)).await;
                    }
                    Some((
                        Ok::<_, Infallible>(Bytes::from(format!("chunk{i};"))),
                        i + 1,
                    ))
                });
                Body::from_stream(chunks)
            }),
        );
    serve(app).await
}

/// A built node, served on its own port.
struct TestNode {
    url: String,
    state: AppState,
}

impl TestNode {
    fn id(&self) -> &str {
        &self.state.config.node.id
    }

    /// Announce this node to the others, as its heartbeat would.
    async fn announce(&self) {
        self.state
            .nodes
            .heartbeat(&self.url, &json!({}), &self.state.manager.ready_models())
            .await
            .unwrap();
    }
}

async fn start_node(
    pool: &PgPool,
    models: Vec<ModelSpec>,
    tune: impl FnOnce(&mut AiConfig),
) -> TestNode {
    let (listener, url) = reserve().await;
    let mut config = AiConfig::default();
    config.node.id = unique("node");
    config.node.advertise_url = Some(url.clone());
    config.models = models;
    config.resources.memory_mb = Some(10_000);
    config.resources.gpus = Some(vec![]);
    config.resources.respect_probe = false;
    config.cache.dir = std::env::temp_dir().join(unique("postrust-ai-gw"));
    tune(&mut config);
    config.validate().unwrap();

    let node = Node::build_with_pool(config, pool.clone(), vec![])
        .await
        .unwrap();
    let state = node.state().clone();
    let app = api::router(state.clone());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    TestNode { url, state }
}

fn http() -> reqwest::Client {
    reqwest::Client::new()
}

fn infer(node: &TestNode, model: &str, path: &str) -> reqwest::RequestBuilder {
    http().get(format!("{}/v1/infer/{model}/{path}", node.url))
}

#[tokio::test]
#[ignore]
async fn a_slow_load_answers_503_with_retry_after_and_the_retry_succeeds() {
    let (pool, _) = common::store().await;
    let upstream = model_server(Duration::from_secs(3)).await;
    let model = unique("slow");
    let node = start_node(&pool, vec![stub_model(&model, &upstream, 1)], |c| {
        c.gateway.load_wait_secs = 1;
    })
    .await;

    let started = Instant::now();
    let refused = infer(&node, &model, "ping").send().await.unwrap();
    assert_eq!(refused.status(), 503);
    assert!(refused.headers().contains_key("retry-after"));
    assert!(
        started.elapsed() < Duration::from_millis(2500),
        "gave up at the load wait, not the load timeout"
    );

    // The load carried on, so a retry finds the model ready.
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let retry = infer(&node, &model, "ping").send().await.unwrap();
        if retry.status() == 200 {
            assert_eq!(retry.text().await.unwrap(), "pong");
            break;
        }
        assert!(Instant::now() < deadline, "never became ready");
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
}

#[tokio::test]
#[ignore]
async fn the_response_body_streams_and_the_lease_is_held_until_it_ends() {
    let (pool, _) = common::store().await;
    let upstream = model_server(Duration::ZERO).await;
    let model = unique("stream");
    let node = start_node(&pool, vec![stub_model(&model, &upstream, 1)], |_| {}).await;

    let started = Instant::now();
    let mut response = infer(&node, &model, "stream").send().await.unwrap();
    assert_eq!(response.status(), 200);
    let mut chunks = Vec::new();
    let mut first_at = None;
    while let Some(chunk) = response.chunk().await.unwrap() {
        first_at.get_or_insert(started.elapsed());
        if chunks.is_empty() {
            // Mid-stream the instance is still busy.
            assert_eq!(node.state.manager.instances()[0].in_flight, 1);
        }
        chunks.push(String::from_utf8(chunk.to_vec()).unwrap());
    }
    assert_eq!(chunks.concat(), "chunk0;chunk1;chunk2;");
    assert!(chunks.len() >= 2, "arrived in pieces: {chunks:?}");
    assert!(
        first_at.unwrap() < Duration::from_millis(350),
        "the first chunk did not wait for the last: {first_at:?}"
    );
    assert!(started.elapsed() >= Duration::from_millis(800));

    // Once the body is done the lease is released.
    let deadline = Instant::now() + Duration::from_secs(2);
    while node.state.manager.instances()[0].in_flight != 0 {
        assert!(Instant::now() < deadline, "the lease outlived the body");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
#[ignore]
async fn a_session_whose_instance_was_evicted_is_rebound_and_told_so() {
    let (pool, _) = common::store().await;
    let upstream = model_server(Duration::ZERO).await;
    let (a, b) = (unique("a"), unique("b"));
    let mut spec_a = stub_model(&a, &upstream, 1);
    let mut spec_b = stub_model(&b, &upstream, 1);
    // Together they do not fit, so loading one means evicting the other.
    spec_a.memory_mb = 600;
    spec_b.memory_mb = 600;
    let node = start_node(&pool, vec![spec_a, spec_b], |c| {
        c.resources.memory_mb = Some(1000);
        c.gateway.load_wait_secs = 1;
    })
    .await;
    let session = unique("session");

    let first = infer(&node, &a, "ping")
        .header("x-session-id", &session)
        .send()
        .await
        .unwrap();
    assert_eq!(first.status(), 200);
    assert!(!first.headers().contains_key("x-session-rebound"));
    let before = first.headers()["x-postrust-ai-instance"].clone();
    first.bytes().await.unwrap();

    // Ordinary work cannot evict a locked instance...
    let normal = infer(&node, &b, "ping")
        .header("x-priority", "high")
        .send()
        .await
        .unwrap();
    assert_eq!(normal.status(), 503, "a session lock protects its instance");

    // ...but critical work can.
    let critical = infer(&node, &b, "ping")
        .header("x-priority", "critical")
        .send()
        .await
        .unwrap();
    assert_eq!(critical.status(), 200);
    critical.bytes().await.unwrap();

    let after = infer(&node, &a, "ping")
        .header("x-session-id", &session)
        .send()
        .await
        .unwrap();
    assert_eq!(after.status(), 200);
    assert_eq!(after.headers()["x-session-rebound"], "true");
    assert_ne!(after.headers()["x-postrust-ai-instance"], before);
    after.bytes().await.unwrap();

    // And the new binding is stable: the next request is not a rebind.
    let next = infer(&node, &a, "ping")
        .header("x-session-id", &session)
        .send()
        .await
        .unwrap();
    assert!(!next.headers().contains_key("x-session-rebound"));
}

#[tokio::test]
#[ignore]
async fn a_session_owned_by_another_node_is_forwarded_to_it() {
    let (pool, _) = common::store().await;
    let upstream = model_server(Duration::ZERO).await;
    let model = unique("fwd");
    let a = start_node(&pool, vec![stub_model(&model, &upstream, 2)], |_| {}).await;
    let b = start_node(&pool, vec![stub_model(&model, &upstream, 2)], |_| {}).await;
    a.announce().await;
    b.announce().await;
    let session = unique("session");

    let owned = infer(&a, &model, "ping")
        .header("x-session-id", &session)
        .send()
        .await
        .unwrap();
    assert_eq!(owned.headers()["x-postrust-ai-node"], a.id());
    let instance = owned.headers()["x-postrust-ai-instance"].clone();
    owned.bytes().await.unwrap();

    // The proxy happened to pick B: it forwards, and the answer is A's.
    let via_b = infer(&b, &model, "ping")
        .header("x-session-id", &session)
        .send()
        .await
        .unwrap();
    assert_eq!(via_b.status(), 200);
    assert_eq!(via_b.headers()["x-postrust-ai-node"], a.id());
    assert_eq!(via_b.headers()["x-postrust-ai-instance"], instance);
    assert!(!via_b.headers().contains_key("x-session-rebound"));
    assert_eq!(via_b.text().await.unwrap(), "pong");
    assert!(
        b.state.manager.instances().is_empty(),
        "B never loaded the model"
    );
}

#[tokio::test]
#[ignore]
async fn a_request_goes_to_a_node_that_already_has_the_model_ready() {
    let (pool, _) = common::store().await;
    let upstream = model_server(Duration::ZERO).await;
    let model = unique("ready");
    let a = start_node(&pool, vec![stub_model(&model, &upstream, 2)], |_| {}).await;
    let b = start_node(&pool, vec![stub_model(&model, &upstream, 2)], |_| {}).await;

    // Make the model ready on A and let B hear about it.
    infer(&a, &model, "ping").send().await.unwrap();
    a.announce().await;
    b.announce().await;

    let via_b = infer(&b, &model, "ping").send().await.unwrap();
    assert_eq!(via_b.headers()["x-postrust-ai-node"], a.id());
    assert!(b.state.manager.instances().is_empty());

    // A request already forwarded once is served where it lands, whatever
    // the registry says: forwarding never loops.
    let forced = infer(&b, &model, "ping")
        .header("x-postrust-ai-forwarded", "1")
        .send()
        .await
        .unwrap();
    assert_eq!(forced.status(), 200);
    assert_eq!(forced.headers()["x-postrust-ai-node"], b.id());
}

#[tokio::test]
#[ignore]
async fn a_session_whose_owner_is_gone_is_taken_over() {
    let (pool, _) = common::store().await;
    let upstream = model_server(Duration::ZERO).await;
    let model = unique("takeover");
    let a = start_node(&pool, vec![stub_model(&model, &upstream, 2)], |_| {}).await;
    let b = start_node(&pool, vec![stub_model(&model, &upstream, 2)], |_| {}).await;
    a.announce().await;
    b.announce().await;
    let session = unique("session");

    infer(&a, &model, "ping")
        .header("x-session-id", &session)
        .send()
        .await
        .unwrap();
    a.state.nodes.deregister().await.unwrap();

    let taken = infer(&b, &model, "ping")
        .header("x-session-id", &session)
        .send()
        .await
        .unwrap();
    assert_eq!(taken.status(), 200);
    assert_eq!(taken.headers()["x-postrust-ai-node"], b.id());
    assert_eq!(taken.headers()["x-session-rebound"], "true");
}
