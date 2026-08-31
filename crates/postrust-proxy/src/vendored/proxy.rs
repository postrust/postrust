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
use hyper::server::conn::http1;
use hyper::service::service_fn;
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
                            let service = self.clone();
                            tokio::spawn(async move {
                                let service_fn = service_fn(|req| {
                                    let svc = service.clone();
                                    async move {
                                        svc.handle_request(req, client_addr, "http").await
                                    }
                                });

                                if let Err(err) = http1::Builder::new()
                                    .serve_connection(hyper_util::rt::TokioIo::new(stream), service_fn)
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
