//! Vendored message handler from rpxy-lib: message_handler/*.rs
//!
//! This module handles request/response manipulation:
//! - X-Forwarded-* header handling
//! - Host header rewriting
//! - Request parsing

use crate::config::Route;
use crate::vendored::hyper_ext::{empty_body, string_body, ProxyBody};
use hyper::header::{
    HeaderName, HeaderValue, CONNECTION, CONTENT_LENGTH, HOST, TRANSFER_ENCODING, UPGRADE,
};
use hyper::{Method, Version};
use hyper::{Request, Response, StatusCode};
use std::net::SocketAddr;

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

    /// Whether the request's body framing is ambiguous.
    ///
    /// RFC 9112 section 6.1: a message carrying both `Content-Length` and
    /// `Transfer-Encoding` is either a smuggling attempt or a broken client.
    /// `Transfer-Encoding` wins per the spec, but a proxy that quietly
    /// normalises the disagreement is how a smuggling chain starts, so reject
    /// it -- which is also how hyper's own parser already treats a duplicate
    /// `Content-Length`.
    ///
    /// Must be called *before* [`Self::strip_hop_by_hop_headers`], which
    /// removes the `Transfer-Encoding` this inspects.
    pub fn has_ambiguous_framing(request: &Request<ProxyBody>) -> bool {
        let headers = request.headers();
        headers.contains_key(CONTENT_LENGTH) && headers.contains_key(TRANSFER_ENCODING)
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
    pub fn add_forwarding_headers(
        request: &mut Request<ProxyBody>,
        client_addr: SocketAddr,
        proto: &str,
    ) {
        let authority = request.uri().authority().map(|a| a.to_string());
        let headers = request.headers_mut();
        let client_ip = client_addr.ip().to_string();

        // X-Forwarded-For: append client IP
        if let Some(existing) = headers.get(&headers::X_FORWARDED_FOR) {
            let mut new_value = existing.to_str().unwrap_or("").to_string();
            new_value.push_str(", ");
            new_value.push_str(&client_ip);
            if let Ok(value) = HeaderValue::from_str(&new_value) {
                headers.insert(headers::X_FORWARDED_FOR.clone(), value);
            }
        } else if let Ok(value) = HeaderValue::from_str(&client_ip) {
            headers.insert(headers::X_FORWARDED_FOR.clone(), value);
        }

        // X-Real-IP: set if not present
        if !headers.contains_key(&headers::X_REAL_IP) {
            if let Ok(value) = HeaderValue::from_str(&client_ip) {
                headers.insert(headers::X_REAL_IP.clone(), value);
            }
        }

        // X-Forwarded-Proto
        if let Ok(value) = HeaderValue::from_str(proto) {
            headers.insert(headers::X_FORWARDED_PROTO.clone(), value);
        }

        // X-Forwarded-Host: preserve the original authority. HTTP/2 carries it in
        // the :authority pseudo-header rather than Host, which reaches us on the
        // URI, so fall back to that before giving up.
        let original_host = headers
            .get(HOST)
            .cloned()
            .or_else(|| authority.and_then(|a| HeaderValue::from_str(&a).ok()));
        if let Some(host) = original_host {
            headers.insert(headers::X_FORWARDED_HOST.clone(), host);
        }
    }

    /// Rewrite the Host header for upstream.
    pub fn rewrite_host_header(request: &mut Request<ProxyBody>, upstream_host: &str) {
        if let Ok(value) = HeaderValue::from_str(upstream_host) {
            request.headers_mut().insert(HOST, value);
        }
    }

    /// Strip path prefix from request URI.
    pub fn strip_path_prefix(request: &mut Request<ProxyBody>, prefix: &str) {
        let uri = request.uri();
        let path = uri.path();

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

    /// Create a 502 Bad Gateway response.
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
    fn test_ambiguous_framing_detected() {
        // Regression: stripping Transfer-Encoding leaves a stale Content-Length
        // describing a body hyper has already re-framed, so this must be
        // rejected up front rather than forwarded.
        let request = Request::builder()
            .uri("/test")
            .header("host", "a")
            .header("content-length", "6")
            .header("transfer-encoding", "chunked")
            .body(empty_body())
            .unwrap();

        assert!(MessageHandler::has_ambiguous_framing(&request));
    }

    #[test]
    fn test_content_length_alone_is_not_ambiguous() {
        let request = Request::builder()
            .uri("/test")
            .header("host", "a")
            .header("content-length", "6")
            .body(empty_body())
            .unwrap();

        assert!(!MessageHandler::has_ambiguous_framing(&request));
    }

    #[test]
    fn test_transfer_encoding_alone_is_not_ambiguous() {
        let request = Request::builder()
            .uri("/test")
            .header("host", "a")
            .header("transfer-encoding", "chunked")
            .body(empty_body())
            .unwrap();

        assert!(!MessageHandler::has_ambiguous_framing(&request));
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
        MessageHandler::add_forwarding_headers(&mut request, client_addr, "http");

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
        MessageHandler::add_forwarding_headers(&mut request, client_addr, "https");

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
}
