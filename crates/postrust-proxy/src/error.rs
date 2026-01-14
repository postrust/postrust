//! Error types for the proxy module.

use thiserror::Error;

/// Result type for proxy operations.
pub type ProxyResult<T> = Result<T, ProxyError>;

/// Errors that can occur in the proxy module.
#[derive(Error, Debug)]
pub enum ProxyError {
    /// Configuration error
    #[error("Configuration error: {0}")]
    Config(String),

    /// Database error
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    /// IO error
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// TLS error
    #[error("TLS error: {0}")]
    Tls(String),

    /// ACME error
    #[error("ACME error: {0}")]
    Acme(String),

    /// HTTP error
    #[error("HTTP error: {0}")]
    Http(String),

    /// Upstream error
    #[error("Upstream error: {0}")]
    Upstream(String),

    /// Health check error
    #[error("Health check error: {0}")]
    HealthCheck(String),

    /// Rate limit exceeded
    #[error("Rate limit exceeded")]
    RateLimitExceeded,

    /// Route not found
    #[error("Route not found: {0}")]
    RouteNotFound(String),

    /// Backend not found
    #[error("Backend not found: {0}")]
    BackendNotFound(String),

    /// No healthy backends
    #[error("No healthy backends available for upstream: {0}")]
    NoHealthyBackends(String),

    /// Invalid URL
    #[error("Invalid URL: {0}")]
    InvalidUrl(#[from] url::ParseError),

    /// TOML parsing error
    #[error("TOML parsing error: {0}")]
    Toml(#[from] toml::de::Error),

    /// File watcher error
    #[error("File watcher error: {0}")]
    Notify(#[from] notify::Error),

    /// Internal error
    #[error("Internal error: {0}")]
    Internal(String),

    // SaaS Domain Management Errors

    /// Domain verification error
    #[error("Domain verification error: {0}")]
    Verification(String),

    /// Authentication error
    #[error("Authentication error: {0}")]
    Auth(String),

    /// Validation error
    #[error("Validation error: {0}")]
    Validation(String),

    /// Quota exceeded
    #[error("Quota exceeded: {0}")]
    QuotaExceeded(String),

    /// Resource conflict
    #[error("Conflict: {0}")]
    Conflict(String),

    /// Tenant suspended
    #[error("Tenant suspended")]
    TenantSuspended,

    /// Not found
    #[error("Not found: {0}")]
    NotFound(String),

    /// Forbidden
    #[error("Forbidden: {0}")]
    Forbidden(String),
}

impl ProxyError {
    /// Get the HTTP status code for this error.
    pub fn status_code(&self) -> http::StatusCode {
        match self {
            ProxyError::Config(_) => http::StatusCode::INTERNAL_SERVER_ERROR,
            ProxyError::Database(_) => http::StatusCode::INTERNAL_SERVER_ERROR,
            ProxyError::Io(_) => http::StatusCode::INTERNAL_SERVER_ERROR,
            ProxyError::Tls(_) => http::StatusCode::INTERNAL_SERVER_ERROR,
            ProxyError::Acme(_) => http::StatusCode::INTERNAL_SERVER_ERROR,
            ProxyError::Http(_) => http::StatusCode::BAD_GATEWAY,
            ProxyError::Upstream(_) => http::StatusCode::BAD_GATEWAY,
            ProxyError::HealthCheck(_) => http::StatusCode::SERVICE_UNAVAILABLE,
            ProxyError::RateLimitExceeded => http::StatusCode::TOO_MANY_REQUESTS,
            ProxyError::RouteNotFound(_) => http::StatusCode::NOT_FOUND,
            ProxyError::BackendNotFound(_) => http::StatusCode::NOT_FOUND,
            ProxyError::NoHealthyBackends(_) => http::StatusCode::SERVICE_UNAVAILABLE,
            ProxyError::InvalidUrl(_) => http::StatusCode::BAD_REQUEST,
            ProxyError::Toml(_) => http::StatusCode::INTERNAL_SERVER_ERROR,
            ProxyError::Notify(_) => http::StatusCode::INTERNAL_SERVER_ERROR,
            ProxyError::Internal(_) => http::StatusCode::INTERNAL_SERVER_ERROR,
            ProxyError::Verification(_) => http::StatusCode::BAD_REQUEST,
            ProxyError::Auth(_) => http::StatusCode::UNAUTHORIZED,
            ProxyError::Validation(_) => http::StatusCode::BAD_REQUEST,
            ProxyError::QuotaExceeded(_) => http::StatusCode::PAYMENT_REQUIRED,
            ProxyError::Conflict(_) => http::StatusCode::CONFLICT,
            ProxyError::TenantSuspended => http::StatusCode::FORBIDDEN,
            ProxyError::NotFound(_) => http::StatusCode::NOT_FOUND,
            ProxyError::Forbidden(_) => http::StatusCode::FORBIDDEN,
        }
    }
}
