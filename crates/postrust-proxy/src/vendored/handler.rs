//! Vendored message handler from rpxy-lib: message_handler/*.rs
//!
//! This module handles request/response manipulation:
//! - X-Forwarded-* header handling
//! - Host header rewriting
//! - Request parsing

use crate::config::Route;
use crate::vendored::hyper_ext::{empty_body, string_body, ProxyBody};
use hyper::header::{
    HeaderName, HeaderValue, CONNECTION, CONTENT_LENGTH, HOST, TRANSFER_ENCODING,
};
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

impl MessageHandler {
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

        // X-Forwarded-Host: preserve original Host
        if let Some(host) = headers.get(HOST).cloned() {
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

    /// Create a 504 Gateway Timeout response.
    pub fn gateway_timeout() -> Response<ProxyBody> {
        Self::error_response(StatusCode::GATEWAY_TIMEOUT, "Gateway Timeout")
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
