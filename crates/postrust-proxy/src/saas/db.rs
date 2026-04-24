//! Database access for the SaaS domain management module.

use crate::error::ProxyResult;
use crate::saas::types::*;
use chrono::Utc;
use sqlx::{PgPool, Row};
use uuid::Uuid;

fn verification_method_str(method: &VerificationMethod) -> &'static str {
    match method {
        VerificationMethod::Dns => "dns",
        VerificationMethod::Http => "http",
    }
}

fn verification_status_str(status: &VerificationStatus) -> &'static str {
    match status {
        VerificationStatus::Pending => "pending",
        VerificationStatus::Verified => "verified",
        VerificationStatus::Failed => "failed",
        VerificationStatus::Expired => "expired",
    }
}

fn ssl_provider_str(provider: &SslProvider) -> &'static str {
    match provider {
        SslProvider::Acme => "acme",
        SslProvider::Manual => "manual",
        SslProvider::None => "none",
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

fn path_match_type_str(path_type: &DomainPathMatchType) -> &'static str {
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

pub async fn check_domain_quota(pool: &PgPool, tenant_id: Uuid) -> ProxyResult<(i64, i32)> {
    let row = sqlx::query(
        r#"
        SELECT
            COUNT(d.id) AS domains_count,
            t.max_domains
        FROM proxy_tenants t
        LEFT JOIN proxy_domains d ON d.tenant_id = t.id
        WHERE t.id = $1
        GROUP BY t.max_domains
        "#,
    )
    .bind(tenant_id)
    .fetch_one(pool)
    .await?;

    Ok((row.get("domains_count"), row.get("max_domains")))
}

pub async fn domain_exists(pool: &PgPool, domain: &str) -> ProxyResult<bool> {
    let exists = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM proxy_domains WHERE domain = $1)",
    )
    .bind(domain)
    .fetch_one(pool)
    .await?;

    Ok(exists)
}

pub async fn create_domain(
    pool: &PgPool,
    tenant_id: Uuid,
    req: CreateDomainRequest,
    verification_token: &str,
) -> ProxyResult<Domain> {
    let row = sqlx::query_as::<_, DomainRow>(
        r#"
        INSERT INTO proxy_domains (
            tenant_id,
            domain,
            verification_method,
            verification_token,
            ssl_provider
        )
        VALUES ($1, $2, $3, $4, $5)
        RETURNING
            id,
            tenant_id,
            domain,
            verification_status,
            verification_method,
            verification_token,
            verification_attempts,
            verified_at,
            last_verification_attempt,
            ssl_status,
            ssl_provider,
            ssl_expires_at,
            enabled,
            created_at,
            updated_at
        "#,
    )
    .bind(tenant_id)
    .bind(req.domain)
    .bind(verification_method_str(&req.verification_method))
    .bind(verification_token)
    .bind(ssl_provider_str(&req.ssl_provider))
    .fetch_one(pool)
    .await?;

    Ok(row.into())
}

pub async fn create_verification_challenge(
    pool: &PgPool,
    domain_id: Uuid,
    challenge_type: &str,
    token: &str,
    expected_value: &str,
) -> ProxyResult<VerificationChallenge> {
    sqlx::query_as::<_, VerificationChallenge>(
        r#"
        INSERT INTO proxy_verification_challenges (
            domain_id,
            challenge_type,
            token,
            expected_value
        )
        VALUES ($1, $2, $3, $4)
        RETURNING
            id,
            domain_id,
            challenge_type,
            token,
            expected_value,
            status,
            error_message,
            created_at,
            expires_at,
            verified_at
        "#,
    )
    .bind(domain_id)
    .bind(challenge_type)
    .bind(token)
    .bind(expected_value)
    .fetch_one(pool)
    .await
    .map_err(Into::into)
}

pub async fn get_domain_for_tenant(
    pool: &PgPool,
    id: Uuid,
    tenant_id: Uuid,
) -> ProxyResult<Option<Domain>> {
    let row = sqlx::query_as::<_, DomainRow>(
        r#"
        SELECT
            id,
            tenant_id,
            domain,
            verification_status,
            verification_method,
            verification_token,
            verification_attempts,
            verified_at,
            last_verification_attempt,
            ssl_status,
            ssl_provider,
            ssl_expires_at,
            enabled,
            created_at,
            updated_at
        FROM proxy_domains
        WHERE id = $1 AND tenant_id = $2
        "#,
    )
    .bind(id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(Into::into))
}

pub async fn list_domains(pool: &PgPool, tenant_id: Uuid) -> ProxyResult<Vec<Domain>> {
    let rows = sqlx::query_as::<_, DomainRow>(
        r#"
        SELECT
            id,
            tenant_id,
            domain,
            verification_status,
            verification_method,
            verification_token,
            verification_attempts,
            verified_at,
            last_verification_attempt,
            ssl_status,
            ssl_provider,
            ssl_expires_at,
            enabled,
            created_at,
            updated_at
        FROM proxy_domains
        WHERE tenant_id = $1
        ORDER BY created_at DESC
        "#,
    )
    .bind(tenant_id)
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(Into::into).collect())
}

pub async fn delete_domain(pool: &PgPool, id: Uuid, tenant_id: Uuid) -> ProxyResult<bool> {
    let result = sqlx::query("DELETE FROM proxy_domains WHERE id = $1 AND tenant_id = $2")
        .bind(id)
        .bind(tenant_id)
        .execute(pool)
        .await?;

    Ok(result.rows_affected() > 0)
}

pub async fn record_verification_attempt(pool: &PgPool, id: Uuid) -> ProxyResult<()> {
    sqlx::query(
        r#"
        UPDATE proxy_domains
        SET
            verification_attempts = verification_attempts + 1,
            last_verification_attempt = NOW(),
            updated_at = NOW()
        WHERE id = $1
        "#,
    )
    .bind(id)
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn update_verification_status(
    pool: &PgPool,
    id: Uuid,
    status: VerificationStatus,
) -> ProxyResult<()> {
    let verified_at = if status == VerificationStatus::Verified {
        Some(Utc::now())
    } else {
        None
    };

    sqlx::query(
        r#"
        UPDATE proxy_domains
        SET
            verification_status = $2,
            verified_at = CASE WHEN $3 IS NULL THEN verified_at ELSE $3 END,
            updated_at = NOW()
        WHERE id = $1
        "#,
    )
    .bind(id)
    .bind(verification_status_str(&status))
    .bind(verified_at)
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn update_ssl_status(
    pool: &PgPool,
    id: Uuid,
    status: SslStatus,
    expires_at: Option<chrono::DateTime<Utc>>,
) -> ProxyResult<()> {
    sqlx::query(
        r#"
        UPDATE proxy_domains
        SET
            ssl_status = $2,
            ssl_expires_at = COALESCE($3, ssl_expires_at),
            updated_at = NOW()
        WHERE id = $1
        "#,
    )
    .bind(id)
    .bind(ssl_status_str(&status))
    .bind(expires_at)
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn enable_domain(pool: &PgPool, id: Uuid) -> ProxyResult<bool> {
    let result = sqlx::query(
        "UPDATE proxy_domains SET enabled = true, updated_at = NOW() WHERE id = $1",
    )
    .bind(id)
    .execute(pool)
    .await?;

    Ok(result.rows_affected() > 0)
}

pub async fn disable_domain(pool: &PgPool, id: Uuid) -> ProxyResult<bool> {
    let result = sqlx::query(
        "UPDATE proxy_domains SET enabled = false, updated_at = NOW() WHERE id = $1",
    )
    .bind(id)
    .execute(pool)
    .await?;

    Ok(result.rows_affected() > 0)
}

pub async fn create_route(
    pool: &PgPool,
    domain_id: Uuid,
    tenant_id: Uuid,
    req: CreateDomainRouteRequest,
) -> ProxyResult<DomainRoute> {
    let row = sqlx::query_as::<_, DomainRouteRow>(
        r#"
        INSERT INTO proxy_domain_routes (
            domain_id,
            tenant_id,
            name,
            path_pattern,
            path_type,
            methods,
            priority,
            upstream_id,
            strip_path,
            add_headers,
            remove_headers,
            rate_limit_requests,
            rate_limit_window_secs,
            timeout_secs
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
        RETURNING
            id,
            domain_id,
            tenant_id,
            name,
            path_pattern,
            path_type,
            methods,
            priority,
            upstream_id,
            strip_path,
            add_headers,
            remove_headers,
            rate_limit_requests,
            rate_limit_window_secs,
            timeout_secs,
            enabled,
            created_at,
            updated_at
        "#,
    )
    .bind(domain_id)
    .bind(tenant_id)
    .bind(req.name)
    .bind(req.path_pattern)
    .bind(path_match_type_str(&req.path_type))
    .bind(req.methods)
    .bind(req.priority)
    .bind(req.upstream_id)
    .bind(req.strip_path)
    .bind(serde_json::to_value(req.add_headers).unwrap_or_default())
    .bind(req.remove_headers)
    .bind(req.rate_limit_requests)
    .bind(req.rate_limit_window_secs)
    .bind(req.timeout_secs)
    .fetch_one(pool)
    .await?;

    Ok(row.into())
}

pub async fn get_route_for_tenant(
    pool: &PgPool,
    id: Uuid,
    tenant_id: Uuid,
) -> ProxyResult<Option<DomainRoute>> {
    let row = sqlx::query_as::<_, DomainRouteRow>(
        r#"
        SELECT
            id,
            domain_id,
            tenant_id,
            name,
            path_pattern,
            path_type,
            methods,
            priority,
            upstream_id,
            strip_path,
            add_headers,
            remove_headers,
            rate_limit_requests,
            rate_limit_window_secs,
            timeout_secs,
            enabled,
            created_at,
            updated_at
        FROM proxy_domain_routes
        WHERE id = $1 AND tenant_id = $2
        "#,
    )
    .bind(id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(Into::into))
}

pub async fn list_routes_for_domain(
    pool: &PgPool,
    domain_id: Uuid,
    tenant_id: Uuid,
) -> ProxyResult<Vec<DomainRoute>> {
    let rows = sqlx::query_as::<_, DomainRouteRow>(
        r#"
        SELECT
            id,
            domain_id,
            tenant_id,
            name,
            path_pattern,
            path_type,
            methods,
            priority,
            upstream_id,
            strip_path,
            add_headers,
            remove_headers,
            rate_limit_requests,
            rate_limit_window_secs,
            timeout_secs,
            enabled,
            created_at,
            updated_at
        FROM proxy_domain_routes
        WHERE domain_id = $1 AND tenant_id = $2
        ORDER BY priority DESC, created_at ASC
        "#,
    )
    .bind(domain_id)
    .bind(tenant_id)
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(Into::into).collect())
}

pub async fn update_route(
    pool: &PgPool,
    id: Uuid,
    tenant_id: Uuid,
    req: UpdateDomainRouteRequest,
) -> ProxyResult<Option<DomainRoute>> {
    let existing = match get_route_for_tenant(pool, id, tenant_id).await? {
        Some(route) => route,
        None => return Ok(None),
    };

    let row = sqlx::query_as::<_, DomainRouteRow>(
        r#"
        UPDATE proxy_domain_routes
        SET
            name = $3,
            path_pattern = $4,
            path_type = $5,
            methods = $6,
            priority = $7,
            upstream_id = $8,
            strip_path = $9,
            add_headers = $10,
            remove_headers = $11,
            rate_limit_requests = $12,
            rate_limit_window_secs = $13,
            timeout_secs = $14,
            enabled = $15,
            updated_at = NOW()
        WHERE id = $1 AND tenant_id = $2
        RETURNING
            id,
            domain_id,
            tenant_id,
            name,
            path_pattern,
            path_type,
            methods,
            priority,
            upstream_id,
            strip_path,
            add_headers,
            remove_headers,
            rate_limit_requests,
            rate_limit_window_secs,
            timeout_secs,
            enabled,
            created_at,
            updated_at
        "#,
    )
    .bind(id)
    .bind(tenant_id)
    .bind(req.name.unwrap_or(existing.name))
    .bind(req.path_pattern.unwrap_or(existing.path_pattern))
    .bind(
        req.path_type
            .as_ref()
            .map(path_match_type_str)
            .unwrap_or_else(|| path_match_type_str(&existing.path_type)),
    )
    .bind(req.methods.or(existing.methods))
    .bind(req.priority.unwrap_or(existing.priority))
    .bind(req.upstream_id.or(existing.upstream_id))
    .bind(req.strip_path.unwrap_or(existing.strip_path))
    .bind(serde_json::to_value(req.add_headers.unwrap_or(existing.add_headers)).unwrap_or_default())
    .bind(req.remove_headers.unwrap_or(existing.remove_headers))
    .bind(req.rate_limit_requests.or(existing.rate_limit_requests))
    .bind(req.rate_limit_window_secs.or(existing.rate_limit_window_secs))
    .bind(req.timeout_secs.unwrap_or(existing.timeout_secs))
    .bind(req.enabled.unwrap_or(existing.enabled))
    .fetch_one(pool)
    .await?;

    Ok(Some(row.into()))
}

pub async fn delete_route(pool: &PgPool, id: Uuid, tenant_id: Uuid) -> ProxyResult<bool> {
    let result = sqlx::query("DELETE FROM proxy_domain_routes WHERE id = $1 AND tenant_id = $2")
        .bind(id)
        .bind(tenant_id)
        .execute(pool)
        .await?;

    Ok(result.rows_affected() > 0)
}

pub async fn create_upstream(
    pool: &PgPool,
    tenant_id: Uuid,
    req: CreateUpstreamRequest,
) -> ProxyResult<DomainUpstream> {
    let upstream_row = sqlx::query_as::<_, DomainUpstreamRow>(
        r#"
        INSERT INTO proxy_domain_upstreams (
            tenant_id,
            name,
            lb_strategy,
            health_check_enabled,
            health_check_path,
            health_check_interval_secs,
            health_check_timeout_secs,
            healthy_threshold,
            unhealthy_threshold
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
        RETURNING
            id,
            tenant_id,
            name,
            lb_strategy,
            health_check_enabled,
            health_check_path,
            health_check_interval_secs,
            health_check_timeout_secs,
            healthy_threshold,
            unhealthy_threshold,
            enabled,
            created_at,
            updated_at
        "#,
    )
    .bind(tenant_id)
    .bind(req.name)
    .bind(lb_strategy_str(&req.lb_strategy))
    .bind(req.health_check_enabled)
    .bind(req.health_check_path)
    .bind(req.health_check_interval_secs)
    .bind(req.health_check_timeout_secs)
    .bind(req.healthy_threshold)
    .bind(req.unhealthy_threshold)
    .fetch_one(pool)
    .await?;

    let mut upstream: DomainUpstream = upstream_row.into();
    let mut backends = Vec::with_capacity(req.backends.len());
    for backend in req.backends {
        backends.push(create_backend(pool, upstream.id, backend).await?);
    }
    upstream.backends = backends;

    Ok(upstream)
}

pub async fn get_upstream_for_tenant(
    pool: &PgPool,
    id: Uuid,
    tenant_id: Uuid,
) -> ProxyResult<Option<DomainUpstream>> {
    let row = sqlx::query_as::<_, DomainUpstreamRow>(
        r#"
        SELECT
            id,
            tenant_id,
            name,
            lb_strategy,
            health_check_enabled,
            health_check_path,
            health_check_interval_secs,
            health_check_timeout_secs,
            healthy_threshold,
            unhealthy_threshold,
            enabled,
            created_at,
            updated_at
        FROM proxy_domain_upstreams
        WHERE id = $1 AND tenant_id = $2
        "#,
    )
    .bind(id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?;

    match row {
        Some(upstream_row) => {
            let mut upstream: DomainUpstream = upstream_row.into();
            upstream.backends = list_backends_for_upstream(pool, upstream.id).await?;
            Ok(Some(upstream))
        }
        None => Ok(None),
    }
}

pub async fn list_upstreams(pool: &PgPool, tenant_id: Uuid) -> ProxyResult<Vec<DomainUpstream>> {
    let rows = sqlx::query_as::<_, DomainUpstreamRow>(
        r#"
        SELECT
            id,
            tenant_id,
            name,
            lb_strategy,
            health_check_enabled,
            health_check_path,
            health_check_interval_secs,
            health_check_timeout_secs,
            healthy_threshold,
            unhealthy_threshold,
            enabled,
            created_at,
            updated_at
        FROM proxy_domain_upstreams
        WHERE tenant_id = $1
        ORDER BY created_at DESC
        "#,
    )
    .bind(tenant_id)
    .fetch_all(pool)
    .await?;

    let mut upstreams = Vec::with_capacity(rows.len());
    for row in rows {
        let mut upstream: DomainUpstream = row.into();
        upstream.backends = list_backends_for_upstream(pool, upstream.id).await?;
        upstreams.push(upstream);
    }

    Ok(upstreams)
}

pub async fn update_upstream(
    pool: &PgPool,
    id: Uuid,
    tenant_id: Uuid,
    req: UpdateUpstreamRequest,
) -> ProxyResult<Option<DomainUpstream>> {
    let existing = match get_upstream_for_tenant(pool, id, tenant_id).await? {
        Some(upstream) => upstream,
        None => return Ok(None),
    };

    let row = sqlx::query_as::<_, DomainUpstreamRow>(
        r#"
        UPDATE proxy_domain_upstreams
        SET
            name = $3,
            lb_strategy = $4,
            health_check_enabled = $5,
            health_check_path = $6,
            health_check_interval_secs = $7,
            health_check_timeout_secs = $8,
            healthy_threshold = $9,
            unhealthy_threshold = $10,
            enabled = $11,
            updated_at = NOW()
        WHERE id = $1 AND tenant_id = $2
        RETURNING
            id,
            tenant_id,
            name,
            lb_strategy,
            health_check_enabled,
            health_check_path,
            health_check_interval_secs,
            health_check_timeout_secs,
            healthy_threshold,
            unhealthy_threshold,
            enabled,
            created_at,
            updated_at
        "#,
    )
    .bind(id)
    .bind(tenant_id)
    .bind(req.name.unwrap_or(existing.name))
    .bind(
        req.lb_strategy
            .as_ref()
            .map(lb_strategy_str)
            .unwrap_or_else(|| lb_strategy_str(&existing.lb_strategy)),
    )
    .bind(req.health_check_enabled.unwrap_or(existing.health_check_enabled))
    .bind(req.health_check_path.unwrap_or(existing.health_check_path))
    .bind(
        req.health_check_interval_secs
            .unwrap_or(existing.health_check_interval_secs),
    )
    .bind(
        req.health_check_timeout_secs
            .unwrap_or(existing.health_check_timeout_secs),
    )
    .bind(req.healthy_threshold.unwrap_or(existing.healthy_threshold))
    .bind(req.unhealthy_threshold.unwrap_or(existing.unhealthy_threshold))
    .bind(req.enabled.unwrap_or(existing.enabled))
    .fetch_one(pool)
    .await?;

    let mut upstream: DomainUpstream = row.into();
    upstream.backends = list_backends_for_upstream(pool, id).await?;

    Ok(Some(upstream))
}

pub async fn delete_upstream(pool: &PgPool, id: Uuid, tenant_id: Uuid) -> ProxyResult<bool> {
    let result = sqlx::query("DELETE FROM proxy_domain_upstreams WHERE id = $1 AND tenant_id = $2")
        .bind(id)
        .bind(tenant_id)
        .execute(pool)
        .await?;

    Ok(result.rows_affected() > 0)
}

pub async fn create_backend(
    pool: &PgPool,
    upstream_id: Uuid,
    req: CreateBackendRequest,
) -> ProxyResult<DomainBackend> {
    sqlx::query_as::<_, DomainBackend>(
        r#"
        INSERT INTO proxy_domain_backends (upstream_id, address, scheme, weight)
        VALUES ($1, $2, $3, $4)
        RETURNING id, upstream_id, address, scheme, weight, enabled, created_at
        "#,
    )
    .bind(upstream_id)
    .bind(req.address)
    .bind(req.scheme)
    .bind(req.weight)
    .fetch_one(pool)
    .await
    .map_err(Into::into)
}

pub async fn delete_backend(
    pool: &PgPool,
    backend_id: Uuid,
    upstream_id: Uuid,
    tenant_id: Uuid,
) -> ProxyResult<bool> {
    let result = sqlx::query(
        r#"
        DELETE FROM proxy_domain_backends b
        USING proxy_domain_upstreams u
        WHERE b.id = $1
          AND b.upstream_id = $2
          AND u.id = b.upstream_id
          AND u.tenant_id = $3
        "#,
    )
    .bind(backend_id)
    .bind(upstream_id)
    .bind(tenant_id)
    .execute(pool)
    .await?;

    Ok(result.rows_affected() > 0)
}

async fn list_backends_for_upstream(pool: &PgPool, upstream_id: Uuid) -> ProxyResult<Vec<DomainBackend>> {
    sqlx::query_as::<_, DomainBackend>(
        r#"
        SELECT id, upstream_id, address, scheme, weight, enabled, created_at
        FROM proxy_domain_backends
        WHERE upstream_id = $1
        ORDER BY created_at ASC
        "#,
    )
    .bind(upstream_id)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

pub async fn create_api_key(
    pool: &PgPool,
    tenant_id: Uuid,
    req: CreateApiKeyRequest,
    key_hash: &str,
    key_prefix: &str,
) -> ProxyResult<ApiKeyRow> {
    sqlx::query_as::<_, ApiKeyRow>(
        r#"
        INSERT INTO proxy_api_keys (tenant_id, name, key_hash, key_prefix, scopes, expires_at)
        VALUES ($1, $2, $3, $4, $5, $6)
        RETURNING
            id,
            tenant_id,
            name,
            key_hash,
            key_prefix,
            scopes,
            last_used_at,
            expires_at,
            enabled,
            created_at
        "#,
    )
    .bind(tenant_id)
    .bind(req.name)
    .bind(key_hash)
    .bind(key_prefix)
    .bind(req.scopes)
    .bind(req.expires_at)
    .fetch_one(pool)
    .await
    .map_err(Into::into)
}

pub async fn validate_api_key_by_hash(
    pool: &PgPool,
    key_hash: &str,
) -> ProxyResult<Option<ApiKeyRow>> {
    sqlx::query_as::<_, ApiKeyRow>(
        r#"
        SELECT
            id,
            tenant_id,
            name,
            key_hash,
            key_prefix,
            scopes,
            last_used_at,
            expires_at,
            enabled,
            created_at
        FROM proxy_api_keys
        WHERE key_hash = $1
          AND (expires_at IS NULL OR expires_at > NOW())
        "#,
    )
    .bind(key_hash)
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

pub async fn update_last_used(pool: &PgPool, key_id: Uuid) -> ProxyResult<()> {
    sqlx::query("UPDATE proxy_api_keys SET last_used_at = NOW() WHERE id = $1")
        .bind(key_id)
        .execute(pool)
        .await?;

    Ok(())
}

pub async fn list_api_keys(pool: &PgPool, tenant_id: Uuid) -> ProxyResult<Vec<ApiKey>> {
    let rows = sqlx::query_as::<_, ApiKeyRow>(
        r#"
        SELECT
            id,
            tenant_id,
            name,
            key_hash,
            key_prefix,
            scopes,
            last_used_at,
            expires_at,
            enabled,
            created_at
        FROM proxy_api_keys
        WHERE tenant_id = $1
        ORDER BY created_at DESC
        "#,
    )
    .bind(tenant_id)
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(Into::into).collect())
}

pub async fn get_api_key_for_tenant(
    pool: &PgPool,
    id: Uuid,
    tenant_id: Uuid,
) -> ProxyResult<Option<ApiKey>> {
    let row = sqlx::query_as::<_, ApiKeyRow>(
        r#"
        SELECT
            id,
            tenant_id,
            name,
            key_hash,
            key_prefix,
            scopes,
            last_used_at,
            expires_at,
            enabled,
            created_at
        FROM proxy_api_keys
        WHERE id = $1 AND tenant_id = $2
        "#,
    )
    .bind(id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(Into::into))
}

pub async fn delete_api_key(pool: &PgPool, id: Uuid, tenant_id: Uuid) -> ProxyResult<bool> {
    let result = sqlx::query("DELETE FROM proxy_api_keys WHERE id = $1 AND tenant_id = $2")
        .bind(id)
        .bind(tenant_id)
        .execute(pool)
        .await?;

    Ok(result.rows_affected() > 0)
}

pub async fn disable_api_key(pool: &PgPool, id: Uuid, tenant_id: Uuid) -> ProxyResult<bool> {
    let result = sqlx::query(
        "UPDATE proxy_api_keys SET enabled = false WHERE id = $1 AND tenant_id = $2",
    )
    .bind(id)
    .bind(tenant_id)
    .execute(pool)
    .await?;

    Ok(result.rows_affected() > 0)
}

pub async fn enable_api_key(pool: &PgPool, id: Uuid, tenant_id: Uuid) -> ProxyResult<bool> {
    let result = sqlx::query(
        "UPDATE proxy_api_keys SET enabled = true WHERE id = $1 AND tenant_id = $2",
    )
    .bind(id)
    .bind(tenant_id)
    .execute(pool)
    .await?;

    Ok(result.rows_affected() > 0)
}

pub async fn is_tenant_active(pool: &PgPool, tenant_id: Uuid) -> ProxyResult<bool> {
    let status = sqlx::query_scalar::<_, Option<String>>(
        "SELECT status FROM proxy_tenants WHERE id = $1",
    )
    .bind(tenant_id)
    .fetch_one(pool)
    .await?;

    Ok(matches!(status.as_deref(), Some("active")))
}

pub async fn get_tenant_usage(pool: &PgPool, tenant_id: Uuid) -> ProxyResult<TenantUsage> {
    let row = sqlx::query(
        r#"
        SELECT
            t.max_domains AS domains_limit,
            (SELECT COUNT(*) FROM proxy_domains d WHERE d.tenant_id = t.id) AS domains_count,
            (
                SELECT COUNT(*)
                FROM proxy_domains d
                WHERE d.tenant_id = t.id AND d.verification_status = 'verified'
            ) AS verified_domains,
            (SELECT COUNT(*) FROM proxy_domain_routes r WHERE r.tenant_id = t.id) AS routes_count,
            (SELECT COUNT(*) FROM proxy_domain_upstreams u WHERE u.tenant_id = t.id) AS upstreams_count,
            (SELECT COUNT(*) FROM proxy_api_keys k WHERE k.tenant_id = t.id) AS api_keys_count
        FROM proxy_tenants t
        WHERE t.id = $1
        "#,
    )
    .bind(tenant_id)
    .fetch_one(pool)
    .await?;

    Ok(TenantUsage {
        domains_count: row.get("domains_count"),
        domains_limit: row.get("domains_limit"),
        verified_domains: row.get("verified_domains"),
        routes_count: row.get("routes_count"),
        upstreams_count: row.get("upstreams_count"),
        api_keys_count: row.get("api_keys_count"),
    })
}
