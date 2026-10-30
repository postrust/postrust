//! Database access layer for the SaaS domain management module.
//!
//! All queries run against the schema defined in
//! `migrations/20240115000001_saas_domains.sql`. Queries use runtime sqlx
//! (`query`/`query_as`) rather than the compile-time-checked macros so the
//! crate builds without a live database connection.

use crate::error::{ProxyError, ProxyResult};
use crate::saas::types::*;
use sqlx::{PgConnection, PgPool, Row};
use std::time::Duration;
use uuid::Uuid;

// ============================================================================
// Enum <-> string helpers
//
// The enum columns are plain VARCHAR with CHECK constraints, so we bind their
// canonical string form directly and let the `From<*Row>` impls decode back.
// ============================================================================

fn verification_method_str(method: &VerificationMethod) -> &'static str {
    match method {
        VerificationMethod::Dns => "dns",
        VerificationMethod::Http => "http",
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

/// A tenant's limits, read with the tenant row locked.
///
/// Every quota check takes this lock first and holds it until the insert
/// commits. Counting and then inserting without it lets two concurrent
/// requests both see room for one more, and both insert.
pub(crate) struct TenantLimits {
    pub(crate) max_domains: i32,
    pub(crate) max_routes_per_domain: i32,
    pub(crate) max_upstreams: i32,
    pub(crate) max_backends_per_upstream: i32,
    pub(crate) max_api_keys: i32,
}

pub(crate) async fn lock_tenant_limits(
    conn: &mut PgConnection,
    tenant_id: Uuid,
) -> ProxyResult<TenantLimits> {
    let row = sqlx::query(
        "SELECT COALESCE(max_domains, 0) AS max_domains, \
                COALESCE(max_routes_per_domain, 0) AS max_routes_per_domain, \
                max_upstreams, max_backends_per_upstream, max_api_keys \
         FROM proxy_tenants WHERE id = $1 FOR UPDATE",
    )
    .bind(tenant_id)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or_else(|| ProxyError::NotFound("Tenant not found".into()))?;

    Ok(TenantLimits {
        max_domains: row.try_get("max_domains")?,
        max_routes_per_domain: row.try_get("max_routes_per_domain")?,
        max_upstreams: row.try_get("max_upstreams")?,
        max_backends_per_upstream: row.try_get("max_backends_per_upstream")?,
        max_api_keys: row.try_get("max_api_keys")?,
    })
}

/// Refuse when `current` has reached `max`.
fn within_quota(current: i64, max: i32, what: &str) -> ProxyResult<()> {
    if current >= i64::from(max) {
        return Err(ProxyError::QuotaExceeded(format!(
            "{what} limit reached ({current}/{max})"
        )));
    }
    Ok(())
}

/// Register a domain for a tenant, with its verification challenge.
///
/// `req.domain` must already be in [`normalize_domain`] form. Names are
/// compared on `lower(domain)`, which is what the unique index is on: rows
/// written before names were normalised may still be mixed case, and must
/// still collide.
///
/// [`normalize_domain`]: crate::saas::validation::normalize_domain
///
/// One transaction, with the tenant
/// row locked for the quota check, and in this order:
///
/// 1. A registration of the same name that is still unverified after
///    `pending_ttl` is deleted, whoever holds it. That is what stops a pending
///    row squatting a name for ever: past its time it no longer blocks
///    anybody, and the next registration of the name clears it.
/// 2. The quota is checked.
/// 3. A live registration of the name by anyone is a conflict. The unique
///    index on `lower(domain)` backs this up against a concurrent insert.
/// 4. The domain and its challenge are inserted. The challenge expires when
///    the registration does.
pub(crate) async fn register_domain(
    pool: &PgPool,
    tenant_id: Uuid,
    req: &CreateDomainRequest,
    verification_token: &str,
    expected_value: &str,
    pending_ttl: Duration,
) -> ProxyResult<Domain> {
    let ttl_secs = pending_ttl.as_secs_f64();
    let mut tx = pool.begin().await?;

    let limits = lock_tenant_limits(&mut tx, tenant_id).await?;

    let expired = sqlx::query(
        "DELETE FROM proxy_domains \
         WHERE lower(domain) = lower($1) \
           AND verification_status <> 'verified' \
           AND created_at < NOW() - make_interval(secs => $2)",
    )
    .bind(&req.domain)
    .bind(ttl_secs)
    .execute(&mut *tx)
    .await?;
    if expired.rows_affected() > 0 {
        tracing::info!(
            domain = %req.domain,
            "removed an unverified registration that had outlived its time"
        );
    }

    let current: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM proxy_domains WHERE tenant_id = $1")
            .bind(tenant_id)
            .fetch_one(&mut *tx)
            .await?;
    within_quota(current, limits.max_domains, "Domain")?;

    let taken: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM proxy_domains WHERE lower(domain) = lower($1))",
    )
    .bind(&req.domain)
    .fetch_one(&mut *tx)
    .await?;
    if taken {
        return Err(ProxyError::Conflict("Domain already registered".into()));
    }

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
    .fetch_one(&mut *tx)
    .await?;

    sqlx::query(
        "INSERT INTO proxy_verification_challenges \
            (domain_id, challenge_type, token, expected_value, expires_at) \
         VALUES ($1, $2, $3, $4, NOW() + make_interval(secs => $5))",
    )
    .bind(row.id)
    .bind(verification_method_str(&req.verification_method))
    .bind(verification_token)
    .bind(expected_value)
    .bind(ttl_secs)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(Domain::from(row))
}

/// A live HTTP verification challenge, with the domain it was issued for.
pub struct HttpChallenge {
    /// The domain the challenge belongs to. The endpoint serving the challenge
    /// must refuse to answer for any other host.
    pub domain: String,
    /// The content the verifier expects, as stored when the challenge was
    /// issued. Never recomputed from the token at serve time.
    pub expected_value: String,
}

/// Look up a live HTTP verification challenge by its token.
///
/// Returns `None` for a token that does not exist, one that has expired, and
/// one whose challenge is already resolved. The caller must additionally check
/// that the request arrived for [`HttpChallenge::domain`]: a token is a
/// bearer secret for exactly one domain, and answering with it on any other
/// host hands that secret to whoever asked.
pub async fn find_live_http_challenge(
    pool: &PgPool,
    token: &str,
) -> ProxyResult<Option<HttpChallenge>> {
    let row = sqlx::query(
        "SELECT d.domain, c.expected_value \
         FROM proxy_verification_challenges c \
         JOIN proxy_domains d ON d.id = c.domain_id \
         WHERE c.token = $1 \
           AND c.challenge_type = 'http' \
           AND c.status IN ('pending', 'checking') \
           AND c.expires_at > NOW()",
    )
    .bind(token)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|row| HttpChallenge {
        domain: row.get("domain"),
        expected_value: row.get("expected_value"),
    }))
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

/// Apply a partial update to a domain, scoped to a tenant.
///
/// Only the fields present in the request change. `COALESCE` on a bound
/// `Option` is what makes that work in one statement: a `None` binds as NULL
/// and the column keeps its value.
///
/// Moving a *verified* domain to `acme` also resets its SSL state to `pending`,
/// which is the issuance worker's inbox -- otherwise switching the provider
/// would leave the domain sitting in whatever state the old provider left, and
/// nothing would ever pick it up.
pub async fn update_domain(
    pool: &PgPool,
    id: Uuid,
    tenant_id: Uuid,
    verification_method: Option<&VerificationMethod>,
    ssl_provider: Option<&SslProvider>,
) -> ProxyResult<Option<Domain>> {
    let queue_for_acme = matches!(ssl_provider, Some(SslProvider::Acme));
    let mut tx = pool.begin().await?;

    let row = sqlx::query_as::<_, DomainRow>(
        "UPDATE proxy_domains SET \
             verification_method = COALESCE($3, verification_method), \
             ssl_provider = COALESCE($4, ssl_provider), \
             ssl_status = CASE \
                 WHEN $5 AND verification_status = 'verified' THEN 'pending' \
                 ELSE ssl_status END, \
             ssl_attempts = CASE \
                 WHEN $5 AND verification_status = 'verified' THEN 0 \
                 ELSE ssl_attempts END, \
             ssl_error = CASE \
                 WHEN $5 AND verification_status = 'verified' THEN NULL \
                 ELSE ssl_error END, \
             ssl_last_attempt_at = CASE \
                 WHEN $5 AND verification_status = 'verified' THEN NULL \
                 ELSE ssl_last_attempt_at END, \
             updated_at = NOW() \
         WHERE id = $1 AND tenant_id = $2 \
         RETURNING *",
    )
    .bind(id)
    .bind(tenant_id)
    .bind(verification_method.map(verification_method_str))
    .bind(ssl_provider.map(ssl_provider_str))
    .bind(queue_for_acme)
    .fetch_optional(&mut *tx)
    .await?;

    // The live challenge follows the method. The well-known endpoint serves
    // only `http` challenges, so a domain switched from DNS to HTTP would
    // otherwise have nothing to answer with.
    if let (Some(row), Some(method)) = (&row, verification_method) {
        sqlx::query(
            "UPDATE proxy_verification_challenges SET challenge_type = $2 \
             WHERE domain_id = $1 AND status IN ('pending', 'checking')",
        )
        .bind(row.id)
        .bind(verification_method_str(method))
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(row.map(Domain::from))
}

/// Record an uploaded certificate against a domain, scoped to a tenant.
///
/// Sets the provider to `manual` and the status to `active`: an operator who has
/// just supplied a certificate is not waiting for one. `auto_renew` is turned
/// off on the certificate row, because nothing here can renew a certificate it
/// did not obtain -- the renewal scan must not pick it up and mark it pending
/// for an ACME worker that has no authorization for the domain.
pub async fn record_uploaded_certificate(
    pool: &PgPool,
    id: Uuid,
    tenant_id: Uuid,
    expires_at: chrono::DateTime<chrono::Utc>,
) -> ProxyResult<Option<Domain>> {
    let mut tx = pool.begin().await?;

    let row = sqlx::query_as::<_, DomainRow>(
        "UPDATE proxy_domains SET \
             ssl_provider = 'manual', \
             ssl_status = 'active', \
             ssl_expires_at = $3, \
             ssl_error = NULL, \
             ssl_attempts = 0, \
             updated_at = NOW() \
         WHERE id = $1 AND tenant_id = $2 AND verification_status = 'verified' \
         RETURNING *",
    )
    .bind(id)
    .bind(tenant_id)
    .bind(expires_at)
    .fetch_optional(&mut *tx)
    .await?;

    let Some(row) = row else {
        tx.rollback().await?;
        return Ok(None);
    };

    sqlx::query(
        "UPDATE proxy_certificates SET \
             provider = 'manual', auto_renew = false, \
             tenant_id = $2, domain_id = $3, updated_at = NOW() \
         WHERE domain = $1",
    )
    .bind(&row.domain)
    .bind(tenant_id)
    .bind(id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(Some(Domain::from(row)))
}

/// Queue a verified ACME domain for issuance, scoped to a tenant.
///
/// Clears the attempt count, the error and the backoff so the worker picks it
/// up on its next pass rather than waiting out a wait computed for a cause the
/// caller has just fixed. Returns `None` when there is no such verified ACME
/// domain for the tenant.
pub async fn queue_for_issuance(
    pool: &PgPool,
    id: Uuid,
    tenant_id: Uuid,
) -> ProxyResult<Option<Domain>> {
    let row = sqlx::query_as::<_, DomainRow>(
        "UPDATE proxy_domains SET \
             ssl_status = 'pending', \
             ssl_attempts = 0, \
             ssl_error = NULL, \
             ssl_last_attempt_at = NULL, \
             updated_at = NOW() \
         WHERE id = $1 AND tenant_id = $2 \
           AND ssl_provider = 'acme' \
           AND verification_status = 'verified' \
         RETURNING *",
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

/// Delete a domain scoped to a tenant.
///
/// Returns `None` when there was no such domain, and otherwise the names of
/// the certificates that were linked to it. Their rows go with the domain, by
/// the foreign key's cascade; the names are returned so the caller can clear
/// the certificate store's file and memory caches too, which the database
/// cannot reach.
pub async fn delete_domain(
    pool: &PgPool,
    id: Uuid,
    tenant_id: Uuid,
) -> ProxyResult<Option<Vec<String>>> {
    let mut tx = pool.begin().await?;

    let certificates: Vec<String> = sqlx::query_scalar(
        "SELECT c.domain FROM proxy_certificates c \
         JOIN proxy_domains d ON d.id = c.domain_id \
         WHERE d.id = $1 AND d.tenant_id = $2",
    )
    .bind(id)
    .bind(tenant_id)
    .fetch_all(&mut *tx)
    .await?;

    let result = sqlx::query("DELETE FROM proxy_domains WHERE id = $1 AND tenant_id = $2")
        .bind(id)
        .bind(tenant_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;

    Ok((result.rows_affected() > 0).then_some(certificates))
}

/// What became of a request to start a verification attempt.
pub(crate) enum AttemptStart {
    /// The attempt is recorded; go ahead and check.
    Started(Domain),
    /// Already verified. Nothing to check.
    AlreadyVerified,
    /// Too soon after the last attempt.
    TooSoon,
    /// The registration has used all its attempts.
    Exhausted,
    /// The registration, or its challenge, has outlived its time. The domain
    /// has been marked `expired`.
    Expired,
    /// No such domain for this tenant.
    NotFound,
}

/// Record a verification attempt, if one is allowed now.
///
/// The checks and the increment are one `UPDATE`, so two requests arriving
/// together cannot both pass the interval check. Each attempt makes outbound
/// DNS or HTTP requests on the tenant's say-so; unthrottled, the verify
/// endpoint is a way to make this proxy fetch a URL as often as asked.
pub(crate) async fn start_verification_attempt(
    pool: &PgPool,
    id: Uuid,
    tenant_id: Uuid,
    min_interval: Duration,
    max_attempts: i32,
    pending_ttl: Duration,
) -> ProxyResult<AttemptStart> {
    let started = sqlx::query_as::<_, DomainRow>(
        "UPDATE proxy_domains d SET \
             verification_attempts = d.verification_attempts + 1, \
             last_verification_attempt = NOW(), \
             updated_at = NOW() \
         WHERE d.id = $1 AND d.tenant_id = $2 \
           AND d.verification_status <> 'verified' \
           AND d.verification_attempts < $3 \
           AND (d.last_verification_attempt IS NULL \
                OR d.last_verification_attempt <= NOW() - make_interval(secs => $4)) \
           AND d.created_at >= NOW() - make_interval(secs => $5) \
           AND EXISTS ( \
               SELECT 1 FROM proxy_verification_challenges c \
               WHERE c.domain_id = d.id \
                 AND c.status IN ('pending', 'checking') \
                 AND c.expires_at > NOW()) \
         RETURNING d.*",
    )
    .bind(id)
    .bind(tenant_id)
    .bind(max_attempts)
    .bind(min_interval.as_secs_f64())
    .bind(pending_ttl.as_secs_f64())
    .fetch_optional(pool)
    .await?;

    if let Some(row) = started {
        return Ok(AttemptStart::Started(Domain::from(row)));
    }

    // Refused: work out why, for the caller. Not race-free, and it does not
    // need to be -- the decision has already been made, atomically, above.
    let Some(domain) = get_domain_for_tenant(pool, id, tenant_id).await? else {
        return Ok(AttemptStart::NotFound);
    };
    if domain.verification_status == VerificationStatus::Verified {
        return Ok(AttemptStart::AlreadyVerified);
    }
    if domain.verification_attempts >= max_attempts {
        return Ok(AttemptStart::Exhausted);
    }

    let live_challenge: bool = sqlx::query_scalar(
        "SELECT EXISTS ( \
             SELECT 1 FROM proxy_verification_challenges c \
             JOIN proxy_domains d ON d.id = c.domain_id \
             WHERE c.domain_id = $1 \
               AND c.status IN ('pending', 'checking') \
               AND c.expires_at > NOW() \
               AND d.created_at >= NOW() - make_interval(secs => $2))",
    )
    .bind(id)
    .bind(pending_ttl.as_secs_f64())
    .fetch_one(pool)
    .await?;
    if !live_challenge {
        sqlx::query(
            "UPDATE proxy_domains SET verification_status = 'expired', updated_at = NOW() \
             WHERE id = $1 AND verification_status <> 'verified'",
        )
        .bind(id)
        .execute(pool)
        .await?;
        return Ok(AttemptStart::Expired);
    }

    Ok(AttemptStart::TooSoon)
}

/// Mark a domain verified, and its challenge resolved.
///
/// Resolving the challenge is what stops the well-known endpoint answering for
/// it: an unresolved challenge on a verified domain is a token anyone can
/// still fetch. A domain asking for ACME is queued for issuance in the same
/// transaction.
pub(crate) async fn mark_verified(pool: &PgPool, id: Uuid) -> ProxyResult<()> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        "UPDATE proxy_domains SET \
             verification_status = 'verified', \
             verified_at = NOW(), \
             ssl_status = CASE WHEN ssl_provider = 'acme' THEN 'pending' ELSE ssl_status END, \
             updated_at = NOW() \
         WHERE id = $1",
    )
    .bind(id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE proxy_verification_challenges \
         SET status = 'verified', verified_at = NOW() \
         WHERE domain_id = $1 AND status IN ('pending', 'checking')",
    )
    .bind(id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Enable a verified domain, scoped to a tenant.
///
/// The verification condition is in the statement, not only in the caller, so
/// that nothing can enable a domain that is not verified at the moment of the
/// write.
pub async fn enable_domain(pool: &PgPool, id: Uuid, tenant_id: Uuid) -> ProxyResult<bool> {
    let result = sqlx::query(
        "UPDATE proxy_domains SET enabled = true, updated_at = NOW() \
         WHERE id = $1 AND tenant_id = $2 AND verification_status = 'verified'",
    )
    .bind(id)
    .bind(tenant_id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

/// Disable a domain, scoped to a tenant.
pub async fn disable_domain(pool: &PgPool, id: Uuid, tenant_id: Uuid) -> ProxyResult<bool> {
    let result = sqlx::query(
        "UPDATE proxy_domains SET enabled = false, updated_at = NOW() \
         WHERE id = $1 AND tenant_id = $2",
    )
    .bind(id)
    .bind(tenant_id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

// ============================================================================
// Routes
// ============================================================================

/// Create a route for a domain.
///
/// In one transaction with the tenant row locked: the domain must belong to
/// the tenant and be verified, the upstream must belong to the tenant, and the
/// domain must have room under `max_routes_per_domain`. A route on an
/// unverified domain would be traffic configuration for a name nobody has
/// proved control of.
pub async fn create_route(
    pool: &PgPool,
    domain_id: Uuid,
    tenant_id: Uuid,
    req: CreateDomainRouteRequest,
) -> ProxyResult<DomainRoute> {
    let add_headers = serde_json::to_value(&req.add_headers).unwrap_or_default();
    let mut tx = pool.begin().await?;

    let limits = lock_tenant_limits(&mut tx, tenant_id).await?;

    let status: Option<String> = sqlx::query_scalar(
        "SELECT verification_status FROM proxy_domains WHERE id = $1 AND tenant_id = $2",
    )
    .bind(domain_id)
    .bind(tenant_id)
    .fetch_optional(&mut *tx)
    .await?;
    match status.as_deref() {
        None => return Err(ProxyError::NotFound("Domain not found".into())),
        Some("verified") => {}
        Some(_) => {
            return Err(ProxyError::Validation(
                "Domain must be verified before routes can be added".into(),
            ))
        }
    }

    let upstream_owned: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM proxy_domain_upstreams WHERE id = $1 AND tenant_id = $2)",
    )
    .bind(req.upstream_id)
    .bind(tenant_id)
    .fetch_one(&mut *tx)
    .await?;
    if !upstream_owned {
        return Err(ProxyError::NotFound("Upstream not found".into()));
    }

    let current: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM proxy_domain_routes WHERE domain_id = $1")
            .bind(domain_id)
            .fetch_one(&mut *tx)
            .await?;
    within_quota(current, limits.max_routes_per_domain, "Route")?;

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
    .fetch_one(&mut *tx)
    .await?;

    tx.commit().await?;
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
async fn load_backends<'c, E>(executor: E, upstream_id: Uuid) -> ProxyResult<Vec<DomainBackend>>
where
    E: sqlx::Executor<'c, Database = sqlx::Postgres>,
{
    let backends = sqlx::query_as::<_, DomainBackend>(
        "SELECT * FROM proxy_domain_backends WHERE upstream_id = $1 ORDER BY created_at",
    )
    .bind(upstream_id)
    .fetch_all(executor)
    .await?;
    Ok(backends)
}

/// Create an upstream and any backends supplied inline, all or nothing.
///
/// One transaction. It used to insert the upstream, then each backend, each
/// on its own -- so a backend that failed left the upstream behind, and the
/// caller got a 500 for a request that had half happened.
pub async fn create_upstream(
    pool: &PgPool,
    tenant_id: Uuid,
    req: CreateUpstreamRequest,
) -> ProxyResult<DomainUpstream> {
    let mut tx = pool.begin().await?;

    let limits = lock_tenant_limits(&mut tx, tenant_id).await?;
    let current: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM proxy_domain_upstreams WHERE tenant_id = $1")
            .bind(tenant_id)
            .fetch_one(&mut *tx)
            .await?;
    within_quota(current, limits.max_upstreams, "Upstream")?;
    if req.backends.len() > limits.max_backends_per_upstream.max(0) as usize {
        return Err(ProxyError::QuotaExceeded(format!(
            "Backend limit per upstream is {}",
            limits.max_backends_per_upstream
        )));
    }

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
    .fetch_one(&mut *tx)
    .await?;

    let mut upstream = DomainUpstream::from(row);

    for backend in &req.backends {
        insert_backend(&mut tx, upstream.id, backend).await?;
    }
    upstream.backends = load_backends(&mut *tx, upstream.id).await?;

    tx.commit().await?;
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

async fn insert_backend(
    conn: &mut PgConnection,
    upstream_id: Uuid,
    req: &CreateBackendRequest,
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
    .fetch_one(&mut *conn)
    .await?;
    Ok(backend)
}

/// Add a backend to one of the tenant's upstreams, within its quota.
pub async fn create_backend(
    pool: &PgPool,
    upstream_id: Uuid,
    tenant_id: Uuid,
    req: CreateBackendRequest,
) -> ProxyResult<DomainBackend> {
    let mut tx = pool.begin().await?;

    let limits = lock_tenant_limits(&mut tx, tenant_id).await?;
    let owned: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM proxy_domain_upstreams WHERE id = $1 AND tenant_id = $2)",
    )
    .bind(upstream_id)
    .bind(tenant_id)
    .fetch_one(&mut *tx)
    .await?;
    if !owned {
        return Err(ProxyError::NotFound("Upstream not found".into()));
    }

    let current: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM proxy_domain_backends WHERE upstream_id = $1")
            .bind(upstream_id)
            .fetch_one(&mut *tx)
            .await?;
    within_quota(current, limits.max_backends_per_upstream, "Backend")?;

    let backend = insert_backend(&mut tx, upstream_id, &req).await?;
    tx.commit().await?;
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

/// Insert a new API key row (the raw key is never stored, only its hash),
/// within the tenant's `max_api_keys`.
pub async fn create_api_key(
    pool: &PgPool,
    tenant_id: Uuid,
    req: CreateApiKeyRequest,
    key_hash: &str,
    key_prefix: &str,
) -> ProxyResult<ApiKeyRow> {
    let mut tx = pool.begin().await?;
    let limits = lock_tenant_limits(&mut tx, tenant_id).await?;
    let current: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM proxy_api_keys WHERE tenant_id = $1")
            .bind(tenant_id)
            .fetch_one(&mut *tx)
            .await?;
    within_quota(current, limits.max_api_keys, "API key")?;

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
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
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
            COALESCE((SELECT max_routes_per_domain FROM proxy_tenants WHERE id = $1), 0) AS routes_per_domain_limit, \
            (SELECT COUNT(*) FROM proxy_domain_upstreams WHERE tenant_id = $1) AS upstreams_count, \
            COALESCE((SELECT max_upstreams FROM proxy_tenants WHERE id = $1), 0) AS upstreams_limit, \
            COALESCE((SELECT max_backends_per_upstream FROM proxy_tenants WHERE id = $1), 0) AS backends_per_upstream_limit, \
            (SELECT COUNT(*) FROM proxy_api_keys WHERE tenant_id = $1) AS api_keys_count, \
            COALESCE((SELECT max_api_keys FROM proxy_tenants WHERE id = $1), 0) AS api_keys_limit",
    )
    .bind(tenant_id)
    .fetch_one(pool)
    .await?;

    Ok(TenantUsage {
        domains_count: row.get::<i64, _>("domains_count"),
        domains_limit: row.get::<i32, _>("domains_limit"),
        verified_domains: row.get::<i64, _>("verified_domains"),
        routes_count: row.get::<i64, _>("routes_count"),
        routes_per_domain_limit: row.get::<i32, _>("routes_per_domain_limit"),
        upstreams_count: row.get::<i64, _>("upstreams_count"),
        upstreams_limit: row.get::<i32, _>("upstreams_limit"),
        backends_per_upstream_limit: row.get::<i32, _>("backends_per_upstream_limit"),
        api_keys_count: row.get::<i64, _>("api_keys_count"),
        api_keys_limit: row.get::<i32, _>("api_keys_limit"),
    })
}
