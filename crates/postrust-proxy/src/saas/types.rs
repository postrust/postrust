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
    /// Primary key.
    pub id: Uuid,
    /// Display name, for humans.
    pub name: String,
    /// Short URL-safe identifier, unique across tenants.
    pub slug: String,
    /// Contact address for this tenant.
    pub email: String,
    /// Whether the tenant may act. A suspended tenant keeps its data.
    pub status: TenantStatus,
    /// Plan name. Free-form: quotas are the two fields below, not this string.
    pub plan: String,
    /// How many domains this tenant may register.
    pub max_domains: i32,
    /// How many routes each of its domains may have.
    pub max_routes_per_domain: i32,
    /// Arbitrary per-tenant settings, untouched by the proxy.
    pub settings: serde_json::Value,
    /// When the tenant was created.
    pub created_at: DateTime<Utc>,
    /// When it last changed.
    pub updated_at: DateTime<Utc>,
}

/// Database row for tenant.
///
/// The `*Row` types in this module are `pub(crate)`: they mirror the schema
/// column for column and exist only so `sqlx::query_as` has something to decode
/// into. Making them public would freeze the schema under semver, which is a
/// promise about the database rather than about the API.
#[derive(Clone, Debug, sqlx::FromRow)]
pub(crate) struct TenantRow {
    pub(crate) id: Uuid,
    pub(crate) name: String,
    pub(crate) slug: String,
    pub(crate) email: String,
    pub(crate) status: String,
    pub(crate) plan: String,
    pub(crate) max_domains: i32,
    pub(crate) max_routes_per_domain: i32,
    pub(crate) settings: serde_json::Value,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) updated_at: DateTime<Utc>,
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
    /// Primary key.
    pub id: Uuid,
    /// The tenant that owns this domain.
    pub tenant_id: Uuid,
    /// The name itself, globally unique across all tenants. Not updatable: it
    /// is what [`Self::verification_token`] proves control of.
    pub domain: String,
    /// Whether ownership has been proved.
    pub verification_status: VerificationStatus,
    /// How ownership is to be proved. DNS is the default, and the sound one.
    pub verification_method: VerificationMethod,
    /// The secret the challenge carries. Bearer-equivalent for this domain.
    pub verification_token: String,
    /// How many verification attempts have been made.
    pub verification_attempts: i32,
    /// When verification succeeded, if it has.
    pub verified_at: Option<DateTime<Utc>>,
    /// When verification was last attempted.
    pub last_verification_attempt: Option<DateTime<Utc>>,
    /// Where the certificate has got to. `pending` is the issuance worker's
    /// inbox.
    pub ssl_status: SslStatus,
    /// Where the certificate comes from.
    pub ssl_provider: SslProvider,
    /// When the current certificate expires, if there is one.
    pub ssl_expires_at: Option<DateTime<Utc>>,
    /// Whether the proxy should serve this domain. Set independently of
    /// verification, so a verified domain can be taken out of service.
    pub enabled: bool,
    /// When the domain was registered.
    pub created_at: DateTime<Utc>,
    /// When it last changed.
    pub updated_at: DateTime<Utc>,
}

/// Database row for domain with string types.
#[derive(Clone, Debug, sqlx::FromRow)]
pub(crate) struct DomainRow {
    pub(crate) id: Uuid,
    pub(crate) tenant_id: Uuid,
    pub(crate) domain: String,
    pub(crate) verification_status: String,
    pub(crate) verification_method: String,
    pub(crate) verification_token: String,
    pub(crate) verification_attempts: i32,
    pub(crate) verified_at: Option<DateTime<Utc>>,
    pub(crate) last_verification_attempt: Option<DateTime<Utc>>,
    pub(crate) ssl_status: String,
    pub(crate) ssl_provider: String,
    pub(crate) ssl_expires_at: Option<DateTime<Utc>>,
    pub(crate) enabled: bool,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) updated_at: DateTime<Utc>,
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
    /// Primary key.
    pub id: Uuid,
    /// The domain this route belongs to.
    pub domain_id: Uuid,
    /// The owning tenant, denormalised so a route can be authorised without
    /// joining to its domain.
    pub tenant_id: Uuid,
    /// Name, unique within the domain.
    pub name: String,
    /// The path to match, read according to [`Self::path_type`].
    pub path_pattern: String,
    /// How to read [`Self::path_pattern`].
    pub path_type: DomainPathMatchType,
    /// Methods to match. `None` or empty means any.
    pub methods: Option<Vec<String>>,
    /// Higher is matched first.
    pub priority: i32,
    /// The upstream to forward to.
    pub upstream_id: Option<Uuid>,
    /// Whether to remove the matched prefix before forwarding.
    pub strip_path: bool,
    /// Headers to add to the forwarded request.
    pub add_headers: HashMap<String, String>,
    /// Headers to remove from the forwarded request.
    pub remove_headers: Vec<String>,
    /// Requests allowed per window. Set with
    /// [`Self::rate_limit_window_secs`] or not at all.
    pub rate_limit_requests: Option<i32>,
    /// The window those requests are counted over, in seconds.
    pub rate_limit_window_secs: Option<i32>,
    /// Request timeout in seconds.
    pub timeout_secs: i32,
    /// Whether this route participates in matching.
    pub enabled: bool,
    /// When the route was created.
    pub created_at: DateTime<Utc>,
    /// When it last changed.
    pub updated_at: DateTime<Utc>,
}

/// Database row for domain route.
#[derive(Clone, Debug, sqlx::FromRow)]
pub(crate) struct DomainRouteRow {
    pub(crate) id: Uuid,
    pub(crate) domain_id: Uuid,
    pub(crate) tenant_id: Uuid,
    pub(crate) name: String,
    pub(crate) path_pattern: String,
    pub(crate) path_type: String,
    pub(crate) methods: Option<Vec<String>>,
    pub(crate) priority: i32,
    pub(crate) upstream_id: Option<Uuid>,
    pub(crate) strip_path: bool,
    pub(crate) add_headers: serde_json::Value,
    pub(crate) remove_headers: Vec<String>,
    pub(crate) rate_limit_requests: Option<i32>,
    pub(crate) rate_limit_window_secs: Option<i32>,
    pub(crate) timeout_secs: i32,
    pub(crate) enabled: bool,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) updated_at: DateTime<Utc>,
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
    /// Primary key.
    pub id: Uuid,
    /// The owning tenant.
    pub tenant_id: Uuid,
    /// Name, unique within the tenant. Routes reference it.
    pub name: String,
    /// How requests are spread across the backends.
    pub lb_strategy: DomainLoadBalanceStrategy,
    /// Whether backends are health-checked.
    pub health_check_enabled: bool,
    /// The path health checks request.
    pub health_check_path: String,
    /// Seconds between checks.
    pub health_check_interval_secs: i32,
    /// Seconds a single check may take.
    pub health_check_timeout_secs: i32,
    /// Consecutive successes before a backend is used again.
    pub healthy_threshold: i32,
    /// Consecutive failures before a backend is taken out.
    pub unhealthy_threshold: i32,
    /// Whether this upstream may receive traffic.
    pub enabled: bool,
    /// When the upstream was created.
    pub created_at: DateTime<Utc>,
    /// When it last changed.
    pub updated_at: DateTime<Utc>,
    /// Backends (loaded separately)
    #[serde(default)]
    pub backends: Vec<DomainBackend>,
}

/// Database row for domain upstream.
#[derive(Clone, Debug, sqlx::FromRow)]
pub(crate) struct DomainUpstreamRow {
    pub(crate) id: Uuid,
    pub(crate) tenant_id: Uuid,
    pub(crate) name: String,
    pub(crate) lb_strategy: String,
    pub(crate) health_check_enabled: bool,
    pub(crate) health_check_path: String,
    pub(crate) health_check_interval_secs: i32,
    pub(crate) health_check_timeout_secs: i32,
    pub(crate) healthy_threshold: i32,
    pub(crate) unhealthy_threshold: i32,
    pub(crate) enabled: bool,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) updated_at: DateTime<Utc>,
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
    /// Primary key.
    pub id: Uuid,
    /// The upstream this backend belongs to.
    pub upstream_id: Uuid,
    /// `host:port`.
    pub address: String,
    /// `http` or `https`.
    pub scheme: String,
    /// Relative share of traffic under a weighted strategy.
    pub weight: i32,
    /// Whether this backend may receive traffic.
    pub enabled: bool,
    /// When the backend was added.
    pub created_at: DateTime<Utc>,
}

/// An API key for tenant authentication.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApiKey {
    /// Primary key.
    pub id: Uuid,
    /// The tenant this key authenticates.
    pub tenant_id: Uuid,
    /// Label, for telling keys apart.
    pub name: String,
    /// The key itself, **present only in the response that created it**.
    ///
    /// Only a hash is stored, so this cannot be shown again. A caller that does
    /// not keep it has to create another key.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// The first few characters, enough to identify a key in a log without
    /// being enough to use it.
    pub key_prefix: String,
    /// What this key may do, as `resource:action` strings.
    pub scopes: Vec<String>,
    /// When the key was last used to authenticate.
    pub last_used_at: Option<DateTime<Utc>>,
    /// When the key stops working, if it ever does.
    pub expires_at: Option<DateTime<Utc>>,
    /// Whether the key is accepted. Revoking sets this rather than deleting,
    /// so `last_used_at` survives for an audit.
    pub enabled: bool,
    /// When the key was created.
    pub created_at: DateTime<Utc>,
}

/// Database row for API key.
#[derive(Clone, Debug, sqlx::FromRow)]
pub(crate) struct ApiKeyRow {
    pub(crate) id: Uuid,
    pub(crate) tenant_id: Uuid,
    pub(crate) name: String,
    pub(crate) key_prefix: String,
    pub(crate) scopes: Vec<String>,
    pub(crate) last_used_at: Option<DateTime<Utc>>,
    pub(crate) expires_at: Option<DateTime<Utc>>,
    pub(crate) enabled: bool,
    pub(crate) created_at: DateTime<Utc>,
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
    /// Primary key.
    pub id: Uuid,
    /// The domain whose ownership this challenge proves.
    pub domain_id: Uuid,
    /// `dns` or `http`.
    pub challenge_type: String,
    /// The token the challenge is keyed by. A bearer secret for one domain.
    pub token: String,
    /// The exact content the verifier expects. Served verbatim rather than
    /// recomputed, so the check cannot be satisfied by anyone who can guess the
    /// formula.
    pub expected_value: String,
    /// `pending`, `checking`, `verified` or `failed`.
    pub status: String,
    /// Why the last attempt failed, if it did.
    pub error_message: Option<String>,
    /// When the challenge was issued.
    pub created_at: DateTime<Utc>,
    /// When it stops being answerable.
    pub expires_at: DateTime<Utc>,
    /// When it was satisfied, if it was.
    pub verified_at: Option<DateTime<Utc>>,
}

/// Audit log entry.
#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct AuditLogEntry {
    /// Primary key.
    pub id: Uuid,
    /// The tenant acted on, if the action had one.
    pub tenant_id: Option<Uuid>,
    /// The domain acted on, if the action had one.
    pub domain_id: Option<Uuid>,
    /// What happened.
    pub action: String,
    /// What kind of actor did it -- an API key, a JWT, the system.
    pub actor_type: String,
    /// Which actor, as far as it is known.
    pub actor_id: Option<String>,
    /// Anything else worth keeping about the action.
    pub details: serde_json::Value,
    /// Where the request came from.
    pub ip_address: Option<IpAddr>,
    /// When it happened.
    pub created_at: DateTime<Utc>,
}

// ============================================================================
// API Request/Response Types
// ============================================================================

/// Create tenant request.
#[derive(Clone, Debug, Deserialize)]
pub struct CreateTenantRequest {
    /// Display name.
    pub name: String,
    /// Short URL-safe identifier. Must not already be taken.
    pub slug: String,
    /// Contact address.
    pub email: String,
    /// Plan name. Defaults to the free plan.
    pub plan: Option<String>,
}

/// Update tenant request.
#[derive(Clone, Debug, Deserialize)]
pub struct UpdateTenantRequest {
    /// New display name, if it is changing.
    pub name: Option<String>,
    /// New contact address, if it is changing.
    pub email: Option<String>,
    /// New plan, if it is changing.
    pub plan: Option<String>,
    /// Replacement settings blob. Replaces rather than merges.
    pub settings: Option<serde_json::Value>,
}

/// Create domain request.
#[derive(Clone, Debug, Deserialize)]
pub struct CreateDomainRequest {
    /// The name to register. Globally unique across tenants.
    pub domain: String,
    /// How ownership will be proved. Defaults to DNS, which is the sound one:
    /// HTTP verification is answered by this proxy once the domain points at
    /// it, so it shows the domain resolves here rather than that the caller
    /// controls it.
    #[serde(default)]
    pub verification_method: VerificationMethod,
    /// Where the certificate will come from. Defaults to ACME.
    #[serde(default)]
    pub ssl_provider: SslProvider,
}

/// Fields of a domain that can be changed after it is created.
///
/// The domain name itself is deliberately absent. It is the identity of the
/// record and what the verification token proves control of, so renaming would
/// silently carry a proof of ownership over to a name nobody has proved
/// anything about. Delete and re-add instead.
#[derive(Clone, Debug, Deserialize, Default)]
pub struct UpdateDomainRequest {
    /// Switch between DNS and HTTP verification.
    ///
    /// Takes effect on the next verification attempt. Changing it does not
    /// un-verify a domain that is already verified.
    #[serde(default)]
    pub verification_method: Option<VerificationMethod>,

    /// Switch how the certificate is obtained.
    ///
    /// Moving a verified domain to `acme` queues it for issuance.
    #[serde(default)]
    pub ssl_provider: Option<SslProvider>,
}

/// Domain response with verification instructions.
#[derive(Clone, Debug, Serialize)]
pub struct DomainResponse {
    /// The domain as stored. Flattened, so the response is the domain's own
    /// fields plus the instructions.
    #[serde(flatten)]
    pub domain: Domain,
    /// What the caller has to do to prove ownership.
    pub verification_instructions: VerificationInstructions,
}

/// Verification instructions.
#[derive(Clone, Debug, Serialize)]
pub struct VerificationInstructions {
    /// Which method these instructions are for. Only that method's fields are
    /// populated.
    pub method: VerificationMethod,
    /// DNS: the record type to create, always `TXT`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dns_record_type: Option<String>,
    /// DNS: the name to create it at.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dns_record_name: Option<String>,
    /// DNS: the value it must hold.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dns_record_value: Option<String>,
    /// HTTP: the URL the verifier will fetch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http_url: Option<String>,
    /// HTTP: the content it must return.
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
    Failed {
        /// Why, in terms the caller can act on.
        reason: String,
    },
}

/// Create domain route request.
#[derive(Clone, Debug, Deserialize)]
pub struct CreateDomainRouteRequest {
    /// Name, unique within the domain.
    pub name: String,
    /// The path to match. Defaults to `/`, which catches everything.
    #[serde(default = "default_path_pattern")]
    pub path_pattern: String,
    /// How to read the pattern. Defaults to prefix.
    #[serde(default)]
    pub path_type: DomainPathMatchType,
    /// Methods to match. Omit for any.
    pub methods: Option<Vec<String>>,
    /// The upstream to forward to. Must belong to the same tenant.
    pub upstream_id: Uuid,
    /// Whether to remove the matched prefix before forwarding.
    #[serde(default)]
    pub strip_path: bool,
    /// Higher is matched first.
    #[serde(default = "default_priority")]
    pub priority: i32,
    /// Headers to add to the forwarded request.
    #[serde(default)]
    pub add_headers: HashMap<String, String>,
    /// Headers to remove from the forwarded request.
    #[serde(default)]
    pub remove_headers: Vec<String>,
    /// Requests allowed per window. Set with the window or not at all.
    pub rate_limit_requests: Option<i32>,
    /// The window those requests are counted over, in seconds.
    pub rate_limit_window_secs: Option<i32>,
    /// Request timeout in seconds.
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
    /// New name, if it is changing.
    pub name: Option<String>,
    /// New path pattern, if it is changing.
    pub path_pattern: Option<String>,
    /// New path-match type, if it is changing.
    pub path_type: Option<DomainPathMatchType>,
    /// Replacement method list. Replaces rather than adds.
    pub methods: Option<Vec<String>>,
    /// A different upstream to forward to.
    pub upstream_id: Option<Uuid>,
    /// Whether to strip the matched prefix.
    pub strip_path: Option<bool>,
    /// New priority.
    pub priority: Option<i32>,
    /// Replacement set of headers to add. Replaces rather than merges.
    pub add_headers: Option<HashMap<String, String>>,
    /// Replacement list of headers to remove.
    pub remove_headers: Option<Vec<String>>,
    /// New request allowance per window.
    pub rate_limit_requests: Option<i32>,
    /// New window, in seconds.
    pub rate_limit_window_secs: Option<i32>,
    /// New request timeout, in seconds.
    pub timeout_secs: Option<i32>,
    /// Whether the route participates in matching.
    pub enabled: Option<bool>,
}

/// Create upstream request.
#[derive(Clone, Debug, Deserialize)]
pub struct CreateUpstreamRequest {
    /// Name, unique within the tenant. Routes reference it.
    pub name: String,
    /// How requests are spread across the backends.
    #[serde(default)]
    pub lb_strategy: DomainLoadBalanceStrategy,
    /// Whether backends are health-checked.
    #[serde(default = "default_true")]
    pub health_check_enabled: bool,
    /// The path health checks request.
    #[serde(default = "default_health_path")]
    pub health_check_path: String,
    /// Seconds between checks.
    #[serde(default = "default_health_interval")]
    pub health_check_interval_secs: i32,
    /// Seconds a single check may take.
    #[serde(default = "default_health_timeout")]
    pub health_check_timeout_secs: i32,
    /// Consecutive successes before a backend is used again.
    #[serde(default = "default_healthy_threshold")]
    pub healthy_threshold: i32,
    /// Consecutive failures before a backend is taken out.
    #[serde(default = "default_unhealthy_threshold")]
    pub unhealthy_threshold: i32,
    /// Backends to create alongside the upstream.
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
    /// New name, if it is changing.
    pub name: Option<String>,
    /// New load-balancing strategy.
    pub lb_strategy: Option<DomainLoadBalanceStrategy>,
    /// Whether to health-check the backends.
    pub health_check_enabled: Option<bool>,
    /// New health-check path.
    pub health_check_path: Option<String>,
    /// New interval between checks, in seconds.
    pub health_check_interval_secs: Option<i32>,
    /// New per-check timeout, in seconds.
    pub health_check_timeout_secs: Option<i32>,
    /// New healthy threshold.
    pub healthy_threshold: Option<i32>,
    /// New unhealthy threshold.
    pub unhealthy_threshold: Option<i32>,
    /// Whether the upstream may receive traffic.
    ///
    /// Backends are deliberately absent: this type cannot express a backend's
    /// id, so treating a list here as the new set would re-create every backend
    /// under a fresh id. Use the backend endpoints.
    pub enabled: Option<bool>,
}

/// Create backend request.
#[derive(Clone, Debug, Deserialize)]
pub struct CreateBackendRequest {
    /// `host:port`.
    pub address: String,
    /// `http` or `https`. Defaults to `http`.
    #[serde(default = "default_scheme")]
    pub scheme: String,
    /// Relative share of traffic under a weighted strategy.
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
    /// Label, for telling keys apart later.
    pub name: String,
    /// What the key may do, as `resource:action` strings. Defaults to reading
    /// and writing domains.
    #[serde(default = "default_scopes")]
    pub scopes: Vec<String>,
    /// When the key should stop working. `None` means never.
    pub expires_at: Option<DateTime<Utc>>,
}

fn default_scopes() -> Vec<String> {
    vec!["domains:read".to_string(), "domains:write".to_string()]
}

/// Certificate upload request.
#[derive(Clone, Debug, Deserialize)]
pub struct UploadCertificateRequest {
    /// The certificate chain, PEM-encoded, leaf first.
    pub cert_pem: String,
    /// The private key, PEM-encoded. Must match the leaf: the upload is
    /// refused otherwise, because a mismatch produces a listener that fails
    /// every handshake instead.
    pub key_pem: String,
}

/// Tenant usage stats.
#[derive(Clone, Debug, Serialize)]
pub struct TenantUsage {
    /// Domains registered.
    pub domains_count: i64,
    /// How many the tenant's plan allows.
    pub domains_limit: i32,
    /// How many of those domains are verified.
    pub verified_domains: i64,
    /// Routes across all of them.
    pub routes_count: i64,
    /// Upstreams defined.
    pub upstreams_count: i64,
    /// API keys, including revoked ones.
    pub api_keys_count: i64,
}
