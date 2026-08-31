//! Vendored forwarder client from rpxy-lib: forwarder/client.rs
//!
//! This module handles forwarding requests to upstream backends.

use crate::config::{Backend, UpstreamHttpVersion};
use crate::vendored::hyper_ext::{empty_body, IncomingBodyExt, ProxyBody};
use crate::vendored::types::ProxyError;
use hyper::body::Incoming;
use hyper::header::{CONNECTION, HOST, UPGRADE};
use hyper::{Method, Request, Response, StatusCode, Version};
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor as HyperTokioExecutor;
use hyper_util::rt::TokioIo;
use std::time::Duration;
use base64::Engine;
use rand::RngCore;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tracing::debug;

/// HTTP connector type alias.
type HttpConnector = hyper_util::client::legacy::connect::HttpConnector;

/// Forwarder client for sending requests to upstream backends.
pub struct ForwarderClient {
    /// HTTP/1.1 client, used for every backend that has not opted into h2c.
    http_client: Client<HttpConnector, ProxyBody>,
    /// Prior-knowledge h2c client, for backends declared as `http_version = "h2c"`.
    ///
    /// A separate client because the choice is per-connection and h2c has no
    /// ALPN to negotiate it -- the pool has to be told up front which protocol
    /// its connections speak.
    http2_client: Client<HttpConnector, ProxyBody>,
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

        let mut http2_connector = HttpConnector::new();
        http2_connector.set_nodelay(true);
        let http2_client = Client::builder(HyperTokioExecutor::new())
            .pool_idle_timeout(Duration::from_secs(30))
            .http2_only(true)
            .build(http2_connector);

        Self {
            http_client,
            http2_client,
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

        // HTTP/2 is a per-hop protocol, so the inbound version never carries onto
        // the upstream connection: it is whatever this backend was declared to
        // speak. Getting this wrong is not subtle -- the client rejects the
        // request as UserUnsupportedVersion before a byte goes out.
        let (client, version) = match backend.http_version {
            UpstreamHttpVersion::H2c => (&self.http2_client, Version::HTTP_2),
            UpstreamHttpVersion::Http11 => (&self.http_client, Version::HTTP_11),
        };
        *request.version_mut() = version;

        // Send request with timeout
        let response = tokio::time::timeout(self.timeout, client.request(request))
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

    /// Forward an HTTP/2 extended CONNECT (RFC 8441) to an HTTP/1.1 WebSocket
    /// origin, translating between the two handshakes.
    ///
    /// The two are not the same conversation, so this is a rewrite rather than
    /// a relay:
    ///
    /// - HTTP/2 carries the intent in `:method = CONNECT` and `:protocol`;
    ///   HTTP/1.1 carries it in `Upgrade` plus a `Connection` token.
    /// - HTTP/2 has no `Sec-WebSocket-Key`. The nonce exists to stop an HTTP/1.1
    ///   cache from mistaking the handshake for an ordinary response, which does
    ///   not apply to a multiplexed stream. The h1 origin still requires one, so
    ///   the proxy generates it here.
    /// - Success is `101` on the wire to the origin but `200` back to the
    ///   client; a 101 has no meaning on an HTTP/2 stream.
    ///
    /// The origin's `Sec-WebSocket-Accept` is not verified against the generated
    /// key. Status 101 plus a websocket `Upgrade` is taken as sufficient, which
    /// avoids a SHA-1 dependency for a check that guards against a
    /// misconfigured origin rather than an attacker.
    pub async fn forward_h2_websocket(
        &self,
        backend: &Backend,
        mut request: Request<ProxyBody>,
        protocol: &str,
    ) -> Result<Response<ProxyBody>, ProxyError> {
        let client_upgrade = hyper::upgrade::on(&mut request);

        let path_and_query = request
            .uri()
            .path_and_query()
            .map(|pq| pq.as_str().to_string())
            .unwrap_or_else(|| "/".to_string());

        // A fresh nonce for the HTTP/1.1 leg, which the HTTP/2 client never sent.
        let mut nonce = [0u8; 16];
        rand::rng().fill_bytes(&mut nonce);
        let websocket_key = base64::engine::general_purpose::STANDARD.encode(nonce);

        let mut builder = Request::builder()
            .method(Method::GET)
            .uri(&path_and_query)
            .version(Version::HTTP_11)
            .header(HOST, &backend.address)
            .header(UPGRADE, protocol)
            .header(CONNECTION, "upgrade")
            .header("sec-websocket-version", "13")
            .header("sec-websocket-key", &websocket_key);

        // Carry the client's own headers over. The handshake fields above are
        // ours to set, so skip any the client happened to send.
        const REWRITTEN: [&str; 5] = [
            "host",
            "upgrade",
            "connection",
            "sec-websocket-key",
            "sec-websocket-version",
        ];
        for (name, value) in request.headers() {
            if !REWRITTEN.contains(&name.as_str()) {
                builder = builder.header(name, value);
            }
        }

        let upstream_request = builder
            .body(empty_body())
            .map_err(|e| ProxyError::Request(format!("could not build upgrade request: {}", e)))?;

        let stream = tokio::time::timeout(self.timeout, TcpStream::connect(&backend.address))
            .await
            .map_err(|_| ProxyError::Timeout)?
            .map_err(|e| ProxyError::Connection(e.to_string()))?;
        if let Err(e) = stream.set_nodelay(true) {
            debug!("Could not set TCP_NODELAY on the upstream socket: {}", e);
        }

        let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
            .await
            .map_err(|e| ProxyError::Connection(e.to_string()))?;
        tokio::spawn(async move {
            if let Err(e) = conn.with_upgrades().await {
                debug!("Upstream upgrade connection error: {}", e);
            }
        });

        let mut response = tokio::time::timeout(self.timeout, sender.send_request(upstream_request))
            .await
            .map_err(|_| ProxyError::Timeout)?
            .map_err(|e| ProxyError::Connection(e.to_string()))?;

        if response.status() != StatusCode::SWITCHING_PROTOCOLS {
            // The origin declined. Pass its answer back on the HTTP/2 stream,
            // minus the 101 semantics that do not translate.
            let status = response.status();
            let mut out = Response::builder().status(status);
            for (name, value) in response.headers() {
                if name != UPGRADE && name != CONNECTION {
                    out = out.header(name, value);
                }
            }
            return out
                .body(empty_body())
                .map_err(|e| ProxyError::Response(e.to_string()));
        }

        let upstream_upgrade = hyper::upgrade::on(&mut response);

        tokio::spawn(async move {
            match tokio::try_join!(client_upgrade, upstream_upgrade) {
                Ok((client_io, upstream_io)) => {
                    Self::splice(TokioIo::new(client_io), TokioIo::new(upstream_io)).await
                }
                Err(e) => debug!("RFC 8441 upgrade handshake failed: {}", e),
            }
        });

        // 200, not 101: on an HTTP/2 stream a 2xx is what completes an extended
        // CONNECT. Carry across only what the client still needs to know.
        let mut out = Response::builder().status(StatusCode::OK);
        for name in ["sec-websocket-protocol", "sec-websocket-extensions"] {
            if let Some(value) = response.headers().get(name) {
                out = out.header(name, value);
            }
        }
        out.body(empty_body())
            .map_err(|e| ProxyError::Response(e.to_string()))
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
