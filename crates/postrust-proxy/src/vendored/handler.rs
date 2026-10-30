//! Vendored message handler from rpxy-lib: message_handler/*.rs
//!
//! This module handles request/response manipulation:
//! - X-Forwarded-* header handling
//! - Host header rewriting
//! - Request parsing

use crate::config::Route;
use crate::vendored::hyper_ext::{empty_body, string_body, ProxyBody};
use hyper::header::{HeaderName, HeaderValue, CONNECTION, FORWARDED, HOST, UPGRADE};
use hyper::{Method, Version};
use hyper::{Request, Response, StatusCode};
use std::net::{IpAddr, SocketAddr};

/// Standard X-Forwarded headers.
pub mod headers {
    use hyper::header::HeaderName;

    pub static X_FORWARDED_FOR: HeaderName = HeaderName::from_static("x-forwarded-for");
    pub static X_FORWARDED_PROTO: HeaderName = HeaderName::from_static("x-forwarded-proto");
    pub static X_FORWARDED_HOST: HeaderName = HeaderName::from_static("x-forwarded-host");
    pub static X_REAL_IP: HeaderName = HeaderName::from_static("x-real-ip");
}

/// Headers that are hop-by-hop and must never reach an upstream.
///
/// RFC 9110 section 7.6.1. `Transfer-Encoding` belongs here even though it
/// describes the body: the server side of hyper decodes the inbound body and
/// the client side frames it again, so the inbound value describes a wire
/// format the upstream never sees. Passing it through is what lets a client
/// disagree with the proxy about where one request ends and the next begins.
const HOP_BY_HOP_HEADERS: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    // Connection-specific by definition (RFC 9113 section 3.2.1 retired it,
    // RFC 7540 required it to be named in `Connection`). Listed rather than
    // left to the `Connection` tokens because a client need not name it, and
    // an `h2c` upgrade the proxy has declined must not reach the backend
    // carrying half its handshake.
    "http2-settings",
    // Never standard, still sent, and some servers still act on it.
    "proxy-connection",
];

/// Headers a `Connection` token may not remove.
///
/// Naming these in `Connection` is not a legitimate use of the field, and
/// honouring it would hand a client a way to strip framing or identity
/// headers on their way through the proxy.
const CONNECTION_TOKEN_EXEMPT: &[&str] = &["host", "content-length"];

/// Message handler for request/response manipulation.
pub struct MessageHandler;

/// Drop a `:port` from an authority, leaving the host.
///
/// Bracket-aware: the colons inside `[::1]` are part of the address, so a naive
/// `split(':').next()` would return `[` for every IPv6 client.
fn strip_port(authority: &str) -> &str {
    if authority.starts_with('[') {
        return match authority.find(']') {
            Some(end) => &authority[..end + 1],
            None => authority,
        };
    }
    match authority.split_once(':') {
        Some((host, _)) => host,
        None => authority,
    }
}

impl MessageHandler {
    /// The protocol a request is asking to upgrade to, if it is a valid
    /// upgrade request.
    ///
    /// Requires both `Upgrade: <proto>` and an `upgrade` token in `Connection`,
    /// per RFC 9110 section 7.8 -- `Upgrade` alone is not a request to switch.
    /// HTTP/2 is excluded deliberately: it carries no `Upgrade` header and
    /// negotiates extended CONNECT instead (RFC 8441), which is not implemented.
    pub fn upgrade_protocol(request: &Request<ProxyBody>) -> Option<String> {
        if request.version() != Version::HTTP_11 {
            return None;
        }

        let headers = request.headers();
        let asks_for_upgrade = headers.get_all(CONNECTION).iter().any(|value| {
            value
                .to_str()
                .map(|v| {
                    v.split(',')
                        .any(|t| t.trim().eq_ignore_ascii_case("upgrade"))
                })
                .unwrap_or(false)
        });
        if !asks_for_upgrade {
            return None;
        }

        headers
            .get(UPGRADE)
            .and_then(|v| v.to_str().ok())
            .map(|v| v.trim().to_ascii_lowercase())
            .filter(|v| !v.is_empty())
    }

    /// The protocol an HTTP/2 extended CONNECT is asking for (RFC 8441).
    ///
    /// HTTP/2 has no `Upgrade`; a WebSocket is opened with `:method = CONNECT`
    /// plus a `:protocol` pseudo-header, which hyper surfaces as a
    /// [`hyper::ext::Protocol`] extension. This is only ever offered when the
    /// listener advertised SETTINGS_ENABLE_CONNECT_PROTOCOL.
    pub fn h2_connect_protocol(request: &Request<ProxyBody>) -> Option<String> {
        if request.version() != Version::HTTP_2 || request.method() != Method::CONNECT {
            return None;
        }
        request
            .extensions()
            .get::<hyper::ext::Protocol>()
            .map(|protocol| protocol.as_str().to_ascii_lowercase())
    }

    /// Re-apply the upgrade headers that [`Self::strip_hop_by_hop_headers`] took.
    ///
    /// An upgrade is the one case where these headers legitimately travel to the
    /// next hop. Only a normalised `Connection: upgrade` and the requested
    /// protocol go back -- not whatever token list the client sent -- so the
    /// carve-out cannot be used to smuggle other headers through.
    pub fn restore_upgrade_headers(request: &mut Request<ProxyBody>, protocol: &str) {
        if let Ok(value) = HeaderValue::from_str(protocol) {
            let headers = request.headers_mut();
            headers.insert(UPGRADE, value);
            headers.insert(CONNECTION, HeaderValue::from_static("upgrade"));
        }
    }

    /// Whether `protocol`, as returned by [`Self::upgrade_protocol`] or
    /// [`Self::h2_connect_protocol`], is one the proxy may tunnel.
    ///
    /// An exact, case-insensitive match against `allowed` -- so a request for
    /// several protocols at once (`Upgrade: websocket, h2c`) is not a match
    /// for any of them, and is declined.
    pub fn upgrade_allowed(protocol: &str, allowed: &[String]) -> bool {
        allowed
            .iter()
            .any(|p| p.trim().eq_ignore_ascii_case(protocol))
    }

    // There is no check here for a request carrying both `Content-Length` and
    // `Transfer-Encoding`. There was one, and it could never fire: hyper's
    // HTTP/1 parser refuses a `Transfer-Encoding` whose last coding is not
    // `chunked`, and when both are present it frames the body by
    // `Transfer-Encoding` and drops `Content-Length` before the request
    // reaches us (RFC 9112 section 6.3). The body is re-framed on the way to
    // the upstream, and `Transfer-Encoding` is stripped below as hop-by-hop,
    // so the two can never disagree downstream either.

    /// Why a request path cannot be routed safely, if it cannot.
    ///
    /// Routes match the path as sent, and a backend resolves it after
    /// decoding and normalising -- so any path the two would read differently
    /// is a way past a route's path rules. `/public/../admin` matched the
    /// `/public` route and was forwarded verbatim, to a backend that served
    /// `/admin`. Refused:
    ///
    /// - dot-segments (`.` and `..`), literal or percent-encoded in any case,
    ///   and with a `;parameter` after them, which Tomcat-style servers strip
    ///   before resolving;
    /// - an encoded slash or backslash (`%2f`, `%5c`), which a backend may
    ///   decode into a separator the route never saw;
    /// - a literal backslash, which some servers treat as a separator.
    ///
    /// Refusing rather than normalising: a client that sends these is either
    /// probing or broken, and normalising would mean the proxy and the
    /// backend each have to agree with a third reading of the path.
    pub fn unsafe_path_reason(path: &str) -> Option<&'static str> {
        let lower = path.to_ascii_lowercase();
        if lower.contains("%2f") || lower.contains("%5c") || path.contains('\\') {
            return Some("Encoded or backslash path separators are not allowed");
        }
        for segment in lower.split('/') {
            let segment = segment.split(';').next().unwrap_or("");
            let decoded = segment.replace("%2e", ".");
            if decoded == "." || decoded == ".." {
                return Some("Dot-segments are not allowed in the request path");
            }
        }
        None
    }

    /// Remove hop-by-hop headers before forwarding, per RFC 9110 section 7.6.1.
    ///
    /// Two sets go: the standard hop-by-hop headers, and every header named as
    /// a token in the incoming `Connection` field. Call this *before*
    /// [`Self::add_forwarding_headers`] -- a client that names `x-forwarded-for`
    /// in `Connection` then strips only its own inbound value, and we set a
    /// fresh one afterwards, rather than being able to suppress ours.
    pub fn strip_hop_by_hop_headers(request: &mut Request<ProxyBody>) {
        let headers = request.headers_mut();

        // Read the Connection tokens up front: removing headers invalidates any
        // borrow of the values we would otherwise still be holding.
        let mut named: Vec<String> = Vec::new();
        for value in headers.get_all(CONNECTION).iter() {
            let Ok(value) = value.to_str() else { continue };
            for token in value.split(',') {
                let token = token.trim().to_ascii_lowercase();
                if !token.is_empty() {
                    named.push(token);
                }
            }
        }

        for name in HOP_BY_HOP_HEADERS {
            headers.remove(*name);
        }

        for token in named {
            if CONNECTION_TOKEN_EXEMPT.contains(&token.as_str()) {
                continue;
            }
            if let Ok(name) = HeaderName::from_bytes(token.as_bytes()) {
                headers.remove(name);
            }
        }
    }

    /// Add X-Forwarded-* headers to a request.
    ///
    /// A backend uses these to decide who the client is -- for its logs, its
    /// own rate limits, its IP allow-lists -- so a value the client wrote is a
    /// value the client chose. Unless `trust_client` is set
    /// (`server.trust_forwarded_headers`), everything the client sent is
    /// replaced with what the proxy saw: `X-Forwarded-For` and `X-Real-IP`
    /// become the socket's peer address, and `Forwarded` is removed. It used
    /// to be appended to, set only if absent, and passed through, which let
    /// any client claim any address.
    ///
    /// With `trust_client`, the client is another proxy: its chain is kept and
    /// this hop appended to `X-Forwarded-For` and `Forwarded`, and the
    /// `X-Forwarded-Proto` and `X-Forwarded-Host` it set are left as they
    /// are, since they describe the original request better than this hop
    /// can. `X-Real-IP` is still the peer address: it names one address, and
    /// the one this proxy saw is the one it can vouch for.
    pub fn add_forwarding_headers(
        request: &mut Request<ProxyBody>,
        client_addr: SocketAddr,
        proto: &str,
        trust_client: bool,
    ) {
        let authority = request.uri().authority().map(|a| a.to_string());
        let headers = request.headers_mut();
        let client_ip = client_addr.ip().to_canonical();
        let client_ip_text = client_ip.to_string();

        // X-Forwarded-For. Every line the client sent is part of the chain
        // when it is trusted; none of it is when it is not.
        let mut chain: Vec<String> = Vec::new();
        if trust_client {
            for value in headers.get_all(&headers::X_FORWARDED_FOR).iter() {
                if let Ok(value) = value.to_str() {
                    let value = value.trim();
                    if !value.is_empty() {
                        chain.push(value.to_string());
                    }
                }
            }
        }
        chain.push(client_ip_text.clone());
        headers.remove(&headers::X_FORWARDED_FOR);
        if let Ok(value) = HeaderValue::from_str(&chain.join(", ")) {
            headers.insert(headers::X_FORWARDED_FOR.clone(), value);
        }

        // X-Real-IP: always the peer.
        if let Ok(value) = HeaderValue::from_str(&client_ip_text) {
            headers.insert(headers::X_REAL_IP.clone(), value);
        }

        // Forwarded (RFC 7239). Rewritten only to extend a trusted chain;
        // otherwise dropped, since the backend may well read it in preference
        // to X-Forwarded-For.
        let mut forwarded: Vec<String> = Vec::new();
        if trust_client {
            for value in headers.get_all(FORWARDED).iter() {
                if let Ok(value) = value.to_str() {
                    let value = value.trim();
                    if !value.is_empty() {
                        forwarded.push(value.to_string());
                    }
                }
            }
        }
        headers.remove(FORWARDED);
        if trust_client {
            forwarded.push(format!("for={};proto={}", forwarded_node(client_ip), proto));
            if let Ok(value) = HeaderValue::from_str(&forwarded.join(", ")) {
                headers.insert(FORWARDED, value);
            }
        }

        // X-Forwarded-Proto
        if !(trust_client && headers.contains_key(&headers::X_FORWARDED_PROTO)) {
            if let Ok(value) = HeaderValue::from_str(proto) {
                headers.insert(headers::X_FORWARDED_PROTO.clone(), value);
            }
        }

        // X-Forwarded-Host: preserve the original authority. HTTP/2 carries it in
        // the :authority pseudo-header rather than Host, which reaches us on the
        // URI, so fall back to that before giving up.
        if !(trust_client && headers.contains_key(&headers::X_FORWARDED_HOST)) {
            let original_host = headers
                .get(HOST)
                .cloned()
                .or_else(|| authority.and_then(|a| HeaderValue::from_str(&a).ok()));
            match original_host {
                Some(host) => {
                    headers.insert(headers::X_FORWARDED_HOST.clone(), host);
                }
                None => {
                    headers.remove(&headers::X_FORWARDED_HOST);
                }
            }
        }
    }

    /// Rewrite the Host header for upstream.
    pub fn rewrite_host_header(request: &mut Request<ProxyBody>, upstream_host: &str) {
        if let Ok(value) = HeaderValue::from_str(upstream_host) {
            request.headers_mut().insert(HOST, value);
        }
    }

    /// Strip path prefix from request URI.
    ///
    /// Only on a segment boundary, by the same rule routes match with
    /// ([`crate::config::path_has_prefix`]): `/api` comes off `/api/users`
    /// but not off `/apis`.
    pub fn strip_path_prefix(request: &mut Request<ProxyBody>, prefix: &str) {
        let uri = request.uri();
        let path = uri.path();

        if !crate::config::path_has_prefix(path, prefix) {
            return;
        }
        if let Some(new_path) = path.strip_prefix(prefix) {
            let new_path = if new_path.is_empty() || !new_path.starts_with('/') {
                format!("/{}", new_path.trim_start_matches('/'))
            } else {
                new_path.to_string()
            };

            let new_uri = if let Some(query) = uri.query() {
                format!("{}?{}", new_path, query)
            } else {
                new_path
            };

            if let Ok(new_uri) = new_uri.parse() {
                *request.uri_mut() = new_uri;
            }
        }
    }

    /// Apply route-specific header modifications.
    pub fn apply_route_headers(request: &mut Request<ProxyBody>, route: &Route) {
        let headers = request.headers_mut();

        // Add headers
        for (name, value) in &route.add_headers {
            if let (Ok(name), Ok(value)) = (
                HeaderName::from_bytes(name.as_bytes()),
                HeaderValue::from_str(value),
            ) {
                headers.insert(name, value);
            }
        }

        // Remove headers
        for name in &route.remove_headers {
            if let Ok(name) = HeaderName::from_bytes(name.as_bytes()) {
                headers.remove(name);
            }
        }
    }

    /// Create a synthetic error response.
    pub fn error_response(status: StatusCode, message: &str) -> Response<ProxyBody> {
        Response::builder()
            .status(status)
            .header("content-type", "text/plain; charset=utf-8")
            .body(string_body(message.to_string()))
            .unwrap_or_else(|_| {
                Response::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .body(empty_body())
                    .unwrap()
            })
    }

    /// Create a 400 Bad Request response.
    pub fn bad_request(message: &str) -> Response<ProxyBody> {
        Self::error_response(StatusCode::BAD_REQUEST, message)
    }

    /// Create a 502 Bad Gateway response.
    pub fn bad_gateway(message: &str) -> Response<ProxyBody> {
        Self::error_response(StatusCode::BAD_GATEWAY, message)
    }

    /// Create a 503 Service Unavailable response.
    pub fn service_unavailable(message: &str) -> Response<ProxyBody> {
        Self::error_response(StatusCode::SERVICE_UNAVAILABLE, message)
    }

    /// The host a request is for, without its port.
    ///
    /// **Not just the `Host` header.** HTTP/2 has no `Host`: RFC 9113 section
    /// 8.3.1 replaces it with the `:authority` pseudo-header, which reaches us
    /// on the URI. Reading only the header therefore yields an empty host for
    /// every h2 and h2c request, and a route with a `match.host` matches
    /// nothing -- confirmed against a running proxy, where the same
    /// host-matched route answered 200 over HTTP/1.1 and 404 over HTTP/2.
    ///
    /// `add_forwarding_headers` already had this fallback for
    /// `x-forwarded-host`; route selection did not.
    pub fn request_host(request: &Request<impl hyper::body::Body>) -> String {
        let from_header = request
            .headers()
            .get(hyper::header::HOST)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);

        let host = from_header
            .or_else(|| request.uri().authority().map(|a| a.to_string()))
            .unwrap_or_default();

        strip_port(&host).to_owned()
    }

    /// Create a 504 Gateway Timeout response.
    ///
    /// Takes a message so the body can say how long the request waited. A bare
    /// "Gateway Timeout" leaves the caller unable to tell a route configured
    /// with one second from an upstream that is genuinely wedged.
    pub fn gateway_timeout(message: &str) -> Response<ProxyBody> {
        Self::error_response(StatusCode::GATEWAY_TIMEOUT, message)
    }

    /// Create a 429 Too Many Requests response.
    pub fn too_many_requests() -> Response<ProxyBody> {
        Self::error_response(StatusCode::TOO_MANY_REQUESTS, "Too Many Requests")
    }

    /// Create a 404 Not Found response.
    pub fn not_found() -> Response<ProxyBody> {
        Self::error_response(StatusCode::NOT_FOUND, "Not Found")
    }

    /// Create a 501 Not Implemented response.
    pub fn not_implemented(message: &str) -> Response<ProxyBody> {
        Self::error_response(StatusCode::NOT_IMPLEMENTED, message)
    }
}

/// An address as an RFC 7239 node: IPv6 is bracketed and quoted, since `:` and
/// `[` are not allowed in a bare token.
fn forwarded_node(ip: IpAddr) -> String {
    match ip {
        IpAddr::V4(v4) => v4.to_string(),
        IpAddr::V6(v6) => format!("\"[{v6}]\""),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request_with(headers: &[(&str, &str)], uri: &str) -> Request<ProxyBody> {
        let mut builder = Request::builder().uri(uri);
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        builder.body(empty_body()).unwrap()
    }

    #[test]
    fn the_host_header_gives_the_host() {
        let r = request_with(&[("host", "api.example.com")], "/x");
        assert_eq!(MessageHandler::request_host(&r), "api.example.com");
    }

    #[test]
    fn the_uri_authority_is_used_when_there_is_no_host_header() {
        // This is every HTTP/2 request: RFC 9113 section 8.3.1 replaces Host
        // with :authority, which reaches us on the URI. Reading only the header
        // made every host-matched route 404 over h2 -- confirmed against a
        // running proxy before this was fixed.
        let r = request_with(&[], "http://api.example.com/x");
        assert_eq!(MessageHandler::request_host(&r), "api.example.com");
    }

    #[test]
    fn the_header_wins_over_the_authority() {
        let r = request_with(&[("host", "from-header.test")], "http://from-uri.test/x");
        assert_eq!(MessageHandler::request_host(&r), "from-header.test");
    }

    #[test]
    fn the_port_is_not_part_of_the_host() {
        assert_eq!(
            MessageHandler::request_host(&request_with(&[("host", "api.example.com:8443")], "/x")),
            "api.example.com"
        );
        assert_eq!(
            MessageHandler::request_host(&request_with(&[], "http://api.example.com:8443/x")),
            "api.example.com"
        );
    }

    #[test]
    fn an_ipv6_literal_keeps_its_brackets() {
        // `split(':').next()` would return "[" here, so every IPv6 client would
        // be matched against a host of "[".
        assert_eq!(
            MessageHandler::request_host(&request_with(&[("host", "[::1]:8443")], "/x")),
            "[::1]"
        );
        assert_eq!(
            MessageHandler::request_host(&request_with(&[("host", "[::1]")], "/x")),
            "[::1]"
        );
    }

    #[test]
    fn a_request_with_neither_has_an_empty_host() {
        let r = request_with(&[], "/x");
        assert_eq!(MessageHandler::request_host(&r), "");
    }

    use crate::vendored::hyper_ext::empty_body;

    #[test]
    fn test_strip_path_prefix() {
        let mut request = Request::builder()
            .uri("/api/v1/users")
            .body(empty_body())
            .unwrap();

        MessageHandler::strip_path_prefix(&mut request, "/api/v1");

        assert_eq!(request.uri().path(), "/users");
    }

    #[test]
    fn test_strip_path_prefix_root() {
        let mut request = Request::builder().uri("/api").body(empty_body()).unwrap();

        MessageHandler::strip_path_prefix(&mut request, "/api");

        assert_eq!(request.uri().path(), "/");
    }

    #[test]
    fn test_h2_connect_protocol_detected() {
        // RFC 8441: :method CONNECT plus the :protocol pseudo-header, which
        // hyper surfaces as an extension.
        let mut request = Request::builder()
            .method(Method::CONNECT)
            .version(Version::HTTP_2)
            .uri("/chat")
            .body(empty_body())
            .unwrap();
        request
            .extensions_mut()
            .insert(hyper::ext::Protocol::from_static("websocket"));

        assert_eq!(
            MessageHandler::h2_connect_protocol(&request).as_deref(),
            Some("websocket")
        );
        // The HTTP/1.1 detector must not claim it.
        assert!(MessageHandler::upgrade_protocol(&request).is_none());
    }

    #[test]
    fn test_h2_connect_without_protocol_is_a_plain_connect() {
        let request = Request::builder()
            .method(Method::CONNECT)
            .version(Version::HTTP_2)
            .uri("/chat")
            .body(empty_body())
            .unwrap();

        assert!(MessageHandler::h2_connect_protocol(&request).is_none());
    }

    #[test]
    fn test_h1_connect_is_not_extended_connect() {
        // Extended CONNECT is an HTTP/2 mechanism; the h1 spelling is Upgrade.
        let mut request = Request::builder()
            .method(Method::CONNECT)
            .version(Version::HTTP_11)
            .uri("/chat")
            .body(empty_body())
            .unwrap();
        request
            .extensions_mut()
            .insert(hyper::ext::Protocol::from_static("websocket"));

        assert!(MessageHandler::h2_connect_protocol(&request).is_none());
    }

    #[test]
    fn test_upgrade_protocol_detected() {
        let request = Request::builder()
            .uri("/chat")
            .header("host", "a")
            .header("upgrade", "websocket")
            .header("connection", "Upgrade")
            .body(empty_body())
            .unwrap();

        assert_eq!(
            MessageHandler::upgrade_protocol(&request).as_deref(),
            Some("websocket")
        );
    }

    #[test]
    fn test_upgrade_needs_connection_token() {
        // Upgrade alone is not a request to switch protocols (RFC 9110 s7.8).
        let request = Request::builder()
            .uri("/chat")
            .header("host", "a")
            .header("upgrade", "websocket")
            .body(empty_body())
            .unwrap();

        assert!(MessageHandler::upgrade_protocol(&request).is_none());
    }

    #[test]
    fn test_upgrade_token_found_among_others() {
        let request = Request::builder()
            .uri("/chat")
            .header("host", "a")
            .header("upgrade", "WebSocket")
            .header("connection", "keep-alive, UPGRADE")
            .body(empty_body())
            .unwrap();

        assert_eq!(
            MessageHandler::upgrade_protocol(&request).as_deref(),
            Some("websocket")
        );
    }

    #[test]
    fn test_no_upgrade_without_upgrade_header() {
        let request = Request::builder()
            .uri("/chat")
            .header("host", "a")
            .header("connection", "upgrade")
            .body(empty_body())
            .unwrap();

        assert!(MessageHandler::upgrade_protocol(&request).is_none());
    }

    #[test]
    fn test_http2_never_upgrades() {
        // HTTP/2 negotiates extended CONNECT (RFC 8441), not Upgrade.
        let request = Request::builder()
            .uri("/chat")
            .version(Version::HTTP_2)
            .header("upgrade", "websocket")
            .header("connection", "Upgrade")
            .body(empty_body())
            .unwrap();

        assert!(MessageHandler::upgrade_protocol(&request).is_none());
    }

    #[test]
    fn test_restore_upgrade_headers_normalises_connection() {
        // The carve-out must put back only `upgrade`, never the client's list.
        let mut request = Request::builder()
            .uri("/chat")
            .header("host", "a")
            .header("upgrade", "websocket")
            .header("connection", "Upgrade, X-Sneaky")
            .header("x-sneaky", "leaked")
            .header("x-legit", "keep-me")
            .body(empty_body())
            .unwrap();

        let protocol = MessageHandler::upgrade_protocol(&request).unwrap();
        MessageHandler::strip_hop_by_hop_headers(&mut request);
        MessageHandler::restore_upgrade_headers(&mut request, &protocol);

        assert_eq!(request.headers().get("upgrade").unwrap(), "websocket");
        assert_eq!(request.headers().get("connection").unwrap(), "upgrade");
        assert!(!request.headers().contains_key("x-sneaky"));
        assert_eq!(request.headers().get("x-legit").unwrap(), "keep-me");
    }

    #[test]
    fn test_strips_each_hop_by_hop_header() {
        // One assertion per header in the RFC 9110 section 7.6.1 set.
        for name in HOP_BY_HOP_HEADERS {
            let mut request = Request::builder()
                .uri("/test")
                .header("host", "example.com")
                .header(*name, "some-value")
                .body(empty_body())
                .unwrap();

            MessageHandler::strip_hop_by_hop_headers(&mut request);

            assert!(
                !request.headers().contains_key(*name),
                "hop-by-hop header {} was forwarded",
                name
            );
            // Stripping must not take unrelated headers with it.
            assert_eq!(request.headers().get("host").unwrap(), "example.com");
        }
    }

    #[test]
    fn test_strips_headers_named_in_connection() {
        // The live case found by the HTTP Garden probe.
        let mut request = Request::builder()
            .uri("/test")
            .header("host", "example.com")
            .header("connection", "keep-alive, X-Hop")
            .header("x-hop", "SHOULD-BE-STRIPPED")
            .body(empty_body())
            .unwrap();

        MessageHandler::strip_hop_by_hop_headers(&mut request);

        assert!(!request.headers().contains_key("connection"));
        assert!(!request.headers().contains_key("x-hop"));
        assert_eq!(request.headers().get("host").unwrap(), "example.com");
    }

    #[test]
    fn test_connection_tokens_are_case_insensitive_and_trimmed() {
        let mut request = Request::builder()
            .uri("/test")
            .header("connection", "  X-UPPER  ,\tx-lower,,")
            .header("x-upper", "a")
            .header("x-lower", "b")
            .body(empty_body())
            .unwrap();

        MessageHandler::strip_hop_by_hop_headers(&mut request);

        assert!(!request.headers().contains_key("x-upper"));
        assert!(!request.headers().contains_key("x-lower"));
    }

    #[test]
    fn test_connection_token_cannot_strip_host_or_content_length() {
        // Naming these in Connection is not legitimate; honouring it would let a
        // client strip framing or identity headers on the way through.
        let mut request = Request::builder()
            .uri("/test")
            .header("host", "example.com")
            .header("content-length", "0")
            .header("connection", "host, content-length")
            .body(empty_body())
            .unwrap();

        MessageHandler::strip_hop_by_hop_headers(&mut request);

        assert!(!request.headers().contains_key("connection"));
        assert_eq!(request.headers().get("host").unwrap(), "example.com");
        assert_eq!(request.headers().get("content-length").unwrap(), "0");
    }

    #[test]
    fn test_connection_token_naming_forwarded_header_does_not_suppress_ours() {
        // strip runs before add_forwarding_headers, so the client's inbound value
        // goes and ours replaces it -- no suppression, no spoofing.
        let mut request = Request::builder()
            .uri("/test")
            .header("host", "example.com")
            .header("connection", "x-forwarded-for")
            .header("x-forwarded-for", "1.2.3.4")
            .body(empty_body())
            .unwrap();

        MessageHandler::strip_hop_by_hop_headers(&mut request);
        let client_addr: SocketAddr = "192.168.1.100:12345".parse().unwrap();
        MessageHandler::add_forwarding_headers(&mut request, client_addr, "http", false);

        assert_eq!(
            request.headers().get("x-forwarded-for").unwrap(),
            "192.168.1.100"
        );
    }

    #[test]
    fn test_strip_is_a_no_op_without_hop_by_hop_headers() {
        let mut request = Request::builder()
            .uri("/test")
            .header("host", "example.com")
            .header("accept", "*/*")
            .body(empty_body())
            .unwrap();

        MessageHandler::strip_hop_by_hop_headers(&mut request);

        assert_eq!(request.headers().len(), 2);
    }

    #[test]
    fn test_forwarding_headers() {
        let mut request = Request::builder()
            .uri("/test")
            .header("host", "example.com")
            .body(empty_body())
            .unwrap();

        let client_addr: SocketAddr = "192.168.1.100:12345".parse().unwrap();
        MessageHandler::add_forwarding_headers(&mut request, client_addr, "https", false);

        assert_eq!(
            request.headers().get("x-forwarded-for").unwrap(),
            "192.168.1.100"
        );
        assert_eq!(request.headers().get("x-forwarded-proto").unwrap(), "https");
        assert_eq!(
            request.headers().get("x-forwarded-host").unwrap(),
            "example.com"
        );
    }

    fn forwarded_request(headers: &[(&str, &str)]) -> Request<ProxyBody> {
        let mut request = request_with(headers, "/test");
        let client_addr: SocketAddr = "192.0.2.7:4000".parse().unwrap();
        MessageHandler::add_forwarding_headers(&mut request, client_addr, "https", false);
        request
    }

    #[test]
    fn client_forwarding_headers_are_overwritten_by_default() {
        // Each of these was a way for a client to choose the address the
        // backend saw: X-Real-IP was set only if absent, X-Forwarded-For was
        // appended to, and Forwarded passed straight through.
        let request = forwarded_request(&[
            ("host", "example.com"),
            ("x-real-ip", "10.0.0.1"),
            ("x-forwarded-for", "10.0.0.1"),
            ("x-forwarded-for", "10.0.0.2"),
            ("forwarded", "for=10.0.0.1;proto=https"),
            ("x-forwarded-proto", "https-spoofed"),
            ("x-forwarded-host", "spoofed.test"),
        ]);
        let h = request.headers();
        assert_eq!(h.get("x-real-ip").unwrap(), "192.0.2.7");
        assert_eq!(h.get_all("x-forwarded-for").iter().count(), 1);
        assert_eq!(h.get("x-forwarded-for").unwrap(), "192.0.2.7");
        assert!(!h.contains_key("forwarded"));
        assert_eq!(h.get("x-forwarded-proto").unwrap(), "https");
        assert_eq!(h.get("x-forwarded-host").unwrap(), "example.com");
    }

    #[test]
    fn a_trusted_chain_is_kept_and_extended() {
        let mut request = request_with(
            &[
                ("host", "internal.test"),
                ("x-real-ip", "10.0.0.1"),
                ("x-forwarded-for", "198.51.100.1, 10.0.0.1"),
                ("forwarded", "for=198.51.100.1;proto=https"),
                ("x-forwarded-proto", "https"),
                ("x-forwarded-host", "public.example.com"),
            ],
            "/test",
        );
        let peer: SocketAddr = "[2001:db8::5]:4000".parse().unwrap();
        MessageHandler::add_forwarding_headers(&mut request, peer, "http", true);

        let h = request.headers();
        assert_eq!(
            h.get("x-forwarded-for").unwrap(),
            "198.51.100.1, 10.0.0.1, 2001:db8::5"
        );
        assert_eq!(
            h.get("forwarded").unwrap(),
            "for=198.51.100.1;proto=https, for=\"[2001:db8::5]\";proto=http"
        );
        // What the first proxy saw describes the original request better than
        // this hop can.
        assert_eq!(h.get("x-forwarded-proto").unwrap(), "https");
        assert_eq!(h.get("x-forwarded-host").unwrap(), "public.example.com");
        // X-Real-IP names one address, and it is the one this proxy saw.
        assert_eq!(h.get("x-real-ip").unwrap(), "2001:db8::5");
    }

    #[test]
    fn a_mapped_ipv4_peer_is_reported_as_ipv4() {
        let mut request = request_with(&[("host", "a")], "/");
        let peer: SocketAddr = "[::ffff:192.0.2.9]:1".parse().unwrap();
        MessageHandler::add_forwarding_headers(&mut request, peer, "http", false);
        assert_eq!(
            request.headers().get("x-forwarded-for").unwrap(),
            "192.0.2.9"
        );
    }

    #[test]
    fn dot_segments_and_encoded_separators_are_refused() {
        for path in [
            "/public/../admin",
            "/public/./admin",
            "/public/..",
            "/..",
            "/public/%2e%2e/admin",
            "/public/%2E%2E/admin",
            "/public/.%2e/admin",
            "/public/%2e./admin",
            "/public/%2e/admin",
            "/public%2f..%2fadmin",
            "/public%2F..%2Fadmin",
            "/public/..%2fadmin",
            "/public%5c..%5cadmin",
            "/public/..;/admin",
            "/public/..;jsessionid=x/admin",
        ] {
            assert!(
                MessageHandler::unsafe_path_reason(path).is_some(),
                "{path} should be refused"
            );
        }
    }

    #[test]
    fn ordinary_paths_are_allowed() {
        for path in [
            "/",
            "/public",
            "/public/",
            "/public/a.b",
            "/public/...",
            "/public/.well-known/x",
            "/public/..x",
            "/public/x..",
            "/public/%2e%2e%2e",
            "/public/a%20b",
            "/public//double",
            "",
        ] {
            assert!(
                MessageHandler::unsafe_path_reason(path).is_none(),
                "{path} should be allowed"
            );
        }
    }

    #[test]
    fn a_prefix_is_stripped_only_on_a_segment_boundary() {
        let mut request = request_with(&[], "/apis/x?q=1");
        MessageHandler::strip_path_prefix(&mut request, "/api");
        assert_eq!(request.uri().path(), "/apis/x");

        let mut request = request_with(&[], "/api/x?q=1");
        MessageHandler::strip_path_prefix(&mut request, "/api");
        assert_eq!(request.uri().path(), "/x");
        assert_eq!(request.uri().query(), Some("q=1"));

        // A trailing-slash prefix keeps a leading slash on what is left.
        let mut request = request_with(&[], "/api/x");
        MessageHandler::strip_path_prefix(&mut request, "/api/");
        assert_eq!(request.uri().path(), "/x");
    }

    #[test]
    fn only_allowed_protocols_may_be_upgraded_to() {
        let allowed = vec!["websocket".to_string()];
        assert!(MessageHandler::upgrade_allowed("websocket", &allowed));
        assert!(MessageHandler::upgrade_allowed("WebSocket", &allowed));
        // The smuggling case: an h2c tunnel lets the client send the backend
        // arbitrary requests past every route rule.
        assert!(!MessageHandler::upgrade_allowed("h2c", &allowed));
        assert!(!MessageHandler::upgrade_allowed("websocket, h2c", &allowed));
        assert!(!MessageHandler::upgrade_allowed("websocket", &[]));
    }

    #[test]
    fn an_h2c_upgrade_leaves_nothing_behind_when_stripped() {
        // Declined upgrades are forwarded as ordinary requests, so none of the
        // handshake may survive -- HTTP2-Settings included, even when the
        // client did not name it in Connection.
        let mut request = request_with(
            &[
                ("host", "a"),
                ("connection", "Upgrade"),
                ("upgrade", "h2c"),
                ("http2-settings", "AAMAAABkAAQCAAAAAAIAAAAA"),
            ],
            "/",
        );
        MessageHandler::strip_hop_by_hop_headers(&mut request);
        let h = request.headers();
        assert!(!h.contains_key("upgrade"));
        assert!(!h.contains_key("connection"));
        assert!(!h.contains_key("http2-settings"));
    }
}
