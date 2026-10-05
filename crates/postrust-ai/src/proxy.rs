//! Registration with postrust-proxy.
//!
//! Every node joins one upstream (`postrust-ai` by default) as a backend, and
//! one route sends `route_prefix` to it with the prefix stripped. The proxy
//! spreads requests across nodes; the nodes themselves send session-locked
//! requests to the node that holds the lock and other requests to a node with
//! the model ready, so the proxy needs no knowledge of models or sessions.
//!
//! Registration goes through the proxy's admin API rather than its database
//! tables, because the proxy reads those tables only at startup: the API
//! changes the running routing table and persists the change.
//!
//! Known limitation: the admin API cannot set a route's timeout, so routes it
//! creates get the proxy default of 30 seconds. Long synchronous inference
//! should go through jobs, or the route's timeout should be raised in the
//! proxy's own configuration.

use serde::Deserialize;
use serde_json::{json, Value};

use crate::config::ProxyRegistration;
use crate::error::{AiError, AiResult};

#[derive(Deserialize)]
struct Envelope<T> {
    success: bool,
    data: Option<T>,
    error: Option<String>,
}

#[derive(Deserialize)]
struct UpstreamView {
    id: Option<uuid::Uuid>,
    name: String,
    #[serde(default)]
    backends: Vec<BackendView>,
}

#[derive(Deserialize)]
struct BackendView {
    id: Option<uuid::Uuid>,
    address: String,
}

#[derive(Deserialize)]
struct RouteView {
    name: String,
}

/// What registration created, so shutdown can undo it.
#[derive(Debug, Clone)]
pub struct Registration {
    upstream_id: uuid::Uuid,
    backend_id: uuid::Uuid,
}

/// Talks to the proxy admin API.
pub struct ProxyClient {
    config: ProxyRegistration,
    http: reqwest::Client,
}

impl ProxyClient {
    /// A client.
    pub fn new(config: ProxyRegistration, http: reqwest::Client) -> Self {
        Self { config, http }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.config.admin_url.trim_end_matches('/'))
    }

    async fn call<T: for<'de> Deserialize<'de>>(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> AiResult<T> {
        let mut request = self.http.request(method, self.url(path));
        if let Some(token) = &self.config.admin_token {
            request = request.bearer_auth(token);
        }
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().await?;
        let status = response.status();
        let envelope: Envelope<T> = response
            .json()
            .await
            .map_err(|e| AiError::Upstream(format!("proxy admin {path}: {status}: {e}")))?;
        if !envelope.success {
            return Err(AiError::Upstream(format!(
                "proxy admin {path}: {}",
                envelope.error.unwrap_or_else(|| status.to_string())
            )));
        }
        envelope
            .data
            .ok_or_else(|| AiError::Upstream(format!("proxy admin {path}: no data")))
    }

    /// Join the upstream (creating it and the route if missing).
    pub async fn register(&self, advertise_url: &str) -> AiResult<Registration> {
        let parsed = url::Url::parse(advertise_url)
            .map_err(|e| AiError::Config(format!("advertise url '{advertise_url}': {e}")))?;
        let host = parsed.host_str().ok_or_else(|| {
            AiError::Config(format!("advertise url '{advertise_url}' has no host"))
        })?;
        let port = parsed.port_or_known_default().ok_or_else(|| {
            AiError::Config(format!("advertise url '{advertise_url}' has no port"))
        })?;
        let address = format!("{host}:{port}");
        let scheme = parsed.scheme().to_string();

        let upstreams: Vec<UpstreamView> =
            self.call(reqwest::Method::GET, "/upstreams", None).await?;
        let existing = upstreams
            .into_iter()
            .find(|u| u.name == self.config.upstream_name);

        let registration = match existing {
            None => {
                // Least connections: inference requests vary wildly in
                // length, so round-robin piles them up on one node.
                let created: UpstreamView = self
                    .call(
                        reqwest::Method::POST,
                        "/upstreams",
                        Some(json!({
                            "name": self.config.upstream_name,
                            "lb_strategy": "least_connections",
                            "backends": [{ "address": address, "scheme": scheme }],
                        })),
                    )
                    .await?;
                let backend_id = created
                    .backends
                    .iter()
                    .find(|b| b.address == address)
                    .and_then(|b| b.id);
                Registration {
                    upstream_id: created
                        .id
                        .ok_or_else(|| AiError::Upstream("upstream has no id".into()))?,
                    backend_id: backend_id
                        .ok_or_else(|| AiError::Upstream("backend has no id".into()))?,
                }
            }
            Some(upstream) => {
                let upstream_id = upstream
                    .id
                    .ok_or_else(|| AiError::Upstream("upstream has no id".into()))?;
                let backend_id = match upstream.backends.iter().find(|b| b.address == address) {
                    Some(b) => {
                        b.id.ok_or_else(|| AiError::Upstream("backend has no id".into()))?
                    }
                    None => {
                        let created: BackendView = self
                            .call(
                                reqwest::Method::POST,
                                &format!("/upstreams/{upstream_id}/backends"),
                                Some(json!({ "address": address, "scheme": scheme })),
                            )
                            .await?;
                        created
                            .id
                            .ok_or_else(|| AiError::Upstream("backend has no id".into()))?
                    }
                };
                Registration {
                    upstream_id,
                    backend_id,
                }
            }
        };

        let routes: Vec<RouteView> = self.call(reqwest::Method::GET, "/routes", None).await?;
        if !routes.iter().any(|r| r.name == self.config.upstream_name) {
            let _: Value = self
                .call(
                    reqwest::Method::POST,
                    "/routes",
                    Some(json!({
                        "name": self.config.upstream_name,
                        "path": self.config.route_prefix,
                        "upstream": self.config.upstream_name,
                        "strip_path": true,
                    })),
                )
                .await?;
        }
        tracing::info!(%address, upstream = %self.config.upstream_name, "registered with proxy");
        Ok(registration)
    }

    /// Leave the upstream. The route and upstream stay for the other nodes.
    pub async fn deregister(&self, registration: &Registration) -> AiResult<()> {
        let _: Value = self
            .call(
                reqwest::Method::DELETE,
                &format!(
                    "/upstreams/{}/backends/{}",
                    registration.upstream_id, registration.backend_id
                ),
                None,
            )
            .await
            .or_else(|e| match e {
                // `data` is absent on a unit success; that is fine.
                AiError::Upstream(m) if m.ends_with("no data") => Ok(Value::Null),
                e => Err(e),
            })?;
        Ok(())
    }
}
