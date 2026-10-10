//! Serving through a `NOINHERIT` authenticator.
//!
//! The setup PostgREST recommends: the server logs in as a role with no
//! privileges of its own, granted the roles a token may name, which it becomes
//! with `SET ROLE` for each request. `NOINHERIT` means it holds none of their
//! privileges until it does.
//!
//! The schema cache used to read `information_schema`, whose views show a
//! connection only what it can use *without* switching role. Through such an
//! authenticator that is nothing: every table was "not found", in the server
//! as on Lambda, while a superuser -- which is how every other test here
//! connects -- saw everything.
//!
//! The database must have `scripts/init-db.sql` and `scripts/test-fixtures.sql`
//! loaded, and `DATABASE_URL` must name a role that can create roles. The
//! authenticators these tests create are left in place for the next run:
//!
//! ```text
//! cargo test --package postrust-server --test noinherit_authenticator -- --ignored
//! ```

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::any;
use axum::Router;
use postrust_core::{AppConfig, SchemaCache};
use postrust_server::{handle_request, AppState};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::PgPool;
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::RwLock;
use tower::ServiceExt;

const NOINHERIT: &str = "postrust_test_noinherit_authenticator";
const INHERIT: &str = "postrust_test_inherit_authenticator";
const PASSWORD: &str = "postrust_test_authenticator";
const SCHEMAS: [&str; 2] = ["public", "api"];

fn database_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/postrust_test".to_string())
}

fn schemas() -> Vec<String> {
    SCHEMAS.iter().map(|s| s.to_string()).collect()
}

/// Two authenticators granted every role the fixtures grant privileges to,
/// one `NOINHERIT` and one inheriting -- the setup that always worked, and so
/// the measure of what the other should see.
///
/// Returns pools connected as `(inheriting, NOINHERIT)`.
async fn authenticators() -> (PgPool, PgPool) {
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url())
        .await
        .expect("failed to connect to test database");

    // The tests in this file run concurrently, and two `CREATE ROLE`s racing
    // for one name fail with a unique violation rather than the
    // `duplicate_object` an existence check would catch. Taken in turn, and
    // made what this file needs whatever an earlier run left behind.
    for (role, inherit) in [(NOINHERIT, "NOINHERIT"), (INHERIT, "INHERIT")] {
        sqlx::query(&format!(
            "DO $$ BEGIN \
               PERFORM pg_advisory_xact_lock(hashtext('{role}')); \
               IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = '{role}') THEN \
                 CREATE ROLE {role}; \
               END IF; \
               ALTER ROLE {role} LOGIN {inherit} PASSWORD '{PASSWORD}'; \
               GRANT web_anon, web_user, web_admin TO {role}; \
             END $$"
        ))
        .execute(&admin)
        .await
        .expect("failed to create an authenticator");
    }

    let connect = |role: &'static str| async move {
        let options = PgConnectOptions::from_str(&database_url())
            .expect("DATABASE_URL is not a valid connection string")
            .username(role)
            .password(PASSWORD);
        PgPoolOptions::new()
            .max_connections(2)
            .connect_with(options)
            .await
            .expect("failed to connect as an authenticator")
    };
    let (inheriting, noinherit) = (connect(INHERIT).await, connect(NOINHERIT).await);

    // The premise, checked rather than assumed: the NOINHERIT authenticator
    // cannot read a fixture table as itself. If it could, these tests would
    // pass whether or not the schema cache knew how to look past that.
    let alone: Result<(i64,), _> = sqlx::query_as("SELECT count(*) FROM public.users")
        .fetch_one(&noinherit)
        .await;
    assert!(
        alone.is_err(),
        "the authenticator can read public.users without switching role, \
         so it is not a NOINHERIT authenticator and this test proves nothing"
    );

    (inheriting, noinherit)
}

/// Every table, its columns, its key and what may be written to it, sorted.
fn describe(cache: &SchemaCache) -> Vec<String> {
    let mut tables: Vec<String> = cache
        .tables
        .values()
        .map(|t| {
            let columns: Vec<String> = t
                .columns
                .values()
                .map(|c| format!("{}:{}:{}", c.name, c.data_type, c.nullable))
                .collect();
            format!(
                "{}.{} view={} insert={} update={} delete={} pk={:?} columns={:?}",
                t.schema,
                t.name,
                t.is_view,
                t.insertable,
                t.updatable,
                t.deletable,
                t.pk_cols,
                columns
            )
        })
        .collect();
    tables.sort();
    tables
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_noinherit_authenticator_sees_the_schema_an_inheriting_one_does() {
    let (inheriting, noinherit) = authenticators().await;

    let expected = describe(
        &SchemaCache::load(&inheriting, &schemas())
            .await
            .expect("schema cache as the inheriting authenticator"),
    );
    assert!(
        expected.iter().any(|t| t.starts_with("public.users ")),
        "the fixtures are not loaded: {expected:#?}"
    );
    // Columns, keys and what may be written included: the grants that decide
    // whether a table is insertable are the roles' either way.
    let actual = describe(
        &SchemaCache::load(&noinherit, &schemas())
            .await
            .expect("schema cache as the NOINHERIT authenticator"),
    );
    assert_eq!(actual, expected);
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_request_is_answered_through_the_authenticator() {
    let (_, authenticator) = authenticators().await;

    let mut config = AppConfig::default();
    config.db_schemas = schemas();
    config.db_anon_role = Some("web_anon".to_string());

    let schema_cache = SchemaCache::load(&authenticator, &config.db_schemas)
        .await
        .expect("schema cache as the authenticator");
    let jwt_config = postrust_auth::JwtConfig {
        secret: config.jwt_secret.clone(),
        secret_is_base64: config.jwt_secret_is_base64,
        audience: config.jwt_aud.clone(),
        role_claim_key: config.jwt_role_claim_key.clone(),
        anon_role: config.db_anon_role.clone(),
    };
    let state = Arc::new(AppState {
        pool: authenticator,
        schema_cache: RwLock::new(schema_cache),
        config,
        jwt_config,
        jwt_cache: None,
    });
    // Mounted as the server mounts it: the handler reads the path below `/api`.
    let api: Router<Arc<AppState>> = Router::new()
        .route("/", any(handle_request))
        .route("/{*path}", any(handle_request));
    let app = Router::new().nest("/api", api).with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/users?select=id&limit=1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let rows: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(rows.as_array().map(Vec::len), Some(1), "{rows}");
}
