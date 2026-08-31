//! Vendored forwarder client from rpxy-lib: forwarder/client.rs
//!
//! This module handles forwarding requests to upstream backends.

use crate::config::Backend;
use crate::vendored::hyper_ext::{empty_body, IncomingBodyExt, ProxyBody};
use crate::vendored::types::ProxyError;
use hyper::body::Incoming;
use hyper::{Request, Response, StatusCode, Version};
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor as HyperTokioExecutor;
use hyper_util::rt::TokioIo;
use std::time::Duration;
use tokio::net::TcpStream;
use tracing::debug;

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
        let path_and_query = uri.path_and_query().map(|pq| pq.as_str()).unwrap_or("/");

        let upstream_uri = format!("{}://{}{}", backend.scheme, backend.address, path_and_query);

        // Update request URI
        *request.uri_mut() = upstream_uri
            .parse()
            .map_err(|e| ProxyError::Request(format!("Invalid upstream URI: {}", e)))?;

        // HTTP/2 is a per-hop protocol: an inbound h2c request must not carry its
        // version onto an HTTP/1.1 upstream connection, or the client rejects it
        // as UserUnsupportedVersion before a byte is sent.
        *request.version_mut() = Version::HTTP_11;

        // Send request with timeout
        let response = tokio::time::timeout(self.timeout, self.http_client.request(request))
            .await
            .map_err(|_| ProxyError::Timeout)?
            .map_err(|e| ProxyError::Connection(e.to_string()))?;

        Ok(response)
    }

    /// Forward a protocol-upgrade request (WebSocket) and tunnel the result.
    ///
    /// This cannot use the pooled client: once the upstream answers 101 the
    /// connection stops being request/response and becomes an opaque byte
    /// stream, which a connection pool has no way to hand back. So dial the
    /// backend directly, keep the lower-level connection, and splice the two
    /// upgraded halves together.
    ///
    /// HTTP/1.1 only. WebSocket over HTTP/2 is RFC 8441 extended CONNECT, a
    /// different mechanism that this does not implement.
    pub async fn forward_upgrade(
        &self,
        backend: &Backend,
        mut request: Request<ProxyBody>,
    ) -> Result<Response<ProxyBody>, ProxyError> {
        // Claim the client half of the upgrade before the request is consumed.
        // This also takes the OnUpgrade extension out, so it is not forwarded.
        let client_upgrade = hyper::upgrade::on(&mut request);

        // The low-level h1 sender wants origin-form; Host carries the authority.
        let path_and_query = request
            .uri()
            .path_and_query()
            .map(|pq| pq.as_str().to_string())
            .unwrap_or_else(|| "/".to_string());
        *request.uri_mut() = path_and_query
            .parse()
            .map_err(|e| ProxyError::Request(format!("Invalid upstream URI: {}", e)))?;
        *request.version_mut() = Version::HTTP_11;

        let stream = tokio::time::timeout(self.timeout, TcpStream::connect(&backend.address))
            .await
            .map_err(|_| ProxyError::Timeout)?
            .map_err(|e| ProxyError::Connection(e.to_string()))?;

        let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
            .await
            .map_err(|e| ProxyError::Connection(e.to_string()))?;

        // with_upgrades keeps the connection task alive past the 101 so the
        // upgraded stream is actually reachable.
        tokio::spawn(async move {
            if let Err(e) = conn.with_upgrades().await {
                debug!("Upstream upgrade connection error: {}", e);
            }
        });

        let mut response = tokio::time::timeout(self.timeout, sender.send_request(request))
            .await
            .map_err(|_| ProxyError::Timeout)?
            .map_err(|e| ProxyError::Connection(e.to_string()))?;

        // The upstream declined to upgrade: pass its answer straight through.
        if response.status() != StatusCode::SWITCHING_PROTOCOLS {
            let (parts, body) = response.into_parts();
            return Ok(Response::from_parts(parts, body.boxed_body()));
        }

        let upstream_upgrade = hyper::upgrade::on(&mut response);

        // Both halves only resolve after their 101 has gone out, so the splice
        // has to outlive this call.
        tokio::spawn(async move {
            match tokio::try_join!(client_upgrade, upstream_upgrade) {
                Ok((client_io, upstream_io)) => {
                    let mut client_io = TokioIo::new(client_io);
                    let mut upstream_io = TokioIo::new(upstream_io);
                    match tokio::io::copy_bidirectional(&mut client_io, &mut upstream_io).await {
                        Ok((from_client, from_upstream)) => debug!(
                            "Tunnel closed: {} bytes up, {} bytes down",
                            from_client, from_upstream
                        ),
                        Err(e) => debug!("Tunnel error: {}", e),
                    }
                }
                Err(e) => debug!("Upgrade handshake failed: {}", e),
            }
        });

        // Hand the 101 and its headers back; the body is the tunnel, not content.
        let (parts, _) = response.into_parts();
        Ok(Response::from_parts(parts, empty_body()))
    }
}

impl Default for ForwarderClient {
    fn default() -> Self {
        Self::new(Duration::from_secs(30))
    }
}
