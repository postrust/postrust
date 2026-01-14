//! Vendored forwarder client from rpxy-lib: forwarder/client.rs
//!
//! This module handles forwarding requests to upstream backends.

use crate::config::Backend;
use crate::vendored::hyper_ext::{ProxyBody, TokioExecutor};
use crate::vendored::types::ProxyError;
use hyper::body::Incoming;
use hyper::{Request, Response};
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor as HyperTokioExecutor;
use std::time::Duration;

/// HTTP connector type alias.
type HttpConnector = hyper_util::client::legacy::connect::HttpConnector;

/// Forwarder client for sending requests to upstream backends.
pub struct ForwarderClient {
    /// HTTP client
    http_client: Client<HttpConnector, ProxyBody>,
    /// Request timeout
    timeout: Duration,
}

impl ForwarderClient {
    /// Create a new forwarder client.
    pub fn new(timeout: Duration) -> Self {
        let http_connector = HttpConnector::new();
        let http_client = Client::builder(HyperTokioExecutor::new())
            .pool_idle_timeout(Duration::from_secs(30))
            .pool_max_idle_per_host(32)
            .build(http_connector);

        Self {
            http_client,
            timeout,
        }
    }

    /// Forward a request to a backend.
    pub async fn forward(
        &self,
        backend: &Backend,
        mut request: Request<ProxyBody>,
    ) -> Result<Response<Incoming>, ProxyError> {
        // Build the upstream URL
        let uri = request.uri();
        let path_and_query = uri
            .path_and_query()
            .map(|pq| pq.as_str())
            .unwrap_or("/");

        let upstream_uri = format!(
            "{}://{}{}",
            backend.scheme, backend.address, path_and_query
        );

        // Update request URI
        *request.uri_mut() = upstream_uri
            .parse()
            .map_err(|e| ProxyError::Request(format!("Invalid upstream URI: {}", e)))?;

        // Send request with timeout
        let response = tokio::time::timeout(self.timeout, self.http_client.request(request))
            .await
            .map_err(|_| ProxyError::Timeout)?
            .map_err(|e| ProxyError::Connection(e.to_string()))?;

        Ok(response)
    }
}

impl Default for ForwarderClient {
    fn default() -> Self {
        Self::new(Duration::from_secs(30))
    }
}
