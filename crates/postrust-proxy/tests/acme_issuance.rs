//! End-to-end ACME issuance against a real certificate authority.
//!
//! Runs against [Pebble](https://github.com/letsencrypt/pebble), Let's
//! Encrypt's test CA, which deliberately misbehaves -- it rejects a share of
//! nonces, varies challenge ordering, and returns states a happy-path client
//! does not expect. That is the point: it breaks clients that only handle the
//! good case.
//!
//! `scripts/acme/run.sh` brings up everything and runs this. It needs a CA, a
//! DNS server that resolves the test domain back to us, and a route from the
//! CA's container to the challenge endpoint, so it is not something a bare
//! `cargo test` can arrange.
//!
//! What this actually exercises, end to end:
//!
//! - account registration, and its persistence in `proxy_acme_accounts`
//! - order placement and the `http-01` challenge
//! - the **real** `/.well-known/acme-challenge/{token}` handler, served from
//!   `saas_router`, reading the row the issuer wrote
//! - the CA fetching that challenge over the network and validating it
//! - finalization, the CSR, and the certificate chain coming back
//! - storage in `proxy_certificates`, and challenge cleanup afterwards

// Gated on a non-default feature rather than skipped at runtime. A test that
// decides for itself that it need not run is a test nobody notices has stopped
// running; with a feature, either it is compiled and must pass, or it is not
// there. `scripts/acme/run.sh` and the conformance workflow turn it on.
#![cfg(feature = "pebble-tests")]

use std::sync::Arc;

use postrust_proxy::saas::handlers::{saas_router, SaasState};
use postrust_proxy::tls::{AcmeIssuer, CertificateStore};
use sqlx::{Executor, PgPool};

const MIGRATIONS: [&str; 3] = [
    include_str!("../migrations/20240115000001_saas_domains.sql"),
    include_str!("../migrations/20260901000001_proxy_config.sql"),
    include_str!("../migrations/20260901000002_proxy_acme.sql"),
];

/// See `tests/config_persistence.rs` for why this is locked.
const MIGRATION_LOCK: i64 = 0x706f7374_72757374;

fn env(name: &str) -> String {
    std::env::var(name)
        .unwrap_or_else(|_| panic!("{name} must be set; run this through scripts/acme/run.sh"))
}

async fn migrated_pool() -> PgPool {
    let pool = PgPool::connect(&env("DATABASE_URL"))
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

#[tokio::test]
#[ignore = "requires Pebble; run scripts/acme/run.sh"]
async fn a_certificate_is_issued_end_to_end() {
    let pool = migrated_pool().await;
    let domain = env("ACME_TEST_DOMAIN");
    let challenge_port: u16 = env("ACME_CHALLENGE_PORT").parse().expect("port");

    let cache = tempdir();

    // Serve the real router, so the challenge is answered by the handler that
    // ships rather than by something written for the test.
    let state = SaasState::new(pool.clone(), None, &cache)
        .await
        .expect("saas state");
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", challenge_port))
        .await
        .expect("could not bind the challenge port");
    let server = tokio::spawn(async move {
        axum::serve(listener, saas_router(state)).await.ok();
    });

    let cert_store = Arc::new(
        CertificateStore::new(pool.clone(), &cache)
            .await
            .expect("cert store"),
    );

    let issuer = AcmeIssuer::new(
        env("ACME_DIRECTORY"),
        Some("acme-test@postrust.invalid".to_string()),
        pool.clone(),
        cert_store.clone(),
    )
    .with_root_certificate(env("ACME_ROOT_PEM"));

    let certificate = issuer
        .issue(&domain)
        .await
        .expect("issuance should succeed against Pebble");

    // A chain, and a key, and an expiry we could read.
    let cert_pem = String::from_utf8(certificate.cert_pem.clone()).expect("cert is utf-8");
    assert!(
        cert_pem.contains("BEGIN CERTIFICATE"),
        "no PEM certificate in the chain: {cert_pem:.200}"
    );
    let key_pem = String::from_utf8(certificate.key_pem.clone()).expect("key is utf-8");
    assert!(
        key_pem.contains("PRIVATE KEY"),
        "no PEM private key came back"
    );
    assert!(
        certificate.expires_at.is_some(),
        "expiry could not be read from the issued chain, so renewal cannot be scheduled"
    );

    // The certificate the CA issued is for the domain we asked about.
    let (_, pem) = x509_parser::pem::parse_x509_pem(certificate.cert_pem.as_slice())
        .expect("chain parses as PEM");
    let (_, parsed) =
        x509_parser::parse_x509_certificate(&pem.contents).expect("certificate parses");
    let names: Vec<String> = parsed
        .subject_alternative_name()
        .ok()
        .flatten()
        .map(|san| {
            san.value
                .general_names
                .iter()
                .map(|n| n.to_string())
                .collect()
        })
        .unwrap_or_default();
    assert!(
        names.iter().any(|n| n.contains(&domain)),
        "issued certificate does not cover {domain}; SANs were {names:?}"
    );

    // Stored, and reloadable through the store's own path.
    let stored = cert_store
        .get(&domain)
        .await
        .expect("the certificate should be in the store");
    assert_eq!(stored.cert_pem, certificate.cert_pem);

    // In the database, not only the file cache.
    let in_db: i64 =
        sqlx::query_scalar("SELECT count(*) FROM proxy_certificates WHERE domain = $1")
            .bind(&domain)
            .fetch_one(&pool)
            .await
            .expect("count");
    assert_eq!(in_db, 1, "certificate did not reach proxy_certificates");

    // The account was persisted, so a second issuance does not register again
    // and spend a rate limit.
    let accounts: i64 = sqlx::query_scalar("SELECT count(*) FROM proxy_acme_accounts")
        .fetch_one(&pool)
        .await
        .expect("count");
    assert_eq!(accounts, 1, "the ACME account was not persisted");

    // Challenges are cleaned up: a token left answerable is a value the proxy
    // will hand to anyone who asks for it.
    let leftover: i64 =
        sqlx::query_scalar("SELECT count(*) FROM proxy_acme_challenges WHERE domain = $1")
            .bind(&domain)
            .fetch_one(&pool)
            .await
            .expect("count");
    assert_eq!(leftover, 0, "challenge rows outlived the order");

    server.abort();
}

#[tokio::test]
#[ignore = "requires Pebble; run scripts/acme/run.sh"]
async fn a_domain_that_does_not_resolve_here_fails_with_a_useful_message() {
    let pool = migrated_pool().await;

    let cache = tempdir();
    let cert_store = Arc::new(
        CertificateStore::new(pool.clone(), &cache)
            .await
            .expect("cert store"),
    );

    // No challenge server is started for this one, and challtestsrv resolves
    // every name to the address the harness set -- so the CA reaches something
    // that will not answer. This is by far the most common real failure, and
    // the message has to say what to check.
    let issuer = AcmeIssuer::new(
        env("ACME_DIRECTORY"),
        None,
        pool.clone(),
        cert_store.clone(),
    )
    .with_root_certificate(env("ACME_ROOT_PEM"));

    let error = issuer
        .issue("unreachable.postrust.invalid")
        .await
        .expect_err("issuance should fail when the challenge cannot be fetched");

    let message = error.to_string();
    assert!(
        message.contains("does not resolve to this proxy")
            || message.contains("did not become ready"),
        "the failure should explain what to check, but said: {message}"
    );

    // And it must not leave the token answerable.
    let leftover: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM proxy_acme_challenges WHERE domain = 'unreachable.postrust.invalid'",
    )
    .fetch_one(&pool)
    .await
    .expect("count");
    assert_eq!(leftover, 0, "a failed order left its challenge rows behind");
}

/// A scratch directory for the certificate store's file cache.
fn tempdir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("postrust-acme-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}
