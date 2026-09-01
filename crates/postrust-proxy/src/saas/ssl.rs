//! The background worker that obtains certificates for verified domains.
//!
//! Issuance is not a request. An ACME order involves several round trips to the
//! CA, a challenge the CA has to fetch back over the network, and rate limits
//! that make retrying-with-backoff the only sane failure mode. None of that fits
//! inside an HTTP handler: a thirty-second order would hold the request open,
//! and a client timeout would abandon an order the CA had already started.
//!
//! So the API only ever sets state. Verifying a domain with
//! `ssl_provider = 'acme'` leaves it `pending`; this worker drains `pending`,
//! and `POST /domains/{id}/ssl/retry` puts a `failed` one back.
//!
//! That is also why there is no `POST /domains/{id}/ssl/provision`. An older
//! version of `docs/saas-domains.md` documented one, and it was never in the
//! router.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::error::ProxyResult;
use crate::tls::AcmeIssuer;

/// How often to look for work.
const POLL_INTERVAL: Duration = Duration::from_secs(30);

/// Give up on a domain after this many consecutive failures.
///
/// The usual cause of failure is that the domain's DNS was never pointed at
/// this proxy, which no amount of retrying fixes. Ten attempts with the backoff
/// below spans about a day.
const MAX_ATTEMPTS: i32 = 10;

/// Renew once a certificate is within this long of expiring.
///
/// Let's Encrypt issues for 90 days and recommends renewing at 30, which leaves
/// a month to notice a problem.
const RENEW_WITHIN: chrono::TimeDelta = chrono::TimeDelta::days(30);

/// The first gap between attempts, doubled on each failure.
///
/// Named because the claim query computes the same curve in SQL, and the two
/// have to agree.
const BASE_BACKOFF: Duration = Duration::from_secs(60);

/// The longest gap between attempts.
///
/// The cap matters more than the growth: past a point the failure is a human
/// problem, and hammering the CA for a domain that will never validate spends a
/// rate limit that the domains which *would* validate have to share.
const MAX_BACKOFF: Duration = Duration::from_secs(4 * 60 * 60);

/// How long to wait after the nth consecutive failure.
///
/// One minute, doubling, capped at [`MAX_BACKOFF`]. The shift is clamped only
/// to keep it in range -- `1 << 64` is undefined and `1 << -1` panics in debug,
/// and `ssl_attempts` is a signed column a hand-edited row can make negative.
/// The clamp is set high enough that the cap is what actually binds; an earlier
/// version clamped at 8, which held the value below the cap and made the cap
/// dead code.
fn backoff(attempts: i32) -> Duration {
    let shift = attempts.clamp(0, 12) as u32;
    Duration::from_secs(BASE_BACKOFF.as_secs() << shift).min(MAX_BACKOFF)
}

/// A domain the worker has claimed.
struct Claim {
    id: Uuid,
    domain: String,
    attempts: i32,
}

/// Run until cancelled.
///
/// Errors are logged and the loop continues: a worker that exits on a database
/// hiccup would silently stop issuing certificates for the life of the process,
/// which is exactly the class of failure this module exists to remove.
pub async fn run(pool: PgPool, issuer: Arc<AcmeIssuer>, cancel: CancellationToken) {
    tracing::info!(
        directory = %issuer.directory_url(),
        "ACME issuance worker started"
    );

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                tracing::info!("ACME issuance worker stopped");
                return;
            }
            _ = tokio::time::sleep(POLL_INTERVAL) => {}
        }

        if let Err(error) = tick(&pool, &issuer).await {
            tracing::error!(%error, "ACME issuance pass failed");
        }
    }
}

/// One pass: mark anything due for renewal, then issue for anything pending.
async fn tick(pool: &PgPool, issuer: &AcmeIssuer) -> ProxyResult<()> {
    let renewals = mark_renewals(pool).await?;
    if renewals > 0 {
        tracing::info!(renewals, "certificates due for renewal");
    }

    while let Some(claim) = claim_next(pool).await? {
        match issuer.issue(&claim.domain).await {
            Ok(certificate) => {
                mark_active(pool, claim.id, certificate.expires_at).await?;
                tracing::info!(
                    domain = %claim.domain,
                    expires_at = ?certificate.expires_at,
                    "certificate active"
                );
            }
            Err(error) => {
                let attempts = claim.attempts + 1;
                let give_up = attempts >= MAX_ATTEMPTS;
                mark_failed(pool, claim.id, &error.to_string(), attempts).await?;
                if give_up {
                    tracing::error!(
                        domain = %claim.domain,
                        attempts,
                        %error,
                        "giving up on issuance; use the ssl/retry endpoint after fixing the cause"
                    );
                } else {
                    tracing::warn!(
                        domain = %claim.domain,
                        attempts,
                        retry_in = ?backoff(attempts),
                        %error,
                        "issuance failed, will retry"
                    );
                }
            }
        }
    }

    Ok(())
}

/// Claim one domain to work on, atomically.
///
/// `FOR UPDATE SKIP LOCKED` inside the same statement that flips the status is
/// what makes more than one proxy instance safe: two workers cannot claim the
/// same row, and neither blocks on the other. Without it both would place an
/// order for the same domain and burn two of the CA's rate-limited slots to get
/// one certificate.
///
/// The backoff is computed **in the query**, from the row's own `ssl_attempts`:
///
/// ```text
/// LEAST(base * 2 ^ attempts, cap)
/// ```
///
/// which mirrors [`backoff`]. It has to be one statement. An earlier version
/// claimed the row (setting `ssl_last_attempt_at = NOW()`) and then checked the
/// backoff in a second query -- against the timestamp it had just overwritten,
/// so every row with a failure behind it compared `NOW() < NOW() - wait`, was
/// always ineligible, and was released again. Backoff meant "never retry".
async fn claim_next(pool: &PgPool) -> ProxyResult<Option<Claim>> {
    let row = sqlx::query(
        "UPDATE proxy_domains SET \
             ssl_status = 'provisioning', \
             ssl_last_attempt_at = NOW(), \
             updated_at = NOW() \
         WHERE id = ( \
             SELECT id FROM proxy_domains \
             WHERE verification_status = 'verified' \
               AND ssl_provider = 'acme' \
               AND ssl_status = 'pending' \
               AND ssl_attempts < $1 \
               AND ( \
                   ssl_last_attempt_at IS NULL \
                   OR ssl_last_attempt_at < NOW() - make_interval( \
                       secs => LEAST($2 * (2 ^ ssl_attempts), $3) \
                   ) \
               ) \
             ORDER BY ssl_last_attempt_at ASC NULLS FIRST \
             FOR UPDATE SKIP LOCKED \
             LIMIT 1 \
         ) \
         RETURNING id, domain, ssl_attempts",
    )
    .bind(MAX_ATTEMPTS)
    .bind(BASE_BACKOFF.as_secs() as f64)
    .bind(MAX_BACKOFF.as_secs() as f64)
    .fetch_optional(pool)
    .await?;

    let Some(row) = row else { return Ok(None) };
    Ok(Some(Claim {
        id: row.try_get("id")?,
        domain: row.try_get("domain")?,
        attempts: row.try_get("ssl_attempts")?,
    }))
}

async fn mark_active(
    pool: &PgPool,
    id: Uuid,
    expires_at: Option<DateTime<Utc>>,
) -> ProxyResult<()> {
    sqlx::query(
        "UPDATE proxy_domains SET \
             ssl_status = 'active', \
             ssl_expires_at = $2, \
             ssl_error = NULL, \
             ssl_attempts = 0, \
             updated_at = NOW() \
         WHERE id = $1",
    )
    .bind(id)
    .bind(expires_at)
    .execute(pool)
    .await?;
    Ok(())
}

/// Record a failure.
///
/// Below the attempt limit the row goes back to `pending` so the worker will
/// come back to it; at the limit it becomes `failed`, which the worker does not
/// pick up. `ssl_error` carries the reason either way -- it is the only way an
/// operator finds out why a domain is stuck.
async fn mark_failed(pool: &PgPool, id: Uuid, error: &str, attempts: i32) -> ProxyResult<()> {
    let status = if attempts >= MAX_ATTEMPTS {
        "failed"
    } else {
        "pending"
    };

    sqlx::query(
        "UPDATE proxy_domains SET \
             ssl_status = $3, \
             ssl_error = $2, \
             ssl_attempts = $4, \
             updated_at = NOW() \
         WHERE id = $1",
    )
    .bind(id)
    .bind(error)
    .bind(status)
    .bind(attempts)
    .execute(pool)
    .await?;
    Ok(())
}

/// Move `active` domains whose certificate is expiring back to `pending`.
///
/// Returns how many. Driven off `proxy_certificates.expires_at` rather than
/// `proxy_domains.ssl_expires_at` so that a certificate replaced out of band --
/// uploaded by hand, say -- is still renewed on its real expiry.
async fn mark_renewals(pool: &PgPool) -> ProxyResult<u64> {
    let deadline = Utc::now() + RENEW_WITHIN;

    let result = sqlx::query(
        "UPDATE proxy_domains d SET \
             ssl_status = 'pending', \
             ssl_attempts = 0, \
             updated_at = NOW() \
         WHERE d.ssl_status = 'active' \
           AND d.ssl_provider = 'acme' \
           AND d.verification_status = 'verified' \
           AND EXISTS ( \
               SELECT 1 FROM proxy_certificates c \
               WHERE c.domain = d.domain \
                 AND c.auto_renew \
                 AND c.expires_at IS NOT NULL \
                 AND c.expires_at < $1 \
           )",
    )
    .bind(deadline)
    .execute(pool)
    .await?;

    Ok(result.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_grows_then_stops() {
        // One minute, doubling.
        assert_eq!(backoff(0), Duration::from_secs(60));
        assert_eq!(backoff(1), Duration::from_secs(120));
        assert_eq!(backoff(4), Duration::from_secs(16 * 60));

        // Capped, so a domain that will never validate is neither retried on a
        // schedule measured in days nor hammered for ever.
        assert_eq!(backoff(8), MAX_BACKOFF);
        assert_eq!(backoff(20), MAX_BACKOFF);
        assert_eq!(backoff(i32::MAX), MAX_BACKOFF);
    }

    #[test]
    fn the_cap_is_reachable() {
        // The bug this guards against: clamping the shift below the point where
        // the cap binds, which leaves the cap as dead code and silently changes
        // the longest gap. The first attempt to reach the cap must be within
        // the attempt limit, or the cap never applies in practice either.
        let first_capped = (0..=MAX_ATTEMPTS)
            .find(|n| backoff(*n) == MAX_BACKOFF)
            .expect("the cap is never reached within MAX_ATTEMPTS");
        assert!(
            first_capped < MAX_ATTEMPTS,
            "capped only at the last attempt"
        );
    }

    #[test]
    fn backoff_never_panics_on_a_negative_count() {
        // ssl_attempts is a signed column, so a hand-edited row can be
        // negative. `1u64 << -1` would panic in debug.
        assert_eq!(backoff(-1), Duration::from_secs(60));
        assert_eq!(backoff(i32::MIN), Duration::from_secs(60));
    }

    #[test]
    fn ten_attempts_of_backoff_span_about_a_day() {
        // The claim about MAX_ATTEMPTS in the doc comment, checked rather than
        // asserted in prose.
        let total: u64 = (0..MAX_ATTEMPTS).map(|n| backoff(n).as_secs()).sum();
        let hours = total / 3600;
        assert!(
            (12..=48).contains(&hours),
            "ten attempts span {hours}h, which no longer matches the documented ~1 day"
        );
    }

    #[test]
    fn renewal_threshold_leaves_room_to_notice() {
        // Let's Encrypt issues for 90 days.
        assert!(RENEW_WITHIN < chrono::TimeDelta::days(90));
        assert!(RENEW_WITHIN >= chrono::TimeDelta::days(14));
    }
}

/// Database-backed tests for the claim query and the retry reset.
///
/// `#[ignore]`d: they need PostgreSQL, and run in the same CI job as the other
/// database tests. They live here rather than in `tests/` so they can call
/// [`claim_next`] itself -- an integration test would have to duplicate the SQL,
/// and a duplicate that drifts is a test that passes while production is wrong.
#[cfg(test)]
mod db_tests {
    use super::*;
    use sqlx::Executor;

    const MIGRATIONS: [&str; 3] = [
        include_str!("../../migrations/20240115000001_saas_domains.sql"),
        include_str!("../../migrations/20260901000001_proxy_config.sql"),
        include_str!("../../migrations/20260901000002_proxy_acme.sql"),
    ];

    /// See `tests/config_persistence.rs` for why the migration is locked.
    const MIGRATION_LOCK: i64 = 0x706f7374_72757374;

    /// Serialises the tests in this module against each other.
    ///
    /// The claim queue is global by design -- `claim_next` takes whatever is
    /// due, from any tenant -- so these tests cannot be isolated by namespacing
    /// their rows the way the other database tests are. Run in parallel they
    /// steal each other's domains, and five of ten fail. Locking is the honest
    /// fix: it keeps them testing the real global query instead of a filtered
    /// version written for the tests.
    const CLAIM_LOCK: i64 = 0x706f7374_61636d65;

    /// Exclusive use of the claim queue, with the queue emptied.
    ///
    /// Holds a dedicated connection rather than a pooled one, so that dropping
    /// the guard closes the connection and PostgreSQL releases the session lock
    /// on its own -- `Drop` cannot await an unlock.
    struct Exclusive(#[allow(dead_code)] sqlx::PgConnection);

    async fn exclusive() -> Exclusive {
        use sqlx::Connection;
        let url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
        let mut conn = sqlx::PgConnection::connect(&url)
            .await
            .expect("could not open a dedicated connection");
        sqlx::query("SELECT pg_advisory_lock($1)")
            .bind(CLAIM_LOCK)
            .execute(&mut conn)
            .await
            .expect("could not take the claim lock");

        // Start from an empty queue. Safe under the lock, and it means a test
        // that leaves rows behind cannot affect the next one.
        sqlx::query("DELETE FROM proxy_domains WHERE ssl_provider = 'acme'")
            .execute(&mut conn)
            .await
            .expect("could not clear the queue");

        Exclusive(conn)
    }

    async fn pool() -> PgPool {
        let url = std::env::var("DATABASE_URL")
            .expect("DATABASE_URL must be set for the database-backed tests");
        let pool = PgPool::connect(&url)
            .await
            .expect("could not connect to DATABASE_URL");

        let mut tx = pool.begin().await.expect("begin");
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(MIGRATION_LOCK)
            .execute(&mut *tx)
            .await
            .expect("migration lock");
        for migration in MIGRATIONS {
            (&mut *tx)
                .execute(migration)
                .await
                .expect("could not apply a migration");
        }
        tx.commit().await.expect("commit");
        pool
    }

    /// A tenant to hang domains off.
    async fn tenant(pool: &PgPool) -> Uuid {
        sqlx::query_scalar(
            "INSERT INTO proxy_tenants (name, slug, email) VALUES ($1, $2, $3) RETURNING id",
        )
        .bind("worker test")
        .bind(format!("t-{}", Uuid::new_v4()))
        .bind("worker@postrust.invalid")
        .fetch_one(pool)
        .await
        .expect("insert tenant")
    }

    /// A verified ACME domain with a given attempt history.
    async fn domain(
        pool: &PgPool,
        tenant_id: Uuid,
        ssl_status: &str,
        attempts: i32,
        last_attempt_secs_ago: Option<i64>,
    ) -> (Uuid, String) {
        let name = format!("d-{}.example", Uuid::new_v4());
        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO proxy_domains \
                (tenant_id, domain, verification_status, verification_method, \
                 verification_token, ssl_status, ssl_provider, ssl_attempts, \
                 ssl_last_attempt_at) \
             VALUES ($1, $2, 'verified', 'dns', 'tok', $3, 'acme', $4, \
                 CASE WHEN $5::bigint IS NULL THEN NULL \
                      ELSE NOW() - make_interval(secs => $5::bigint) END) \
             RETURNING id",
        )
        .bind(tenant_id)
        .bind(&name)
        .bind(ssl_status)
        .bind(attempts)
        .bind(last_attempt_secs_ago)
        .fetch_one(pool)
        .await
        .expect("insert domain");
        (id, name)
    }

    /// Drain the queue and report whether `id` came up.
    async fn claims_include(pool: &PgPool, id: Uuid) -> bool {
        let mut found = false;
        while let Some(claim) = claim_next(pool).await.expect("claim") {
            if claim.id == id {
                found = true;
            }
        }
        found
    }

    async fn drain(pool: &PgPool) -> Vec<Uuid> {
        let mut ids = Vec::new();
        while let Some(claim) = claim_next(pool).await.expect("claim") {
            ids.push(claim.id);
        }
        ids
    }

    #[tokio::test]
    #[ignore = "requires PostgreSQL"]
    async fn a_fresh_pending_domain_is_claimed() {
        let pool = pool().await;
        let _exclusive = exclusive().await;
        let t = tenant(&pool).await;
        let (id, _) = domain(&pool, t, "pending", 0, None).await;

        assert!(
            claims_include(&pool, id).await,
            "a verified ACME domain with no attempts should be claimed"
        );

        let status: String =
            sqlx::query_scalar("SELECT ssl_status FROM proxy_domains WHERE id = $1")
                .bind(id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(status, "provisioning", "claiming must take the row");
    }

    #[tokio::test]
    #[ignore = "requires PostgreSQL"]
    async fn a_domain_that_failed_recently_waits() {
        let pool = pool().await;
        let _exclusive = exclusive().await;
        let t = tenant(&pool).await;
        // One failure, ten seconds ago. The first backoff is a minute.
        let (id, _) = domain(&pool, t, "pending", 1, Some(10)).await;

        assert!(
            !claims_include(&pool, id).await,
            "a domain inside its backoff window should not be claimed"
        );
    }

    #[tokio::test]
    #[ignore = "requires PostgreSQL"]
    async fn a_domain_whose_backoff_has_elapsed_is_claimed_again() {
        let pool = pool().await;
        let _exclusive = exclusive().await;
        let t = tenant(&pool).await;
        // One failure, an hour ago. The first backoff is a minute, so it is due.
        //
        // This is the case the original two-query version could never reach: it
        // stamped ssl_last_attempt_at = NOW() and then compared against that, so
        // every row with attempts > 0 looked as though it had just been tried.
        let (id, _) = domain(&pool, t, "pending", 1, Some(3600)).await;

        assert!(
            claims_include(&pool, id).await,
            "a domain past its backoff window must be retried"
        );
    }

    #[tokio::test]
    #[ignore = "requires PostgreSQL"]
    async fn backoff_grows_with_attempts() {
        let pool = pool().await;
        let _exclusive = exclusive().await;
        let t = tenant(&pool).await;

        // Four failures: 60 * 2^4 = 16 minutes. Ten minutes ago is too soon;
        // twenty is due.
        let (too_soon, _) = domain(&pool, t, "pending", 4, Some(10 * 60)).await;
        let (due, _) = domain(&pool, t, "pending", 4, Some(20 * 60)).await;

        let claimed = drain(&pool).await;

        assert!(
            claimed.contains(&due),
            "a domain past its grown backoff must be retried"
        );
        assert!(
            !claimed.contains(&too_soon),
            "the backoff window must grow with the attempt count"
        );
    }

    #[tokio::test]
    #[ignore = "requires PostgreSQL"]
    async fn the_backoff_is_capped() {
        let pool = pool().await;
        let _exclusive = exclusive().await;
        let t = tenant(&pool).await;

        // Nine failures. Uncapped that would be 60 * 2^9 = 8.5 hours; the cap is
        // four, so five hours ago is due.
        let (id, _) = domain(&pool, t, "pending", 9, Some(5 * 60 * 60)).await;

        assert!(
            claims_include(&pool, id).await,
            "the cap must bind, or a domain waits far longer than intended"
        );
    }

    #[tokio::test]
    #[ignore = "requires PostgreSQL"]
    async fn a_domain_at_the_attempt_limit_is_left_alone() {
        let pool = pool().await;
        let _exclusive = exclusive().await;
        let t = tenant(&pool).await;
        // At the limit, long past any backoff.
        let (id, _) = domain(&pool, t, "pending", 10, Some(24 * 60 * 60)).await;

        assert!(
            !claims_include(&pool, id).await,
            "a domain at MAX_ATTEMPTS must not be picked up again without a retry"
        );
    }

    #[tokio::test]
    #[ignore = "requires PostgreSQL"]
    async fn only_pending_acme_domains_are_claimed() {
        let pool = pool().await;
        let _exclusive = exclusive().await;
        let t = tenant(&pool).await;

        let (active, _) = domain(&pool, t, "active", 0, None).await;
        let (failed, _) = domain(&pool, t, "failed", 0, None).await;
        let (provisioning, _) = domain(&pool, t, "provisioning", 0, None).await;

        // Not ACME, and not verified: both must be invisible to the worker.
        let manual: Uuid = sqlx::query_scalar(
            "INSERT INTO proxy_domains \
                (tenant_id, domain, verification_status, verification_token, \
                 ssl_status, ssl_provider) \
             VALUES ($1, $2, 'verified', 'tok', 'pending', 'manual') RETURNING id",
        )
        .bind(t)
        .bind(format!("manual-{}.example", Uuid::new_v4()))
        .fetch_one(&pool)
        .await
        .unwrap();

        let unverified: Uuid = sqlx::query_scalar(
            "INSERT INTO proxy_domains \
                (tenant_id, domain, verification_status, verification_token, \
                 ssl_status, ssl_provider) \
             VALUES ($1, $2, 'pending', 'tok', 'pending', 'acme') RETURNING id",
        )
        .bind(t)
        .bind(format!("unverified-{}.example", Uuid::new_v4()))
        .fetch_one(&pool)
        .await
        .unwrap();

        let claimed = drain(&pool).await;

        for (id, why) in [
            (active, "already active"),
            (failed, "given up on"),
            (provisioning, "already being worked on"),
            (manual, "not an ACME domain"),
            (unverified, "not verified"),
        ] {
            assert!(!claimed.contains(&id), "claimed a domain that is {why}");
        }
    }

    #[tokio::test]
    #[ignore = "requires PostgreSQL"]
    async fn provision_requeues_a_failed_domain_and_clears_its_history() {
        let pool = pool().await;
        let _exclusive = exclusive().await;
        let t = tenant(&pool).await;
        let (id, _) = domain(&pool, t, "failed", 10, Some(60)).await;
        sqlx::query("UPDATE proxy_domains SET ssl_error = 'something' WHERE id = $1")
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();

        let requeued = crate::saas::db::queue_for_issuance(&pool, id, t)
            .await
            .expect("retry");
        assert!(requeued.is_some(), "provision should find the domain");

        let row = sqlx::query(
            "SELECT ssl_status, ssl_attempts, ssl_error, ssl_last_attempt_at \
             FROM proxy_domains WHERE id = $1",
        )
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();

        use sqlx::Row;
        assert_eq!(row.get::<String, _>("ssl_status"), "pending");
        assert_eq!(row.get::<i32, _>("ssl_attempts"), 0);
        assert!(row.get::<Option<String>, _>("ssl_error").is_none());
        // Cleared, so the next pass picks it up now rather than waiting out a
        // backoff computed for a cause the operator has just fixed.
        assert!(row
            .get::<Option<chrono::DateTime<chrono::Utc>>, _>("ssl_last_attempt_at")
            .is_none());

        assert!(
            claims_include(&pool, id).await,
            "a retried domain must be claimable straight away"
        );
    }

    #[tokio::test]
    #[ignore = "requires PostgreSQL"]
    async fn provision_refuses_a_domain_that_is_not_verified() {
        let pool = pool().await;
        let _exclusive = exclusive().await;
        let t = tenant(&pool).await;

        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO proxy_domains \
                (tenant_id, domain, verification_status, verification_token, \
                 ssl_status, ssl_provider) \
             VALUES ($1, $2, 'pending', 'tok', 'pending', 'acme') RETURNING id",
        )
        .bind(t)
        .bind(format!("unverified-{}.example", Uuid::new_v4()))
        .fetch_one(&pool)
        .await
        .unwrap();

        // Queueing an unverified domain would have the worker place an order
        // for a name nobody has proved control of. The CA would refuse it, but
        // the attempt still spends a rate limit.
        let queued = crate::saas::db::queue_for_issuance(&pool, id, t)
            .await
            .expect("queue");
        assert!(queued.is_none(), "queued an unverified domain");
    }

    #[tokio::test]
    #[ignore = "requires PostgreSQL"]
    async fn provision_refuses_a_manual_domain() {
        let pool = pool().await;
        let _exclusive = exclusive().await;
        let t = tenant(&pool).await;

        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO proxy_domains \
                (tenant_id, domain, verification_status, verification_token, \
                 ssl_status, ssl_provider) \
             VALUES ($1, $2, 'verified', 'tok', 'pending', 'manual') RETURNING id",
        )
        .bind(t)
        .bind(format!("manual-{}.example", Uuid::new_v4()))
        .fetch_one(&pool)
        .await
        .unwrap();

        let queued = crate::saas::db::queue_for_issuance(&pool, id, t)
            .await
            .expect("queue");
        assert!(
            queued.is_none(),
            "queued a domain whose certificate does not come from ACME"
        );
    }

    #[tokio::test]
    #[ignore = "requires PostgreSQL"]
    async fn switching_a_verified_domain_to_acme_queues_it() {
        let pool = pool().await;
        let _exclusive = exclusive().await;
        let t = tenant(&pool).await;

        // Verified, manual, and nowhere near the worker's queue.
        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO proxy_domains \
                (tenant_id, domain, verification_status, verification_token, \
                 ssl_status, ssl_provider, ssl_attempts, ssl_error) \
             VALUES ($1, $2, 'verified', 'tok', 'active', 'manual', 4, 'old') \
             RETURNING id",
        )
        .bind(t)
        .bind(format!("switch-{}.example", Uuid::new_v4()))
        .fetch_one(&pool)
        .await
        .unwrap();

        let updated = crate::saas::db::update_domain(
            &pool,
            id,
            t,
            None,
            Some(&crate::saas::types::SslProvider::Acme),
        )
        .await
        .expect("update")
        .expect("domain should be found");

        // Otherwise the domain would sit in whatever state the old provider
        // left it in, and nothing would ever pick it up.
        assert_eq!(
            format!("{:?}", updated.ssl_status),
            "Pending",
            "switching to acme must queue the domain"
        );
        assert!(
            claims_include(&pool, id).await,
            "a domain switched to acme must become claimable"
        );

        let row = sqlx::query("SELECT ssl_attempts, ssl_error FROM proxy_domains WHERE id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
        use sqlx::Row as _;
        assert_eq!(row.get::<i32, _>("ssl_attempts"), 0, "history not cleared");
        assert!(row.get::<Option<String>, _>("ssl_error").is_none());
    }

    #[tokio::test]
    #[ignore = "requires PostgreSQL"]
    async fn switching_an_unverified_domain_to_acme_does_not_queue_it() {
        let pool = pool().await;
        let _exclusive = exclusive().await;
        let t = tenant(&pool).await;

        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO proxy_domains \
                (tenant_id, domain, verification_status, verification_token, \
                 ssl_status, ssl_provider) \
             VALUES ($1, $2, 'pending', 'tok', 'pending', 'manual') RETURNING id",
        )
        .bind(t)
        .bind(format!("switch-unverified-{}.example", Uuid::new_v4()))
        .fetch_one(&pool)
        .await
        .unwrap();

        crate::saas::db::update_domain(
            &pool,
            id,
            t,
            None,
            Some(&crate::saas::types::SslProvider::Acme),
        )
        .await
        .expect("update")
        .expect("domain should be found");

        assert!(
            !claims_include(&pool, id).await,
            "an unverified domain must not become claimable by changing its provider"
        );
    }

    #[tokio::test]
    #[ignore = "requires PostgreSQL"]
    async fn an_update_leaves_absent_fields_alone() {
        let pool = pool().await;
        let _exclusive = exclusive().await;
        let t = tenant(&pool).await;
        let (id, _) = domain(&pool, t, "pending", 3, Some(10)).await;

        // Only the verification method. The provider, and everything the
        // provider clause would have reset, must be untouched.
        let updated = crate::saas::db::update_domain(
            &pool,
            id,
            t,
            Some(&crate::saas::types::VerificationMethod::Http),
            None,
        )
        .await
        .expect("update")
        .expect("domain should be found");

        assert_eq!(format!("{:?}", updated.verification_method), "Http");
        assert_eq!(format!("{:?}", updated.ssl_provider), "Acme");

        let attempts: i32 =
            sqlx::query_scalar("SELECT ssl_attempts FROM proxy_domains WHERE id = $1")
                .bind(id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(attempts, 3, "an unrelated update reset the attempt count");
    }

    #[tokio::test]
    #[ignore = "requires PostgreSQL"]
    async fn an_update_will_not_cross_a_tenant_boundary() {
        let pool = pool().await;
        let _exclusive = exclusive().await;
        let owner = tenant(&pool).await;
        let stranger = tenant(&pool).await;
        let (id, _) = domain(&pool, owner, "pending", 0, None).await;

        let updated = crate::saas::db::update_domain(
            &pool,
            id,
            stranger,
            None,
            Some(&crate::saas::types::SslProvider::None),
        )
        .await
        .expect("update");
        assert!(updated.is_none(), "update crossed a tenant boundary");
    }

    #[tokio::test]
    #[ignore = "requires PostgreSQL"]
    async fn provision_will_not_touch_another_tenants_domain() {
        let pool = pool().await;
        let _exclusive = exclusive().await;
        let owner = tenant(&pool).await;
        let stranger = tenant(&pool).await;
        let (id, _) = domain(&pool, owner, "failed", 10, Some(60)).await;

        let requeued = crate::saas::db::queue_for_issuance(&pool, id, stranger)
            .await
            .expect("retry");
        assert!(requeued.is_none(), "provision crossed a tenant boundary");

        let status: String =
            sqlx::query_scalar("SELECT ssl_status FROM proxy_domains WHERE id = $1")
                .bind(id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(status, "failed", "another tenant's domain was modified");
    }

    #[tokio::test]
    #[ignore = "requires PostgreSQL"]
    async fn two_workers_do_not_claim_the_same_domain() {
        let pool = pool().await;
        let _exclusive = exclusive().await;
        let t = tenant(&pool).await;
        let (id, _) = domain(&pool, t, "pending", 0, None).await;

        // Claiming flips the status inside the same statement, so a second claim
        // cannot see the row. Without that, two instances would each place an
        // order and spend two of the CA's rate-limited slots for one certificate.
        let first = claim_next(&pool).await.expect("claim");
        assert!(first.is_some());

        let others = drain(&pool).await;
        assert!(!others.contains(&id), "the same domain was claimed twice");
    }
}
