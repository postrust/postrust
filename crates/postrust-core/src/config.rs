//! Configuration for Postrust.
//!
//! Mirrors PostgREST's configuration options.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Main application configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AppConfig {
    // ========================================================================
    // Database Settings
    // ========================================================================
    /// PostgreSQL connection URI
    #[serde(default = "default_db_uri")]
    pub db_uri: String,

    /// Schemas to expose via the API
    #[serde(default = "default_db_schemas")]
    pub db_schemas: Vec<String>,

    /// Role for unauthenticated requests
    pub db_anon_role: Option<String>,

    /// Connection pool size
    #[serde(default = "default_pool_size")]
    pub db_pool_size: u32,

    /// Pool acquisition timeout in seconds
    #[serde(default = "default_pool_timeout")]
    pub db_pool_timeout: u64,

    /// Use prepared statements
    #[serde(default = "default_true")]
    pub db_prepared_statements: bool,

    /// Extra search path schemas
    #[serde(default)]
    pub db_extra_search_path: Vec<String>,

    /// LISTEN/NOTIFY channel for schema reload
    #[serde(default = "default_db_channel")]
    pub db_channel: String,

    /// Enable NOTIFY-based schema cache reload
    #[serde(default)]
    pub db_channel_enabled: bool,

    /// Pre-request function to call
    pub db_pre_request: Option<String>,

    /// Maximum rows allowed in a response
    pub db_max_rows: Option<i64>,

    /// Enable aggregate functions
    #[serde(default = "default_true")]
    pub db_aggregates_enabled: bool,

    // ========================================================================
    // Server Settings
    // ========================================================================
    /// Server host to bind
    #[serde(default = "default_host")]
    pub server_host: String,

    /// Server port
    #[serde(default = "default_port")]
    pub server_port: u16,

    /// Unix socket path (alternative to host/port)
    pub server_unix_socket: Option<String>,

    /// Admin server port (for health checks)
    pub admin_server_port: Option<u16>,

    // ========================================================================
    // JWT Settings
    // ========================================================================
    /// JWT secret key (or JWKS URL)
    pub jwt_secret: Option<String>,

    /// JWT secret as base64
    #[serde(default)]
    pub jwt_secret_is_base64: bool,

    /// JWT audience claim to validate
    pub jwt_aud: Option<String>,

    /// JWT claim that contains the role
    #[serde(default = "default_jwt_role_claim")]
    pub jwt_role_claim_key: String,

    /// Cache JWT validations
    #[serde(default = "default_true")]
    pub jwt_cache_enabled: bool,

    /// JWT cache max entries
    #[serde(default = "default_jwt_cache_max")]
    pub jwt_cache_max_lifetime: u64,

    // ========================================================================
    // OpenAPI Settings
    // ========================================================================
    /// OpenAPI server URL
    pub openapi_server_proxy_uri: Option<String>,

    /// OpenAPI mode: disabled, follow-privileges, ignore-privileges, security-definer
    #[serde(default = "default_openapi_mode")]
    pub openapi_mode: OpenApiMode,

    // ========================================================================
    // Logging Settings
    // ========================================================================
    /// Log level: crit, error, warn, info, debug
    #[serde(default = "default_log_level")]
    pub log_level: LogLevel,

    // ========================================================================
    // Role Settings
    // ========================================================================
    /// Per-role settings (isolation level, timeout)
    #[serde(default)]
    pub role_settings: HashMap<String, RoleSettings>,

    /// App-level settings to expose via GUC
    #[serde(default)]
    pub app_settings: HashMap<String, String>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            db_uri: default_db_uri(),
            db_schemas: default_db_schemas(),
            db_anon_role: None,
            db_pool_size: default_pool_size(),
            db_pool_timeout: default_pool_timeout(),
            db_prepared_statements: true,
            db_extra_search_path: vec![],
            db_channel: default_db_channel(),
            db_channel_enabled: false,
            db_pre_request: None,
            db_max_rows: None,
            db_aggregates_enabled: true,
            server_host: default_host(),
            server_port: default_port(),
            server_unix_socket: None,
            admin_server_port: None,
            jwt_secret: None,
            jwt_secret_is_base64: false,
            jwt_aud: None,
            jwt_role_claim_key: default_jwt_role_claim(),
            jwt_cache_enabled: true,
            jwt_cache_max_lifetime: default_jwt_cache_max(),
            openapi_server_proxy_uri: None,
            openapi_mode: OpenApiMode::FollowPrivileges,
            log_level: LogLevel::Error,
            role_settings: HashMap::new(),
            app_settings: HashMap::new(),
        }
    }
}

impl AppConfig {
    /// Load configuration from environment variables.
    pub fn from_env() -> Self {
        let mut config = Self::default();

        if let Ok(uri) = std::env::var("PGRST_DB_URI") {
            config.db_uri = uri;
        }
        if let Ok(uri) = std::env::var("DATABASE_URL") {
            config.db_uri = uri;
        }
        if let Ok(schemas) = std::env::var("PGRST_DB_SCHEMAS") {
            config.db_schemas = schemas.split(',').map(|s| s.trim().to_string()).collect();
        }
        if let Ok(role) = std::env::var("PGRST_DB_ANON_ROLE") {
            config.db_anon_role = Some(role);
        }
        if let Ok(size) = std::env::var("PGRST_DB_POOL") {
            if let Ok(n) = size.parse() {
                config.db_pool_size = n;
            }
        }
        if let Ok(secret) = std::env::var("PGRST_JWT_SECRET") {
            config.jwt_secret = Some(secret);
        }
        if let Ok(aud) = std::env::var("PGRST_JWT_AUD") {
            config.jwt_aud = Some(aud);
        }
        if let Ok(host) = std::env::var("PGRST_SERVER_HOST") {
            config.server_host = host;
        }
        if let Ok(port) = std::env::var("PGRST_SERVER_PORT") {
            if let Ok(p) = port.parse() {
                config.server_port = p;
            }
        }
        if let Ok(port) = std::env::var("PORT") {
            if let Ok(p) = port.parse() {
                config.server_port = p;
            }
        }

        config
    }

    /// Get the default schema (first in the list).
    pub fn default_schema(&self) -> &str {
        self.db_schemas.first().map(|s| s.as_str()).unwrap_or("public")
    }
}

/// Per-role settings.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RoleSettings {
    /// Isolation level for this role
    pub isolation_level: Option<IsolationLevel>,
    /// Statement timeout in milliseconds
    pub statement_timeout: Option<u64>,
}

/// Transaction isolation levels.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum IsolationLevel {
    ReadCommitted,
    RepeatableRead,
    Serializable,
}

impl IsolationLevel {
    pub fn to_sql(&self) -> &'static str {
        match self {
            Self::ReadCommitted => "READ COMMITTED",
            Self::RepeatableRead => "REPEATABLE READ",
            Self::Serializable => "SERIALIZABLE",
        }
    }
}

/// OpenAPI generation mode.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OpenApiMode {
    Disabled,
    FollowPrivileges,
    IgnorePrivileges,
    SecurityDefiner,
}

/// Log levels.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LogLevel {
    Crit,
    Error,
    Warn,
    Info,
    Debug,
}

impl LogLevel {
    pub fn to_tracing(&self) -> tracing::Level {
        match self {
            Self::Crit | Self::Error => tracing::Level::ERROR,
            Self::Warn => tracing::Level::WARN,
            Self::Info => tracing::Level::INFO,
            Self::Debug => tracing::Level::DEBUG,
        }
    }
}

// Default value functions
fn default_db_uri() -> String {
    "postgresql://localhost/postgres".to_string()
}

fn default_db_schemas() -> Vec<String> {
    vec!["public".to_string()]
}

fn default_pool_size() -> u32 {
    10
}

fn default_pool_timeout() -> u64 {
    10
}

fn default_db_channel() -> String {
    "pgrst".to_string()
}

fn default_host() -> String {
    "127.0.0.1".to_string()
}

fn default_port() -> u16 {
    3000
}

fn default_jwt_role_claim() -> String {
    "role".to_string()
}

fn default_jwt_cache_max() -> u64 {
    3600
}

fn default_openapi_mode() -> OpenApiMode {
    OpenApiMode::FollowPrivileges
}

fn default_log_level() -> LogLevel {
    LogLevel::Error
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = AppConfig::default();
        assert_eq!(config.server_port, 3000);
        assert_eq!(config.db_pool_size, 10);
        assert!(config.db_prepared_statements);
    }

    #[test]
    fn test_default_schema() {
        let mut config = AppConfig::default();
        assert_eq!(config.default_schema(), "public");

        config.db_schemas = vec!["api".to_string(), "public".to_string()];
        assert_eq!(config.default_schema(), "api");
    }

    #[test]
    fn test_isolation_level_sql() {
        assert_eq!(IsolationLevel::ReadCommitted.to_sql(), "READ COMMITTED");
        assert_eq!(IsolationLevel::Serializable.to_sql(), "SERIALIZABLE");
    }
}
