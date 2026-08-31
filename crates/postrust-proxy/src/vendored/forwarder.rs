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
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
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
        let mut http_connector = HttpConnector::new();
        // Without this, Nagle batches small writes and, paired with delayed ACK
        // on the far side, adds latency to every small forwarded request.
        http_connector.set_nodelay(true);
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

        // A tunnel relays whatever chunking the client used. With Nagle on, small
        // writes coalesce and the upstream sees frames batched together that the
        // client sent separately -- which changes what the upstream does, not just
        // when it does it. Autobahn 3.4 is exactly that: the client sends a text
        // frame and then a bad frame in one-byte chops, and a batched upstream
        // fails the connection without ever echoing the first.
        if let Err(e) = stream.set_nodelay(true) {
            debug!("Could not set TCP_NODELAY on the upstream socket: {}", e);
        }

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
                Ok((client_io, upstream_io)) => Self::splice(
                    TokioIo::new(client_io),
                    TokioIo::new(upstream_io),
                )
                .await,
                Err(e) => debug!("Upgrade handshake failed: {}", e),
            }
        });

        // Hand the 101 and its headers back; the body is the tunnel, not content.
        let (parts, _) = response.into_parts();
        Ok(Response::from_parts(parts, empty_body()))
    }

    /// Splice two upgraded streams, draining both directions independently.
    ///
    /// Deliberately not `copy_bidirectional`, which returns on the first error
    /// in either direction and drops the other mid-flight. That loses data in a
    /// real and reachable case: the upstream fails the connection and closes
    /// while the client is still writing, the client-to-upstream copy takes an
    /// EPIPE, and whatever the upstream had already sent -- its last frames and
    /// its close -- is discarded instead of reaching the client. Autobahn case
    /// 3.4 caught exactly that, intermittently, because it is a race.
    ///
    /// Each direction here runs to its own end and half-closes the peer's write
    /// side so the other side sees a clean EOF. `join!` rather than `try_join!`
    /// so that one direction failing cannot cancel the other.
    async fn splice<A, B>(client_io: A, upstream_io: B)
    where
        A: AsyncRead + AsyncWrite + Unpin,
        B: AsyncRead + AsyncWrite + Unpin,
    {
        let (mut client_read, mut client_write) = tokio::io::split(client_io);
        let (mut upstream_read, mut upstream_write) = tokio::io::split(upstream_io);

        let to_upstream = async {
            let result = tokio::io::copy(&mut client_read, &mut upstream_write).await;
            // Half-close: tell the upstream the client is done, without tearing
            // down the half still carrying its reply.
            let _ = upstream_write.shutdown().await;
            result
        };

        let to_client = async {
            let result = tokio::io::copy(&mut upstream_read, &mut client_write).await;
            let _ = client_write.shutdown().await;
            result
        };

        let (up, down) = tokio::join!(to_upstream, to_client);

        match (up, down) {
            (Ok(up), Ok(down)) => {
                debug!("Tunnel closed: {} bytes up, {} bytes down", up, down)
            }
            (up, down) => {
                // One side erroring is normal for an abrupt close; log both
                // outcomes rather than only the first failure.
                debug!("Tunnel closed with errors: up={:?} down={:?}", up, down)
            }
        }
    }
}

impl Default for ForwarderClient {
    fn default() -> Self {
        Self::new(Duration::from_secs(30))
    }
}
