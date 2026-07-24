//! Database access layer for the SaaS domain management module.
//!
//! All queries run against the schema defined in
//! `migrations/20240115000001_saas_domains.sql`. Queries use runtime sqlx
//! (`query`/`query_as`) rather than the compile-time-checked macros so the
//! crate builds without a live database connection.

use crate::error::ProxyResult;
use crate::saas::types::*;
use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row};
use uuid::Uuid;

// ============================================================================
// Enum <-> string helpers
//
// The enum columns are plain VARCHAR with CHECK constraints, so we bind their
// canonical string form directly and let the `From<*Row>` impls decode back.
// ============================================================================

fn verification_status_str(status: &VerificationStatus) -> &'static str {
    match status {
        VerificationStatus::Pending => "pending",
        VerificationStatus::Verified => "verified",
        VerificationStatus::Failed => "failed",
        VerificationStatus::Expired => "expired",
    }
}

fn verification_method_str(method: &VerificationMethod) -> &'static str {
    match method {
        VerificationMethod::Dns => "dns",
        VerificationMethod::Http => "http",
    }
}

fn ssl_status_str(status: &SslStatus) -> &'static str {
    match status {
        SslStatus::Pending => "pending",
        SslStatus::Provisioning => "provisioning",
        SslStatus::Active => "active",
        SslStatus::Failed => "failed",
        SslStatus::Expired => "expired",
    }
}

fn ssl_provider_str(provider: &SslProvider) -> &'static str {
    match provider {
        SslProvider::Acme => "acme",
        SslProvider::Manual => "manual",
        SslProvider::None => "none",
    }
}

fn path_type_str(path_type: &DomainPathMatchType) -> &'static str {
    match path_type {
        DomainPathMatchType::Prefix => "prefix",
        DomainPathMatchType::Exact => "exact",
        DomainPathMatchType::Regex => "regex",
    }
}

fn lb_strategy_str(strategy: &DomainLoadBalanceStrategy) -> &'static str {
    match strategy {
        DomainLoadBalanceStrategy::RoundRobin => "round_robin",
        DomainLoadBalanceStrategy::LeastConnections => "least_connections",
        DomainLoadBalanceStrategy::Weighted => "weighted",
        DomainLoadBalanceStrategy::Random => "random",
        DomainLoadBalanceStrategy::Sticky => "sticky",
    }
}

// ============================================================================
// Domains
// ============================================================================

/// Return `(current_domain_count, max_domains)` for a tenant.
pub async fn check_domain_quota(pool: &PgPool, tenant_id: Uuid) -> ProxyResult<(i64, i32)> {
    let row = sqlx::query(
        "SELECT \
            (SELECT COUNT(*) FROM proxy_domains WHERE tenant_id = $1) AS current, \
            COALESCE((SELECT max_domains FROM proxy_tenants WHERE id = $1), 0) AS max",
    )
    .bind(tenant_id)
    .fetch_one(pool)
    .await?;

    Ok((row.get::<i64, _>("current"), row.get::<i32, _>("max")))
}

/// Check whether a domain is already registered (globally unique).
pub async fn domain_exists(pool: &PgPool, domain: &str) -> ProxyResult<bool> {
    let row =
        sqlx::query("SELECT EXISTS(SELECT 1 FROM proxy_domains WHERE domain = $1) AS present")
            .bind(domain)
            .fetch_one(pool)
            .await?;
    Ok(row.get::<bool, _>("present"))
}

/// Insert a new domain and return it.
pub async fn create_domain(
    pool: &PgPool,
    tenant_id: Uuid,
    req: CreateDomainRequest,
    verification_token: &str,
) -> ProxyResult<Domain> {
    let row = sqlx::query_as::<_, DomainRow>(
        "INSERT INTO proxy_domains \
            (tenant_id, domain, verification_method, verification_token, ssl_provider) \
         VALUES ($1, $2, $3, $4, $5) \
         RETURNING *",
    )
    .bind(tenant_id)
    .bind(&req.domain)
    .bind(verification_method_str(&req.verification_method))
    .bind(verification_token)
    .bind(ssl_provider_str(&req.ssl_provider))
    .fetch_one(pool)
    .await?;

    Ok(Domain::from(row))
}

/// Record a verification challenge for a domain.
pub async fn create_verification_challenge(
    pool: &PgPool,
    domain_id: Uuid,
    challenge_type: &str,
    token: &str,
    expected_value: &str,
) -> ProxyResult<()> {
    sqlx::query(
        "INSERT INTO proxy_verification_challenges \
            (domain_id, challenge_type, token, expected_value) \
         VALUES ($1, $2, $3, $4)",
    )
    .bind(domain_id)
    .bind(challenge_type)
    .bind(token)
    .bind(expected_value)
    .execute(pool)
    .await?;
    Ok(())
}

/// Fetch a domain scoped to a tenant.
pub async fn get_domain_for_tenant(
    pool: &PgPool,
    id: Uuid,
    tenant_id: Uuid,
) -> ProxyResult<Option<Domain>> {
    let row = sqlx::query_as::<_, DomainRow>(
        "SELECT * FROM proxy_domains WHERE id = $1 AND tenant_id = $2",
    )
    .bind(id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(Domain::from))
}

/// List all domains for a tenant.
pub async fn list_domains(pool: &PgPool, tenant_id: Uuid) -> ProxyResult<Vec<Domain>> {
    let rows = sqlx::query_as::<_, DomainRow>(
        "SELECT * FROM proxy_domains WHERE tenant_id = $1 ORDER BY created_at DESC",
    )
    .bind(tenant_id)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(Domain::from).collect())
}

/// Delete a domain scoped to a tenant. Returns whether a row was removed.
pub async fn delete_domain(pool: &PgPool, id: Uuid, tenant_id: Uuid) -> ProxyResult<bool> {
    let result = sqlx::query("DELETE FROM proxy_domains WHERE id = $1 AND tenant_id = $2")
        .bind(id)
        .bind(tenant_id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

/// Increment the verification attempt counter and stamp the attempt time.
pub async fn record_verification_attempt(pool: &PgPool, id: Uuid) -> ProxyResult<()> {
    sqlx::query(
        "UPDATE proxy_domains \
         SET verification_attempts = verification_attempts + 1, \
             last_verification_attempt = NOW(), \
             updated_at = NOW() \
         WHERE id = $1",
    )
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Update a domain's verification status (stamping `verified_at` on success).
pub async fn update_verification_status(
    pool: &PgPool,
    id: Uuid,
    status: VerificationStatus,
) -> ProxyResult<()> {
    let status_str = verification_status_str(&status);
    sqlx::query(
        "UPDATE proxy_domains \
         SET verification_status = $2, \
             verified_at = CASE WHEN $2 = 'verified' THEN NOW() ELSE verified_at END, \
             updated_at = NOW() \
         WHERE id = $1",
    )
    .bind(id)
    .bind(status_str)
    .execute(pool)
    .await?;
    Ok(())
}

/// Update a domain's SSL status and optional expiry.
pub async fn update_ssl_status(
    pool: &PgPool,
    id: Uuid,
    status: SslStatus,
    expires_at: Option<DateTime<Utc>>,
) -> ProxyResult<()> {
    sqlx::query(
        "UPDATE proxy_domains \
         SET ssl_status = $2, ssl_expires_at = $3, updated_at = NOW() \
         WHERE id = $1",
    )
    .bind(id)
    .bind(ssl_status_str(&status))
    .bind(expires_at)
    .execute(pool)
    .await?;
    Ok(())
}

/// Enable a domain.
pub async fn enable_domain(pool: &PgPool, id: Uuid) -> ProxyResult<bool> {
    let result =
        sqlx::query("UPDATE proxy_domains SET enabled = true, updated_at = NOW() WHERE id = $1")
            .bind(id)
            .execute(pool)
            .await?;
    Ok(result.rows_affected() > 0)
}

/// Disable a domain.
pub async fn disable_domain(pool: &PgPool, id: Uuid) -> ProxyResult<bool> {
    let result =
        sqlx::query("UPDATE proxy_domains SET enabled = false, updated_at = NOW() WHERE id = $1")
            .bind(id)
            .execute(pool)
            .await?;
    Ok(result.rows_affected() > 0)
}

// ============================================================================
// Routes
// ============================================================================

/// Create a route for a domain.
pub async fn create_route(
    pool: &PgPool,
    domain_id: Uuid,
    tenant_id: Uuid,
    req: CreateDomainRouteRequest,
) -> ProxyResult<DomainRoute> {
    let add_headers = serde_json::to_value(&req.add_headers).unwrap_or_default();

    let row = sqlx::query_as::<_, DomainRouteRow>(
        "INSERT INTO proxy_domain_routes \
            (domain_id, tenant_id, name, path_pattern, path_type, methods, priority, \
             upstream_id, strip_path, add_headers, remove_headers, rate_limit_requests, \
             rate_limit_window_secs, timeout_secs) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14) \
         RETURNING *",
    )
    .bind(domain_id)
    .bind(tenant_id)
    .bind(&req.name)
    .bind(&req.path_pattern)
    .bind(path_type_str(&req.path_type))
    .bind(&req.methods)
    .bind(req.priority)
    .bind(req.upstream_id)
    .bind(req.strip_path)
    .bind(add_headers)
    .bind(&req.remove_headers)
    .bind(req.rate_limit_requests)
    .bind(req.rate_limit_window_secs)
    .bind(req.timeout_secs)
    .fetch_one(pool)
    .await?;

    Ok(DomainRoute::from(row))
}

/// Fetch a route scoped to a tenant.
pub async fn get_route_for_tenant(
    pool: &PgPool,
    id: Uuid,
    tenant_id: Uuid,
) -> ProxyResult<Option<DomainRoute>> {
    let row = sqlx::query_as::<_, DomainRouteRow>(
        "SELECT * FROM proxy_domain_routes WHERE id = $1 AND tenant_id = $2",
    )
    .bind(id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(DomainRoute::from))
}

/// List routes for a domain, highest priority first.
pub async fn list_routes_for_domain(
    pool: &PgPool,
    domain_id: Uuid,
    tenant_id: Uuid,
) -> ProxyResult<Vec<DomainRoute>> {
    let rows = sqlx::query_as::<_, DomainRouteRow>(
        "SELECT * FROM proxy_domain_routes \
         WHERE domain_id = $1 AND tenant_id = $2 \
         ORDER BY priority DESC, created_at",
    )
    .bind(domain_id)
    .bind(tenant_id)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(DomainRoute::from).collect())
}

/// Apply a partial update to a route (only supplied fields change).
pub async fn update_route(
    pool: &PgPool,
    id: Uuid,
    tenant_id: Uuid,
    req: UpdateDomainRouteRequest,
) -> ProxyResult<Option<DomainRoute>> {
    let path_type = req.path_type.as_ref().map(path_type_str);
    let add_headers = req
        .add_headers
        .as_ref()
        .map(|h| serde_json::to_value(h).unwrap_or_default());

    let row = sqlx::query_as::<_, DomainRouteRow>(
        "UPDATE proxy_domain_routes SET \
            name = COALESCE($3, name), \
            path_pattern = COALESCE($4, path_pattern), \
            path_type = COALESCE($5, path_type), \
            methods = COALESCE($6, methods), \
            upstream_id = COALESCE($7, upstream_id), \
            strip_path = COALESCE($8, strip_path), \
            priority = COALESCE($9, priority), \
            add_headers = COALESCE($10, add_headers), \
            remove_headers = COALESCE($11, remove_headers), \
            rate_limit_requests = COALESCE($12, rate_limit_requests), \
            rate_limit_window_secs = COALESCE($13, rate_limit_window_secs), \
            timeout_secs = COALESCE($14, timeout_secs), \
            enabled = COALESCE($15, enabled), \
            updated_at = NOW() \
         WHERE id = $1 AND tenant_id = $2 \
         RETURNING *",
    )
    .bind(id)
    .bind(tenant_id)
    .bind(req.name)
    .bind(req.path_pattern)
    .bind(path_type)
    .bind(req.methods)
    .bind(req.upstream_id)
    .bind(req.strip_path)
    .bind(req.priority)
    .bind(add_headers)
    .bind(req.remove_headers)
    .bind(req.rate_limit_requests)
    .bind(req.rate_limit_window_secs)
    .bind(req.timeout_secs)
    .bind(req.enabled)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(DomainRoute::from))
}

/// Delete a route scoped to a tenant.
pub async fn delete_route(pool: &PgPool, id: Uuid, tenant_id: Uuid) -> ProxyResult<bool> {
    let result = sqlx::query("DELETE FROM proxy_domain_routes WHERE id = $1 AND tenant_id = $2")
        .bind(id)
        .bind(tenant_id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

// ============================================================================
// Upstreams & backends
// ============================================================================

/// Load the backends belonging to an upstream.
async fn load_backends(pool: &PgPool, upstream_id: Uuid) -> ProxyResult<Vec<DomainBackend>> {
    let backends = sqlx::query_as::<_, DomainBackend>(
        "SELECT * FROM proxy_domain_backends WHERE upstream_id = $1 ORDER BY created_at",
    )
    .bind(upstream_id)
    .fetch_all(pool)
    .await?;
    Ok(backends)
}

/// Create an upstream (and any backends supplied inline).
pub async fn create_upstream(
    pool: &PgPool,
    tenant_id: Uuid,
    req: CreateUpstreamRequest,
) -> ProxyResult<DomainUpstream> {
    let row = sqlx::query_as::<_, DomainUpstreamRow>(
        "INSERT INTO proxy_domain_upstreams \
            (tenant_id, name, lb_strategy, health_check_enabled, health_check_path, \
             health_check_interval_secs, health_check_timeout_secs, healthy_threshold, \
             unhealthy_threshold) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) \
         RETURNING *",
    )
    .bind(tenant_id)
    .bind(&req.name)
    .bind(lb_strategy_str(&req.lb_strategy))
    .bind(req.health_check_enabled)
    .bind(&req.health_check_path)
    .bind(req.health_check_interval_secs)
    .bind(req.health_check_timeout_secs)
    .bind(req.healthy_threshold)
    .bind(req.unhealthy_threshold)
    .fetch_one(pool)
    .await?;

    let mut upstream = DomainUpstream::from(row);

    for backend in req.backends {
        create_backend(pool, upstream.id, backend).await?;
    }
    upstream.backends = load_backends(pool, upstream.id).await?;

    Ok(upstream)
}

/// Fetch an upstream (with its backends) scoped to a tenant.
pub async fn get_upstream_for_tenant(
    pool: &PgPool,
    id: Uuid,
    tenant_id: Uuid,
) -> ProxyResult<Option<DomainUpstream>> {
    let row = sqlx::query_as::<_, DomainUpstreamRow>(
        "SELECT * FROM proxy_domain_upstreams WHERE id = $1 AND tenant_id = $2",
    )
    .bind(id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?;

    match row {
        Some(row) => {
            let mut upstream = DomainUpstream::from(row);
            upstream.backends = load_backends(pool, upstream.id).await?;
            Ok(Some(upstream))
        }
        None => Ok(None),
    }
}

/// List upstreams (with backends) for a tenant.
pub async fn list_upstreams(pool: &PgPool, tenant_id: Uuid) -> ProxyResult<Vec<DomainUpstream>> {
    let rows = sqlx::query_as::<_, DomainUpstreamRow>(
        "SELECT * FROM proxy_domain_upstreams WHERE tenant_id = $1 ORDER BY created_at",
    )
    .bind(tenant_id)
    .fetch_all(pool)
    .await?;

    let mut upstreams = Vec::with_capacity(rows.len());
    for row in rows {
        let mut upstream = DomainUpstream::from(row);
        upstream.backends = load_backends(pool, upstream.id).await?;
        upstreams.push(upstream);
    }
    Ok(upstreams)
}

/// Apply a partial update to an upstream.
pub async fn update_upstream(
    pool: &PgPool,
    id: Uuid,
    tenant_id: Uuid,
    req: UpdateUpstreamRequest,
) -> ProxyResult<Option<DomainUpstream>> {
    let lb_strategy = req.lb_strategy.as_ref().map(lb_strategy_str);

    let row = sqlx::query_as::<_, DomainUpstreamRow>(
        "UPDATE proxy_domain_upstreams SET \
            name = COALESCE($3, name), \
            lb_strategy = COALESCE($4, lb_strategy), \
            health_check_enabled = COALESCE($5, health_check_enabled), \
            health_check_path = COALESCE($6, health_check_path), \
            health_check_interval_secs = COALESCE($7, health_check_interval_secs), \
            health_check_timeout_secs = COALESCE($8, health_check_timeout_secs), \
            healthy_threshold = COALESCE($9, healthy_threshold), \
            unhealthy_threshold = COALESCE($10, unhealthy_threshold), \
            enabled = COALESCE($11, enabled), \
            updated_at = NOW() \
         WHERE id = $1 AND tenant_id = $2 \
         RETURNING *",
    )
    .bind(id)
    .bind(tenant_id)
    .bind(req.name)
    .bind(lb_strategy)
    .bind(req.health_check_enabled)
    .bind(req.health_check_path)
    .bind(req.health_check_interval_secs)
    .bind(req.health_check_timeout_secs)
    .bind(req.healthy_threshold)
    .bind(req.unhealthy_threshold)
    .bind(req.enabled)
    .fetch_optional(pool)
    .await?;

    match row {
        Some(row) => {
            let mut upstream = DomainUpstream::from(row);
            upstream.backends = load_backends(pool, upstream.id).await?;
            Ok(Some(upstream))
        }
        None => Ok(None),
    }
}

/// Delete an upstream scoped to a tenant.
pub async fn delete_upstream(pool: &PgPool, id: Uuid, tenant_id: Uuid) -> ProxyResult<bool> {
    let result = sqlx::query("DELETE FROM proxy_domain_upstreams WHERE id = $1 AND tenant_id = $2")
        .bind(id)
        .bind(tenant_id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

/// Add a backend to an upstream.
pub async fn create_backend(
    pool: &PgPool,
    upstream_id: Uuid,
    req: CreateBackendRequest,
) -> ProxyResult<DomainBackend> {
    let backend = sqlx::query_as::<_, DomainBackend>(
        "INSERT INTO proxy_domain_backends (upstream_id, address, scheme, weight) \
         VALUES ($1, $2, $3, $4) \
         RETURNING *",
    )
    .bind(upstream_id)
    .bind(&req.address)
    .bind(&req.scheme)
    .bind(req.weight)
    .fetch_one(pool)
    .await?;
    Ok(backend)
}

/// Delete a backend, verifying it belongs to the tenant's upstream.
pub async fn delete_backend(
    pool: &PgPool,
    backend_id: Uuid,
    upstream_id: Uuid,
    tenant_id: Uuid,
) -> ProxyResult<bool> {
    let result = sqlx::query(
        "DELETE FROM proxy_domain_backends \
         WHERE id = $1 AND upstream_id = $2 \
           AND upstream_id IN (SELECT id FROM proxy_domain_upstreams WHERE id = $2 AND tenant_id = $3)",
    )
    .bind(backend_id)
    .bind(upstream_id)
    .bind(tenant_id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

// ============================================================================
// API keys
// ============================================================================

/// Insert a new API key row (the raw key is never stored, only its hash).
pub async fn create_api_key(
    pool: &PgPool,
    tenant_id: Uuid,
    req: CreateApiKeyRequest,
    key_hash: &str,
    key_prefix: &str,
) -> ProxyResult<ApiKeyRow> {
    let row = sqlx::query_as::<_, ApiKeyRow>(
        "INSERT INTO proxy_api_keys (tenant_id, name, key_hash, key_prefix, scopes, expires_at) \
         VALUES ($1, $2, $3, $4, $5, $6) \
         RETURNING *",
    )
    .bind(tenant_id)
    .bind(&req.name)
    .bind(key_hash)
    .bind(key_prefix)
    .bind(&req.scopes)
    .bind(req.expires_at)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

/// Look up an API key by its hash, excluding expired keys.
pub async fn validate_api_key_by_hash(
    pool: &PgPool,
    key_hash: &str,
) -> ProxyResult<Option<ApiKeyRow>> {
    let row = sqlx::query_as::<_, ApiKeyRow>(
        "SELECT * FROM proxy_api_keys \
         WHERE key_hash = $1 AND (expires_at IS NULL OR expires_at > NOW())",
    )
    .bind(key_hash)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// Stamp an API key's last-used time (best-effort).
pub async fn update_last_used(pool: &PgPool, key_id: Uuid) -> ProxyResult<()> {
    sqlx::query("UPDATE proxy_api_keys SET last_used_at = NOW() WHERE id = $1")
        .bind(key_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// List API keys for a tenant.
pub async fn list_api_keys(pool: &PgPool, tenant_id: Uuid) -> ProxyResult<Vec<ApiKey>> {
    let rows = sqlx::query_as::<_, ApiKeyRow>(
        "SELECT * FROM proxy_api_keys WHERE tenant_id = $1 ORDER BY created_at DESC",
    )
    .bind(tenant_id)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(ApiKey::from).collect())
}

/// Fetch an API key scoped to a tenant.
pub async fn get_api_key_for_tenant(
    pool: &PgPool,
    id: Uuid,
    tenant_id: Uuid,
) -> ProxyResult<Option<ApiKey>> {
    let row = sqlx::query_as::<_, ApiKeyRow>(
        "SELECT * FROM proxy_api_keys WHERE id = $1 AND tenant_id = $2",
    )
    .bind(id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(ApiKey::from))
}

/// Delete an API key scoped to a tenant.
pub async fn delete_api_key(pool: &PgPool, id: Uuid, tenant_id: Uuid) -> ProxyResult<bool> {
    let result = sqlx::query("DELETE FROM proxy_api_keys WHERE id = $1 AND tenant_id = $2")
        .bind(id)
        .bind(tenant_id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

/// Disable an API key without deleting it.
pub async fn disable_api_key(pool: &PgPool, id: Uuid, tenant_id: Uuid) -> ProxyResult<bool> {
    let result =
        sqlx::query("UPDATE proxy_api_keys SET enabled = false WHERE id = $1 AND tenant_id = $2")
            .bind(id)
            .bind(tenant_id)
            .execute(pool)
            .await?;
    Ok(result.rows_affected() > 0)
}

/// Re-enable a disabled API key.
pub async fn enable_api_key(pool: &PgPool, id: Uuid, tenant_id: Uuid) -> ProxyResult<bool> {
    let result =
        sqlx::query("UPDATE proxy_api_keys SET enabled = true WHERE id = $1 AND tenant_id = $2")
            .bind(id)
            .bind(tenant_id)
            .execute(pool)
            .await?;
    Ok(result.rows_affected() > 0)
}

// ============================================================================
// Tenants
// ============================================================================

/// Whether a tenant exists and is active.
pub async fn is_tenant_active(pool: &PgPool, tenant_id: Uuid) -> ProxyResult<bool> {
    let row = sqlx::query(
        "SELECT EXISTS(SELECT 1 FROM proxy_tenants WHERE id = $1 AND status = 'active') AS active",
    )
    .bind(tenant_id)
    .fetch_one(pool)
    .await?;
    Ok(row.get::<bool, _>("active"))
}

/// Aggregate usage statistics for a tenant.
pub async fn get_tenant_usage(pool: &PgPool, tenant_id: Uuid) -> ProxyResult<TenantUsage> {
    let row = sqlx::query(
        "SELECT \
            (SELECT COUNT(*) FROM proxy_domains WHERE tenant_id = $1) AS domains_count, \
            COALESCE((SELECT max_domains FROM proxy_tenants WHERE id = $1), 0) AS domains_limit, \
            (SELECT COUNT(*) FROM proxy_domains WHERE tenant_id = $1 AND verification_status = 'verified') AS verified_domains, \
            (SELECT COUNT(*) FROM proxy_domain_routes WHERE tenant_id = $1) AS routes_count, \
            (SELECT COUNT(*) FROM proxy_domain_upstreams WHERE tenant_id = $1) AS upstreams_count, \
            (SELECT COUNT(*) FROM proxy_api_keys WHERE tenant_id = $1) AS api_keys_count",
    )
    .bind(tenant_id)
    .fetch_one(pool)
    .await?;

    Ok(TenantUsage {
        domains_count: row.get::<i64, _>("domains_count"),
        domains_limit: row.get::<i32, _>("domains_limit"),
        verified_domains: row.get::<i64, _>("verified_domains"),
        routes_count: row.get::<i64, _>("routes_count"),
        upstreams_count: row.get::<i64, _>("upstreams_count"),
        api_keys_count: row.get::<i64, _>("api_keys_count"),
    })
}
