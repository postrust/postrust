//! Vendored proxy service from rpxy-lib: proxy/proxy_main.rs
//!
//! This module provides the main proxy service that handles incoming connections.

use crate::config::ProxyConfig;
use crate::health::HealthChecker;
use crate::ratelimit::{RateLimitKey, RateLimiter};
use crate::vendored::backend::BackendAppManager;
use crate::vendored::forwarder::ForwarderClient;
use crate::vendored::handler::MessageHandler;
use crate::vendored::hyper_ext::{IncomingBodyExt, ProxyBody};
use crate::vendored::types::PathName;
use hyper::body::Incoming;
use hyper::server::conn::http2;
use hyper::service::service_fn;
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto;
use hyper::{Request, Response};
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

/// Proxy service that handles incoming HTTP connections.
pub struct ProxyService {
    /// Backend manager
    backend_manager: Arc<BackendAppManager>,
    /// Forwarder client
    forwarder: Arc<ForwarderClient>,
    /// Rate limiter
    rate_limiter: Arc<RateLimiter>,
    /// Configuration
    config: Arc<RwLock<ProxyConfig>>,
}

impl ProxyService {
    /// Create a new proxy service.
    pub fn new(
        config: Arc<RwLock<ProxyConfig>>,
        health_checker: Arc<HealthChecker>,
        rate_limiter: Arc<RateLimiter>,
    ) -> Self {
        let backend_manager =
            Arc::new(BackendAppManager::new().with_health_checker(health_checker));
        let forwarder = Arc::new(ForwarderClient::default());

        Self {
            backend_manager,
            forwarder,
            rate_limiter,
            config,
        }
    }

    /// Load configuration into the backend manager.
    pub async fn load_config(&self) {
        let config = self.config.read().await;

        // Register upstreams
        for upstream in &config.upstreams {
            self.backend_manager.register_upstream(upstream.clone());
        }

        // Register routes
        for route in &config.routes {
            // Find upstream by name
            if let Some(upstream) = config.upstreams.iter().find(|u| u.name == route.upstream) {
                use crate::vendored::types::ServerName;
                let host = route.match_.host.as_deref().unwrap_or("*");
                let path = route.match_.path.as_deref().unwrap_or("/");
                let host = ServerName::new(host);
                let path = PathName::new(path);
                self.backend_manager
                    .register_route(host, path, upstream.resolved_id());
            }
        }

        info!(
            "Loaded {} routes and {} upstreams",
            config.routes.len(),
            config.upstreams.len()
        );
    }

    /// Start the HTTP proxy server.
    pub async fn serve_http(
        self: Arc<Self>,
        addr: SocketAddr,
        cancel_token: CancellationToken,
    ) -> std::io::Result<()> {
        let listener = TcpListener::bind(addr).await?;
        info!("HTTP proxy listening on {}", addr);

        loop {
            tokio::select! {
                _ = cancel_token.cancelled() => {
                    info!("HTTP proxy stopped");
                    break;
                }
                result = listener.accept() => {
                    match result {
                        Ok((stream, client_addr)) => {
                            // Same reasoning as the upstream leg: relay the
                            // client's own chunking rather than Nagle's.
                            if let Err(e) = stream.set_nodelay(true) {
                                debug!("Could not set TCP_NODELAY on the client socket: {}", e);
                            }
                            let service = self.clone();
                            tokio::spawn(async move {
                                let service_fn = service_fn(|req| {
                                    let svc = service.clone();
                                    async move {
                                        svc.handle_request(req, client_addr, "http").await
                                    }
                                });

                                // The auto builder negotiates HTTP/1.1 and
                                // prior-knowledge h2c on the same port, and the
                                // _with_upgrades variant is what lets a 101 hand
                                // the connection back to us for tunnelling.
                                // enable_connect_protocol advertises
                                // SETTINGS_ENABLE_CONNECT_PROTOCOL, which is what
                                // permits WebSocket over HTTP/2 (RFC 8441).
                                let mut builder = auto::Builder::new(TokioExecutor::new());
                                builder.http2().enable_connect_protocol();
                                if let Err(err) = builder
                                    .serve_connection_with_upgrades(
                                        TokioIo::new(stream),
                                        service_fn,
                                    )
                                    .await
                                {
                                    debug!("Connection error: {}", err);
                                }
                            });
                        }
                        Err(e) => {
                            error!("Accept error: {}", e);
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Serve HTTPS, negotiating the protocol with ALPN.
    ///
    /// ALPN is the whole point: without it a TLS listener negotiates nothing and
    /// every client falls back to HTTP/1.1, which leaves HTTP/2 reachable only
    /// as cleartext h2c and WebSocket only as `ws://`.
    ///
    /// Because ALPN *decides* the protocol rather than guessing it, the h2 path
    /// can use the HTTP/2 builder directly instead of sniffing -- so, unlike the
    /// cleartext port, an invalid preface here is answered with a proper GOAWAY.
    pub async fn serve_https(
        self: Arc<Self>,
        addr: SocketAddr,
        tls_config: Arc<tokio_rustls::rustls::ServerConfig>,
        cancel_token: CancellationToken,
    ) -> std::io::Result<()> {
        let acceptor = tokio_rustls::TlsAcceptor::from(tls_config);
        let listener = TcpListener::bind(addr).await?;
        info!("HTTPS proxy listening on {} (ALPN: h2, http/1.1)", addr);

        loop {
            tokio::select! {
                _ = cancel_token.cancelled() => {
                    info!("HTTPS proxy stopped");
                    break;
                }
                result = listener.accept() => {
                    match result {
                        Ok((stream, client_addr)) => {
                            if let Err(e) = stream.set_nodelay(true) {
                                debug!("Could not set TCP_NODELAY on the client socket: {}", e);
                            }
                            let service = self.clone();
                            let acceptor = acceptor.clone();
                            tokio::spawn(async move {
                                let tls_stream = match acceptor.accept(stream).await {
                                    Ok(s) => s,
                                    Err(e) => {
                                        debug!("TLS handshake failed: {}", e);
                                        return;
                                    }
                                };

                                let is_h2 = tls_stream
                                    .get_ref()
                                    .1
                                    .alpn_protocol()
                                    .map(|p| p == b"h2")
                                    .unwrap_or(false);

                                let service_fn = service_fn(|req| {
                                    let svc = service.clone();
                                    async move {
                                        svc.handle_request(req, client_addr, "https").await
                                    }
                                });

                                let io = TokioIo::new(tls_stream);
                                let result = if is_h2 {
                                    let mut builder = http2::Builder::new(TokioExecutor::new());
                                    builder.enable_connect_protocol();
                                    builder
                                        .serve_connection(io, service_fn)
                                        .await
                                        .map_err(|e| {
                                            Box::<dyn std::error::Error + Send + Sync>::from(
                                                e.to_string(),
                                            )
                                        })
                                } else {
                                    // http/1.1, or a client that offered no ALPN
                                    // at all. Upgrades stay available, which is
                                    // what makes wss:// work.
                                    let mut builder = auto::Builder::new(TokioExecutor::new());
                                    builder.http2().enable_connect_protocol();
                                    builder
                                        .serve_connection_with_upgrades(io, service_fn)
                                        .await
                                        .map_err(|e| {
                                            Box::<dyn std::error::Error + Send + Sync>::from(
                                                e.to_string(),
                                            )
                                        })
                                };

                                if let Err(err) = result {
                                    debug!("HTTPS connection error: {}", err);
                                }
                            });
                        }
                        Err(e) => {
                            error!("Accept error: {}", e);
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Serve HTTP/2 only, on a dedicated port.
    ///
    /// The main listener negotiates by sniffing the opening bytes, which is why
    /// a corrupted HTTP/2 preface there gets an HTTP/1 400 instead of
    /// GOAWAY(PROTOCOL_ERROR): it cannot be told apart from a malformed HTTP/1
    /// request (h2spec 3.5). A listener that only ever speaks HTTP/2 has no
    /// such ambiguity and answers the way the RFC asks.
    ///
    /// This is additive. The main port still serves HTTP/1.1 and h2c together;
    /// nothing is given up by enabling it.
    pub async fn serve_h2c(
        self: Arc<Self>,
        addr: SocketAddr,
        cancel_token: CancellationToken,
    ) -> std::io::Result<()> {
        let listener = TcpListener::bind(addr).await?;
        info!("HTTP/2-only proxy listening on {}", addr);

        loop {
            tokio::select! {
                _ = cancel_token.cancelled() => {
                    info!("HTTP/2-only proxy stopped");
                    break;
                }
                result = listener.accept() => {
                    match result {
                        Ok((stream, client_addr)) => {
                            if let Err(e) = stream.set_nodelay(true) {
                                debug!("Could not set TCP_NODELAY on the client socket: {}", e);
                            }
                            let service = self.clone();
                            tokio::spawn(async move {
                                let service_fn = service_fn(|req| {
                                    let svc = service.clone();
                                    async move {
                                        svc.handle_request(req, client_addr, "http").await
                                    }
                                });

                                let mut builder = http2::Builder::new(TokioExecutor::new());
                                builder.enable_connect_protocol();
                                if let Err(err) = builder
                                    .serve_connection(TokioIo::new(stream), service_fn)
                                    .await
                                {
                                    debug!("HTTP/2 connection error: {}", err);
                                }
                            });
                        }
                        Err(e) => {
                            error!("Accept error: {}", e);
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Handle a single HTTP request.
    async fn handle_request(
        &self,
        request: Request<Incoming>,
        client_addr: SocketAddr,
        proto: &str,
    ) -> Result<Response<ProxyBody>, Infallible> {
        let uri = request.uri().clone();
        let method = request.method().clone();

        // Extract host from request
        let host = request
            .headers()
            .get(hyper::header::HOST)
            .and_then(|h| h.to_str().ok())
            .map(|h| h.split(':').next().unwrap_or(h))
            .unwrap_or("");

        let path = uri.path();

        debug!("{} {} {} from {}", method, host, path, client_addr);

        // Find matching route and upstream
        let (route, upstream_id) = {
            let config = self.config.read().await;
            let matched = config
                .routes
                .iter()
                .filter(|r| r.enabled)
                .filter(|r| {
                    let route_host = r.match_.host.as_deref().unwrap_or("*");
                    route_host == "*" || route_host == host
                })
                .filter(|r| {
                    let route_path = r.match_.path.as_deref().unwrap_or("/");
                    path.starts_with(route_path)
                })
                .max_by_key(|r| {
                    let path_len = r.match_.path.as_ref().map(|p| p.len()).unwrap_or(0);
                    (r.priority, path_len)
                })
                .cloned();

            match matched {
                Some(r) => {
                    // Find upstream ID
                    let upstream_id = config
                        .upstreams
                        .iter()
                        .find(|u| u.name == r.upstream)
                        .map(|u| u.resolved_id());
                    (Some(r), upstream_id)
                }
                None => (None, None),
            }
        };

        let route = match route {
            Some(r) => r,
            None => {
                debug!("No route found for {} {}", host, path);
                return Ok(MessageHandler::not_found());
            }
        };

        let upstream_id = match upstream_id {
            Some(id) => id,
            None => {
                warn!(
                    "Upstream '{}' not found for route {}",
                    route.upstream, route.name
                );
                return Ok(MessageHandler::service_unavailable("Upstream not found"));
            }
        };

        // Rate limiting
        if let Some(ref rate_limit) = route.rate_limit {
            let key = RateLimitKey::Ip(client_addr.ip());
            // Convert requests/window to rps
            let rps = rate_limit
                .requests
                .checked_div(rate_limit.window_secs)
                .unwrap_or(rate_limit.requests);
            if !self
                .rate_limiter
                .check_with_config(key, rps, rate_limit.requests)
            {
                return Ok(MessageHandler::too_many_requests());
            }
        }

        // Select backend
        let backend = match self.backend_manager.select_backend(upstream_id, None) {
            Some(b) => b,
            None => {
                warn!("No healthy backends for route {}", route.name);
                return Ok(MessageHandler::service_unavailable(
                    "No healthy backends available",
                ));
            }
        };

        // Build forwarded request
        let (parts, body) = request.into_parts();
        let mut forwarded_request = Request::from_parts(parts, body.boxed_body());

        // Reject ambiguous body framing before the Transfer-Encoding that
        // signals it is stripped below.
        if MessageHandler::has_ambiguous_framing(&forwarded_request) {
            warn!("Rejecting request with both Content-Length and Transfer-Encoding");
            return Ok(MessageHandler::bad_request(
                "Both Content-Length and Transfer-Encoding present",
            ));
        }

        // An upgrade request keeps its Connection/Upgrade pair, but only after the
        // strip has removed everything else -- see restore_upgrade_headers.
        let upgrade_protocol = MessageHandler::upgrade_protocol(&forwarded_request);
        // The HTTP/2 spelling of the same intent (RFC 8441), which carries no
        // Upgrade header at all and so survives the strip untouched.
        let h2_protocol = MessageHandler::h2_connect_protocol(&forwarded_request);

        // Drop hop-by-hop headers before anything else, so a client cannot name
        // our own X-Forwarded-* headers in Connection to suppress them.
        MessageHandler::strip_hop_by_hop_headers(&mut forwarded_request);

        // Add forwarding headers
        MessageHandler::add_forwarding_headers(&mut forwarded_request, client_addr, proto);

        // Strip path prefix if configured
        if route.strip_path {
            if let Some(ref prefix) = route.match_.path {
                MessageHandler::strip_path_prefix(&mut forwarded_request, prefix);
            }
        }

        // Apply route headers
        MessageHandler::apply_route_headers(&mut forwarded_request, &route);

        // Rewrite host header
        MessageHandler::rewrite_host_header(&mut forwarded_request, &backend.address);

        // WebSocket over HTTP/2: translate to an HTTP/1.1 upgrade for the origin.
        if let Some(protocol) = h2_protocol {
            debug!(
                "Extended CONNECT to {} via {} (RFC 8441)",
                protocol, backend.address
            );
            return match self
                .forwarder
                .forward_h2_websocket(&backend, forwarded_request, &protocol)
                .await
            {
                Ok(response) => Ok(response),
                Err(e) => {
                    error!("Extended CONNECT error to {}: {}", backend.address, e);
                    Ok(MessageHandler::bad_gateway(&format!(
                        "Extended CONNECT error: {}",
                        e
                    )))
                }
            };
        }

        // Tunnel an upgrade rather than treating it as request/response.
        if let Some(protocol) = upgrade_protocol {
            MessageHandler::restore_upgrade_headers(&mut forwarded_request, &protocol);
            debug!("Upgrading connection to {} via {}", protocol, backend.address);
            return match self
                .forwarder
                .forward_upgrade(&backend, forwarded_request)
                .await
            {
                Ok(response) => Ok(response),
                Err(e) => {
                    error!("Upgrade error to {}: {}", backend.address, e);
                    Ok(MessageHandler::bad_gateway(&format!(
                        "Upgrade error: {}",
                        e
                    )))
                }
            };
        }

        // Forward request
        match self.forwarder.forward(&backend, forwarded_request).await {
            Ok(response) => {
                let (parts, body) = response.into_parts();
                Ok(Response::from_parts(parts, body.boxed_body()))
            }
            Err(e) => {
                error!("Forward error to {}: {}", backend.address, e);
                Ok(MessageHandler::bad_gateway(&format!(
                    "Backend error: {}",
                    e
                )))
            }
        }
    }
}
