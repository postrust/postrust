//! Configuration types for the proxy.

use http::HeaderMap;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Main proxy configuration.
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ProxyConfig {
    /// Server settings
    #[serde(default)]
    pub server: ServerConfig,

    /// TLS/ACME settings
    #[serde(default)]
    pub tls: TlsConfig,

    /// Default rate limiting settings
    #[serde(default)]
    pub rate_limit: RateLimitDefaults,

    /// Routes (for file-based config)
    #[serde(default)]
    pub routes: Vec<Route>,

    /// Upstreams (for file-based config)
    #[serde(default)]
    pub upstreams: Vec<Upstream>,
}

/// Server configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ServerConfig {
    /// Listen address for HTTP
    #[serde(default = "default_http_host")]
    pub http_host: String,

    /// Listen port for HTTP
    #[serde(default = "default_http_port")]
    pub http_port: u16,

    /// Dedicated HTTP/2-only listen port.
    ///
    /// Optional and additive: the main port keeps serving HTTP/1.1 and h2c by
    /// sniffing. That sniffing is why a corrupted HTTP/2 preface gets an
    /// HTTP/1 400 rather than GOAWAY(PROTOCOL_ERROR) -- it is indistinguishable
    /// from a malformed HTTP/1 request. A port that only ever speaks HTTP/2 has
    /// no such ambiguity.
    #[serde(default)]
    pub http2_port: Option<u16>,

    /// Listen address for HTTPS
    #[serde(default = "default_https_host")]
    pub https_host: String,

    /// Listen port for HTTPS
    #[serde(default = "default_https_port")]
    pub https_port: u16,

    /// Enable HTTPS listener
    #[serde(default)]
    pub https_enabled: bool,

    /// Database-backed config enabled
    #[serde(default = "default_true")]
    pub database_config: bool,

    /// Config file path (for file-based bootstrap)
    pub config_file: Option<String>,

    /// Enable file watcher for hot-reload
    #[serde(default)]
    pub watch_config_file: bool,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            http_host: default_http_host(),
            http_port: default_http_port(),
            http2_port: None,
            https_host: default_https_host(),
            https_port: default_https_port(),
            https_enabled: false,
            database_config: true,
            config_file: None,
            watch_config_file: false,
        }
    }
}

/// TLS configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TlsConfig {
    /// Enable ACME (Let's Encrypt)
    #[serde(default)]
    pub acme_enabled: bool,

    /// ACME directory URL
    #[serde(default = "default_acme_directory")]
    pub acme_directory: String,

    /// ACME contact email
    pub acme_email: Option<String>,

    /// Certificate storage directory
    #[serde(default = "default_cert_dir")]
    pub cert_dir: String,

    /// Use staging ACME server
    #[serde(default)]
    pub acme_staging: bool,

    /// PEM certificate chain for the HTTPS listener.
    ///
    /// Set this and `key_file` to serve TLS from a certificate on disk. Without
    /// them there is no HTTPS listener at all, which means HTTP/2 is only
    /// reachable as cleartext h2c and WebSocket only as `ws://`.
    #[serde(default)]
    pub cert_file: Option<String>,

    /// PEM private key matching `cert_file`.
    #[serde(default)]
    pub key_file: Option<String>,

    /// ACME directory URL.
    ///
    /// Defaults to Let's Encrypt production. `acme_staging = true` switches to
    /// staging, which issues untrusted certificates against far looser rate
    /// limits -- use it until the whole flow works, because production's limits
    /// are per-domain-per-week and are easy to exhaust while debugging.
    #[serde(default)]
    pub acme_directory_url: Option<String>,

    /// A PEM root to trust for the ACME directory's own TLS.
    ///
    /// Only for a CA with a private root, which in practice means a test CA
    /// such as Pebble. Never needed for Let's Encrypt.
    #[serde(default)]
    pub acme_root_pem: Option<String>,
}

impl Default for TlsConfig {
    fn default() -> Self {
        Self {
            acme_enabled: false,
            acme_directory: default_acme_directory(),
            acme_email: None,
            cert_dir: default_cert_dir(),
            acme_staging: false,
            cert_file: None,
            key_file: None,
            acme_directory_url: None,
            acme_root_pem: None,
        }
    }
}

/// Default rate limiting settings.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RateLimitDefaults {
    /// Default requests per window
    #[serde(default = "default_rate_limit_requests")]
    pub requests: u32,

    /// Default window size in seconds
    #[serde(default = "default_rate_limit_window")]
    pub window_secs: u32,

    /// Burst allowance
    #[serde(default = "default_burst")]
    pub burst: u32,
}

impl Default for RateLimitDefaults {
    fn default() -> Self {
        Self {
            requests: default_rate_limit_requests(),
            window_secs: default_rate_limit_window(),
            burst: default_burst(),
        }
    }
}

/// A proxy route configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Route {
    /// Route ID (database)
    pub id: Option<Uuid>,

    /// Route name
    pub name: String,

    /// Description
    pub description: Option<String>,

    /// Matching criteria
    #[serde(default, rename = "match")]
    pub match_: RouteMatch,

    /// Priority (higher = matched first)
    #[serde(default = "default_priority")]
    pub priority: i32,

    /// Upstream name or ID
    pub upstream: String,

    /// Strip matched path prefix
    #[serde(default)]
    pub strip_path: bool,

    /// Headers to add to proxied requests
    #[serde(default)]
    pub add_headers: HashMap<String, String>,

    /// Headers to remove from proxied requests
    #[serde(default)]
    pub remove_headers: Vec<String>,

    /// Rate limiting for this route
    pub rate_limit: Option<RouteRateLimit>,

    /// Request timeout in seconds
    #[serde(default = "default_timeout")]
    pub timeout_secs: u32,

    /// Retry count on failure
    #[serde(default)]
    pub retry_count: u32,

    /// Route enabled
    #[serde(default = "default_true")]
    pub enabled: bool,
}

/// Route matching criteria.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RouteMatch {
    /// Host pattern (supports wildcards)
    pub host: Option<String>,

    /// Path pattern
    pub path: Option<String>,

    /// Path matching type
    #[serde(default)]
    pub path_type: PathMatchType,

    /// Headers to match
    #[serde(default)]
    pub headers: HashMap<String, String>,

    /// HTTP methods to match
    pub methods: Option<Vec<String>>,
}

impl RouteMatch {
    /// Whether a request matches these criteria.
    ///
    /// Every field that is set has to match; a field left unset matches
    /// anything. That is the only reading under which adding a criterion can
    /// never widen a route.
    ///
    /// `path_type`, `methods` and `headers` were all declarable and all ignored
    /// before this existed -- the route filter compared host and path prefix and
    /// nothing else. Each of those silent no-ops widened a route past what its
    /// author wrote: `path_type = "exact"` on `/health` also matched
    /// `/health-internal`, and a route restricted to `methods = ["GET"]`
    /// accepted `DELETE`.
    pub fn matches(&self, host: &str, path: &str, method: &str, headers: &HeaderMap) -> bool {
        self.host_matches(host)
            && self.path_matches(path)
            && self.method_matches(method)
            && self.headers_match(headers)
    }

    /// Host, exactly or by a leading `*.` wildcard.
    ///
    /// `*` alone means any host, which is what an absent host also means.
    fn host_matches(&self, host: &str) -> bool {
        let Some(pattern) = self.host.as_deref() else {
            return true;
        };
        let pattern = pattern.trim().to_ascii_lowercase();
        let host = host.trim().to_ascii_lowercase();

        if pattern == "*" || pattern.is_empty() {
            return true;
        }
        if pattern == host {
            return true;
        }
        // `*.example.com` covers one label, not the apex and not two labels --
        // the same rule certificates use.
        match pattern.strip_prefix("*.") {
            Some(suffix) if !suffix.is_empty() => host
                .strip_suffix(&format!(".{suffix}"))
                .is_some_and(|label| !label.is_empty() && !label.contains('.')),
            _ => false,
        }
    }

    /// Path, by the declared [`PathMatchType`].
    ///
    /// A regex that does not compile matches nothing, and says so once. The
    /// alternative -- treating it as a prefix, or matching everything -- turns a
    /// typo into a route that catches traffic it was never meant to.
    fn path_matches(&self, path: &str) -> bool {
        let Some(pattern) = self.path.as_deref() else {
            return true;
        };
        match self.path_type {
            PathMatchType::Prefix => path.starts_with(pattern),
            PathMatchType::Exact => path == pattern,
            PathMatchType::Regex => match regex::Regex::new(pattern) {
                Ok(re) => re.is_match(path),
                Err(error) => {
                    tracing::warn!(
                        %pattern, %error,
                        "route path regex does not compile; the route matches nothing"
                    );
                    false
                }
            },
        }
    }

    /// Method, case-insensitively. An empty list means any.
    fn method_matches(&self, method: &str) -> bool {
        match self.methods.as_deref() {
            None | Some([]) => true,
            Some(allowed) => allowed.iter().any(|m| m.eq_ignore_ascii_case(method)),
        }
    }

    /// Every named header must be present with that value, case-insensitively
    /// on the name and exactly on the value.
    fn headers_match(&self, headers: &HeaderMap) -> bool {
        self.headers.iter().all(|(name, expected)| {
            headers
                .get(name.as_str())
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value == expected)
        })
    }
}

/// Path matching type.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PathMatchType {
    /// Prefix matching (default)
    #[default]
    Prefix,
    /// Exact matching
    Exact,
    /// Regex matching
    Regex,
}

/// Per-route rate limiting.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RouteRateLimit {
    /// Requests per window
    pub requests: u32,
    /// Window size in seconds
    pub window_secs: u32,
    /// Rate limit key
    #[serde(default)]
    pub key: RateLimitKey,
}

/// Rate limit key type.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RateLimitKey {
    /// Rate limit by client IP
    #[default]
    ClientIp,
    /// Rate limit by header value
    Header(String),
    /// Rate limit by route (global for route)
    Route,
}

/// An upstream (group of backend servers).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Upstream {
    /// Upstream ID (database)
    pub id: Option<Uuid>,

    /// Upstream name
    pub name: String,

    /// Description
    pub description: Option<String>,

    /// Load balancing strategy
    #[serde(default)]
    pub lb_strategy: LoadBalanceStrategy,

    /// Backend servers
    #[serde(default)]
    pub backends: Vec<Backend>,

    /// Health check configuration
    #[serde(default)]
    pub health_check: HealthCheckConfig,

    /// Upstream enabled
    #[serde(default = "default_true")]
    pub enabled: bool,
}

impl Upstream {
    /// The identifier this upstream is keyed by in the routing tables.
    ///
    /// `id` is only set for upstreams loaded from the database. A TOML config
    /// leaves it `None`, and both registration sites used to skip any upstream
    /// without one -- so a file-configured proxy logged "Loaded N routes and N
    /// upstreams" and then answered every request with 503 "Upstream not
    /// found". Names are unique within a config, so deriving a stable UUID
    /// from the name is enough to key the tables consistently.
    pub fn resolved_id(&self) -> Uuid {
        self.id
            .unwrap_or_else(|| Uuid::new_v5(&Uuid::NAMESPACE_OID, self.name.as_bytes()))
    }
}

/// Load balancing strategy.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LoadBalanceStrategy {
    /// Round-robin (default)
    #[default]
    RoundRobin,
    /// Least connections
    LeastConnections,
    /// Weighted
    Weighted,
    /// Random
    Random,
    /// Sticky (cookie-based)
    Sticky,
}

/// A backend server.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Backend {
    /// Backend ID (database)
    pub id: Option<Uuid>,

    /// Server address (host:port)
    pub address: String,

    /// HTTP or HTTPS
    #[serde(default = "default_scheme")]
    pub scheme: String,

    /// Weight for weighted load balancing
    #[serde(default = "default_weight")]
    pub weight: u32,

    /// Backend enabled
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Protocol to speak on the upstream connection.
    ///
    /// Defaults to HTTP/1.1 regardless of how the client arrived, because
    /// HTTP/2 is a per-hop protocol. Set `http_version = "h2c"` for a backend
    /// that speaks cleartext HTTP/2 with prior knowledge. There is no
    /// negotiation: h2c has no ALPN to fall back on, so this has to be declared.
    #[serde(default)]
    pub http_version: UpstreamHttpVersion,
}

/// Which HTTP version to speak to an upstream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UpstreamHttpVersion {
    /// HTTP/1.1. The default, and correct for almost every backend.
    #[default]
    #[serde(alias = "http1", alias = "http/1.1")]
    Http11,
    /// Cleartext HTTP/2 with prior knowledge.
    #[serde(alias = "http2", alias = "h2")]
    H2c,
}

/// Health check configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HealthCheckConfig {
    /// Health check enabled
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Health check path
    #[serde(default = "default_health_path")]
    pub path: String,

    /// Check interval in seconds
    #[serde(default = "default_health_interval")]
    pub interval_secs: u32,

    /// Check timeout in seconds
    #[serde(default = "default_health_timeout")]
    pub timeout_secs: u32,

    /// Healthy threshold (consecutive successes)
    #[serde(default = "default_healthy_threshold")]
    pub healthy_threshold: u32,

    /// Unhealthy threshold (consecutive failures)
    #[serde(default = "default_unhealthy_threshold")]
    pub unhealthy_threshold: u32,
}

impl Default for HealthCheckConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            path: default_health_path(),
            interval_secs: default_health_interval(),
            timeout_secs: default_health_timeout(),
            healthy_threshold: default_healthy_threshold(),
            unhealthy_threshold: default_unhealthy_threshold(),
        }
    }
}

// Default value functions
fn default_http_host() -> String {
    "0.0.0.0".into()
}
fn default_http_port() -> u16 {
    8080
}
fn default_https_host() -> String {
    "0.0.0.0".into()
}
fn default_https_port() -> u16 {
    8443
}
fn default_true() -> bool {
    true
}
fn default_acme_directory() -> String {
    "https://acme-v02.api.letsencrypt.org/directory".into()
}
fn default_cert_dir() -> String {
    "./certs".into()
}
fn default_rate_limit_requests() -> u32 {
    1000
}
fn default_rate_limit_window() -> u32 {
    60
}
fn default_burst() -> u32 {
    50
}
fn default_priority() -> i32 {
    100
}
fn default_timeout() -> u32 {
    30
}
fn default_scheme() -> String {
    "http".into()
}
fn default_weight() -> u32 {
    100
}
fn default_health_path() -> String {
    "/health".into()
}
fn default_health_interval() -> u32 {
    10
}
fn default_health_timeout() -> u32 {
    5
}
fn default_healthy_threshold() -> u32 {
    2
}
fn default_unhealthy_threshold() -> u32 {
    3
}

/// ACME configuration for automatic certificate management.
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct AcmeConfig {
    /// ACME enabled
    #[serde(default)]
    pub enabled: bool,

    /// Contact email
    pub email: Option<String>,

    /// Use staging server
    #[serde(default)]
    pub staging: bool,

    /// Domains to request certificates for
    #[serde(default)]
    pub domains: Vec<String>,
}

#[cfg(test)]
mod upstream_version_tests {
    use super::*;

    fn header_map(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.insert(
                http::header::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                http::HeaderValue::from_str(value).unwrap(),
            );
        }
        map
    }

    /// A match with nothing set, so each test can set only what it is about.
    fn any() -> RouteMatch {
        RouteMatch::default()
    }

    #[test]
    fn an_empty_match_matches_anything() {
        let m = any();
        assert!(m.matches("example.com", "/anything", "GET", &HeaderMap::new()));
        assert!(m.matches("", "/", "DELETE", &HeaderMap::new()));
    }

    #[test]
    fn a_prefix_path_matches_by_prefix() {
        let m = RouteMatch {
            path: Some("/v1".into()),
            ..any()
        };
        assert!(m.matches("h", "/v1", "GET", &HeaderMap::new()));
        assert!(m.matches("h", "/v1/users", "GET", &HeaderMap::new()));
        assert!(!m.matches("h", "/v2", "GET", &HeaderMap::new()));
    }

    #[test]
    fn an_exact_path_does_not_match_a_longer_one() {
        // The widening this guards against: before `path_type` was honoured,
        // an "exact" route on /health also caught /health-internal.
        let m = RouteMatch {
            path: Some("/health".into()),
            path_type: PathMatchType::Exact,
            ..any()
        };
        assert!(m.matches("h", "/health", "GET", &HeaderMap::new()));
        assert!(!m.matches("h", "/health-internal", "GET", &HeaderMap::new()));
        assert!(!m.matches("h", "/health/deep", "GET", &HeaderMap::new()));
    }

    #[test]
    fn a_regex_path_matches_the_pattern() {
        let m = RouteMatch {
            path: Some(r"^/v[0-9]+/users$".into()),
            path_type: PathMatchType::Regex,
            ..any()
        };
        assert!(m.matches("h", "/v1/users", "GET", &HeaderMap::new()));
        assert!(m.matches("h", "/v22/users", "GET", &HeaderMap::new()));
        assert!(!m.matches("h", "/v1/users/1", "GET", &HeaderMap::new()));
        assert!(!m.matches("h", "/users", "GET", &HeaderMap::new()));
    }

    #[test]
    fn a_regex_that_does_not_compile_matches_nothing() {
        // Not "everything", and not "treat it as a prefix". Either would turn a
        // typo into a route that catches traffic it was never meant to.
        let m = RouteMatch {
            path: Some("/v1/(unclosed".into()),
            path_type: PathMatchType::Regex,
            ..any()
        };
        assert!(!m.matches("h", "/v1/anything", "GET", &HeaderMap::new()));
        assert!(!m.matches("h", "/v1/(unclosed", "GET", &HeaderMap::new()));
    }

    #[test]
    fn methods_are_honoured_case_insensitively() {
        let m = RouteMatch {
            methods: Some(vec!["GET".into(), "head".into()]),
            ..any()
        };
        assert!(m.matches("h", "/", "GET", &HeaderMap::new()));
        assert!(m.matches("h", "/", "get", &HeaderMap::new()));
        assert!(m.matches("h", "/", "HEAD", &HeaderMap::new()));
        // The widening this guards against: a read-only route accepting writes.
        assert!(!m.matches("h", "/", "DELETE", &HeaderMap::new()));
        assert!(!m.matches("h", "/", "POST", &HeaderMap::new()));
    }

    #[test]
    fn an_empty_method_list_means_any() {
        let m = RouteMatch {
            methods: Some(vec![]),
            ..any()
        };
        assert!(m.matches("h", "/", "PATCH", &HeaderMap::new()));
    }

    #[test]
    fn every_named_header_must_be_present_with_that_value() {
        let m = RouteMatch {
            headers: HashMap::from([
                ("x-tenant".to_string(), "acme".to_string()),
                ("x-env".to_string(), "prod".to_string()),
            ]),
            ..any()
        };

        assert!(m.matches(
            "h",
            "/",
            "GET",
            &header_map(&[("x-tenant", "acme"), ("x-env", "prod")])
        ));
        // All of them, not any of them.
        assert!(!m.matches("h", "/", "GET", &header_map(&[("x-tenant", "acme")])));
        // The value has to match too.
        assert!(!m.matches(
            "h",
            "/",
            "GET",
            &header_map(&[("x-tenant", "other"), ("x-env", "prod")])
        ));
        assert!(!m.matches("h", "/", "GET", &HeaderMap::new()));
    }

    #[test]
    fn header_names_are_case_insensitive() {
        let m = RouteMatch {
            headers: HashMap::from([("X-Tenant".to_string(), "acme".to_string())]),
            ..any()
        };
        assert!(m.matches("h", "/", "GET", &header_map(&[("x-tenant", "acme")])));
    }

    #[test]
    fn a_host_matches_exactly_or_by_a_single_label_wildcard() {
        let exact = RouteMatch {
            host: Some("api.example.com".into()),
            ..any()
        };
        assert!(exact.matches("api.example.com", "/", "GET", &HeaderMap::new()));
        assert!(!exact.matches("www.example.com", "/", "GET", &HeaderMap::new()));

        let wild = RouteMatch {
            host: Some("*.example.com".into()),
            ..any()
        };
        assert!(wild.matches("api.example.com", "/", "GET", &HeaderMap::new()));
        // Not the apex, and not two labels.
        assert!(!wild.matches("example.com", "/", "GET", &HeaderMap::new()));
        assert!(!wild.matches("a.b.example.com", "/", "GET", &HeaderMap::new()));
        // And not a suffix match, which would catch an attacker's domain.
        assert!(!wild.matches("api.example.com.evil.test", "/", "GET", &HeaderMap::new()));
    }

    #[test]
    fn a_star_host_matches_anything() {
        let m = RouteMatch {
            host: Some("*".into()),
            ..any()
        };
        assert!(m.matches("anything.test", "/", "GET", &HeaderMap::new()));
        assert!(m.matches("", "/", "GET", &HeaderMap::new()));
    }

    #[test]
    fn host_matching_is_case_insensitive() {
        let m = RouteMatch {
            host: Some("API.Example.COM".into()),
            ..any()
        };
        assert!(m.matches("api.example.com", "/", "GET", &HeaderMap::new()));
    }

    #[test]
    fn every_set_criterion_has_to_hold() {
        // Adding a criterion must never widen a route.
        let m = RouteMatch {
            host: Some("api.example.com".into()),
            path: Some("/v1".into()),
            path_type: PathMatchType::Prefix,
            methods: Some(vec!["POST".into()]),
            headers: HashMap::from([("x-key".to_string(), "k".to_string())]),
        };
        let headers = header_map(&[("x-key", "k")]);

        assert!(m.matches("api.example.com", "/v1/x", "POST", &headers));
        assert!(!m.matches("other.example.com", "/v1/x", "POST", &headers));
        assert!(!m.matches("api.example.com", "/v2/x", "POST", &headers));
        assert!(!m.matches("api.example.com", "/v1/x", "GET", &headers));
        assert!(!m.matches("api.example.com", "/v1/x", "POST", &HeaderMap::new()));
    }

    #[test]
    fn test_backend_defaults_to_http11() {
        // HTTP/2 is per-hop: a backend says nothing, it gets HTTP/1.1.
        let backend: Backend = toml::from_str(r#"address = "127.0.0.1:8080""#).unwrap();
        assert_eq!(backend.http_version, UpstreamHttpVersion::Http11);
    }

    #[test]
    fn test_backend_h2c_aliases() {
        // h2c has no ALPN, so it must be declared; accept the spellings someone
        // would reasonably reach for.
        for spelling in ["h2c", "h2", "http2"] {
            let toml_src = format!("address = \"127.0.0.1:8080\"\nhttp_version = \"{spelling}\"");
            let backend: Backend = toml::from_str(&toml_src).unwrap();
            assert_eq!(
                backend.http_version,
                UpstreamHttpVersion::H2c,
                "spelling {spelling} should select h2c"
            );
        }
    }

    #[test]
    fn test_backend_http11_aliases() {
        for spelling in ["http11", "http1"] {
            let toml_src = format!("address = \"127.0.0.1:8080\"\nhttp_version = \"{spelling}\"");
            let backend: Backend = toml::from_str(&toml_src).unwrap();
            assert_eq!(backend.http_version, UpstreamHttpVersion::Http11);
        }
    }

    #[test]
    fn test_http2_port_is_optional_and_off_by_default() {
        let server: ServerConfig = toml::from_str("").unwrap();
        assert_eq!(server.http2_port, None);

        let server: ServerConfig = toml::from_str("http2_port = 19081").unwrap();
        assert_eq!(server.http2_port, Some(19081));
    }
}
