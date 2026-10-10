//! Registration against a stub of postrust-proxy's admin API, speaking its
//! `{success, data}` envelope. No database.

mod common;

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::routing::{delete, get, post};
use axum::Json;
use common::serve;
use parking_lot::Mutex;
use postrust_ai::config::ProxyRegistration;
use postrust_ai::proxy::ProxyClient;
use postrust_ai::AiError;
use serde_json::{json, Value};
use uuid::Uuid;

/// What the stub proxy holds, and what it was sent.
#[derive(Default)]
struct Proxy {
    upstreams: Vec<Value>,
    routes: Vec<Value>,
    authorization: Vec<Option<String>>,
    /// Answer every call with `success: false`.
    refuse: bool,
}

type Shared = Arc<Mutex<Proxy>>;

fn ok(data: Value) -> Json<Value> {
    Json(json!({ "success": true, "data": data }))
}

fn note(state: &Shared, headers: &HeaderMap) -> Option<Json<Value>> {
    let mut proxy = state.lock();
    proxy.authorization.push(
        headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .map(String::from),
    );
    proxy
        .refuse
        .then(|| Json(json!({ "success": false, "error": "nope" })))
}

async fn stub() -> (String, Shared) {
    let state: Shared = Arc::default();
    let app =
        axum::Router::new()
            .route(
                "/upstreams",
                get(|State(s): State<Shared>, h: HeaderMap| async move {
                    note(&s, &h).unwrap_or_else(|| ok(Value::Array(s.lock().upstreams.clone())))
                })
                .post(
                    |State(s): State<Shared>, h: HeaderMap, Json(body): Json<Value>| async move {
                        if let Some(refused) = note(&s, &h) {
                            return refused;
                        }
                        let backends: Vec<Value> = body["backends"]
                            .as_array()
                            .cloned()
                            .unwrap_or_default()
                            .into_iter()
                            .map(|mut b| {
                                b["id"] = json!(Uuid::new_v4());
                                b
                            })
                            .collect();
                        let upstream = json!({
                            "id": Uuid::new_v4(),
                            "name": body["name"],
                            "lb_strategy": body["lb_strategy"],
                            "backends": backends,
                        });
                        s.lock().upstreams.push(upstream.clone());
                        ok(upstream)
                    },
                ),
            )
            .route(
                "/upstreams/{id}/backends",
                post(
                    |State(s): State<Shared>,
                     h: HeaderMap,
                     Path(id): Path<Uuid>,
                     Json(mut body): Json<Value>| async move {
                        if let Some(refused) = note(&s, &h) {
                            return refused;
                        }
                        body["id"] = json!(Uuid::new_v4());
                        let mut proxy = s.lock();
                        let upstream = proxy
                            .upstreams
                            .iter_mut()
                            .find(|u| u["id"] == json!(id))
                            .expect("known upstream");
                        upstream["backends"]
                            .as_array_mut()
                            .unwrap()
                            .push(body.clone());
                        ok(body)
                    },
                ),
            )
            .route(
                "/upstreams/{id}/backends/{backend}",
                // A bare `{success: true}`, with no `data`, as the proxy answers
                // a delete.
                delete(
                    |State(s): State<Shared>,
                     h: HeaderMap,
                     Path((id, backend)): Path<(Uuid, Uuid)>| async move {
                        if let Some(refused) = note(&s, &h) {
                            return refused;
                        }
                        let mut proxy = s.lock();
                        let upstream = proxy
                            .upstreams
                            .iter_mut()
                            .find(|u| u["id"] == json!(id))
                            .expect("known upstream");
                        upstream["backends"]
                            .as_array_mut()
                            .unwrap()
                            .retain(|b| b["id"] != json!(backend));
                        Json(json!({ "success": true }))
                    },
                ),
            )
            .route(
                "/routes",
                get(|State(s): State<Shared>, h: HeaderMap| async move {
                    note(&s, &h).unwrap_or_else(|| ok(Value::Array(s.lock().routes.clone())))
                })
                .post(
                    |State(s): State<Shared>, h: HeaderMap, Json(body): Json<Value>| async move {
                        if let Some(refused) = note(&s, &h) {
                            return refused;
                        }
                        s.lock().routes.push(body.clone());
                        ok(body)
                    },
                ),
            )
            .with_state(state.clone());
    (serve(app).await, state)
}

fn client(admin_url: &str) -> ProxyClient {
    let mut config = ProxyRegistration::default();
    config.admin_url = admin_url.to_string();
    ProxyClient::new(config, reqwest::Client::new())
}

fn backend_addresses(proxy: &Shared) -> Vec<String> {
    proxy.lock().upstreams[0]["backends"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["address"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn the_first_node_creates_the_upstream_and_the_route() {
    let (url, proxy) = stub().await;
    client(&url).register("http://node-a:7700").await.unwrap();

    let p = proxy.lock();
    assert_eq!(p.upstreams.len(), 1);
    assert_eq!(p.upstreams[0]["name"], json!("postrust-ai"));
    assert_eq!(p.upstreams[0]["lb_strategy"], json!("least_connections"));
    drop(p);
    assert_eq!(backend_addresses(&proxy), vec!["node-a:7700"]);
    let p = proxy.lock();
    assert_eq!(p.routes.len(), 1);
    assert_eq!(p.routes[0]["path"], json!("/ai/"));
    assert_eq!(p.routes[0]["upstream"], json!("postrust-ai"));
    assert_eq!(p.routes[0]["strip_path"], json!(true));
}

#[tokio::test]
async fn a_second_node_adds_a_backend_and_creates_nothing_else() {
    let (url, proxy) = stub().await;
    client(&url).register("http://node-a:7700").await.unwrap();
    client(&url).register("http://node-b:7701").await.unwrap();
    // A restart of a node that is already registered changes nothing.
    client(&url).register("http://node-b:7701").await.unwrap();

    assert_eq!(
        proxy.lock().upstreams.len(),
        1,
        "one upstream for all nodes"
    );
    assert_eq!(proxy.lock().routes.len(), 1, "one route for all nodes");
    assert_eq!(
        backend_addresses(&proxy),
        vec!["node-a:7700", "node-b:7701"]
    );
}

#[tokio::test]
async fn deregistering_removes_only_this_nodes_backend() {
    let (url, proxy) = stub().await;
    let a = client(&url).register("http://node-a:7700").await.unwrap();
    let b = client(&url).register("http://node-b:7701").await.unwrap();

    client(&url).deregister(&a).await.unwrap();
    assert_eq!(backend_addresses(&proxy), vec!["node-b:7701"]);
    assert_eq!(
        proxy.lock().routes.len(),
        1,
        "the route stays for the others"
    );

    client(&url).deregister(&b).await.unwrap();
    assert!(backend_addresses(&proxy).is_empty());
    assert_eq!(proxy.lock().upstreams.len(), 1, "the upstream stays");
}

#[tokio::test]
async fn the_admin_token_is_sent_and_a_refusal_is_an_error() {
    let (url, proxy) = stub().await;
    let mut config = ProxyRegistration::default();
    config.admin_url = url.clone();
    config.admin_token = Some("s3cret".into());
    ProxyClient::new(config, reqwest::Client::new())
        .register("http://node-a:7700")
        .await
        .unwrap();
    assert!(proxy
        .lock()
        .authorization
        .iter()
        .all(|a| a.as_deref() == Some("Bearer s3cret")));

    proxy.lock().refuse = true;
    match client(&url).register("http://node-b:7701").await {
        Err(AiError::Upstream(message)) => assert!(message.contains("nope"), "{message}"),
        other => panic!("wanted an upstream error, got {other:?}"),
    }
}
