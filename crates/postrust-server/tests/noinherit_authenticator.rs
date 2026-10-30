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
//! Comparing two authenticators only says they agree. The fixture schemas made
//! here pin what they should agree *on*: a table granted column by column, a
//! view that can be written through but was granted only SELECT, a table
//! granted to nobody, two enums of one name, an enum table, and a data
//! representation whose cast function only the anonymous role may call.
//!
//! The database must have `scripts/init-db.sql` and `scripts/test-fixtures.sql`
//! loaded, and `DATABASE_URL` must name a superuser. The roles and schemas
//! these tests create are left in place for the next run:
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
use sqlx::AssertSqlSafe;
use sqlx::PgPool;
use std::str::FromStr;
use std::sync::Arc;
use tower::ServiceExt;

const NOINHERIT: &str = "postrust_test_noinherit_authenticator";
const INHERIT: &str = "postrust_test_inherit_authenticator";
/// Granted `pg_read_all_data` and nothing else.
const READER: &str = "postrust_test_reader_authenticator";
const PASSWORD: &str = "postrust_test_authenticator";
const SCHEMAS: [&str; 4] = ["public", "api", "noinherit_a", "noinherit_b"];

/// Created once, under a lock, and recreated when the marker comment on
/// `noinherit_a` names an older version.
const FIXTURE: &str = r#"
DO $setup$
BEGIN
  PERFORM pg_advisory_xact_lock(hashtext('postrust_noinherit_fixture'));
  IF obj_description(to_regnamespace('noinherit_a'), 'pg_namespace')
     IS NOT DISTINCT FROM 'postrust noinherit fixture v1' THEN
    RETURN;
  END IF;
  DROP SCHEMA IF EXISTS noinherit_a CASCADE;
  DROP SCHEMA IF EXISTS noinherit_b CASCADE;
  CREATE SCHEMA noinherit_a;
  CREATE SCHEMA noinherit_b;
  GRANT USAGE ON SCHEMA noinherit_a, noinherit_b TO web_anon;

  CREATE TABLE noinherit_a.column_only (id int PRIMARY KEY, shown text, hidden text);
  GRANT SELECT (id, shown) ON noinherit_a.column_only TO web_anon;

  CREATE VIEW noinherit_a.read_only_view AS SELECT id, shown FROM noinherit_a.column_only;
  GRANT SELECT ON noinherit_a.read_only_view TO web_anon;

  CREATE TABLE noinherit_a.ungranted (id int);

  CREATE TYPE noinherit_a.mood AS ENUM ('calm', 'grumpy');
  CREATE TYPE noinherit_b.mood AS ENUM ('sunny', 'stormy', 'foggy');
  CREATE TABLE noinherit_a.moods (id int PRIMARY KEY, mood noinherit_a.mood);
  CREATE TABLE noinherit_b.moods (id int PRIMARY KEY, mood noinherit_b.mood);
  GRANT SELECT ON noinherit_a.moods, noinherit_b.moods TO web_anon;

  CREATE TABLE noinherit_a.status (value text PRIMARY KEY, comment text);
  INSERT INTO noinherit_a.status VALUES ('open', 'Not done'), ('closed', NULL);
  GRANT SELECT ON noinherit_a.status TO web_anon;

  CREATE DOMAIN noinherit_a.color AS integer;
  CREATE FUNCTION noinherit_a.color_json(noinherit_a.color) RETURNS json
    LANGUAGE sql IMMUTABLE AS $f$ SELECT to_json(lpad(to_hex($1), 6, '0')) $f$;
  CREATE CAST (noinherit_a.color AS json)
    WITH FUNCTION noinherit_a.color_json(noinherit_a.color) AS IMPLICIT;
  REVOKE EXECUTE ON FUNCTION noinherit_a.color_json(noinherit_a.color) FROM PUBLIC;
  GRANT EXECUTE ON FUNCTION noinherit_a.color_json(noinherit_a.color) TO web_anon;

  COMMENT ON SCHEMA noinherit_a IS 'postrust noinherit fixture v1';
END
$setup$
"#;

fn database_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/postrust_test".to_string())
}

fn schemas() -> Vec<String> {
    SCHEMAS.iter().map(|s| s.to_string()).collect()
}

async fn admin() -> PgPool {
    PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url())
        .await
        .expect("failed to connect to test database")
}

/// `DATABASE_URL`, logging in as `role`.
fn url_as(role: &str) -> String {
    let url = database_url();
    let (scheme, rest) = url.split_once("://").expect("DATABASE_URL is a URL");
    let host = rest.split_once('@').map_or(rest, |(_, host)| host);
    format!("{scheme}://{role}:{PASSWORD}@{host}")
}

/// A pool logged in as `role`.
async fn connect_as(role: &str) -> PgPool {
    let options = PgConnectOptions::from_str(&database_url())
        .expect("DATABASE_URL is not a valid connection string")
        .username(role)
        .password(PASSWORD);
    PgPoolOptions::new()
        .max_connections(2)
        .connect_with(options)
        .await
        .expect("failed to connect as an authenticator")
}

/// Two authenticators granted every role the fixtures grant privileges to,
/// one `NOINHERIT` and one inheriting -- the setup that always worked, and so
/// the measure of what the other should see.
///
/// Returns pools connected as `(inheriting, NOINHERIT)`.
async fn authenticators() -> (PgPool, PgPool) {
    let admin = admin().await;
    sqlx::query(FIXTURE)
        .execute(&admin)
        .await
        .expect("failed to create the fixture schemas");

    // The tests in this file run concurrently, and two `CREATE ROLE`s racing
    // for one name fail with a unique violation rather than the
    // `duplicate_object` an existence check would catch. Taken in turn, and
    // made what this file needs whatever an earlier run left behind.
    for (role, inherit) in [(NOINHERIT, "NOINHERIT"), (INHERIT, "INHERIT")] {
        sqlx::query(AssertSqlSafe(format!(
            "DO $$ BEGIN \
               PERFORM pg_advisory_xact_lock(hashtext('{role}')); \
               IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = '{role}') THEN \
                 CREATE ROLE {role}; \
               END IF; \
               ALTER ROLE {role} LOGIN {inherit} PASSWORD '{PASSWORD}'; \
               GRANT web_anon, web_user, web_admin TO {role}; \
             END $$"
        )))
        .execute(&admin)
        .await
        .expect("failed to create an authenticator");
    }

    let (inheriting, noinherit) = (connect_as(INHERIT).await, connect_as(NOINHERIT).await);

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
    // The login the pool has, for a build that serves requests through a
    // connection of its own (`pg-socket`). Left at its default, that build
    // connected to `postgresql://localhost/postgres` -- no such login -- and
    // answered 500.
    config.db_uri = url_as(NOINHERIT);

    let schema_cache = SchemaCache::load(&authenticator, &config.db_schemas)
        .await
        .expect("schema cache as the authenticator");
    let state = Arc::new({
        let mut state = AppState::new(authenticator, schema_cache, config)
            .expect("the test's JWT secret parses");
        // Uncached, so a test sees each request validated as it was written.
        state.jwt_cache = None;
        state
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

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_schema_cache_holds_what_the_grants_say() {
    let (_, noinherit) = authenticators().await;
    let cache = SchemaCache::load(&noinherit, &schemas())
        .await
        .expect("schema cache as the NOINHERIT authenticator");
    let table = |schema: &str, name: &str| {
        cache
            .tables
            .values()
            .find(|t| t.schema == schema && t.name == name)
    };

    // Granted column by column: visible, with only the granted columns, and
    // nothing may be written.
    let column_only = table("noinherit_a", "column_only").expect("column_only is visible");
    let names: Vec<&str> = column_only.columns.keys().map(String::as_str).collect();
    assert_eq!(names, ["id", "shown"]);
    assert!(!column_only.insertable && !column_only.updatable && !column_only.deletable);

    // PostgreSQL could write through this view, but nobody was granted to.
    let view = table("noinherit_a", "read_only_view").expect("read_only_view is visible");
    assert!(view.is_view);
    assert!(!view.insertable && !view.updatable && !view.deletable);

    // Granted to nobody: not part of the API.
    assert!(table("noinherit_a", "ungranted").is_none());

    // Two enums of one name, each with its own labels.
    let labels = |schema: &str| {
        table(schema, "moods")
            .and_then(|t| t.columns.get("mood"))
            .map(|c| c.enum_values.clone())
            .unwrap_or_default()
    };
    assert_eq!(labels("noinherit_a"), ["calm", "grumpy"]);
    assert_eq!(labels("noinherit_b"), ["sunny", "stormy", "foggy"]);

    // A cast only the anonymous role may call is still a representation.
    assert!(
        cache
            .representations
            .keys()
            .any(|(source, target)| source == "noinherit_a.color" && target == "json"),
        "{:?}",
        cache.representations.keys().collect::<Vec<_>>()
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn an_enum_table_is_read_as_a_role_that_may_read_it() {
    let (_, noinherit) = authenticators().await;
    let values = postrust_core::schema_cache::enum_values(
        &noinherit,
        "noinherit_a",
        "status",
        "value",
        true,
    )
    .await
    .expect("enum values through the NOINHERIT authenticator");
    assert_eq!(
        values,
        [
            ("closed".to_string(), None),
            ("open".to_string(), Some("Not done".to_string()))
        ]
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_member_of_pg_read_all_data_sees_every_table() {
    let admin = admin().await;
    sqlx::query(FIXTURE)
        .execute(&admin)
        .await
        .expect("failed to create the fixture schemas");
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT FROM pg_roles WHERE rolname = 'pg_read_all_data')",
    )
    .fetch_one(&admin)
    .await
    .unwrap();
    if !exists {
        eprintln!("pg_read_all_data needs PostgreSQL 14 or later; skipped");
        return;
    }
    sqlx::query(AssertSqlSafe(format!(
        "DO $$ BEGIN \
           PERFORM pg_advisory_xact_lock(hashtext('{READER}')); \
           IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = '{READER}') THEN \
             CREATE ROLE {READER}; \
           END IF; \
           ALTER ROLE {READER} LOGIN INHERIT PASSWORD '{PASSWORD}'; \
           GRANT pg_read_all_data TO {READER}; \
         END $$"
    )))
    .execute(&admin)
    .await
    .expect("failed to create the reader");

    // `has_table_privilege` counts the predefined role, and
    // `information_schema` with it; a scan of the ACLs alone would not.
    let cache = SchemaCache::load(&connect_as(READER).await, &schemas())
        .await
        .expect("schema cache as the reader");
    assert!(cache
        .tables
        .values()
        .any(|t| t.schema == "noinherit_a" && t.name == "ungranted"));
}
