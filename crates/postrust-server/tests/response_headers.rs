//! Response headers and content negotiation, as PostgREST answers them.
//!
//! What a write counts, what a 416 carries, where an offset may start, whose
//! `Content-Type` wins when a function sets one, which of a list of accepted
//! types is served, and which preference is claimed as applied. The last
//! tests are compatibility-mode only: a 405 and a preflight without `Allow`,
//! a plain `vnd.pgrst.array+json` reported as JSON, and a root that refuses
//! what it cannot produce.
//!
//! `DATABASE_URL` must name a superuser on a database with
//! `scripts/init-db.sql` loaded (for `web_anon`). Run with:
//!
//! ```text
//! cargo test --package postrust-server --test response_headers -- --ignored
//! ```

use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode};
use axum::Router;
use postrust_core::AppConfig;
use postrust_server::{Builder, Host};
use sqlx::postgres::PgPoolOptions;
use sqlx::AssertSqlSafe;
use tower::ServiceExt;

fn database_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/postrust_test".to_string())
}

async fn schema(name: &str) {
    let schema = format!("response_headers_{name}");
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url())
        .await
        .unwrap();
    for statement in [
        format!("DROP SCHEMA IF EXISTS {schema} CASCADE"),
        format!("CREATE SCHEMA {schema}"),
        format!("GRANT USAGE ON SCHEMA {schema} TO web_anon"),
        format!("SET search_path = {schema}"),
        "CREATE TABLE items (id int PRIMARY KEY, name text)".into(),
        "INSERT INTO items VALUES (1, 'a'), (2, 'b'), (3, 'c')".into(),
        "CREATE TABLE no_pk (a int, b text)".into(),
        "CREATE TABLE empty (id int)".into(),
        "CREATE FUNCTION no_rows() RETURNS SETOF items LANGUAGE sql AS $$ SELECT * FROM items WHERE false $$"
            .into(),
        "CREATE TABLE organizations (id int PRIMARY KEY, referee int)".into(),
        "INSERT INTO organizations VALUES (1, NULL), (2, 1)".into(),
        // A media handler for any type, whose final function chooses the
        // `Content-Type` of its own for two of them.
        "CREATE DOMAIN \"*/*\" AS bytea".into(),
        "CREATE TABLE nums (val int)".into(),
        "INSERT INTO nums VALUES (1), (2)".into(),
        "CREATE FUNCTION some_trans (state \"*/*\", next nums) RETURNS \"*/*\" AS \
         $$ SELECT (state || E'\\n' || next.val::text::bytea)::\"*/*\" $$ LANGUAGE sql"
            .into(),
        "CREATE FUNCTION some_final (data \"*/*\") RETURNS \"*/*\" AS $$ \
         DECLARE accepted text := current_setting('request.headers', true)::json->>'accept'; \
         BEGIN \
           IF accepted = 'magic/number' THEN \
             PERFORM set_config('response.headers', \
               json_build_array(json_build_object('Content-Type', accepted))::text, true); \
           END IF; \
           RETURN ('x' || data)::\"*/*\"; \
         END $$ LANGUAGE plpgsql"
            .into(),
        "CREATE AGGREGATE some_agg (nums) (initcond = '', stype = \"*/*\", \
         sfunc = some_trans, finalfunc = some_final)"
            .into(),
        "CREATE FUNCTION answer() RETURNS int LANGUAGE sql AS $$ SELECT 42 $$".into(),
        format!("GRANT ALL ON ALL TABLES IN SCHEMA {schema} TO web_anon"),
        format!("GRANT EXECUTE ON ALL FUNCTIONS IN SCHEMA {schema} TO web_anon"),
    ] {
        sqlx::query(AssertSqlSafe(statement))
            .execute(&pool)
            .await
            .unwrap();
    }
}

async fn router(name: &str, compat_mode: bool) -> Router {
    let mut config = AppConfig::default();
    config.db_uri = database_url();
    config.db_schemas = vec![format!("response_headers_{name}")];
    config.db_anon_role = Some("web_anon".to_string());
    config.db_pool_size = 2;
    config.compat_mode = compat_mode;
    Builder::new(config, Host::SERVER)
        .build()
        .await
        .unwrap()
        .router
}

struct Answer {
    status: StatusCode,
    headers: HeaderMap,
    body: String,
}

impl Answer {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|v| v.to_str().ok())
    }
}

async fn send(
    router: &Router,
    method: &str,
    uri: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> Answer {
    let mut request = Request::builder().method(method).uri(uri);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    if !body.is_empty() {
        request = request.header("content-type", "application/json");
    }
    let response = router
        .clone()
        .oneshot(request.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    Answer {
        status,
        headers,
        body: String::from_utf8_lossy(&body).into_owned(),
    }
}

const COUNT: (&str, &str) = ("Prefer", "count=exact");

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn a_write_reports_what_it_affected_as_its_total() {
    for compat in [false, true] {
        let name = if compat { "writes_c" } else { "writes_d" };
        schema(name).await;
        let router = router(name, compat).await;

        let created = send(
            &router,
            "POST",
            "/api/items",
            &[COUNT],
            r#"{"id": 10, "name": "x"}"#,
        )
        .await;
        assert_eq!(created.header("content-range"), Some("*/1"));

        let updated = send(
            &router,
            "PATCH",
            "/api/items?id=lt.10",
            &[COUNT, ("Prefer", "return=representation")],
            r#"{"name": "y"}"#,
        )
        .await;
        assert_eq!(updated.header("content-range"), Some("0-2/3"));

        let deleted = send(&router, "DELETE", "/api/items?id=eq.10", &[COUNT], "").await;
        assert_eq!(deleted.header("content-range"), Some("*/1"));

        // Without the preference the total stays unknown.
        let plain = send(&router, "DELETE", "/api/items?id=eq.1", &[], "").await;
        assert_eq!(plain.header("content-range"), Some("*/*"));
    }
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn an_unsatisfiable_range_carries_its_headers() {
    schema("range").await;
    let router = router("range", false).await;

    let answer = send(&router, "GET", "/api/items?offset=100", &[COUNT], "").await;
    assert_eq!(answer.status, StatusCode::RANGE_NOT_SATISFIABLE);
    assert_eq!(answer.header("content-range"), Some("*/3"));
    assert_eq!(answer.header("preference-applied"), Some("count=exact"));
    assert_one_json_content_type(&answer);
    assert_eq!(
        body_json(&answer),
        serde_json::json!({
            "code": "PGRST103",
            "message": "Requested range not satisfiable",
            "details": "An offset of 100 was requested, but there are only 3 rows.",
            "hint": null
        })
    );

    // The same refusal from a function and from an empty table, as PostgREST's
    // own suite has them (RangeSpec.hs:37 and :281): `*/0`, one Content-Type.
    let rpc = send(&router, "GET", "/api/rpc/no_rows?offset=1", &[COUNT], "").await;
    assert_eq!(
        rpc.status,
        StatusCode::RANGE_NOT_SATISFIABLE,
        "{}",
        rpc.body
    );
    assert_eq!(rpc.header("content-range"), Some("*/0"));
    assert_eq!(rpc.header("preference-applied"), Some("count=exact"));
    assert_one_json_content_type(&rpc);
    let empty = send(&router, "GET", "/api/empty?offset=1", &[COUNT], "").await;
    assert_eq!(empty.status, StatusCode::RANGE_NOT_SATISFIABLE);
    assert_eq!(empty.header("content-range"), Some("*/0"));
    assert_one_json_content_type(&empty);
    for refused in [&rpc, &empty] {
        assert_eq!(
            body_json(refused),
            serde_json::json!({
                "code": "PGRST103",
                "message": "Requested range not satisfiable",
                "details": "An offset of 1 was requested, but there are only 0 rows.",
                "hint": null
            })
        );
    }
}

fn body_json(answer: &Answer) -> serde_json::Value {
    serde_json::from_str(&answer.body).unwrap()
}

fn assert_one_json_content_type(answer: &Answer) {
    assert_eq!(
        answer.headers.get_all("content-type").iter().count(),
        1,
        "{:?}",
        answer.headers
    );
    assert_eq!(
        answer.header("content-type"),
        Some("application/json; charset=utf-8")
    );
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn a_negative_offset_starts_at_the_first_row() {
    schema("offset").await;
    let router = router("offset", false).await;

    let answer = send(&router, "GET", "/api/items?offset=-4", &[], "").await;
    assert_eq!(answer.status, StatusCode::OK);
    assert_eq!(answer.header("content-range"), Some("0-2/*"));
}

