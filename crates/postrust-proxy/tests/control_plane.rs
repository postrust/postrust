//! The SaaS control plane, driven through `saas_router` against PostgreSQL.
//!
//! Each test here is one of the ways a tenant could reach past its own
//! credential, its own names or the network it was meant to stay out of, and
//! checks it is refused through the router a deployment actually mounts --
//! not through the function underneath, which can be right while the handler
//! forgets to call it.
//!
//! These need a live PostgreSQL, so they are `#[ignore]`d:
//!
//!     DATABASE_URL=postgres://postgres:postgres@localhost:5432/postrust_test \
//!         cargo test -p postrust-proxy --test control_plane -- --ignored
//!
//! Every test applies the migrations itself (see `tests/config_persistence.rs`
//! for why under a lock), creates its own tenants, and names its domains with
//! a fresh UUID, so the file runs in parallel with itself and against a
//! database other runs have used.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use postrust_proxy::saas::api_keys::hash_api_key;
use postrust_proxy::saas::handlers::{saas_router, SaasState};
use postrust_proxy::saas::{ReservedDomains, SaasOptions, UploadTrust};
use serde_json::{json, Value};
use sqlx::{AssertSqlSafe, Executor, PgPool};
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

const MIGRATIONS: [&str; 4] = [
    include_str!("../migrations/20240115000001_saas_domains.sql"),
    include_str!("../migrations/20260901000001_proxy_config.sql"),
    include_str!("../migrations/20260901000002_proxy_acme.sql"),
    include_str!("../migrations/20261030000001_proxy_control_plane.sql"),
];

/// See `tests/config_persistence.rs` for why the migration is locked.
const MIGRATION_LOCK: i64 = 0x706f7374_72757374;

const JWT_SECRET: &str = "control-plane-test-secret";

/// Connect, with the schema in place.
///
/// The migrations are applied once per process, not once per test. They
/// re-run cleanly, but re-running takes table locks -- `DROP TRIGGER` in the
/// first migration is ACCESS EXCLUSIVE -- that deadlock with the transactions
/// of the tests already running alongside.
async fn migrated_pool() -> PgPool {
    static MIGRATED: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

    let url = std::env::var("DATABASE_URL")
        .expect("DATABASE_URL must be set for the database-backed tests");
    let pool = PgPool::connect(&url)
        .await
        .expect("could not connect to DATABASE_URL");

    MIGRATED
        .get_or_init(|| async {
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
        })
        .await;
    pool
}

/// The router, its pool, and where its certificate store keeps files.
struct Harness {
    app: axum::Router,
    pool: PgPool,
    cert_dir: std::path::PathBuf,
}

impl Harness {
    async fn new() -> Self {
        Self::with(|_| {}).await
    }

    async fn with(configure: impl FnOnce(&mut SaasOptions)) -> Self {
        let pool = migrated_pool().await;
        let mut options = SaasOptions::default();
        options.jwt_secret = Some(JWT_SECRET.to_string());
        configure(&mut options);
        Self::on(pool, options).await
    }

    async fn on(pool: PgPool, options: SaasOptions) -> Self {
        let cert_dir = std::env::temp_dir().join(format!("postrust-cp-{}", Uuid::new_v4()));
        let state = SaasState::with_options(pool.clone(), options, &cert_dir)
            .await
            .expect("saas state");
        Self {
            app: saas_router(state),
            pool,
            cert_dir,
        }
    }

    /// A fresh active tenant.
    async fn tenant(&self) -> Uuid {
        sqlx::query_scalar(
            "INSERT INTO proxy_tenants (name, slug, email) VALUES ('cp', $1, 'cp@postrust.invalid') \
             RETURNING id",
        )
        .bind(format!("cp-{}", Uuid::new_v4()))
        .fetch_one(&self.pool)
        .await
        .expect("insert tenant")
    }

    /// An API key for `tenant` with exactly these scopes, written directly so
    /// the test does not depend on the endpoint it may be testing.
    async fn key(&self, tenant: Uuid, scopes: &[&str]) -> String {
        let raw = format!("pr_test_{}", Uuid::new_v4().simple());
        let scopes: Vec<String> = scopes.iter().map(|s| s.to_string()).collect();
        sqlx::query(
            "INSERT INTO proxy_api_keys (tenant_id, name, key_hash, key_prefix, scopes) \
             VALUES ($1, 'test', $2, $3, $4)",
        )
        .bind(tenant)
        .bind(hash_api_key(&raw))
        .bind(&raw[..8])
        .bind(&scopes)
        .execute(&self.pool)
        .await
        .expect("insert key");
        format!("ApiKey {raw}")
    }

    async fn call(
        &self,
        method: &str,
        path: &str,
        auth: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        self.request(method, path, &[("authorization", auth)], body)
            .await
    }

    async fn request(
        &self,
        method: &str,
        path: &str,
        headers: &[(&str, &str)],
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut builder = Request::builder().method(method).uri(path);
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        let request = match body {
            Some(body) => builder
                .header("content-type", "application/json")
                .body(Body::from(body.to_string())),
            None => builder.body(Body::empty()),
        }
        .unwrap();

        let response = self.app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let value = serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()));
        (status, value)
    }

    /// Register a domain through the API and return its id and token.
    async fn domain(&self, key: &str, name: &str) -> (Uuid, String) {
        let (status, body) = self
            .call(
                "POST",
                "/api/v1/domains",
                key,
                Some(json!({ "domain": name })),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        (
            body["data"]["id"].as_str().unwrap().parse().unwrap(),
            body["data"]["verification_token"]
                .as_str()
                .unwrap()
                .to_string(),
        )
    }

    /// Mark a domain verified, as a successful check would.
    async fn verify(&self, id: Uuid) {
        sqlx::query(
            "UPDATE proxy_domains SET verification_status = 'verified', verified_at = NOW() \
             WHERE id = $1",
        )
        .bind(id)
        .execute(&self.pool)
        .await
        .unwrap();
    }

    async fn upstream(&self, key: &str) -> Uuid {
        let (status, body) = self
            .call(
                "POST",
                "/api/v1/upstreams",
                key,
                Some(json!({ "name": unique("up") })),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        body["data"]["id"].as_str().unwrap().parse().unwrap()
    }
}

fn unique(prefix: &str) -> String {
    format!("{prefix}-{}", Uuid::new_v4().simple())
}

fn name() -> String {
    format!("{}.test", unique("d"))
}

const ALL: &[&str] = &["*"];

// ---------------------------------------------------------------------------
// 1. API-key endpoints are scoped, and a key cannot mint more than it holds.
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn api_key_endpoints_need_api_key_scopes() {
    let h = Harness::new().await;
    let t = h.tenant().await;
    let other = h.key(t, ALL).await;
    let other_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM proxy_api_keys WHERE tenant_id = $1 ORDER BY created_at LIMIT 1",
    )
    .bind(t)
    .fetch_one(&h.pool)
    .await
    .unwrap();

    for scopes in [
        &["domains:read"][..],
        &[],
        &["domains:write", "upstreams:write"],
    ] {
        let key = h.key(t, scopes).await;
        let (status, body) = h
            .call(
                "POST",
                "/api/v1/auth/api-keys",
                &key,
                Some(json!({ "name": "x", "scopes": ["*"] })),
            )
            .await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "{scopes:?} created a key: {body}"
        );
        assert_eq!(body["code"], "insufficient_scope");

        let (status, _) = h.call("GET", "/api/v1/auth/api-keys", &key, None).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{scopes:?} listed keys");

        let (status, _) = h
            .call(
                "DELETE",
                &format!("/api/v1/auth/api-keys/{other_id}"),
                &key,
                None,
            )
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{scopes:?} revoked a key");
    }

    // The `*` key is still there and still works.
    let (status, _) = h.call("GET", "/api/v1/auth/api-keys", &other, None).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_key_cannot_grant_more_than_it_holds() {
    let h = Harness::new().await;
    let t = h.tenant().await;
    let narrow = h.key(t, &["api_keys:write", "domains:read"]).await;

    for scopes in [
        json!(["*"]),
        json!(["domains:write"]),
        json!(["routes:read"]),
    ] {
        let (status, body) = h
            .call(
                "POST",
                "/api/v1/auth/api-keys",
                &narrow,
                Some(json!({ "name": "escalate", "scopes": scopes })),
            )
            .await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "{scopes} was granted: {body}"
        );
    }

    // The default scopes include domains:write, which this key lacks.
    let (status, _) = h
        .call(
            "POST",
            "/api/v1/auth/api-keys",
            &narrow,
            Some(json!({ "name": "default" })),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // A subset is fine, and the new key has exactly what was asked for.
    let (status, body) = h
        .call(
            "POST",
            "/api/v1/auth/api-keys",
            &narrow,
            Some(json!({ "name": "subset", "scopes": ["domains:read"] })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["data"]["scopes"], json!(["domains:read"]));
    let minted = format!("ApiKey {}", body["data"]["key"].as_str().unwrap());
    let (status, _) = h.call("GET", "/api/v1/domains", &minted, None).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = h.call("GET", "/api/v1/upstreams", &minted, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Unknown scopes are refused as invalid, for every caller.
    let star = h.key(t, ALL).await;
    for key in [&narrow, &star] {
        let (status, body) = h
            .call(
                "POST",
                "/api/v1/auth/api-keys",
                key,
                Some(json!({ "name": "typo", "scopes": ["domain:read"] })),
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(body["code"], "validation_failed");
    }

    // Only `*` mints `*`.
    let (status, _) = h
        .call(
            "POST",
            "/api/v1/auth/api-keys",
            &star,
            Some(json!({ "name": "star", "scopes": ["*"] })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_narrow_key_cannot_revoke_a_broader_one() {
    let h = Harness::new().await;
    let t = h.tenant().await;
    let star = h.key(t, ALL).await;
    let narrow = h.key(t, &["api_keys:write"]).await;

    let (_, body) = h.call("GET", "/api/v1/auth/api-keys", &star, None).await;
    let keys = body["data"].as_array().unwrap();
    let id_of = |scope: &str| {
        keys.iter().find(|k| k["scopes"] == json!([scope])).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let (star_id, narrow_id) = (id_of("*"), id_of("api_keys:write"));

    let (status, _) = h
        .call(
            "DELETE",
            &format!("/api/v1/auth/api-keys/{star_id}"),
            &narrow,
            None,
        )
        .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "a narrow key revoked the * key"
    );

    let (status, _) = h
        .call(
            "DELETE",
            &format!("/api/v1/auth/api-keys/{narrow_id}"),
            &star,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
}

// ---------------------------------------------------------------------------
// 2. Every read checks its scope.
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn every_read_needs_its_scope() {
    let h = Harness::new().await;
    let t = h.tenant().await;
    let star = h.key(t, ALL).await;
    let (domain, _) = h.domain(&star, &name()).await;
    let upstream = h.upstream(&star).await;
    let anything = Uuid::new_v4();

    let reads = [
        ("/api/v1/domains".to_string(), "domains:read"),
        (format!("/api/v1/domains/{domain}"), "domains:read"),
        (format!("/api/v1/domains/{domain}/routes"), "routes:read"),
        (
            format!("/api/v1/domains/{domain}/routes/{anything}"),
            "routes:read",
        ),
        ("/api/v1/upstreams".to_string(), "upstreams:read"),
        (format!("/api/v1/upstreams/{upstream}"), "upstreams:read"),
        ("/api/v1/tenant/me".to_string(), "tenant:read"),
        ("/api/v1/tenant/usage".to_string(), "tenant:read"),
        ("/api/v1/auth/api-keys".to_string(), "api_keys:read"),
    ];

    let none = h.key(t, &[]).await;
    for (path, scope) in &reads {
        let (status, body) = h.call("GET", path, &none, None).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "GET {path} with no scopes: {body}"
        );
        assert!(
            body.get("data").is_none(),
            "GET {path} returned data with no scopes"
        );

        // The scope it needs is enough, or the 404 a missing row gets.
        let key = h.key(t, &[scope]).await;
        let (status, _) = h.call("GET", path, &key, None).await;
        assert!(
            status == StatusCode::OK || status == StatusCode::NOT_FOUND,
            "GET {path} with {scope}: {status}"
        );
    }

    // Reading one resource does not read the others.
    let domains_only = h.key(t, &["domains:read"]).await;
    for path in ["/api/v1/upstreams", "/api/v1/tenant/usage"] {
        let (status, _) = h.call("GET", path, &domains_only, None).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{path}");
    }
    let (status, _) = h
        .call(
            "GET",
            &format!("/api/v1/domains/{domain}/routes"),
            &domains_only,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

// ---------------------------------------------------------------------------
// 3. A domain is one name, whatever its case.
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_case_variant_of_an_owned_domain_is_refused() {
    let h = Harness::new().await;
    let owner = h.key(h.tenant().await, ALL).await;
    let squatter = h.key(h.tenant().await, ALL).await;

    let victim = name();
    let (status, body) = h
        .call(
            "POST",
            "/api/v1/domains",
            &owner,
            Some(json!({ "domain": victim, "verification_method": "http" })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let owner_token = body["data"]["verification_token"]
        .as_str()
        .unwrap()
        .to_string();

    for variant in [
        victim.to_uppercase(),
        format!("{victim}."),
        format!("{}.", victim.to_uppercase()),
    ] {
        let (status, body) = h
            .call(
                "POST",
                "/api/v1/domains",
                &squatter,
                Some(json!({ "domain": variant })),
            )
            .await;
        assert_eq!(
            status,
            StatusCode::CONFLICT,
            "{variant} was registered: {body}"
        );
        assert_eq!(body["code"], "conflict");
    }

    // The only challenge answerable on the victim's host is the owner's.
    let (status, body) = h
        .request(
            "GET",
            &format!("/.well-known/postrust-verification/{owner_token}"),
            &[("host", &victim.to_uppercase())],
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body,
        Value::String(format!("postrust-verify={owner_token}"))
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn domains_are_stored_normalised() {
    let h = Harness::new().await;
    let key = h.key(h.tenant().await, ALL).await;
    let base = name();

    let (status, body) = h
        .call(
            "POST",
            "/api/v1/domains",
            &key,
            Some(json!({ "domain": format!("{}.", base.to_uppercase()) })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["data"]["domain"], base);
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn names_that_are_not_public_domains_are_refused() {
    let h = Harness::new().await;
    let key = h.key(h.tenant().await, ALL).await;

    for (domain, hint) in [
        ("bücher.test", "punycode"),
        ("127.0.0.1", "IP"),
        ("[::1]", "IP"),
        ("::1", "IP"),
        ("intranet", "two labels"),
        ("a_b.test", "letters"),
        ("127.1", "numeric"),
        ("api.localhost", "reserved"),
        ("metadata.google.internal", "reserved"),
    ] {
        let (status, body) = h
            .call(
                "POST",
                "/api/v1/domains",
                &key,
                Some(json!({ "domain": domain })),
            )
            .await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "{domain} was accepted: {body}"
        );
        let error = body["error"].as_str().unwrap();
        assert!(error.contains(hint), "{domain}: {error}");
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_database_refuses_a_case_duplicate_written_directly() {
    let h = Harness::new().await;
    let t = h.tenant().await;
    let base = name();
    for domain in [base.clone(), base.to_uppercase()] {
        let inserted = sqlx::query(
            "INSERT INTO proxy_domains (tenant_id, domain, verification_token) VALUES ($1, $2, 't')",
        )
        .bind(t)
        .bind(&domain)
        .execute(&h.pool)
        .await;
        if domain == base {
            inserted.expect("the first insert");
        } else {
            let error = inserted.expect_err("a case duplicate was inserted");
            let db = error.as_database_error().expect("a database error");
            assert_eq!(db.kind(), sqlx::error::ErrorKind::UniqueViolation);
        }
    }
}

/// The migration's handling of rows that already collide, run in a schema of
/// its own so the index on the shared tables is not disturbed.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_migration_keeps_the_verified_row_of_a_case_collision() {
    use sqlx::Connection;
    let url = std::env::var("DATABASE_URL").unwrap();
    let mut conn = sqlx::PgConnection::connect(&url).await.unwrap();
    let schema = format!("cp_{}", Uuid::new_v4().simple());

    conn.execute(AssertSqlSafe(format!(
        "CREATE SCHEMA {schema}; SET search_path TO {schema}"
    )))
    .await
    .unwrap();
    // The first migration alters proxy_certificates if a table of that name
    // exists anywhere; give it one here, shaped as a pre-ACME install had it.
    conn.execute(
        "CREATE TABLE proxy_certificates (domain VARCHAR(253) PRIMARY KEY, cert_pem BYTEA NOT NULL, \
         key_pem BYTEA NOT NULL, expires_at TIMESTAMPTZ, \
         created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(), updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW())",
    )
    .await
    .unwrap();
    for migration in &MIGRATIONS[..3] {
        conn.execute(*migration).await.unwrap();
    }

    let tenant: Uuid = sqlx::query_scalar(
        "INSERT INTO proxy_tenants (name, slug, email) VALUES ('m', 'm', 'm@x') RETURNING id",
    )
    .fetch_one(&mut conn)
    .await
    .unwrap();
    let insert = |domain: &'static str, status: &'static str, age: i32| {
        sqlx::query_scalar::<_, Uuid>(
            "INSERT INTO proxy_domains (tenant_id, domain, verification_token, verification_status, \
             verified_at, created_at) \
             VALUES ($1, $2, 't', $3, CASE WHEN $3 = 'verified' THEN NOW() END, \
                     NOW() - make_interval(days => $4)) RETURNING id",
        )
        .bind(tenant)
        .bind(domain)
        .bind(status)
        .bind(age)
    };
    // Registered first, never verified.
    let first = insert("dup.test", "pending", 3)
        .fetch_one(&mut conn)
        .await
        .unwrap();
    // Registered later by someone else, but verified: this one proved control.
    let verified = insert("Dup.test", "verified", 2)
        .fetch_one(&mut conn)
        .await
        .unwrap();
    let third = insert("DUP.TEST", "pending", 1)
        .fetch_one(&mut conn)
        .await
        .unwrap();
    let unrelated = insert("other.test", "pending", 1)
        .fetch_one(&mut conn)
        .await
        .unwrap();

    conn.execute(MIGRATIONS[3]).await.unwrap();
    // And again: it has to be safe to re-run.
    conn.execute(MIGRATIONS[3]).await.unwrap();

    let left: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM proxy_domains ORDER BY domain")
        .fetch_all(&mut conn)
        .await
        .unwrap();
    assert_eq!(left, vec![verified, unrelated]);

    let removed: Vec<(Uuid, Uuid)> = sqlx::query_as(
        "SELECT removed_domain_id, kept_domain_id FROM proxy_domain_case_conflicts \
         ORDER BY removed_domain_id",
    )
    .fetch_all(&mut conn)
    .await
    .unwrap();
    let mut expected = vec![(first, verified), (third, verified)];
    expected.sort();
    assert_eq!(removed, expected);

    let duplicate = insert("dUP.test", "pending", 0).fetch_one(&mut conn).await;
    assert!(
        duplicate.is_err(),
        "the index does not refuse a case duplicate"
    );

    conn.execute(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
        .await
        .unwrap();
}

// ---------------------------------------------------------------------------
// 4. Certificates need a verified, unreserved domain and a trusted chain;
//    a pending registration does not hold a name for ever.
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn reserved_names_cannot_be_registered() {
    let platform = format!("{}.example", unique("platform"));
    let api = format!("api.{}.example", unique("other"));
    let (p, a) = (platform.clone(), api.clone());
    let h = Harness::with(move |o| {
        o.domains.reserved = ReservedDomains::default().with_suffix(&p).with_exact(&a);
    })
    .await;
    let key = h.key(h.tenant().await, ALL).await;

    for domain in [
        platform.clone(),
        format!("app.{platform}"),
        platform.to_uppercase(),
        api.clone(),
    ] {
        let (status, body) = h
            .call(
                "POST",
                "/api/v1/domains",
                &key,
                Some(json!({ "domain": domain })),
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{domain}: {body}");
        assert!(body["error"].as_str().unwrap().contains("reserved"));
    }

    // An exact reservation does not cover its subdomains.
    let (status, _) = h
        .call(
            "POST",
            "/api/v1/domains",
            &key,
            Some(json!({ "domain": format!("x.{api}") })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
}

const LEAF: &str = include_str!("testdata/leaf.pem");
const LEAF_KEY: &str = include_str!("testdata/leaf.key.pem");
const CA: &str = include_str!("testdata/ca.pem");
const SELF_SIGNED: &str = include_str!("testdata/selfsigned.pem");
const SELF_SIGNED_KEY: &str = include_str!("testdata/selfsigned.key.pem");
/// The name the fixtures cover. Shared, so exactly one test uses it.
const UPLOAD_DOMAIN: &str = "upload-chain.test";

fn private_roots() -> Arc<rustls::RootCertStore> {
    let mut roots = rustls::RootCertStore::empty();
    for cert in rustls_pemfile::certs(&mut CA.as_bytes()) {
        roots.add(cert.unwrap()).unwrap();
    }
    Arc::new(roots)
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn certificate_upload_needs_a_verified_domain_and_a_trusted_chain() {
    let h = Harness::with(|o| o.domains.upload_trust = UploadTrust::Roots(private_roots())).await;
    sqlx::query("DELETE FROM proxy_domains WHERE lower(domain) = $1")
        .bind(UPLOAD_DOMAIN)
        .execute(&h.pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM proxy_certificates WHERE domain = $1")
        .bind(UPLOAD_DOMAIN)
        .execute(&h.pool)
        .await
        .unwrap();

    let t = h.tenant().await;
    let key = h.key(t, ALL).await;
    let (id, _) = h.domain(&key, UPLOAD_DOMAIN).await;
    let upload = |cert: &str, key_pem: &str| json!({ "cert_pem": cert, "key_pem": key_pem });
    let path = format!("/api/v1/domains/{id}/ssl/upload");
    let chain = format!("{LEAF}{CA}");

    // Unverified: refused, however good the certificate.
    let (status, body) = h
        .call("POST", &path, &key, Some(upload(&chain, LEAF_KEY)))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body["error"].as_str().unwrap().contains("verified"));
    let stored: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM proxy_certificates WHERE domain = $1")
            .bind(UPLOAD_DOMAIN)
            .fetch_one(&h.pool)
            .await
            .unwrap();
    assert_eq!(
        stored, 0,
        "an unverified domain's certificate reached the store"
    );

    // Nor may it carry routes.
    let upstream = h.upstream(&key).await;
    let (status, _) = h
        .call(
            "POST",
            &format!("/api/v1/domains/{id}/routes"),
            &key,
            Some(json!({ "name": "r", "upstream_id": upstream })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    h.verify(id).await;

    // Verified, but self-signed: trusted by nobody.
    let (status, body) = h
        .call(
            "POST",
            &path,
            &key,
            Some(upload(SELF_SIGNED, SELF_SIGNED_KEY)),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body["error"].as_str().unwrap().contains("not trusted"));

    // Under the public roots, the private CA's leaf is refused too.
    let public = Harness::on(h.pool.clone(), SaasOptions::default()).await;
    let (status, _) = public
        .call("POST", &path, &key, Some(upload(&chain, LEAF_KEY)))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // With the private root trusted, the chain is accepted, stored, and
    // linked to its domain.
    let (status, body) = h
        .call("POST", &path, &key, Some(upload(&chain, LEAF_KEY)))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let linked: Option<Uuid> =
        sqlx::query_scalar("SELECT domain_id FROM proxy_certificates WHERE domain = $1")
            .bind(UPLOAD_DOMAIN)
            .fetch_one(&h.pool)
            .await
            .unwrap();
    assert_eq!(linked, Some(id));
    let key_file = h.cert_dir.join(format!("{UPLOAD_DOMAIN}.key"));
    assert!(key_file.exists(), "the store did not cache the key");

    // Deleting the domain takes the certificate and its cached key with it.
    let (status, _) = h
        .call("DELETE", &format!("/api/v1/domains/{id}"), &key, None)
        .await;
    assert_eq!(status, StatusCode::OK);
    let stored: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM proxy_certificates WHERE domain = $1")
            .bind(UPLOAD_DOMAIN)
            .fetch_one(&h.pool)
            .await
            .unwrap();
    assert_eq!(stored, 0, "the certificate outlived its domain");
    assert!(
        !key_file.exists(),
        "the private key outlived its domain on disk"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_stale_pending_registration_stops_holding_its_name() {
    let h = Harness::new().await;
    let squatter = h.key(h.tenant().await, ALL).await;
    let owner = h.key(h.tenant().await, ALL).await;
    let contested = name();

    let (squat, _) = h.domain(&squatter, &contested).await;

    // While it is fresh, the name is taken.
    let (status, _) = h
        .call(
            "POST",
            "/api/v1/domains",
            &owner,
            Some(json!({ "domain": contested })),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);

    // Eight days on, unverified, it no longer is.
    sqlx::query("UPDATE proxy_domains SET created_at = NOW() - INTERVAL '8 days' WHERE id = $1")
        .bind(squat)
        .execute(&h.pool)
        .await
        .unwrap();
    let (claimed, _) = h.domain(&owner, &contested).await;
    assert_ne!(claimed, squat);

    let gone: bool =
        sqlx::query_scalar("SELECT NOT EXISTS(SELECT 1 FROM proxy_domains WHERE id = $1)")
            .bind(squat)
            .fetch_one(&h.pool)
            .await
            .unwrap();
    assert!(gone, "the stale registration was left in place");
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_verified_registration_does_not_go_stale() {
    let h = Harness::new().await;
    let owner = h.key(h.tenant().await, ALL).await;
    let other = h.key(h.tenant().await, ALL).await;
    let held = name();
    let (id, _) = h.domain(&owner, &held).await;
    h.verify(id).await;
    sqlx::query("UPDATE proxy_domains SET created_at = NOW() - INTERVAL '90 days' WHERE id = $1")
        .bind(id)
        .execute(&h.pool)
        .await
        .unwrap();

    let (status, _) = h
        .call(
            "POST",
            "/api/v1/domains",
            &other,
            Some(json!({ "domain": held })),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
}

// ---------------------------------------------------------------------------
// 5. Verification is throttled and honours expiry. (The SSRF defences of the
//    HTTP check itself are unit-tested in saas::verification.)
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn verification_is_throttled_and_expires() {
    let h = Harness::new().await;
    let key = h.key(h.tenant().await, ALL).await;
    let (id, _) = h.domain(&key, &name()).await;
    let verify = format!("/api/v1/domains/{id}/verify");
    let set = |sql: &'static str| {
        let pool = h.pool.clone();
        async move {
            sqlx::query(sql).bind(id).execute(&pool).await.unwrap();
        }
    };

    // Just attempted: too soon. No lookup is made, so no network is needed.
    set("UPDATE proxy_domains SET last_verification_attempt = NOW() WHERE id = $1").await;
    let (status, body) = h.call("POST", &verify, &key, None).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
    assert_eq!(body["code"], "rate_limited");

    // Out of attempts.
    set(
        "UPDATE proxy_domains SET last_verification_attempt = NOW() - INTERVAL '1 hour', \
         verification_attempts = 50 WHERE id = $1",
    )
    .await;
    let (status, body) = h.call("POST", &verify, &key, None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body["data"]["failed"]["reason"]
            .as_str()
            .unwrap()
            .contains("all its verification attempts"),
        "{body}"
    );

    // The challenge has expired: refused, and the domain marked so.
    set("UPDATE proxy_domains SET verification_attempts = 0 WHERE id = $1").await;
    set("UPDATE proxy_verification_challenges SET expires_at = NOW() - INTERVAL '1 second' WHERE domain_id = $1")
        .await;
    let (status, body) = h.call("POST", &verify, &key, None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body["data"]["failed"]["reason"]
            .as_str()
            .unwrap()
            .contains("expired"),
        "{body}"
    );
    let status: String =
        sqlx::query_scalar("SELECT verification_status FROM proxy_domains WHERE id = $1")
            .bind(id)
            .fetch_one(&h.pool)
            .await
            .unwrap();
    assert_eq!(status, "expired");

    // An expired challenge is no longer served either.
    let token: String =
        sqlx::query_scalar("SELECT verification_token FROM proxy_domains WHERE id = $1")
            .bind(id)
            .fetch_one(&h.pool)
            .await
            .unwrap();
    let host: String = sqlx::query_scalar("SELECT domain FROM proxy_domains WHERE id = $1")
        .bind(id)
        .fetch_one(&h.pool)
        .await
        .unwrap();
    let (status, _) = h
        .request(
            "GET",
            &format!("/.well-known/postrust-verification/{token}"),
            &[("host", &host)],
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_verified_domain_is_not_checked_again() {
    let h = Harness::new().await;
    let key = h.key(h.tenant().await, ALL).await;
    let (id, _) = h.domain(&key, &name()).await;
    h.verify(id).await;
    let before: i32 =
        sqlx::query_scalar("SELECT verification_attempts FROM proxy_domains WHERE id = $1")
            .bind(id)
            .fetch_one(&h.pool)
            .await
            .unwrap();

    let (status, body) = h
        .call("POST", &format!("/api/v1/domains/{id}/verify"), &key, None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"], "verified");

    let after: i32 =
        sqlx::query_scalar("SELECT verification_attempts FROM proxy_domains WHERE id = $1")
            .bind(id)
            .fetch_one(&h.pool)
            .await
            .unwrap();
    assert_eq!(before, after, "a verified domain made an outbound check");
}

// ---------------------------------------------------------------------------
// 6. Database errors are not returned to tenants.
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn constraint_violations_are_answered_without_the_database_text() {
    let h = Harness::new().await;
    let key = h.key(h.tenant().await, ALL).await;
    let upstream = unique("dup");
    let create = json!({ "name": upstream });

    let (status, _) = h
        .call("POST", "/api/v1/upstreams", &key, Some(create.clone()))
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, body) = h
        .call("POST", "/api/v1/upstreams", &key, Some(create))
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "conflict");
    let text = body.to_string();
    for leak in [
        "duplicate key",
        "proxy_domain_upstreams",
        "constraint",
        &upstream,
    ] {
        assert!(!text.contains(leak), "the response leaked {leak:?}: {text}");
    }
}

// ---------------------------------------------------------------------------
// 7. Upstreams with backends: the trigger works, and creation is atomic.
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn an_upstream_with_backends_is_created_whole() {
    let h = Harness::new().await;
    let t = h.tenant().await;
    let key = h.key(t, ALL).await;

    let (status, body) = h
        .call(
            "POST",
            "/api/v1/upstreams",
            &key,
            Some(json!({
                "name": unique("up"),
                "backends": [
                    { "address": "api.example.com:443", "scheme": "https" },
                    { "address": "8.8.8.8:80" },
                ],
            })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["data"]["backends"].as_array().unwrap().len(), 2);

    // Adding and removing one fires the same trigger.
    let id = body["data"]["id"].as_str().unwrap();
    let (status, added) = h
        .call(
            "POST",
            &format!("/api/v1/upstreams/{id}/backends"),
            &key,
            Some(json!({ "address": "backend.example.com:8080" })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{added}");
    let backend = added["data"]["id"].as_str().unwrap();
    let (status, _) = h
        .call(
            "DELETE",
            &format!("/api/v1/upstreams/{id}/backends/{backend}"),
            &key,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    // One bad backend refuses the lot: no upstream is left behind.
    let name = unique("half");
    let (status, _) = h
        .call(
            "POST",
            "/api/v1/upstreams",
            &key,
            Some(json!({
                "name": name,
                "backends": [
                    { "address": "api.example.com:443" },
                    { "address": "api.example.com:443", "scheme": "gopher" },
                ],
            })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let left: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM proxy_domain_upstreams WHERE tenant_id = $1 AND name = $2",
    )
    .bind(t)
    .bind(&name)
    .fetch_one(&h.pool)
    .await
    .unwrap();
    assert_eq!(left, 0, "a refused upstream was half-created");
}

// ---------------------------------------------------------------------------
// 8. Tenant backends, headers and paths are validated.
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn backends_cannot_point_inside_the_network() {
    let h = Harness::new().await;
    let key = h.key(h.tenant().await, ALL).await;
    let upstream = h.upstream(&key).await;

    for address in [
        "169.254.169.254:80",
        "10.0.0.5:5432",
        "127.0.0.1:8080",
        "[::1]:80",
        "[::ffff:169.254.169.254]:80",
        "localhost:6379",
        "redis.internal:6379",
        "backend:80",
    ] {
        let (status, body) = h
            .call(
                "POST",
                &format!("/api/v1/upstreams/{upstream}/backends"),
                &key,
                Some(json!({ "address": address })),
            )
            .await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "{address} was accepted: {body}"
        );
    }

    let (status, body) = h
        .call(
            "POST",
            &format!("/api/v1/upstreams/{upstream}/backends"),
            &key,
            Some(json!({ "address": "Origin.Example.com:443", "scheme": "ftp" })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body["error"].as_str().unwrap().contains("scheme"));

    let (status, body) = h
        .call(
            "POST",
            &format!("/api/v1/upstreams/{upstream}/backends"),
            &key,
            Some(json!({ "address": "Origin.Example.com:443", "scheme": "https" })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["data"]["address"], "origin.example.com:443");
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn headers_and_paths_are_validated() {
    let h = Harness::new().await;
    let key = h.key(h.tenant().await, ALL).await;

    for health_check_path in ["health", "/a b", "/a\r\nHost: evil"] {
        let (status, body) = h
            .call(
                "POST",
                "/api/v1/upstreams",
                &key,
                Some(json!({ "name": unique("hc"), "health_check_path": health_check_path })),
            )
            .await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "{health_check_path:?}: {body}"
        );
    }

    let upstream = h.upstream(&key).await;
    let (status, body) = h
        .call(
            "PUT",
            &format!("/api/v1/upstreams/{upstream}"),
            &key,
            Some(json!({ "health_check_path": "/ok\nX: y" })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    let (domain, _) = h.domain(&key, &name()).await;
    h.verify(domain).await;
    let routes = format!("/api/v1/domains/{domain}/routes");
    for bad in [
        json!({ "add_headers": { "X-Ok": "a\r\nInjected: 1" } }),
        json!({ "add_headers": { "Bad Name": "v" } }),
        json!({ "add_headers": { "Host": "other.example" } }),
        json!({ "remove_headers": ["x\r\ny"] }),
        json!({ "path_pattern": "no-slash" }),
        json!({ "path_pattern": "(", "path_type": "regex" }),
    ] {
        let mut request = json!({ "name": unique("r"), "upstream_id": upstream });
        request
            .as_object_mut()
            .unwrap()
            .extend(bad.as_object().unwrap().clone());
        let (status, body) = h.call("POST", &routes, &key, Some(request)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}: {body}");
    }

    let (status, body) = h
        .call(
            "POST",
            &routes,
            &key,
            Some(json!({
                "name": unique("r"),
                "upstream_id": upstream,
                "add_headers": { "X-Tenant": "acme" },
                "remove_headers": ["x-powered-by"],
            })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
}

// ---------------------------------------------------------------------------
// 9. Proxy JWTs need exp and the proxy's audience; no scopes means none.
// ---------------------------------------------------------------------------

fn jwt(tenant: Uuid, extra: Value) -> String {
    let mut claims = json!({
        "sub": "user",
        "tenant_id": tenant,
        "exp": chrono::Utc::now().timestamp() + 600,
        "aud": "postrust-proxy",
    });
    for (k, v) in extra.as_object().unwrap() {
        if v.is_null() {
            claims.as_object_mut().unwrap().remove(k);
        } else {
            claims[k] = v.clone();
        }
    }
    let token = jsonwebtoken::encode(
        &jsonwebtoken::Header::default(),
        &claims,
        &jsonwebtoken::EncodingKey::from_secret(JWT_SECRET.as_bytes()),
    )
    .unwrap();
    format!("Bearer {token}")
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn jwts_are_held_to_exp_audience_and_their_scopes() {
    let h = Harness::new().await;
    let t = h.tenant().await;

    let (status, body) = h
        .call(
            "GET",
            "/api/v1/domains",
            &jwt(t, json!({ "exp": null, "scopes": ["*"] })),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "no exp: {body}");

    for aud in [Value::Null, json!("postrust"), json!("authenticated")] {
        let (status, _) = h
            .call(
                "GET",
                "/api/v1/domains",
                &jwt(t, json!({ "aud": aud, "scopes": ["*"] })),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "aud {aud}");
    }

    let (status, _) = h
        .call("GET", "/api/v1/domains", &jwt(t, json!({})), None)
        .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "a token without scopes could read"
    );

    let (status, _) = h
        .call(
            "GET",
            "/api/v1/domains",
            &jwt(t, json!({ "scopes": ["domains:read"] })),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    // A configured audience replaces the default.
    let custom = Harness::with(|o| o.jwt_audience = "tenant-api".into()).await;
    let (status, _) = custom
        .call(
            "GET",
            "/api/v1/domains",
            &jwt(t, json!({ "scopes": ["*"] })),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = custom
        .call(
            "GET",
            "/api/v1/domains",
            &jwt(t, json!({ "scopes": ["*"], "aud": "tenant-api" })),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
}

// ---------------------------------------------------------------------------
// 10. Quotas.
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn quotas_are_enforced() {
    let h = Harness::new().await;
    let t = h.tenant().await;
    let key = h.key(t, ALL).await;
    sqlx::query(
        "UPDATE proxy_tenants SET max_routes_per_domain = 1, max_upstreams = 2, \
         max_backends_per_upstream = 1, max_api_keys = 1 WHERE id = $1",
    )
    .bind(t)
    .execute(&h.pool)
    .await
    .unwrap();

    let upstream = h.upstream(&key).await;
    let (domain, _) = h.domain(&key, &name()).await;
    h.verify(domain).await;
    let routes = format!("/api/v1/domains/{domain}/routes");
    let route =
        |path: &str| json!({ "name": unique("r"), "upstream_id": upstream, "path_pattern": path });

    let (status, body) = h.call("POST", &routes, &key, Some(route("/a"))).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let (status, body) = h.call("POST", &routes, &key, Some(route("/b"))).await;
    assert_eq!(status, StatusCode::PAYMENT_REQUIRED, "{body}");
    assert_eq!(body["code"], "quota_exceeded");

    // Two backends inline against a limit of one.
    let (status, _) = h
        .call(
            "POST",
            "/api/v1/upstreams",
            &key,
            Some(json!({
                "name": unique("up"),
                "backends": [{ "address": "a.example.com:80" }, { "address": "b.example.com:80" }],
            })),
        )
        .await;
    assert_eq!(status, StatusCode::PAYMENT_REQUIRED);

    // One backend, then a second added.
    let backends = format!("/api/v1/upstreams/{upstream}/backends");
    let (status, _) = h
        .call(
            "POST",
            &backends,
            &key,
            Some(json!({ "address": "a.example.com:80" })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, _) = h
        .call(
            "POST",
            &backends,
            &key,
            Some(json!({ "address": "b.example.com:80" })),
        )
        .await;
    assert_eq!(status, StatusCode::PAYMENT_REQUIRED);

    // The second upstream fits, the third does not.
    h.upstream(&key).await;
    let (status, _) = h
        .call(
            "POST",
            "/api/v1/upstreams",
            &key,
            Some(json!({ "name": unique("up") })),
        )
        .await;
    assert_eq!(status, StatusCode::PAYMENT_REQUIRED);

    // The key in use already fills the key quota.
    let (status, _) = h
        .call(
            "POST",
            "/api/v1/auth/api-keys",
            &key,
            Some(json!({ "name": "more", "scopes": ["domains:read"] })),
        )
        .await;
    assert_eq!(status, StatusCode::PAYMENT_REQUIRED);

    let (status, usage) = h.call("GET", "/api/v1/tenant/usage", &key, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(usage["data"]["upstreams_limit"], 2);
    assert_eq!(usage["data"]["api_keys_limit"], 1);
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn another_tenants_domain_cannot_be_enabled_or_disabled() {
    let h = Harness::new().await;
    let owner = h.key(h.tenant().await, ALL).await;
    let stranger = h.key(h.tenant().await, ALL).await;
    let (id, _) = h.domain(&owner, &name()).await;
    h.verify(id).await;

    for action in ["enable", "disable"] {
        let (status, _) = h
            .call(
                "POST",
                &format!("/api/v1/domains/{id}/{action}"),
                &stranger,
                None,
            )
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{action}");
    }
    let (status, _) = h
        .call(
            "POST",
            &format!("/api/v1/domains/{id}/enable"),
            &owner,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
}
