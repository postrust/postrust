//! Vendored message handler from rpxy-lib: message_handler/*.rs
//!
//! This module handles request/response manipulation:
//! - X-Forwarded-* header handling
//! - Host header rewriting
//! - Request parsing

use crate::config::Route;
use crate::vendored::hyper_ext::{ProxyBody, empty_body, string_body};
use hyper::header::{HeaderName, HeaderValue, HOST};
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

/// Message handler for request/response manipulation.
pub struct MessageHandler;

impl MessageHandler {
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

        if path.starts_with(prefix) {
            let new_path = &path[prefix.len()..];
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
        let mut request = Request::builder()
            .uri("/api")
            .body(empty_body())
            .unwrap();

        MessageHandler::strip_path_prefix(&mut request, "/api");

        assert_eq!(request.uri().path(), "/");
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
        assert_eq!(
            request.headers().get("x-forwarded-proto").unwrap(),
            "https"
        );
        assert_eq!(
            request.headers().get("x-forwarded-host").unwrap(),
            "example.com"
        );
    }
}
