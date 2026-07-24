//! Type definitions for the SaaS domain management module.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::IpAddr;
use uuid::Uuid;

/// Tenant status.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "VARCHAR", rename_all = "snake_case")]
pub enum TenantStatus {
    /// Active tenant
    #[default]
    Active,
    /// Suspended tenant
    Suspended,
    /// Pending activation
    Pending,
}

/// A SaaS tenant (customer).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Tenant {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub email: String,
    pub status: TenantStatus,
    pub plan: String,
    pub max_domains: i32,
    pub max_routes_per_domain: i32,
    pub settings: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Database row for tenant.
#[derive(Clone, Debug, sqlx::FromRow)]
pub struct TenantRow {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub email: String,
    pub status: String,
    pub plan: String,
    pub max_domains: i32,
    pub max_routes_per_domain: i32,
    pub settings: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<TenantRow> for Tenant {
    fn from(row: TenantRow) -> Self {
        Self {
            id: row.id,
            name: row.name,
            slug: row.slug,
            email: row.email,
            status: match row.status.as_str() {
                "suspended" => TenantStatus::Suspended,
                "pending" => TenantStatus::Pending,
                _ => TenantStatus::Active,
            },
            plan: row.plan,
            max_domains: row.max_domains,
            max_routes_per_domain: row.max_routes_per_domain,
            settings: row.settings,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

/// Domain verification status.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "VARCHAR", rename_all = "snake_case")]
pub enum VerificationStatus {
    /// Pending verification
    #[default]
    Pending,
    /// Successfully verified
    Verified,
    /// Verification failed
    Failed,
    /// Verification expired
    Expired,
}

/// Domain verification method.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "VARCHAR", rename_all = "snake_case")]
pub enum VerificationMethod {
    /// DNS TXT record verification
    #[default]
    Dns,
    /// HTTP file verification
    Http,
}

/// SSL certificate status.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "VARCHAR", rename_all = "snake_case")]
pub enum SslStatus {
    /// Pending provisioning
    #[default]
    Pending,
    /// Currently provisioning
    Provisioning,
    /// Certificate active
    Active,
    /// Provisioning failed
    Failed,
    /// Certificate expired
    Expired,
}

/// SSL provider type.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "VARCHAR", rename_all = "snake_case")]
pub enum SslProvider {
    /// Automatic via ACME/Let's Encrypt
    #[default]
    Acme,
    /// Manual certificate upload
    Manual,
    /// No SSL
    None,
}

/// A custom domain.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Domain {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub domain: String,
    pub verification_status: VerificationStatus,
    pub verification_method: VerificationMethod,
    pub verification_token: String,
    pub verification_attempts: i32,
    pub verified_at: Option<DateTime<Utc>>,
    pub last_verification_attempt: Option<DateTime<Utc>>,
    pub ssl_status: SslStatus,
    pub ssl_provider: SslProvider,
    pub ssl_expires_at: Option<DateTime<Utc>>,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Database row for domain with string types.
#[derive(Clone, Debug, sqlx::FromRow)]
pub struct DomainRow {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub domain: String,
    pub verification_status: String,
    pub verification_method: String,
    pub verification_token: String,
    pub verification_attempts: i32,
    pub verified_at: Option<DateTime<Utc>>,
    pub last_verification_attempt: Option<DateTime<Utc>>,
    pub ssl_status: String,
    pub ssl_provider: String,
    pub ssl_expires_at: Option<DateTime<Utc>>,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<DomainRow> for Domain {
    fn from(row: DomainRow) -> Self {
        Self {
            id: row.id,
            tenant_id: row.tenant_id,
            domain: row.domain,
            verification_status: match row.verification_status.as_str() {
                "verified" => VerificationStatus::Verified,
                "failed" => VerificationStatus::Failed,
                "expired" => VerificationStatus::Expired,
                _ => VerificationStatus::Pending,
            },
            verification_method: match row.verification_method.as_str() {
                "http" => VerificationMethod::Http,
                _ => VerificationMethod::Dns,
            },
            verification_token: row.verification_token,
            verification_attempts: row.verification_attempts,
            verified_at: row.verified_at,
            last_verification_attempt: row.last_verification_attempt,
            ssl_status: match row.ssl_status.as_str() {
                "provisioning" => SslStatus::Provisioning,
                "active" => SslStatus::Active,
                "failed" => SslStatus::Failed,
                "expired" => SslStatus::Expired,
                _ => SslStatus::Pending,
            },
            ssl_provider: match row.ssl_provider.as_str() {
                "manual" => SslProvider::Manual,
                "none" => SslProvider::None,
                _ => SslProvider::Acme,
            },
            ssl_expires_at: row.ssl_expires_at,
            enabled: row.enabled,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

/// Path matching type.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "VARCHAR", rename_all = "snake_case")]
pub enum DomainPathMatchType {
    /// Prefix matching (default)
    #[default]
    Prefix,
    /// Exact matching
    Exact,
    /// Regex matching
    Regex,
}

/// A domain route.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DomainRoute {
    pub id: Uuid,
    pub domain_id: Uuid,
    pub tenant_id: Uuid,
    pub name: String,
    pub path_pattern: String,
    pub path_type: DomainPathMatchType,
    pub methods: Option<Vec<String>>,
    pub priority: i32,
    pub upstream_id: Option<Uuid>,
    pub strip_path: bool,
    pub add_headers: HashMap<String, String>,
    pub remove_headers: Vec<String>,
    pub rate_limit_requests: Option<i32>,
    pub rate_limit_window_secs: Option<i32>,
    pub timeout_secs: i32,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Database row for domain route.
#[derive(Clone, Debug, sqlx::FromRow)]
pub struct DomainRouteRow {
    pub id: Uuid,
    pub domain_id: Uuid,
    pub tenant_id: Uuid,
    pub name: String,
    pub path_pattern: String,
    pub path_type: String,
    pub methods: Option<Vec<String>>,
    pub priority: i32,
    pub upstream_id: Option<Uuid>,
    pub strip_path: bool,
    pub add_headers: serde_json::Value,
    pub remove_headers: Vec<String>,
    pub rate_limit_requests: Option<i32>,
    pub rate_limit_window_secs: Option<i32>,
    pub timeout_secs: i32,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<DomainRouteRow> for DomainRoute {
    fn from(row: DomainRouteRow) -> Self {
        let add_headers: HashMap<String, String> =
            serde_json::from_value(row.add_headers).unwrap_or_default();

        Self {
            id: row.id,
            domain_id: row.domain_id,
            tenant_id: row.tenant_id,
            name: row.name,
            path_pattern: row.path_pattern,
            path_type: match row.path_type.as_str() {
                "exact" => DomainPathMatchType::Exact,
                "regex" => DomainPathMatchType::Regex,
                _ => DomainPathMatchType::Prefix,
            },
            methods: row.methods,
            priority: row.priority,
            upstream_id: row.upstream_id,
            strip_path: row.strip_path,
            add_headers,
            remove_headers: row.remove_headers,
            rate_limit_requests: row.rate_limit_requests,
            rate_limit_window_secs: row.rate_limit_window_secs,
            timeout_secs: row.timeout_secs,
            enabled: row.enabled,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

/// Load balancing strategy.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "VARCHAR", rename_all = "snake_case")]
pub enum DomainLoadBalanceStrategy {
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

/// A domain upstream (backend server group).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DomainUpstream {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub name: String,
    pub lb_strategy: DomainLoadBalanceStrategy,
    pub health_check_enabled: bool,
    pub health_check_path: String,
    pub health_check_interval_secs: i32,
    pub health_check_timeout_secs: i32,
    pub healthy_threshold: i32,
    pub unhealthy_threshold: i32,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Backends (loaded separately)
    #[serde(default)]
    pub backends: Vec<DomainBackend>,
}

/// Database row for domain upstream.
#[derive(Clone, Debug, sqlx::FromRow)]
pub struct DomainUpstreamRow {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub name: String,
    pub lb_strategy: String,
    pub health_check_enabled: bool,
    pub health_check_path: String,
    pub health_check_interval_secs: i32,
    pub health_check_timeout_secs: i32,
    pub healthy_threshold: i32,
    pub unhealthy_threshold: i32,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<DomainUpstreamRow> for DomainUpstream {
    fn from(row: DomainUpstreamRow) -> Self {
        Self {
            id: row.id,
            tenant_id: row.tenant_id,
            name: row.name,
            lb_strategy: match row.lb_strategy.as_str() {
                "least_connections" => DomainLoadBalanceStrategy::LeastConnections,
                "weighted" => DomainLoadBalanceStrategy::Weighted,
                "random" => DomainLoadBalanceStrategy::Random,
                "sticky" => DomainLoadBalanceStrategy::Sticky,
                _ => DomainLoadBalanceStrategy::RoundRobin,
            },
            health_check_enabled: row.health_check_enabled,
            health_check_path: row.health_check_path,
            health_check_interval_secs: row.health_check_interval_secs,
            health_check_timeout_secs: row.health_check_timeout_secs,
            healthy_threshold: row.healthy_threshold,
            unhealthy_threshold: row.unhealthy_threshold,
            enabled: row.enabled,
            created_at: row.created_at,
            updated_at: row.updated_at,
            backends: Vec::new(),
        }
    }
}

/// A backend server.
#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct DomainBackend {
    pub id: Uuid,
    pub upstream_id: Uuid,
    pub address: String,
    pub scheme: String,
    pub weight: i32,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
}

/// An API key for tenant authentication.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApiKey {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub name: String,
    /// Only available on creation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    pub key_prefix: String,
    pub scopes: Vec<String>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
}

/// Database row for API key.
#[derive(Clone, Debug, sqlx::FromRow)]
pub struct ApiKeyRow {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub name: String,
    pub key_hash: String,
    pub key_prefix: String,
    pub scopes: Vec<String>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
}

impl From<ApiKeyRow> for ApiKey {
    fn from(row: ApiKeyRow) -> Self {
        Self {
            id: row.id,
            tenant_id: row.tenant_id,
            name: row.name,
            key: None, // Never expose the key after creation
            key_prefix: row.key_prefix,
            scopes: row.scopes,
            last_used_at: row.last_used_at,
            expires_at: row.expires_at,
            enabled: row.enabled,
            created_at: row.created_at,
        }
    }
}

/// Verification challenge status.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChallengeStatus {
    /// Pending verification
    #[default]
    Pending,
    /// Currently checking
    Checking,
    /// Successfully verified
    Verified,
    /// Verification failed
    Failed,
}

/// A verification challenge.
#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct VerificationChallenge {
    pub id: Uuid,
    pub domain_id: Uuid,
    pub challenge_type: String,
    pub token: String,
    pub expected_value: String,
    pub status: String,
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub verified_at: Option<DateTime<Utc>>,
}

/// Audit log entry.
#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct AuditLogEntry {
    pub id: Uuid,
    pub tenant_id: Option<Uuid>,
    pub domain_id: Option<Uuid>,
    pub action: String,
    pub actor_type: String,
    pub actor_id: Option<String>,
    pub details: serde_json::Value,
    pub ip_address: Option<IpAddr>,
    pub created_at: DateTime<Utc>,
}

// ============================================================================
// API Request/Response Types
// ============================================================================

/// Create tenant request.
#[derive(Clone, Debug, Deserialize)]
pub struct CreateTenantRequest {
    pub name: String,
    pub slug: String,
    pub email: String,
    pub plan: Option<String>,
}

/// Update tenant request.
#[derive(Clone, Debug, Deserialize)]
pub struct UpdateTenantRequest {
    pub name: Option<String>,
    pub email: Option<String>,
    pub plan: Option<String>,
    pub settings: Option<serde_json::Value>,
}

/// Create domain request.
#[derive(Clone, Debug, Deserialize)]
pub struct CreateDomainRequest {
    pub domain: String,
    #[serde(default)]
    pub verification_method: VerificationMethod,
    #[serde(default)]
    pub ssl_provider: SslProvider,
}

/// Domain response with verification instructions.
#[derive(Clone, Debug, Serialize)]
pub struct DomainResponse {
    #[serde(flatten)]
    pub domain: Domain,
    pub verification_instructions: VerificationInstructions,
}

/// Verification instructions.
#[derive(Clone, Debug, Serialize)]
pub struct VerificationInstructions {
    pub method: VerificationMethod,
    /// For DNS verification
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dns_record_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dns_record_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dns_record_value: Option<String>,
    /// For HTTP verification
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http_expected_content: Option<String>,
}

impl VerificationInstructions {
    /// Create DNS verification instructions.
    pub fn dns(domain: &str, token: &str) -> Self {
        Self {
            method: VerificationMethod::Dns,
            dns_record_type: Some("TXT".to_string()),
            dns_record_name: Some(format!("_postrust-verification.{}", domain)),
            dns_record_value: Some(format!("postrust-verify={}", token)),
            http_url: None,
            http_expected_content: None,
        }
    }

    /// Create HTTP verification instructions.
    pub fn http(domain: &str, token: &str) -> Self {
        Self {
            method: VerificationMethod::Http,
            dns_record_type: None,
            dns_record_name: None,
            dns_record_value: None,
            http_url: Some(format!(
                "https://{}/.well-known/postrust-verification/{}",
                domain, token
            )),
            http_expected_content: Some(format!("postrust-verify={}", token)),
        }
    }
}

/// Verification result.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationResult {
    /// Domain verified successfully
    Verified,
    /// Verification pending
    Pending,
    /// Verification failed
    Failed { reason: String },
}

/// Create domain route request.
#[derive(Clone, Debug, Deserialize)]
pub struct CreateDomainRouteRequest {
    pub name: String,
    #[serde(default = "default_path_pattern")]
    pub path_pattern: String,
    #[serde(default)]
    pub path_type: DomainPathMatchType,
    pub methods: Option<Vec<String>>,
    pub upstream_id: Uuid,
    #[serde(default)]
    pub strip_path: bool,
    #[serde(default = "default_priority")]
    pub priority: i32,
    #[serde(default)]
    pub add_headers: HashMap<String, String>,
    #[serde(default)]
    pub remove_headers: Vec<String>,
    pub rate_limit_requests: Option<i32>,
    pub rate_limit_window_secs: Option<i32>,
    #[serde(default = "default_timeout")]
    pub timeout_secs: i32,
}

fn default_path_pattern() -> String {
    "/".to_string()
}

fn default_priority() -> i32 {
    100
}

fn default_timeout() -> i32 {
    30
}

/// Update domain route request.
#[derive(Clone, Debug, Deserialize)]
pub struct UpdateDomainRouteRequest {
    pub name: Option<String>,
    pub path_pattern: Option<String>,
    pub path_type: Option<DomainPathMatchType>,
    pub methods: Option<Vec<String>>,
    pub upstream_id: Option<Uuid>,
    pub strip_path: Option<bool>,
    pub priority: Option<i32>,
    pub add_headers: Option<HashMap<String, String>>,
    pub remove_headers: Option<Vec<String>>,
    pub rate_limit_requests: Option<i32>,
    pub rate_limit_window_secs: Option<i32>,
    pub timeout_secs: Option<i32>,
    pub enabled: Option<bool>,
}

/// Create upstream request.
#[derive(Clone, Debug, Deserialize)]
pub struct CreateUpstreamRequest {
    pub name: String,
    #[serde(default)]
    pub lb_strategy: DomainLoadBalanceStrategy,
    #[serde(default = "default_true")]
    pub health_check_enabled: bool,
    #[serde(default = "default_health_path")]
    pub health_check_path: String,
    #[serde(default = "default_health_interval")]
    pub health_check_interval_secs: i32,
    #[serde(default = "default_health_timeout")]
    pub health_check_timeout_secs: i32,
    #[serde(default = "default_healthy_threshold")]
    pub healthy_threshold: i32,
    #[serde(default = "default_unhealthy_threshold")]
    pub unhealthy_threshold: i32,
    #[serde(default)]
    pub backends: Vec<CreateBackendRequest>,
}

fn default_true() -> bool {
    true
}

fn default_health_path() -> String {
    "/health".to_string()
}

fn default_health_interval() -> i32 {
    30
}

fn default_health_timeout() -> i32 {
    5
}

fn default_healthy_threshold() -> i32 {
    2
}

fn default_unhealthy_threshold() -> i32 {
    3
}

/// Update upstream request.
#[derive(Clone, Debug, Deserialize)]
pub struct UpdateUpstreamRequest {
    pub name: Option<String>,
    pub lb_strategy: Option<DomainLoadBalanceStrategy>,
    pub health_check_enabled: Option<bool>,
    pub health_check_path: Option<String>,
    pub health_check_interval_secs: Option<i32>,
    pub health_check_timeout_secs: Option<i32>,
    pub healthy_threshold: Option<i32>,
    pub unhealthy_threshold: Option<i32>,
    pub enabled: Option<bool>,
}

/// Create backend request.
#[derive(Clone, Debug, Deserialize)]
pub struct CreateBackendRequest {
    pub address: String,
    #[serde(default = "default_scheme")]
    pub scheme: String,
    #[serde(default = "default_weight")]
    pub weight: i32,
}

fn default_scheme() -> String {
    "http".to_string()
}

fn default_weight() -> i32 {
    100
}

/// Create API key request.
#[derive(Clone, Debug, Deserialize)]
pub struct CreateApiKeyRequest {
    pub name: String,
    #[serde(default = "default_scopes")]
    pub scopes: Vec<String>,
    pub expires_at: Option<DateTime<Utc>>,
}

fn default_scopes() -> Vec<String> {
    vec!["domains:read".to_string(), "domains:write".to_string()]
}

/// Certificate upload request.
#[derive(Clone, Debug, Deserialize)]
pub struct UploadCertificateRequest {
    pub cert_pem: String,
    pub key_pem: String,
}

/// Tenant usage stats.
#[derive(Clone, Debug, Serialize)]
pub struct TenantUsage {
    pub domains_count: i64,
    pub domains_limit: i32,
    pub verified_domains: i64,
    pub routes_count: i64,
    pub upstreams_count: i64,
    pub api_keys_count: i64,
}
