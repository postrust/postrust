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

    /// Isolation level for the transaction every request runs in.
    ///
    /// `READ COMMITTED` unless set, which is PostgreSQL's own default; a
    /// stricter level is issued as `SET TRANSACTION ISOLATION LEVEL` on the
    /// request's transaction.
    #[serde(default = "default_tx_isolation")]
    pub db_tx_isolation: IsolationLevel,

    /// Use prepared statements
    #[serde(default = "default_true")]
    pub db_prepared_statements: bool,

    /// Schemas added to the `search_path` of every request, after the
    /// request's own schema.
    ///
    /// These are not exposed as API endpoints; they are where a function an
    /// exposed one calls, or a type it references, is allowed to live. An
    /// aggregate returning a domain declared elsewhere cannot resolve that
    /// domain without this. `public` by default, as in PostgREST.
    #[serde(default = "default_extra_search_path")]
    pub db_extra_search_path: Vec<String>,

    /// LISTEN/NOTIFY channel for schema reload
    #[serde(default = "default_db_channel")]
    pub db_channel: String,

    /// Enable NOTIFY-based schema cache reload
    #[serde(default)]
    pub db_channel_enabled: bool,

    /// Function called on the request's transaction before its own query,
    /// after every setting is applied. Anything it raises aborts the request.
    pub db_pre_request: Option<String>,

    /// Maximum rows allowed in a response
    pub db_max_rows: Option<i64>,

    /// Enable aggregate functions in `select`.
    ///
    /// Off by default, as in PostgREST: an aggregate over a large table costs
    /// far more than the request looks like it asks for.
    #[serde(default)]
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

    /// Origins allowed by CORS.
    ///
    /// Empty means every origin is allowed, which is the default and what
    /// `PGRST_SERVER_CORS_ORIGINS="*"` selects explicitly. A non-empty list
    /// restricts the `Access-Control-Allow-Origin` response to exactly those
    /// origins; anything else gets no CORS headers and is refused by the
    /// browser.
    #[serde(default)]
    pub server_cors_origins: Vec<String>,

    /// Maximum accepted request body, in bytes.
    ///
    /// The REST handler applies this when it reads the body; the routes that
    /// take a body extractor get it through axum's `DefaultBodyLimit`. Before
    /// this was read, the limit in force was a 10 MiB constant in the request
    /// handler, whatever the variable said.
    #[serde(default = "default_max_body_size")]
    pub max_body_size: usize,

    /// Listen on a Unix domain socket instead of host and port.
    pub server_unix_socket: Option<String>,

    /// Serve liveness and readiness on a second port of their own.
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

    /// Cache validated tokens, so a repeated token is not verified again.
    #[serde(default = "default_true")]
    pub jwt_cache_enabled: bool,

    /// How long a cached validation may be reused, in seconds.
    ///
    /// A token's own `exp` bounds this; the cache never answers with a token
    /// past its expiry. `0` turns the cache off, as in PostgREST.
    #[serde(default = "default_jwt_cache_max")]
    pub jwt_cache_max_lifetime: u64,

    // ========================================================================
    // Hasura Authentication
    // ========================================================================
    /// The shared secret that authenticates an administrator, and the switch
    /// that makes `x-hasura-*` headers mean anything at all.
    ///
    /// Unset, this server ignores those headers and reads session variables
    /// only from a verified token -- because honouring a header nothing
    /// authenticated would let any caller name its own identity, and a policy
    /// reading a value the caller chose is not a policy. Hasura instead treats
    /// an unsecured deployment as wholly administrative; the divergence is
    /// deliberate and is recorded in `postrust_auth::hasura`.
    pub hasura_admin_secret: Option<String>,

    /// The role a request falls to when nothing authenticated it. Hasura's
    /// `HASURA_GRAPHQL_UNAUTHORIZED_ROLE`; unset means such a request is
    /// refused rather than answered as a stranger.
    pub hasura_unauthorized_role: Option<String>,

    // ========================================================================
    // OpenAPI Settings
    // ========================================================================
    /// Address to advertise in the specification's `servers`, for when the
    /// server sits behind a reverse proxy and its own address is not the one
    /// clients use.
    pub openapi_server_proxy_uri: Option<String>,

    /// OpenAPI mode: disabled, follow-privileges, ignore-privileges
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

    // ========================================================================
    // Compatibility Settings
    // ========================================================================
    /// PostgREST compatibility mode.
    ///
    /// When enabled, the REST surface is also served at the root (so canonical
    /// PostgREST paths like `/rpc/<name>` and `/<table>` work in addition to
    /// the `/api`-prefixed paths), and RPC responses are un-wrapped to match
    /// PostgREST's shape (bare object/scalar for non-set-returning functions,
    /// a top-level array for set-returning ones) instead of the array-wrapped,
    /// function-name-keyed default.
    #[serde(default)]
    pub compat_mode: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            db_uri: default_db_uri(),
            db_schemas: default_db_schemas(),
            db_anon_role: None,
            db_pool_size: default_pool_size(),
            db_pool_timeout: default_pool_timeout(),
            db_tx_isolation: default_tx_isolation(),
            db_prepared_statements: true,
            db_extra_search_path: default_extra_search_path(),
            db_channel: default_db_channel(),
            db_channel_enabled: false,
            db_pre_request: None,
            db_max_rows: None,
            db_aggregates_enabled: false,
            server_host: default_host(),
            server_port: default_port(),
            server_cors_origins: Vec::new(),
            max_body_size: default_max_body_size(),
            server_unix_socket: None,
            admin_server_port: None,
            jwt_secret: None,
            jwt_secret_is_base64: false,
            jwt_aud: None,
            jwt_role_claim_key: default_jwt_role_claim(),
            jwt_cache_enabled: true,
            jwt_cache_max_lifetime: default_jwt_cache_max(),
            hasura_admin_secret: None,
            hasura_unauthorized_role: None,
            openapi_server_proxy_uri: None,
            openapi_mode: OpenApiMode::FollowPrivileges,
            log_level: default_log_level(),
            role_settings: HashMap::new(),
            app_settings: HashMap::new(),
            compat_mode: false,
        }
    }
}

/// Say that a variable was set to something unusable, rather than silently
/// applying the default. A rejected value and an unset one are different
/// mistakes and the operator can only fix the one they are told about.
fn warn_ignored(var: &str, value: &str, expected: &str) {
    tracing::warn!("Ignoring {}={:?}: expected {}", var, value, expected);
}

/// The truthy spellings accepted for a boolean environment variable.
fn env_bool(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "true" | "1" | "yes" | "on"
    )
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
        // `PGRST_DB_POOL` is PostgREST's own name; `PGRST_DB_POOL_SIZE` is the
        // one this project's documentation has always given, including the
        // production checklist in docs/deployment.md. Only the former was read,
        // so anybody who followed our own instructions silently kept the
        // default pool.
        for var in ["PGRST_DB_POOL", "PGRST_DB_POOL_SIZE"] {
            if let Ok(size) = std::env::var(var) {
                match size.parse::<u32>() {
                    Ok(n) if n > 0 => config.db_pool_size = n,
                    _ => warn_ignored(var, &size, "a positive integer"),
                }
            }
        }
        if let Ok(secs) = std::env::var("PGRST_DB_POOL_TIMEOUT") {
            match secs.parse::<u64>() {
                Ok(n) if n > 0 => config.db_pool_timeout = n,
                _ => warn_ignored("PGRST_DB_POOL_TIMEOUT", &secs, "a positive integer"),
            }
        }
        if let Ok(level) = std::env::var("PGRST_DB_TX_ISOLATION") {
            match IsolationLevel::parse_config(&level) {
                Some(l) => config.db_tx_isolation = l,
                None => warn_ignored(
                    "PGRST_DB_TX_ISOLATION",
                    &level,
                    "one of: read committed, repeatable read, serializable",
                ),
            }
        }
        if let Ok(v) = std::env::var("PGRST_DB_EXTRA_SEARCH_PATH") {
            config.db_extra_search_path = v
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }

        if let Ok(v) = std::env::var("PGRST_DB_AGGREGATES_ENABLED") {
            config.db_aggregates_enabled = env_bool(&v);
        }
        if let Ok(v) = std::env::var("PGRST_DB_PREPARED_STATEMENTS") {
            config.db_prepared_statements = env_bool(&v);
        }
        if let Ok(v) = std::env::var("PGRST_DB_PRE_REQUEST") {
            if !v.trim().is_empty() {
                config.db_pre_request = Some(v.trim().to_string());
            }
        }
        if let Ok(v) = std::env::var("PGRST_DB_CHANNEL") {
            if !v.trim().is_empty() {
                config.db_channel = v.trim().to_string();
            }
        }
        if let Ok(v) = std::env::var("PGRST_DB_CHANNEL_ENABLED") {
            config.db_channel_enabled = env_bool(&v);
        }

        // `PGRST_DB_MAX_ROWS` mirrors PostgREST's `db-max-rows`. Accept both.
        for var in ["PGRST_DB_MAX_ROWS", "PGRST_MAX_ROWS"] {
            if let Ok(max_rows) = std::env::var(var) {
                match max_rows.parse::<i64>() {
                    Ok(n) if n >= 0 => config.db_max_rows = Some(n),
                    _ => warn_ignored(var, &max_rows, "a non-negative integer"),
                }
            }
        }
        if let Ok(secret) = std::env::var("PGRST_JWT_SECRET") {
            config.jwt_secret = Some(secret);
        }
        if let Ok(aud) = std::env::var("PGRST_JWT_AUD") {
            config.jwt_aud = Some(aud);
        }
        // Both of these reach the JWT layer already -- `JwtConfig` is built
        // from them in every binary. Only the environment read was missing, so
        // a base64 secret was verified against its own text and a role claim
        // outside `role` was never found.
        if let Ok(v) = std::env::var("PGRST_JWT_SECRET_IS_BASE64") {
            config.jwt_secret_is_base64 = env_bool(&v);
        }
        if let Ok(key) = std::env::var("PGRST_JWT_ROLE_CLAIM_KEY") {
            if !key.trim().is_empty() {
                config.jwt_role_claim_key = key;
            }
        }
        if let Ok(v) = std::env::var("PGRST_JWT_CACHE_ENABLED") {
            config.jwt_cache_enabled = env_bool(&v);
        }
        // Zero is meaningful here -- it turns the cache off, as in PostgREST --
        // so unlike the other durations it is not rejected.
        if let Ok(v) = std::env::var("PGRST_JWT_CACHE_MAX_LIFETIME") {
            match v.parse::<u64>() {
                Ok(n) => config.jwt_cache_max_lifetime = n,
                Err(_) => warn_ignored(
                    "PGRST_JWT_CACHE_MAX_LIFETIME",
                    &v,
                    "a non-negative integer number of seconds",
                ),
            }
        }
        // Hasura's own spelling is accepted beside ours, because a deployment
        // migrating from it has these in a compose file already and the point
        // of the mode is that the client does not have to be rewritten. The
        // `PGRST_` name wins where both are set, on the grounds that it is the
        // more specific instruction to this server.
        for var in ["HASURA_GRAPHQL_ADMIN_SECRET", "PGRST_HASURA_ADMIN_SECRET"] {
            if let Ok(secret) = std::env::var(var) {
                if !secret.is_empty() {
                    config.hasura_admin_secret = Some(secret);
                }
            }
        }
        for var in [
            "HASURA_GRAPHQL_UNAUTHORIZED_ROLE",
            "PGRST_HASURA_UNAUTHORIZED_ROLE",
        ] {
            if let Ok(role) = std::env::var(var) {
                if !role.is_empty() {
                    config.hasura_unauthorized_role = Some(role);
                }
            }
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
        // A bare `*` is the documented way to spell "any origin", and is the
        // default. An empty list means the same thing, so a value of only
        // separators does not accidentally lock everybody out.
        if let Ok(origins) = std::env::var("PGRST_SERVER_CORS_ORIGINS") {
            config.server_cors_origins = if origins.trim() == "*" {
                Vec::new()
            } else {
                origins
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            };
        }
        if let Ok(bytes) = std::env::var("PGRST_MAX_BODY_SIZE") {
            match bytes.parse::<usize>() {
                Ok(n) if n > 0 => config.max_body_size = n,
                _ => warn_ignored("PGRST_MAX_BODY_SIZE", &bytes, "a positive integer"),
            }
        }
        if let Ok(level) = std::env::var("PGRST_LOG_LEVEL") {
            match LogLevel::parse_config(&level) {
                Some(l) => config.log_level = l,
                None => warn_ignored(
                    "PGRST_LOG_LEVEL",
                    &level,
                    "one of: crit, error, warn, info, debug",
                ),
            }
        }
        if let Ok(path) = std::env::var("PGRST_SERVER_UNIX_SOCKET") {
            if !path.trim().is_empty() {
                config.server_unix_socket = Some(path.trim().to_string());
            }
        }
        if let Ok(port) = std::env::var("PGRST_ADMIN_SERVER_PORT") {
            match port.parse::<u16>() {
                Ok(p) if p > 0 => config.admin_server_port = Some(p),
                _ => warn_ignored("PGRST_ADMIN_SERVER_PORT", &port, "a port number"),
            }
        }
        if let Ok(uri) = std::env::var("PGRST_OPENAPI_SERVER_PROXY_URI") {
            if !uri.trim().is_empty() {
                config.openapi_server_proxy_uri = Some(uri.trim().to_string());
            }
        }
        if let Ok(mode) = std::env::var("PGRST_OPENAPI_MODE") {
            match OpenApiMode::parse_config(&mode) {
                Some(m) => config.openapi_mode = m,
                None => warn_ignored(
                    "PGRST_OPENAPI_MODE",
                    &mode,
                    "one of: disabled, follow-privileges, ignore-privileges",
                ),
            }
        }

        // `PGRST_ROLE_SETTINGS` is JSON rather than a per-role variable,
        // because a PostgreSQL role name may hold characters an environment
        // variable name cannot.
        if let Ok(raw) = std::env::var("PGRST_ROLE_SETTINGS") {
            match serde_json::from_str::<HashMap<String, RoleSettings>>(&raw) {
                Ok(settings) => config.role_settings = settings,
                Err(e) => warn_ignored(
                    "PGRST_ROLE_SETTINGS",
                    &raw,
                    &format!("a JSON object of role to settings ({e})"),
                ),
            }
        }

        // `PGRST_APP_SETTINGS_<NAME>` becomes `app.settings.<name>`, which is
        // where PostgREST puts it and where a policy expects to read it back.
        for (key, value) in std::env::vars() {
            if let Some(name) = key.strip_prefix("PGRST_APP_SETTINGS_") {
                if !name.is_empty() {
                    config.app_settings.insert(name.to_ascii_lowercase(), value);
                }
            }
        }
        // Accept either the PGRST_-prefixed name (for parity with other options)
        // or a POSTRUST_-prefixed alias.
        for var in ["PGRST_COMPAT_MODE", "POSTRUST_COMPAT_MODE"] {
            if let Ok(v) = std::env::var(var) {
                config.compat_mode = env_bool(&v);
            }
        }

        config
    }

    /// Get the default schema (first in the list).
    pub fn default_schema(&self) -> &str {
        self.db_schemas
            .first()
            .map(|s| s.as_str())
            .unwrap_or("public")
    }
}

// Serialized as the spelling configuration uses, not as the Rust variant name,
// so that what this writes is what `parse_config` reads back. Deriving
// `Serialize` here instead emits `"ReadCommitted"`, which then fails to
// deserialize -- and `Serializable` happens to survive the round trip, which
// makes the breakage look intermittent rather than total.
impl Serialize for IsolationLevel {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_config_str())
    }
}

impl<'de> Deserialize<'de> for IsolationLevel {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::parse_config(&raw).ok_or_else(|| {
            serde::de::Error::custom(format!(
                "unknown isolation level {raw:?}: expected one of \
                 read committed, repeatable read, serializable"
            ))
        })
    }
}

impl Serialize for OpenApiMode {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_config_str())
    }
}

impl<'de> Deserialize<'de> for OpenApiMode {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::parse_config(&raw).ok_or_else(|| {
            serde::de::Error::custom(format!(
                "unknown OpenAPI mode {raw:?}: expected one of \
                 disabled, follow-privileges, ignore-privileges"
            ))
        })
    }
}

impl Serialize for LogLevel {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_config_str())
    }
}

impl<'de> Deserialize<'de> for LogLevel {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::parse_config(&raw).ok_or_else(|| {
            serde::de::Error::custom(format!(
                "unknown log level {raw:?}: expected one of \
                 crit, error, warn, info, debug"
            ))
        })
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
///
/// Deserialized from the spelling the documentation uses (`"serializable"`,
/// `"read committed"`), not from the Rust variant name, so a `role_settings`
/// entry reads the same as the environment variable.
#[derive(Clone, Debug, PartialEq, Eq)]
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

    /// Parse the spelling used in configuration.
    ///
    /// Case-insensitive, and a hyphen, underscore or space separates the words
    /// equally -- the documentation writes `read committed` and an environment
    /// file is as likely to carry `read-committed`.
    pub fn parse_config(s: &str) -> Option<Self> {
        match s
            .trim()
            .to_ascii_lowercase()
            .replace(['-', '_'], " ")
            .as_str()
        {
            "read committed" => Some(Self::ReadCommitted),
            "repeatable read" => Some(Self::RepeatableRead),
            "serializable" => Some(Self::Serializable),
            _ => None,
        }
    }

    /// The inverse of [`parse_config`](Self::parse_config).
    pub fn as_config_str(&self) -> &'static str {
        match self {
            Self::ReadCommitted => "read committed",
            Self::RepeatableRead => "repeatable read",
            Self::Serializable => "serializable",
        }
    }
}

/// OpenAPI generation mode.
///
/// PostgREST also has a `security-definer` mode, where the specification comes
/// from a user-supplied `SECURITY DEFINER` function. There is no configuration
/// here for naming that function, so the variant is absent rather than present
/// and behaving as one of the others.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpenApiMode {
    Disabled,
    FollowPrivileges,
    IgnorePrivileges,
}

impl OpenApiMode {
    /// Parse the spelling used in configuration. Case-insensitive, and a
    /// hyphen or an underscore separates the words equally.
    pub fn parse_config(s: &str) -> Option<Self> {
        match s
            .trim()
            .to_ascii_lowercase()
            .replace(['-', '_'], " ")
            .as_str()
        {
            "disabled" => Some(Self::Disabled),
            "follow privileges" => Some(Self::FollowPrivileges),
            "ignore privileges" => Some(Self::IgnorePrivileges),
            _ => None,
        }
    }

    /// The inverse of [`parse_config`](Self::parse_config).
    pub fn as_config_str(&self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::FollowPrivileges => "follow-privileges",
            Self::IgnorePrivileges => "ignore-privileges",
        }
    }
}

/// Log levels.
#[derive(Clone, Debug, PartialEq, Eq)]
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

    /// Parse the spelling used in configuration. Case-insensitive.
    pub fn parse_config(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "crit" | "critical" => Some(Self::Crit),
            "error" => Some(Self::Error),
            "warn" | "warning" => Some(Self::Warn),
            "info" => Some(Self::Info),
            "debug" => Some(Self::Debug),
            _ => None,
        }
    }

    /// The inverse of [`parse_config`](Self::parse_config).
    pub fn as_config_str(&self) -> &'static str {
        match self {
            Self::Crit => "crit",
            Self::Error => "error",
            Self::Warn => "warn",
            Self::Info => "info",
            Self::Debug => "debug",
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

fn default_tx_isolation() -> IsolationLevel {
    IsolationLevel::ReadCommitted
}

fn default_max_body_size() -> usize {
    10 * 1024 * 1024
}

fn default_extra_search_path() -> Vec<String> {
    vec!["public".to_string()]
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

// `info`, not `error`. The documentation has always said `info`, and the
// server's own fallback filter is `postrust=info`, so `error` was a third
// answer that agreed with neither -- harmless only for as long as nothing
// read the field.
fn default_log_level() -> LogLevel {
    LogLevel::Info
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

    #[test]
    fn isolation_level_accepts_the_documented_spellings() {
        // The documentation writes `read committed`; an environment file is as
        // likely to carry either separator, or capitals.
        for s in [
            "read committed",
            "READ COMMITTED",
            "read-committed",
            " Read_Committed ",
        ] {
            assert_eq!(
                IsolationLevel::parse_config(s),
                Some(IsolationLevel::ReadCommitted),
                "{s:?}"
            );
        }
        assert_eq!(
            IsolationLevel::parse_config("serializable"),
            Some(IsolationLevel::Serializable)
        );
        assert_eq!(
            IsolationLevel::parse_config("repeatable read"),
            Some(IsolationLevel::RepeatableRead)
        );
        // Not a level. Rejected rather than resolved to a default, so the
        // caller can say so.
        assert_eq!(IsolationLevel::parse_config("dirty read"), None);
        assert_eq!(IsolationLevel::parse_config(""), None);
    }

    #[test]
    fn log_level_accepts_the_documented_spellings() {
        assert_eq!(LogLevel::parse_config("INFO"), Some(LogLevel::Info));
        assert_eq!(LogLevel::parse_config("warning"), Some(LogLevel::Warn));
        assert_eq!(LogLevel::parse_config("crit"), Some(LogLevel::Crit));
        // `trace` was listed in the documentation and is not a level this
        // server has. It is rejected rather than silently read as `debug`.
        assert_eq!(LogLevel::parse_config("trace"), None);
    }

    #[test]
    fn default_log_level_matches_the_documented_one() {
        // The struct default disagreed with both the documentation and the
        // server's own fallback filter for as long as nothing read it.
        assert_eq!(AppConfig::default().log_level, LogLevel::Info);
    }

    #[test]
    fn env_bool_accepts_the_usual_spellings() {
        for s in ["true", "TRUE", "1", "yes", "on", " On "] {
            assert!(env_bool(s), "{s:?}");
        }
        for s in ["false", "0", "no", "off", "", "maybe"] {
            assert!(!env_bool(s), "{s:?}");
        }
    }

    // `from_env` reads process-global state, so these run one at a time.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Set the given variables, load, then remove them again.
    fn with_env(vars: &[(&str, &str)]) -> AppConfig {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        for (k, v) in vars {
            std::env::set_var(k, v);
        }
        let config = AppConfig::from_env();
        for (k, _) in vars {
            std::env::remove_var(k);
        }
        config
    }

    #[test]
    fn documented_variables_are_read() {
        // Every one of these was documented and read by nothing. The features
        // behind them already worked -- only the environment read was missing.
        let config = with_env(&[
            ("PGRST_DB_POOL_SIZE", "42"),
            ("PGRST_DB_POOL_TIMEOUT", "7"),
            ("PGRST_DB_TX_ISOLATION", "serializable"),
            ("PGRST_JWT_SECRET_IS_BASE64", "true"),
            ("PGRST_JWT_ROLE_CLAIM_KEY", "user.role"),
            ("PGRST_LOG_LEVEL", "warn"),
            ("PGRST_MAX_BODY_SIZE", "1024"),
            (
                "PGRST_SERVER_CORS_ORIGINS",
                "https://a.example, https://b.example",
            ),
        ]);

        assert_eq!(config.db_pool_size, 42);
        assert_eq!(config.db_pool_timeout, 7);
        assert_eq!(config.db_tx_isolation, IsolationLevel::Serializable);
        assert!(config.jwt_secret_is_base64);
        assert_eq!(config.jwt_role_claim_key, "user.role");
        assert_eq!(config.log_level, LogLevel::Warn);
        assert_eq!(config.max_body_size, 1024);
        assert_eq!(
            config.server_cors_origins,
            vec![
                "https://a.example".to_string(),
                "https://b.example".to_string()
            ]
        );
    }

    #[test]
    fn pool_size_accepts_postgrests_own_name() {
        assert_eq!(with_env(&[("PGRST_DB_POOL", "5")]).db_pool_size, 5);
    }

    #[test]
    fn a_star_origin_means_any_origin() {
        // Empty is how "any" is carried, so `*` must not become a literal
        // origin that matches nothing.
        assert!(with_env(&[("PGRST_SERVER_CORS_ORIGINS", "*")])
            .server_cors_origins
            .is_empty());
        assert!(with_env(&[("PGRST_SERVER_CORS_ORIGINS", " , ")])
            .server_cors_origins
            .is_empty());
    }

    #[test]
    fn an_unusable_value_leaves_the_default() {
        let config = with_env(&[
            ("PGRST_DB_POOL_SIZE", "lots"),
            ("PGRST_DB_POOL_TIMEOUT", "0"),
            ("PGRST_MAX_BODY_SIZE", "-1"),
            ("PGRST_DB_TX_ISOLATION", "dirty read"),
            ("PGRST_LOG_LEVEL", "shout"),
        ]);

        let d = AppConfig::default();
        assert_eq!(config.db_pool_size, d.db_pool_size);
        assert_eq!(config.db_pool_timeout, d.db_pool_timeout);
        assert_eq!(config.max_body_size, d.max_body_size);
        assert_eq!(config.db_tx_isolation, d.db_tx_isolation);
        assert_eq!(config.log_level, d.log_level);
    }

    #[test]
    fn the_formerly_dead_fields_are_read() {
        // Every one of these was a public field that nothing set and nothing
        // consumed.
        let config = with_env(&[
            ("PGRST_DB_PREPARED_STATEMENTS", "false"),
            ("PGRST_DB_PRE_REQUEST", "auth.check"),
            ("PGRST_DB_CHANNEL", "my_channel"),
            ("PGRST_DB_CHANNEL_ENABLED", "yes"),
            ("PGRST_JWT_CACHE_ENABLED", "false"),
            ("PGRST_JWT_CACHE_MAX_LIFETIME", "60"),
            ("PGRST_SERVER_UNIX_SOCKET", "/tmp/postrust.sock"),
            ("PGRST_ADMIN_SERVER_PORT", "3001"),
            ("PGRST_OPENAPI_SERVER_PROXY_URI", "https://api.example.com"),
            ("PGRST_OPENAPI_MODE", "ignore-privileges"),
        ]);

        assert!(!config.db_prepared_statements);
        assert_eq!(config.db_pre_request.as_deref(), Some("auth.check"));
        assert_eq!(config.db_channel, "my_channel");
        assert!(config.db_channel_enabled);
        assert!(!config.jwt_cache_enabled);
        assert_eq!(config.jwt_cache_max_lifetime, 60);
        assert_eq!(
            config.server_unix_socket.as_deref(),
            Some("/tmp/postrust.sock")
        );
        assert_eq!(config.admin_server_port, Some(3001));
        assert_eq!(
            config.openapi_server_proxy_uri.as_deref(),
            Some("https://api.example.com")
        );
        assert_eq!(config.openapi_mode, OpenApiMode::IgnorePrivileges);
    }

    #[test]
    fn a_zero_jwt_cache_lifetime_is_kept_because_it_turns_the_cache_off() {
        // Unlike the other durations, 0 is a meaningful value here and must
        // not be rejected as unusable.
        assert_eq!(
            with_env(&[("PGRST_JWT_CACHE_MAX_LIFETIME", "0")]).jwt_cache_max_lifetime,
            0
        );
    }

    #[test]
    fn app_settings_come_from_a_prefix() {
        let config = with_env(&[
            ("PGRST_APP_SETTINGS_JWT_LIFETIME", "3600"),
            ("PGRST_APP_SETTINGS_TenantName", "acme"),
        ]);
        // Lower-cased, because that is the name a policy reads back with
        // `current_setting('app.settings.<name>')`.
        assert_eq!(
            config.app_settings.get("jwt_lifetime").map(String::as_str),
            Some("3600")
        );
        assert_eq!(
            config.app_settings.get("tenantname").map(String::as_str),
            Some("acme")
        );
    }

    #[test]
    fn role_settings_parse_from_json_using_the_documented_spellings() {
        let config = with_env(&[(
            "PGRST_ROLE_SETTINGS",
            r#"{"web_user":{"isolation_level":"serializable","statement_timeout":5000}}"#,
        )]);
        let web_user = config.role_settings.get("web_user").expect("role present");
        assert_eq!(web_user.isolation_level, Some(IsolationLevel::Serializable));
        assert_eq!(web_user.statement_timeout, Some(5000));
    }

    #[test]
    fn unparseable_role_settings_leave_the_map_empty() {
        assert!(with_env(&[("PGRST_ROLE_SETTINGS", "{not json")])
            .role_settings
            .is_empty());
        // A valid JSON object naming a level that does not exist is refused
        // as a whole rather than half-applied.
        assert!(with_env(&[(
            "PGRST_ROLE_SETTINGS",
            r#"{"r":{"isolation_level":"dirty"}}"#
        )])
        .role_settings
        .is_empty());
    }

    #[test]
    fn openapi_mode_accepts_the_documented_spellings() {
        assert_eq!(
            OpenApiMode::parse_config("follow-privileges"),
            Some(OpenApiMode::FollowPrivileges)
        );
        assert_eq!(
            OpenApiMode::parse_config("IGNORE_PRIVILEGES"),
            Some(OpenApiMode::IgnorePrivileges)
        );
        assert_eq!(
            OpenApiMode::parse_config("disabled"),
            Some(OpenApiMode::Disabled)
        );
        // PostgREST's fourth mode needs a function this server has no
        // configuration for, so it is not accepted rather than aliased.
        assert_eq!(OpenApiMode::parse_config("security-definer"), None);
    }

    #[test]
    fn the_config_enums_round_trip_through_json() {
        // Deriving `Serialize` emits the Rust variant name while the
        // hand-written `Deserialize` reads the configuration spelling, so a
        // value this type writes could not be read back. `Serializable`
        // survived by coincidence -- its variant name lowercases to the
        // accepted spelling -- which made the breakage look intermittent.
        for level in [
            IsolationLevel::ReadCommitted,
            IsolationLevel::RepeatableRead,
            IsolationLevel::Serializable,
        ] {
            let json = serde_json::to_string(&level).unwrap();
            let back: IsolationLevel = serde_json::from_str(&json).unwrap();
            assert_eq!(back, level, "{json}");
        }

        for mode in [
            OpenApiMode::Disabled,
            OpenApiMode::FollowPrivileges,
            OpenApiMode::IgnorePrivileges,
        ] {
            let json = serde_json::to_string(&mode).unwrap();
            assert_eq!(serde_json::from_str::<OpenApiMode>(&json).unwrap(), mode);
        }

        for level in [
            LogLevel::Crit,
            LogLevel::Error,
            LogLevel::Warn,
            LogLevel::Info,
            LogLevel::Debug,
        ] {
            let json = serde_json::to_string(&level).unwrap();
            assert_eq!(serde_json::from_str::<LogLevel>(&json).unwrap(), level);
        }
    }

    #[test]
    fn a_whole_config_round_trips_through_json() {
        // The struct derives both, which invites persisting it.
        let mut config = AppConfig::default();
        config.role_settings.insert(
            "web_user".to_string(),
            RoleSettings {
                isolation_level: Some(IsolationLevel::RepeatableRead),
                statement_timeout: Some(5000),
            },
        );

        let json = serde_json::to_string(&config).unwrap();
        let back: AppConfig = serde_json::from_str(&json).expect("config should round trip");

        assert_eq!(back.db_tx_isolation, config.db_tx_isolation);
        assert_eq!(back.log_level, config.log_level);
        assert_eq!(back.openapi_mode, config.openapi_mode);
        assert_eq!(
            back.role_settings["web_user"].isolation_level,
            Some(IsolationLevel::RepeatableRead)
        );
    }

    #[test]
    fn the_serialized_spelling_is_the_configuration_spelling() {
        // Not the Rust variant name: what is written should be what an
        // operator could paste into the environment.
        assert_eq!(
            serde_json::to_string(&IsolationLevel::ReadCommitted).unwrap(),
            "\"read committed\""
        );
        assert_eq!(
            serde_json::to_string(&OpenApiMode::FollowPrivileges).unwrap(),
            "\"follow-privileges\""
        );
        assert_eq!(serde_json::to_string(&LogLevel::Warn).unwrap(), "\"warn\"");
    }

    #[test]
    fn an_empty_role_claim_key_is_not_accepted() {
        // An empty key would look up nothing and every request would fall to
        // the anonymous role.
        assert_eq!(
            with_env(&[("PGRST_JWT_ROLE_CLAIM_KEY", "  ")]).jwt_role_claim_key,
            AppConfig::default().jwt_role_claim_key
        );
    }
}
