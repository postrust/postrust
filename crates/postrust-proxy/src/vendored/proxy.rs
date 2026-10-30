//! Vendored proxy service from rpxy-lib: proxy/proxy_main.rs
//!
//! This module provides the main proxy service that handles incoming connections.

use crate::config::{ProxyConfig, RateLimitKey as RouteRateLimitKey, Route, ServerConfig};
use crate::health::HealthChecker;
use crate::ratelimit::{per_second, RateLimitKey, RateLimiter};
use crate::vendored::backend::BackendAppManager;
use crate::vendored::conn::{ConnActivity, IdleIo, InFlight};
use crate::vendored::forwarder::ForwarderClient;
use crate::vendored::handler::MessageHandler;
use crate::vendored::hyper_ext::timer::Timeout;
use crate::vendored::hyper_ext::{IncomingBodyExt, ProxyBody};
use crate::vendored::types::PathName;
use http_body_util::BodyExt;
use hyper::body::Incoming;
use hyper::server::conn::http2;
use hyper::{Request, Response};
use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
use hyper_util::server::conn::auto;
use std::convert::Infallible;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{RwLock, Semaphore};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

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
    /// One slot per open client connection, shared by every listener, sized
    /// by `server.max_connections` when the first listener starts.
    connection_slots: OnceLock<Arc<Semaphore>>,
    /// Whether the rate limiter's cleanup task has been started.
    cleanup_started: AtomicBool,
}

/// The `server` settings a listener applies to each connection, read once when
/// it starts.
#[derive(Clone, Debug)]
struct ListenerSettings {
    header_read_timeout: Duration,
    idle_timeout: Duration,
    http2_keepalive_interval: Option<Duration>,
    http2_keepalive_timeout: Duration,
}

impl ListenerSettings {
    fn from_server(server: &ServerConfig) -> Self {
        // A zero timeout is read as one second, not as "never": "never" is
        // the behaviour these exist to prevent, and nobody setting a timeout
        // means it.
        let at_least_a_second = |secs: u64| Duration::from_secs(secs.max(1));
        Self {
            header_read_timeout: at_least_a_second(server.header_read_timeout_secs),
            idle_timeout: at_least_a_second(server.idle_timeout_secs),
            http2_keepalive_interval: match server.http2_keepalive_interval_secs {
                0 => None,
                secs => Some(Duration::from_secs(secs)),
            },
            http2_keepalive_timeout: at_least_a_second(server.http2_keepalive_timeout_secs),
        }
    }

    /// The builder for a port that speaks HTTP/1.1 and HTTP/2 both.
    ///
    /// Both halves need a timer. hyper's timeouts are inert without one --
    /// `header_read_timeout` was configured by default and never fired, which
    /// is what let a client trickle a request head forever.
    fn auto_builder(&self) -> auto::Builder<TokioExecutor> {
        let mut builder = auto::Builder::new(TokioExecutor::new());
        builder
            .http1()
            .timer(TokioTimer::new())
            .header_read_timeout(self.header_read_timeout);
        // enable_connect_protocol advertises SETTINGS_ENABLE_CONNECT_PROTOCOL,
        // which is what permits WebSocket over HTTP/2 (RFC 8441).
        builder
            .http2()
            .timer(TokioTimer::new())
            .enable_connect_protocol()
            .keep_alive_interval(self.http2_keepalive_interval)
            .keep_alive_timeout(self.http2_keepalive_timeout);
        builder
    }

    /// The builder for a connection known to be HTTP/2.
    fn http2_builder(&self) -> http2::Builder<TokioExecutor> {
        let mut builder = http2::Builder::new(TokioExecutor::new());
        builder
            .timer(TokioTimer::new())
            .enable_connect_protocol()
            .keep_alive_interval(self.http2_keepalive_interval)
            .keep_alive_timeout(self.http2_keepalive_timeout);
        builder
    }
}

/// The hyper service for one client connection.
///
/// Marks each request as in flight on the connection from the moment hyper
/// hands it over until its response body has been sent, which is what keeps
/// the connection's idle timeout from firing under a request that is merely
/// slow. See [`crate::vendored::conn`].
#[derive(Clone)]
struct ConnectionService {
    service: Arc<ProxyService>,
    activity: Arc<ConnActivity>,
    client_addr: SocketAddr,
    proto: &'static str,
}

impl hyper::service::Service<Request<Incoming>> for ConnectionService {
    type Response = Response<ProxyBody>;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn call(&self, mut request: Request<Incoming>) -> Self::Future {
        let in_flight = InFlight::new(&self.activity);
        // A copy rides on the request, for a tunnel to keep: see
        // ForwarderClient::forward_upgrade.
        request.extensions_mut().insert(in_flight.clone());
        let service = self.service.clone();
        let client_addr = self.client_addr;
        let proto = self.proto;
        Box::pin(async move {
            let response = service.handle_request(request, client_addr, proto).await?;
            Ok(response.map(|body| {
                body.map_frame(move |frame| {
                    let _held_until_the_body_is_dropped = &in_flight;
                    frame
                })
                .boxed()
            }))
        })
    }
}

/// A rate limit's scope: the route it is declared on.
///
/// The database id when there is one; otherwise derived from the name, which
/// is unique across the configuration (see `merge_from_database`).
fn route_scope(route: &Route) -> Uuid {
    route.id.unwrap_or_else(|| {
        Uuid::new_v5(
            &Uuid::NAMESPACE_OID,
            format!("postrust-proxy route {}", route.name).as_bytes(),
        )
    })
}

impl ProxyService {
    /// Create a new proxy service.
    ///
    /// The `server` settings -- connection limit, timeouts -- are read from
    /// `config` when each listener starts.
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
            connection_slots: OnceLock::new(),
            cleanup_started: AtomicBool::new(false),
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
        self.serve_http_listener(listener, cancel_token).await
    }

    /// [`serve_http`](Self::serve_http) on a listener that is already bound --
    /// to port 0, say, when the caller needs to know which port it got.
    pub async fn serve_http_listener(
        self: Arc<Self>,
        listener: TcpListener,
        cancel_token: CancellationToken,
    ) -> std::io::Result<()> {
        self.run_listener(
            listener,
            cancel_token,
            "HTTP proxy",
            |service, io, activity, client_addr, settings| async move {
                let conn = ConnectionService {
                    service,
                    activity,
                    client_addr,
                    proto: "http",
                };
                // The auto builder negotiates HTTP/1.1 and prior-knowledge h2c
                // on the same port, and the _with_upgrades variant is what lets
                // a 101 hand the connection back to us for tunnelling.
                if let Err(err) = settings
                    .auto_builder()
                    .serve_connection_with_upgrades(TokioIo::new(io), conn)
                    .await
                {
                    debug!("Connection error: {}", err);
                }
            },
        )
        .await
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
        let listener = TcpListener::bind(addr).await?;
        info!("HTTPS proxy listening on {} (ALPN: h2, http/1.1)", addr);
        self.serve_https_listener(listener, tls_config, cancel_token)
            .await
    }

    /// [`serve_https`](Self::serve_https) on a listener that is already bound.
    pub async fn serve_https_listener(
        self: Arc<Self>,
        listener: TcpListener,
        tls_config: Arc<tokio_rustls::rustls::ServerConfig>,
        cancel_token: CancellationToken,
    ) -> std::io::Result<()> {
        let acceptor = tokio_rustls::TlsAcceptor::from(tls_config);
        self.run_listener(
            listener,
            cancel_token,
            "HTTPS proxy",
            move |service, io, activity, client_addr, settings| {
                let acceptor = acceptor.clone();
                async move {
                    // The handshake runs on the idle-timed socket, so a client
                    // that opens a connection and never finishes the handshake
                    // is dropped like any other idle one.
                    let tls_stream = match acceptor.accept(io).await {
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

                    let conn = ConnectionService {
                        service,
                        activity,
                        client_addr,
                        proto: "https",
                    };

                    let io = TokioIo::new(tls_stream);
                    let result = if is_h2 {
                        settings
                            .http2_builder()
                            .serve_connection(io, conn)
                            .await
                            .map_err(|e| {
                                Box::<dyn std::error::Error + Send + Sync>::from(e.to_string())
                            })
                    } else {
                        // http/1.1, or a client that offered no ALPN at all.
                        // Upgrades stay available, which is what makes wss://
                        // work.
                        settings
                            .auto_builder()
                            .serve_connection_with_upgrades(io, conn)
                            .await
                            .map_err(|e| {
                                Box::<dyn std::error::Error + Send + Sync>::from(e.to_string())
                            })
                    };

                    if let Err(err) = result {
                        debug!("HTTPS connection error: {}", err);
                    }
                }
            },
        )
        .await
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
        self.serve_h2c_listener(listener, cancel_token).await
    }

    /// [`serve_h2c`](Self::serve_h2c) on a listener that is already bound.
    pub async fn serve_h2c_listener(
        self: Arc<Self>,
        listener: TcpListener,
        cancel_token: CancellationToken,
    ) -> std::io::Result<()> {
        self.run_listener(
            listener,
            cancel_token,
            "HTTP/2-only proxy",
            |service, io, activity, client_addr, settings| async move {
                let conn = ConnectionService {
                    service,
                    activity,
                    client_addr,
                    proto: "http",
                };
                if let Err(err) = settings
                    .http2_builder()
                    .serve_connection(TokioIo::new(io), conn)
                    .await
                {
                    debug!("HTTP/2 connection error: {}", err);
                }
            },
        )
        .await
    }

    /// The accept loop every listener shares.
    ///
    /// A connection slot is taken *before* accepting, so at
    /// `server.max_connections` the proxy simply stops accepting and new
    /// connections queue in the kernel's backlog, rather than being accepted
    /// only to be dropped. The slot travels with the socket (see
    /// [`ConnActivity`]) and is released when the socket closes, including
    /// after an upgrade has handed it to a tunnel.
    async fn run_listener<F, Fut>(
        self: Arc<Self>,
        listener: TcpListener,
        cancel_token: CancellationToken,
        what: &'static str,
        serve: F,
    ) -> std::io::Result<()>
    where
        F: Fn(Arc<Self>, IdleIo<TcpStream>, Arc<ConnActivity>, SocketAddr, ListenerSettings) -> Fut,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let (settings, max_connections) = {
            let config = self.config.read().await;
            (
                ListenerSettings::from_server(&config.server),
                config.server.max_connections,
            )
        };
        let slots = self
            .connection_slots
            .get_or_init(|| {
                Arc::new(Semaphore::new(
                    max_connections.clamp(1, Semaphore::MAX_PERMITS),
                ))
            })
            .clone();
        self.start_background_tasks(&cancel_token);

        loop {
            let slot = tokio::select! {
                _ = cancel_token.cancelled() => break,
                slot = slots.clone().acquire_owned() => match slot {
                    Ok(slot) => slot,
                    // Only a closed semaphore fails, and this one never is.
                    Err(_) => break,
                },
            };

            let accepted = tokio::select! {
                _ = cancel_token.cancelled() => break,
                accepted = listener.accept() => accepted,
            };

            match accepted {
                Ok((stream, client_addr)) => {
                    // Same reasoning as the upstream leg: relay the client's
                    // own chunking rather than Nagle's.
                    if let Err(e) = stream.set_nodelay(true) {
                        debug!("Could not set TCP_NODELAY on the client socket: {}", e);
                    }
                    let activity = ConnActivity::new(Some(slot));
                    let io = IdleIo::new(stream, activity.clone(), settings.idle_timeout);
                    tokio::spawn(serve(
                        self.clone(),
                        io,
                        activity,
                        client_addr,
                        settings.clone(),
                    ));
                }
                Err(e) => {
                    // Typically EMFILE. Retrying at once would spin a core on
                    // an error that will not clear until something closes, so
                    // back off briefly.
                    error!("Accept error: {}", e);
                    drop(slot);
                    tokio::select! {
                        _ = cancel_token.cancelled() => break,
                        _ = tokio::time::sleep(Duration::from_millis(100)) => {}
                    }
                }
            }
        }

        info!("{} stopped", what);
        Ok(())
    }

    /// Start what serving depends on, once, whichever listener starts first.
    ///
    /// The rate limiter's cleanup is here rather than left to the caller
    /// because nothing called it: every client that was ever rate limited
    /// stayed in memory for the life of the process.
    fn start_background_tasks(&self, cancel_token: &CancellationToken) {
        if !self.cleanup_started.swap(true, Ordering::AcqRel) {
            tokio::spawn(
                self.rate_limiter
                    .clone()
                    .start_cleanup(cancel_token.clone()),
            );
        }
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

        // Host, falling back to the URI authority. HTTP/2 has no Host header,
        // so reading only the header routed nothing for any host-matched route
        // over h2 or h2c. See MessageHandler::request_host.
        let host = MessageHandler::request_host(&request);
        let host = host.as_str();

        let path = uri.path();

        debug!("{} {} {} from {}", method, host, path, client_addr);

        // Before routing, since routing is what this protects.
        if let Some(reason) = MessageHandler::unsafe_path_reason(path) {
            debug!("Refusing {} from {}: {}", path, client_addr, reason);
            return Ok(MessageHandler::bad_request(reason));
        }

        // Find matching route and upstream
        let (route, upstream, server) = {
            let config = self.config.read().await;
            let server = (
                config.server.allowed_upgrade_protocols.clone(),
                config.server.trust_forwarded_headers,
                Duration::from_secs(config.server.tunnel_idle_timeout_secs.max(1)),
            );
            // The whole of RouteMatch, not just host and path prefix. The
            // previous filter chain ignored path_type, methods and headers,
            // each of which silently widened a route past what its author
            // wrote -- `path_type = "exact"` on /health also matched
            // /health-internal, and `methods = ["GET"]` accepted DELETE.
            let matched = config
                .routes
                .iter()
                .filter(|r| r.enabled)
                .filter(|r| {
                    r.match_
                        .matches(host, path, method.as_str(), request.headers())
                })
                .max_by_key(|r| {
                    let path_len = r.match_.path.as_ref().map(|p| p.len()).unwrap_or(0);
                    (r.priority, path_len)
                })
                .cloned();

            match matched {
                Some(r) => {
                    let upstream = config
                        .upstreams
                        .iter()
                        .find(|u| u.name == r.upstream)
                        .map(|u| (u.resolved_id(), u.enabled));
                    (Some(r), upstream, server)
                }
                None => (None, None, server),
            }
        };
        let (allowed_upgrades, trust_forwarded, tunnel_idle) = server;

        let route = match route {
            Some(r) => r,
            None => {
                debug!("No route found for {} {}", host, path);
                return Ok(MessageHandler::not_found());
            }
        };

        let upstream_id = match upstream {
            Some((id, true)) => id,
            Some((_, false)) => {
                debug!(
                    "Upstream '{}' for route {} is disabled",
                    route.upstream, route.name
                );
                return Ok(MessageHandler::service_unavailable("Upstream disabled"));
            }
            None => {
                warn!(
                    "Upstream '{}' not found for route {}",
                    route.upstream, route.name
                );
                return Ok(MessageHandler::service_unavailable("Upstream not found"));
            }
        };

        // Rate limiting, in this route's own buckets, by the key the route
        // asks for. It used to be one bucket per client IP for every route,
        // whatever `key` said, sized by whichever route was reached first.
        if let Some(ref rate_limit) = route.rate_limit {
            let scope = route_scope(&route);
            let by_ip = || RateLimitKey::client_ip(client_addr.ip());
            let key = match &rate_limit.key {
                RouteRateLimitKey::ClientIp => by_ip(),
                RouteRateLimitKey::Route => RateLimitKey::Route(scope),
                // A request without the header is counted by its address.
                // One shared bucket for every such request would let a single
                // anonymous client spend it for all the others.
                RouteRateLimitKey::Header(name) => {
                    let values: Vec<&[u8]> = request
                        .headers()
                        .get_all(name.as_str())
                        .iter()
                        .map(|v| v.as_bytes())
                        .collect();
                    if values.is_empty() {
                        by_ip()
                    } else {
                        RateLimitKey::header_value(&values.join(&b',')[..])
                    }
                }
            };
            let rate = per_second(rate_limit.requests, rate_limit.window_secs);
            if !self
                .rate_limiter
                .check_with_config(scope, key, rate, rate_limit.requests)
            {
                return Ok(MessageHandler::too_many_requests());
            }
        }

        // Select backend
        let backend = match self.backend_manager.select_backend(upstream_id, None) {
            Some(b) => b,
            None => {
                warn!("No enabled backend for route {}", route.name);
                return Ok(MessageHandler::service_unavailable("No backend available"));
            }
        };

        // Build forwarded request
        let (parts, body) = request.into_parts();
        let mut forwarded_request = Request::from_parts(parts, body.boxed_body());

        // An upgrade request keeps its Connection/Upgrade pair, but only after the
        // strip has removed everything else -- see restore_upgrade_headers.
        //
        // Only for an allowed protocol. Anything else is forwarded as an
        // ordinary request, with the strip below removing the upgrade.
        let upgrade_protocol = MessageHandler::upgrade_protocol(&forwarded_request).filter(|p| {
            let allowed = MessageHandler::upgrade_allowed(p, &allowed_upgrades);
            if !allowed {
                debug!("Declining an upgrade to {:?} from {}", p, client_addr);
            }
            allowed
        });
        // The HTTP/2 spelling of the same intent (RFC 8441), which carries no
        // Upgrade header at all and so survives the strip untouched. It cannot
        // be declined by forwarding it as something else -- a CONNECT is not
        // an ordinary request -- so a protocol not allowed is refused.
        let h2_protocol = MessageHandler::h2_connect_protocol(&forwarded_request);
        if let Some(ref protocol) = h2_protocol {
            if !MessageHandler::upgrade_allowed(protocol, &allowed_upgrades) {
                debug!(
                    "Refusing extended CONNECT for {:?} from {}",
                    protocol, client_addr
                );
                return Ok(MessageHandler::not_implemented(
                    "Extended CONNECT protocol not supported",
                ));
            }
        }

        // Drop hop-by-hop headers before anything else, so a client cannot name
        // our own X-Forwarded-* headers in Connection to suppress them.
        MessageHandler::strip_hop_by_hop_headers(&mut forwarded_request);

        // Add forwarding headers
        MessageHandler::add_forwarding_headers(
            &mut forwarded_request,
            client_addr,
            proto,
            trust_forwarded,
        );

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
                .forward_h2_websocket(&backend, forwarded_request, &protocol, tunnel_idle)
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
            debug!(
                "Upgrading connection to {} via {}",
                protocol, backend.address
            );
            return match self
                .forwarder
                .forward_upgrade(&backend, forwarded_request, tunnel_idle)
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

        // Forward request, under the route's timeout.
        //
        // `route.timeout_secs` was declarable and unread: an upstream that
        // accepted a connection and then never answered held the request open
        // for as long as it liked, and the timer machinery this uses was sitting
        // in hyper_ext with nothing calling it.
        //
        // 504 rather than 502: the distinction is the difference between "the
        // upstream refused" and "the upstream is not answering", and an operator
        // reads them differently.
        //
        // The in-flight marker is the listener's, and only a tunnel needs it;
        // it has no business travelling into the upstream client.
        forwarded_request.extensions_mut().remove::<InFlight>();
        let timeout = Duration::from_secs(u64::from(route.timeout_secs));
        let forwarded = Timeout::new(self.forwarder.forward(&backend, forwarded_request), timeout);

        match forwarded.await {
            Ok(Ok(response)) => {
                let (parts, body) = response.into_parts();
                Ok(Response::from_parts(parts, body.boxed_body()))
            }
            Ok(Err(e)) => {
                error!("Forward error to {}: {}", backend.address, e);
                Ok(MessageHandler::bad_gateway(&format!(
                    "Backend error: {}",
                    e
                )))
            }
            Err(_) => {
                error!(
                    "Upstream {} did not answer within {}s (route {})",
                    backend.address, route.timeout_secs, route.name
                );
                Ok(MessageHandler::gateway_timeout(&format!(
                    "Upstream did not answer within {}s",
                    route.timeout_secs
                )))
            }
        }
    }
}
