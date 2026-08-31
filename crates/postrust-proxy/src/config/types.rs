//! Configuration types for the proxy.

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
}

impl Default for TlsConfig {
    fn default() -> Self {
        Self {
            acme_enabled: false,
            acme_directory: default_acme_directory(),
            acme_email: None,
            cert_dir: default_cert_dir(),
            acme_staging: false,
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
