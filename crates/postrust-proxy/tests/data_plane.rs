//! The proxy as a client sees it: a real listener on 127.0.0.1, driven with raw
//! TCP so that each test controls exactly which bytes are sent and when.
//!
//! Raw sockets rather than an HTTP client because most of what these cover is
//! what a well-behaved client would never do -- stop halfway through a request
//! head, open a connection and say nothing, ask to upgrade to h2c, send a path
//! with `..` in it -- and a client library would either refuse or tidy it up.
//!
//! The backend is a raw socket too: it answers every request with the request
//! head it received, which is how the tests see what the proxy forwarded.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use postrust_proxy::config::ProxyConfig;
use postrust_proxy::health::HealthChecker;
use postrust_proxy::ratelimit::RateLimiter;
use postrust_proxy::vendored::ProxyService;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

/// Read up to the end of a message head; `None` on EOF before one arrived.
async fn read_head<S: AsyncRead + Unpin>(stream: &mut S) -> Option<String> {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        match stream.read(&mut byte).await {
            Ok(1) => head.push(byte[0]),
            _ => return None,
        }
    }
    Some(String::from_utf8_lossy(&head).into_owned())
}

fn content_length(head: &str) -> usize {
    head.lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.trim()
                .eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse().ok())?
        })
        .unwrap_or(0)
}

/// A backend that echoes each request head as the response body, and accepts
/// a WebSocket upgrade by answering 101 and then holding the connection.
async fn backend() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                while let Some(head) = read_head(&mut stream).await {
                    let mut body = vec![0u8; content_length(&head)];
                    if stream.read_exact(&mut body).await.is_err() {
                        return;
                    }
                    if head.to_ascii_lowercase().contains("upgrade: websocket") {
                        let _ = stream
                            .write_all(
                                b"HTTP/1.1 101 Switching Protocols\r\n\
                                  Upgrade: websocket\r\nConnection: Upgrade\r\n\r\n",
                            )
                            .await;
                        // Hold the tunnel open, silently, until the proxy
                        // closes it.
                        let mut sink = Vec::new();
                        let _ = stream.read_to_end(&mut sink).await;
                        return;
                    }
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{}",
                        head.len(),
                        head
                    );
                    if stream.write_all(response.as_bytes()).await.is_err() {
                        return;
                    }
                }
            });
        }
    });
    addr
}

/// A config with one backend and a handful of routes, plus `server` settings.
fn config(server: &str, backend: SocketAddr) -> ProxyConfig {
    let toml_src = format!(
        r#"
[server]
{server}

[[upstreams]]
name = "up"
[[upstreams.backends]]
address = "{backend}"

[[upstreams]]
name = "off"
enabled = false
[[upstreams.backends]]
address = "{backend}"

[[upstreams]]
name = "drained"
[[upstreams.backends]]
address = "{backend}"
enabled = false

[[routes]]
name = "public"
upstream = "up"
[routes.match]
path = "/public"

[[routes]]
name = "stripped"
upstream = "up"
strip_path = true
[routes.match]
path = "/strip"

[[routes]]
name = "limited-a"
upstream = "up"
[routes.match]
path = "/limited-a"
[routes.rate_limit]
requests = 2
window_secs = 60

[[routes]]
name = "limited-b"
upstream = "up"
[routes.match]
path = "/limited-b"
[routes.rate_limit]
requests = 2
window_secs = 60

[[routes]]
name = "by-key"
upstream = "up"
[routes.match]
path = "/by-key"
[routes.rate_limit]
requests = 1
window_secs = 60
key = {{ header = "x-api-key" }}

[[routes]]
name = "disabled-upstream"
upstream = "off"
[routes.match]
path = "/off"

[[routes]]
name = "disabled-backends"
upstream = "drained"
[routes.match]
path = "/drained"
"#
    );
    toml::from_str(&toml_src).expect("test config should parse")
}

/// Start a proxy on 127.0.0.1:0 and return where it is listening.
async fn proxy(server: &str) -> (SocketAddr, CancellationToken) {
    let config = config(server, backend().await);
    let rate_limiter = Arc::new(RateLimiter::new(config.rate_limit.clone()));
    let service = Arc::new(ProxyService::new(
        Arc::new(RwLock::new(config)),
        Arc::new(HealthChecker::new()),
        rate_limiter,
    ));
    service.load_config().await;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let cancel = CancellationToken::new();
    tokio::spawn(service.serve_http_listener(listener, cancel.clone()));
    (addr, cancel)
}

struct Answer {
    status: u16,
    body: String,
}

/// Read one response on a kept-alive connection.
async fn read_response(stream: &mut TcpStream) -> Option<Answer> {
    let head = read_head(stream).await?;
    let status = head.split_whitespace().nth(1)?.parse().ok()?;
    let mut body = vec![0u8; content_length(&head)];
    stream.read_exact(&mut body).await.ok()?;
    Some(Answer {
        status,
        body: String::from_utf8_lossy(&body).to_ascii_lowercase(),
    })
}

/// Send one request with extra header lines, on a fresh connection.
async fn get(addr: SocketAddr, path: &str, headers: &str) -> Answer {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let request = format!("GET {path} HTTP/1.1\r\nHost: test\r\n{headers}\r\n");
    stream.write_all(request.as_bytes()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), read_response(&mut stream))
        .await
        .expect("no response within 5s")
        .expect("connection closed without a response")
}

/// Wait for the peer to close, returning how long it took; panics after
/// `limit`.
async fn closed_within(stream: &mut TcpStream, limit: Duration) -> Duration {
    let started = Instant::now();
    let mut buf = [0u8; 1024];
    loop {
        match tokio::time::timeout_at((started + limit).into(), stream.read(&mut buf)).await {
            Err(_) => panic!("connection still open after {limit:?}"),
            Ok(Ok(0)) | Ok(Err(_)) => return started.elapsed(),
            // A 408 or similar on the way out is fine; keep reading to EOF.
            Ok(Ok(_)) => continue,
        }
    }
}

#[tokio::test]
async fn a_request_head_that_never_finishes_is_dropped() {
    // The slowloris case. `header_read_timeout` was set by hyper's default
    // and never fired, because no timer had been given to the builder.
    let (addr, _cancel) = proxy("header_read_timeout_secs = 1\nidle_timeout_secs = 30").await;
    let mut stream = TcpStream::connect(addr).await.unwrap();
    stream
        .write_all(b"GET /public HTTP/1.1\r\nHost: test\r\nX-Slow: 1\r\n")
        .await
        .unwrap();
    let elapsed = closed_within(&mut stream, Duration::from_secs(5)).await;
    assert!(elapsed >= Duration::from_millis(800), "{elapsed:?}");
}

#[tokio::test]
async fn a_connection_that_sends_nothing_is_dropped() {
    // Not reachable by the header timeout: the cleartext port has to read the
    // first bytes before it knows which protocol it is speaking.
    let (addr, _cancel) = proxy("header_read_timeout_secs = 30\nidle_timeout_secs = 1").await;
    let mut stream = TcpStream::connect(addr).await.unwrap();
    closed_within(&mut stream, Duration::from_secs(5)).await;
}

#[tokio::test]
async fn a_tls_handshake_that_never_starts_is_dropped() {
    // The handshake runs on the idle-timed socket, so a client that connects
    // to the HTTPS port and sends nothing is closed like any other.
    let config = config("idle_timeout_secs = 1", backend().await);
    let service = Arc::new(ProxyService::new(
        Arc::new(RwLock::new(config)),
        Arc::new(HealthChecker::new()),
        Arc::new(RateLimiter::default()),
    ));
    service.load_config().await;
    let key = postrust_proxy::tls::SniCertResolver::certified_key(
        include_bytes!("../src/tls/testdata/valid.pem"),
        include_bytes!("../src/tls/testdata/valid.key.pem"),
    )
    .unwrap();
    let tls = postrust_proxy::tls::build_server_config_with_resolver(Arc::new(
        postrust_proxy::tls::SniCertResolver::new(Some(key)),
    ));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let cancel = CancellationToken::new();
    tokio::spawn(service.serve_https_listener(listener, tls, cancel.clone()));

    let mut stream = TcpStream::connect(addr).await.unwrap();
    closed_within(&mut stream, Duration::from_secs(5)).await;
}

#[tokio::test]
async fn a_kept_alive_connection_is_closed_once_idle() {
    let (addr, _cancel) = proxy("header_read_timeout_secs = 30\nidle_timeout_secs = 1").await;
    let mut stream = TcpStream::connect(addr).await.unwrap();
    stream
        .write_all(b"GET /public HTTP/1.1\r\nHost: test\r\n\r\n")
        .await
        .unwrap();
    assert_eq!(read_response(&mut stream).await.unwrap().status, 200);
    closed_within(&mut stream, Duration::from_secs(5)).await;
}

#[tokio::test]
async fn connections_past_the_limit_wait_for_a_slot() {
    let (addr, _cancel) = proxy("max_connections = 1\nidle_timeout_secs = 30").await;

    let mut first = TcpStream::connect(addr).await.unwrap();
    first
        .write_all(b"GET /public HTTP/1.1\r\nHost: test\r\n\r\n")
        .await
        .unwrap();
    assert_eq!(read_response(&mut first).await.unwrap().status, 200);

    // The kernel completes the handshake, but the proxy does not accept it
    // while the only slot is taken, so the request goes unanswered.
    let mut second = TcpStream::connect(addr).await.unwrap();
    second
        .write_all(b"GET /public HTTP/1.1\r\nHost: test\r\n\r\n")
        .await
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(700), read_response(&mut second))
            .await
            .is_err(),
        "a second connection was served past max_connections = 1"
    );

    // Closing the first frees the slot.
    drop(first);
    let answer = tokio::time::timeout(Duration::from_secs(5), read_response(&mut second))
        .await
        .expect("the waiting connection was not served once a slot freed")
        .unwrap();
    assert_eq!(answer.status, 200);
}

#[tokio::test]
async fn a_quiet_tunnel_is_closed() {
    let (addr, _cancel) = proxy("tunnel_idle_timeout_secs = 1\nidle_timeout_secs = 30").await;
    let mut stream = TcpStream::connect(addr).await.unwrap();
    stream
        .write_all(
            b"GET /public/ws HTTP/1.1\r\nHost: test\r\nConnection: Upgrade\r\n\
              Upgrade: websocket\r\nSec-WebSocket-Version: 13\r\n\
              Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n",
        )
        .await
        .unwrap();
    let head = read_head(&mut stream).await.unwrap();
    assert!(head.starts_with("HTTP/1.1 101"), "{head}");
    closed_within(&mut stream, Duration::from_secs(5)).await;
}

#[tokio::test]
async fn an_h2c_upgrade_is_forwarded_as_a_plain_request() {
    // Tunnelled, h2c let the client speak HTTP/2 straight to the backend and
    // send it any request on any path, past every route rule.
    let (addr, _cancel) = proxy("").await;
    let answer = get(
        addr,
        "/public",
        "Connection: Upgrade, HTTP2-Settings\r\nUpgrade: h2c\r\n\
         HTTP2-Settings: AAMAAABkAAQCAAAAAAIAAAAA\r\n",
    )
    .await;
    assert_eq!(answer.status, 200);
    assert!(!answer.body.contains("upgrade"), "{}", answer.body);
    assert!(!answer.body.contains("http2-settings"), "{}", answer.body);

    // Nor is it tunnelled when the client leaves HTTP2-Settings out of
    // Connection.
    let answer = get(
        addr,
        "/public",
        "Connection: Upgrade\r\nUpgrade: h2c\r\nHTTP2-Settings: AAMAAABkAAQCAAAAAAIAAAAA\r\n",
    )
    .await;
    assert_eq!(answer.status, 200);
    assert!(!answer.body.contains("http2-settings"), "{}", answer.body);
}

#[tokio::test]
async fn paths_that_would_escape_their_route_are_refused() {
    let (addr, _cancel) = proxy("").await;
    for path in [
        "/public/../admin",
        "/public/%2e%2e/admin",
        "/public/%2E%2E/admin",
        "/public%2f..%2fadmin",
        "/public/..%2Fadmin",
        "/public/./admin",
    ] {
        assert_eq!(get(addr, path, "").await.status, 400, "{path}");
    }

    // Segment boundaries: /public is not a prefix of /publicity.
    assert_eq!(get(addr, "/publicity", "").await.status, 404);
    assert_eq!(get(addr, "/public", "").await.status, 200);
    assert_eq!(get(addr, "/public/a/b", "").await.status, 200);

    // strip_path takes the prefix and only the prefix.
    let answer = get(addr, "/strip/inner?q=1", "").await;
    assert_eq!(answer.status, 200);
    assert!(
        answer.body.starts_with("get /inner?q=1 "),
        "{}",
        answer.body
    );
}

#[tokio::test]
async fn forwarding_headers_say_what_the_proxy_saw() {
    let (addr, _cancel) = proxy("").await;
    let answer = get(
        addr,
        "/public",
        "X-Forwarded-For: 203.0.113.9\r\nX-Real-IP: 203.0.113.9\r\n\
         Forwarded: for=203.0.113.9\r\n",
    )
    .await;
    assert_eq!(answer.status, 200);
    assert!(
        answer.body.contains("x-forwarded-for: 127.0.0.1\r\n"),
        "{}",
        answer.body
    );
    assert!(
        answer.body.contains("x-real-ip: 127.0.0.1\r\n"),
        "{}",
        answer.body
    );
    assert!(!answer.body.contains("203.0.113.9"), "{}", answer.body);
    assert!(!answer.body.contains("forwarded: for="), "{}", answer.body);
}

#[tokio::test]
async fn trusted_forwarding_headers_are_appended_to() {
    let (addr, _cancel) = proxy("trust_forwarded_headers = true").await;
    let answer = get(addr, "/public", "X-Forwarded-For: 203.0.113.9\r\n").await;
    assert!(
        answer
            .body
            .contains("x-forwarded-for: 203.0.113.9, 127.0.0.1\r\n"),
        "{}",
        answer.body
    );
}

#[tokio::test]
async fn rate_limits_are_kept_per_route_and_by_the_configured_key() {
    let (addr, _cancel) = proxy("").await;

    assert_eq!(get(addr, "/limited-a", "").await.status, 200);
    assert_eq!(get(addr, "/limited-a", "").await.status, 200);
    assert_eq!(get(addr, "/limited-a", "").await.status, 429);

    // Another route's limit is its own: spending route a's burst does not
    // touch route b's.
    assert_eq!(get(addr, "/limited-b", "").await.status, 200);

    // Keyed by header: one bucket per key, not per address.
    assert_eq!(get(addr, "/by-key", "X-Api-Key: one\r\n").await.status, 200);
    assert_eq!(get(addr, "/by-key", "X-Api-Key: one\r\n").await.status, 429);
    assert_eq!(get(addr, "/by-key", "X-Api-Key: two\r\n").await.status, 200);
}

#[tokio::test]
async fn disabled_upstreams_and_backends_get_no_traffic() {
    let (addr, _cancel) = proxy("").await;
    assert_eq!(get(addr, "/off", "").await.status, 503);
    assert_eq!(get(addr, "/drained", "").await.status, 503);
    assert_eq!(get(addr, "/public", "").await.status, 200);
}
