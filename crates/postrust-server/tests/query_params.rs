//! HTTP-level integration tests for REST query parameters.
//!
//! These tests exercise the full request path -- parse, plan, build SQL, execute
//! -- against a real PostgreSQL database, which is the only place where type
//! coercion and range handling can actually be verified.
//!
//! The database must have `scripts/init-db.sql` and `scripts/test-fixtures.sql`
//! loaded. Set `DATABASE_URL` to point at it, then run:
//!
//! ```text
//! cargo test --package postrust-server --test query_params -- --ignored
//! ```

use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode};
use axum::routing::any;
use axum::Router;
use postrust_core::{AppConfig, SchemaCache};
use postrust_server::{handle_request, AppState};
use serde_json::Value;
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use tokio::sync::RwLock;
use tower::ServiceExt;

/// The role the fixtures grant read access to.
const ANON_ROLE: &str = "web_anon";

fn database_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/postrust_test".to_string())
}

/// Build the REST router, mirroring how `main.rs` mounts it under `/api`.
async fn test_app() -> Router {
    build_app(None).await
}

/// Build the REST router with a server-side row ceiling (`PGRST_MAX_ROWS`).
async fn test_app_with_max_rows(max_rows: i64) -> Router {
    build_app(Some(max_rows)).await
}

async fn build_app(max_rows: Option<i64>) -> Router {
    let db_uri = database_url();

    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&db_uri)
        .await
        .expect("failed to connect to test database");

    let config = AppConfig {
        db_uri,
        db_schemas: vec!["public".to_string()],
        db_anon_role: Some(ANON_ROLE.to_string()),
        db_max_rows: max_rows,
        ..AppConfig::default()
    };

    let schema_cache = SchemaCache::load(&pool, &config.db_schemas)
        .await
        .expect("failed to load schema cache");

    let jwt_config = postrust_auth::JwtConfig {
        secret: config.jwt_secret.clone(),
        secret_is_base64: config.jwt_secret_is_base64,
        audience: config.jwt_aud.clone(),
        role_claim_key: config.jwt_role_claim_key.clone(),
        anon_role: config.db_anon_role.clone(),
    };

    let state = Arc::new(AppState {
        pool,
        schema_cache: RwLock::new(schema_cache),
        config,
        jwt_config,
    });

    let api_router: Router<Arc<AppState>> = Router::new()
        .route("/", any(handle_request))
        .route("/{*path}", any(handle_request));

    Router::new().nest("/api", api_router).with_state(state)
}

/// Issue a GET and return status, headers and parsed JSON body.
async fn get_with_headers(uri: &str, headers: &[(&str, &str)]) -> (StatusCode, HeaderMap, Value) {
    let app = test_app().await;

    let mut builder = Request::builder().method("GET").uri(uri);
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let request = builder
        .body(Body::empty())
        .expect("failed to build request");

    let response = app.oneshot(request).await.expect("request failed");
    let status = response.status();
    let response_headers = response.headers().clone();

    let bytes = axum::body::to_bytes(response.into_body(), 16 * 1024 * 1024)
        .await
        .expect("failed to read response body");

    // An empty body (e.g. a 404) is reported as JSON null so callers can assert
    // on status without special-casing.
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or_else(|e| {
            panic!(
                "response for {} was not valid JSON: {} -- body: {}",
                uri,
                e,
                String::from_utf8_lossy(&bytes)
            )
        })
    };

    (status, response_headers, body)
}

async fn get(uri: &str) -> (StatusCode, Value) {
    let (status, _, body) = get_with_headers(uri, &[]).await;
    (status, body)
}

/// Assert the request succeeded and return the rows.
fn rows(uri: &str, status: StatusCode, body: Value) -> Vec<Value> {
    assert_eq!(
        status,
        StatusCode::OK,
        "expected 200 for {} -- body: {}",
        uri,
        body
    );
    body.as_array()
        .unwrap_or_else(|| panic!("expected an array for {} -- got: {}", uri, body))
        .clone()
}

async fn get_rows(uri: &str) -> Vec<Value> {
    let (status, body) = get(uri).await;
    rows(uri, status, body)
}

/// Extract an integer column from each row.
fn ids(rows: &[Value], column: &str) -> Vec<i64> {
    rows.iter()
        .map(|row| {
            row.get(column)
                .and_then(|v| v.as_i64())
                .unwrap_or_else(|| panic!("row is missing integer column {}: {}", column, row))
        })
        .collect()
}

// ===========================================================================
// select
// ===========================================================================

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn select_returns_only_requested_columns() {
    let rows = get_rows("/api/products?select=id,name&order=id.asc&limit=1").await;

    assert_eq!(rows.len(), 1);
    let row = rows[0].as_object().expect("row should be an object");
    let mut keys: Vec<&str> = row.keys().map(|k| k.as_str()).collect();
    keys.sort();
    assert_eq!(keys, vec!["id", "name"]);
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn select_without_param_returns_all_columns() {
    let rows = get_rows("/api/products?order=id.asc&limit=1").await;

    let row = rows[0].as_object().expect("row should be an object");
    for expected in ["id", "name", "price", "stock", "category", "is_active"] {
        assert!(
            row.contains_key(expected),
            "expected column {} in {:?}",
            expected,
            row.keys().collect::<Vec<_>>()
        );
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn select_rejects_unknown_column() {
    let (status, body) = get("/api/products?select=id,no_such_column").await;

    assert!(
        status.is_client_error(),
        "expected a 4xx for an unknown column, got {} -- body: {}",
        status,
        body
    );
}

// ===========================================================================
// limit / offset
//
// Regression coverage: `limit` and `offset` were parsed into
// `QueryParams::ranges` and never read, so every request returned the full
// table.
// ===========================================================================

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn limit_restricts_row_count() {
    let rows = get_rows("/api/products?limit=3").await;
    assert_eq!(rows.len(), 3, "limit=3 should return exactly 3 rows");
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn limit_larger_than_table_returns_all_rows() {
    let rows = get_rows("/api/products?limit=1000").await;
    assert_eq!(rows.len(), 10, "the products fixture has 10 rows");
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn offset_skips_leading_rows() {
    let rows = get_rows("/api/products?order=id.asc&offset=8").await;

    assert_eq!(ids(&rows, "id"), vec![9, 10]);
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn limit_and_offset_combine() {
    let rows = get_rows("/api/products?order=id.asc&limit=2&offset=4").await;

    assert_eq!(ids(&rows, "id"), vec![5, 6]);
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn limit_applies_after_ordering() {
    let rows = get_rows("/api/products?order=id.desc&limit=2").await;

    assert_eq!(ids(&rows, "id"), vec![10, 9]);
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn non_numeric_limit_is_rejected() {
    let (status, body) = get("/api/products?limit=not-a-number").await;

    assert!(
        status.is_client_error(),
        "expected a 4xx for a non-numeric limit, got {} -- body: {}",
        status,
        body
    );
}

// ===========================================================================
// Range header
// ===========================================================================

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn range_header_limits_rows() {
    let (status, _, body) =
        get_with_headers("/api/products?order=id.asc", &[("Range", "0-2")]).await;
    let rows = rows("/api/products (Range: 0-2)", status, body);

    assert_eq!(ids(&rows, "id"), vec![1, 2, 3]);
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn limit_param_takes_precedence_over_range_header() {
    let (status, _, body) =
        get_with_headers("/api/products?order=id.asc&limit=2", &[("Range", "0-9")]).await;
    let rows = rows("/api/products (limit=2, Range: 0-9)", status, body);

    assert_eq!(
        ids(&rows, "id"),
        vec![1, 2],
        "the limit query parameter should win over the Range header"
    );
}

// ===========================================================================
// Filters and type coercion
//
// Regression coverage: filter values are bound as text, so comparing against a
// non-text column failed with `operator does not exist: integer = text` until
// the placeholder gained an explicit cast.
// ===========================================================================

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn eq_filter_on_integer_column() {
    let rows = get_rows("/api/products?id=eq.4").await;

    assert_eq!(ids(&rows, "id"), vec![4]);
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn eq_filter_on_text_column() {
    let rows = get_rows("/api/products?category=eq.Books&order=id.asc").await;

    assert_eq!(ids(&rows, "id"), vec![1, 3, 7]);
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn eq_filter_on_boolean_column() {
    let rows = get_rows("/api/products?is_active=eq.false").await;

    assert_eq!(ids(&rows, "id"), vec![6]);
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn comparison_filter_on_numeric_column() {
    let rows = get_rows("/api/products?price=gt.100&order=id.asc").await;

    assert_eq!(ids(&rows, "id"), vec![2, 5, 8]);
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn comparison_filters_bound_a_range() {
    let rows = get_rows("/api/products?stock=gte.100&stock=lte.500&order=id.asc").await;

    assert_eq!(ids(&rows, "id"), vec![1, 2, 5, 8, 9]);
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn in_filter_on_integer_column() {
    let rows = get_rows("/api/products?id=in.(1,3,5)&order=id.asc").await;

    assert_eq!(ids(&rows, "id"), vec![1, 3, 5]);
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn negated_filter_on_integer_column() {
    let rows = get_rows("/api/products?id=not.eq.4").await;

    assert_eq!(rows.len(), 9);
    assert!(!ids(&rows, "id").contains(&4));
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn negated_comparison_filter() {
    let rows = get_rows("/api/products?price=not.gt.100&order=id.asc").await;

    assert_eq!(ids(&rows, "id"), vec![1, 3, 4, 6, 7, 9, 10]);
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn negated_in_filter() {
    let rows = get_rows("/api/products?id=not.in.(1,2,3)&order=id.asc").await;

    assert_eq!(ids(&rows, "id"), vec![4, 5, 6, 7, 8, 9, 10]);
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn negated_is_null_filter() {
    let rows = get_rows("/api/posts?published_at=not.is.null&select=id,published_at").await;

    assert!(!rows.is_empty(), "some posts are published");
    for row in &rows {
        assert_ne!(
            row.get("published_at"),
            Some(&Value::Null),
            "not.is.null should exclude NULL rows"
        );
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn is_null_filter() {
    let rows = get_rows("/api/posts?published_at=is.null&select=id,published_at").await;

    for row in &rows {
        assert_eq!(
            row.get("published_at"),
            Some(&Value::Null),
            "is.null should only match NULL rows"
        );
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn filter_combines_with_select_order_and_limit() {
    let rows =
        get_rows("/api/products?category=eq.Books&select=id,name&order=id.desc&limit=2").await;

    assert_eq!(ids(&rows, "id"), vec![7, 3]);
    let row = rows[0].as_object().expect("row should be an object");
    assert_eq!(row.len(), 2, "only id and name were selected");
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn filter_value_is_not_interpreted_as_sql() {
    // The value must reach PostgreSQL as data, not as part of the statement.
    // Percent-encoded: `Books';DROP TABLE products;--`
    let (status, body) =
        get("/api/products?category=eq.Books%27%3BDROP%20TABLE%20products%3B--").await;

    assert!(
        status == StatusCode::OK || status.is_client_error(),
        "unexpected status {} -- body: {}",
        status,
        body
    );

    let rows = get_rows("/api/products?limit=1000").await;
    assert_eq!(rows.len(), 10, "products table should be intact");
}

// ===========================================================================
// max_rows ceiling
//
// Regression coverage: `db_max_rows` was declared in the config, never read
// from the environment and never enforced, so a request with no `limit`
// returned the entire table -- documented as capped at 1000 rows.
// ===========================================================================

/// Issue a GET against an app configured with a row ceiling.
async fn get_rows_capped(uri: &str, max_rows: i64) -> Vec<Value> {
    let app = test_app_with_max_rows(max_rows).await;
    let request = Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
        .expect("failed to build request");

    let response = app.oneshot(request).await.expect("request failed");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 16 * 1024 * 1024)
        .await
        .expect("failed to read response body");
    let body: Value = serde_json::from_slice(&bytes).expect("response was not valid JSON");

    rows(uri, status, body)
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn max_rows_caps_a_request_with_no_limit() {
    let rows = get_rows_capped("/api/products?order=id.asc", 4).await;

    assert_eq!(
        ids(&rows, "id"),
        vec![1, 2, 3, 4],
        "an unbounded request must be capped by max_rows"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn max_rows_caps_a_larger_requested_limit() {
    let rows = get_rows_capped("/api/products?order=id.asc&limit=1000", 3).await;

    assert_eq!(rows.len(), 3, "max_rows must bound a larger explicit limit");
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn max_rows_leaves_a_smaller_requested_limit_alone() {
    let rows = get_rows_capped("/api/products?order=id.asc&limit=2", 100).await;

    assert_eq!(
        ids(&rows, "id"),
        vec![1, 2],
        "a limit below the ceiling should be honoured as-is"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn max_rows_applies_with_offset() {
    let rows = get_rows_capped("/api/products?order=id.asc&offset=8", 5).await;

    assert_eq!(
        ids(&rows, "id"),
        vec![9, 10],
        "the ceiling applies to the page, not the whole table"
    );
}

// ===========================================================================
// order
// ===========================================================================

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn order_ascending_and_descending() {
    let ascending = get_rows("/api/products?order=id.asc&select=id").await;
    let descending = get_rows("/api/products?order=id.desc&select=id").await;

    let mut reversed = ids(&ascending, "id");
    reversed.reverse();
    assert_eq!(ids(&descending, "id"), reversed);
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn order_on_numeric_column() {
    let rows = get_rows("/api/products?order=price.asc&select=id,price&limit=1").await;

    assert_eq!(ids(&rows, "id"), vec![4], "9.99 is the lowest price");
}

// ===========================================================================
// Resource embedding
//
// Regression: `select=id,posts(id)` parsed the nested selection and threw it
// away, so the request returned 200 with the embed silently missing.
// ===========================================================================

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn embeds_a_to_many_relation() {
    let rows = get_rows("/api/users?select=id,name,posts(id,title)&order=id.asc&limit=2").await;

    let posts = rows[0]["posts"]
        .as_array()
        .expect("posts must be embedded as an array");
    assert!(!posts.is_empty(), "user 1 has posts in the fixtures");

    // Only the requested columns of the relation come back.
    let post = posts[0].as_object().expect("post should be an object");
    let mut keys: Vec<&str> = post.keys().map(|k| k.as_str()).collect();
    keys.sort();
    assert_eq!(keys, vec!["id", "title"]);
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn embeds_a_to_one_relation() {
    let rows = get_rows("/api/posts?select=id,title,users(name)&order=id.asc&limit=1").await;

    assert_eq!(
        rows[0]["users"]["name"].as_str(),
        Some("Alice Johnson"),
        "a to-one embed resolves to an object, not a list"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn embeds_nested_relations_two_levels_deep() {
    let rows = get_rows("/api/users?select=id,posts(id,comments(id))&order=id.asc&limit=1").await;

    let posts = rows[0]["posts"].as_array().expect("posts array");
    let with_comments = posts
        .iter()
        .find(|p| {
            p["comments"]
                .as_array()
                .map(|c| !c.is_empty())
                .unwrap_or(false)
        })
        .expect("at least one post has comments");

    assert!(with_comments["comments"][0]["id"].is_number());
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn embed_join_column_is_not_leaked_into_the_response() {
    // `posts` joins on users.id, which has to be selected to do the join even
    // though the client only asked for the name.
    let rows = get_rows("/api/users?select=name,posts(id)&order=id.asc&limit=1").await;

    let row = rows[0].as_object().expect("row object");
    let mut keys: Vec<&str> = row.keys().map(|k| k.as_str()).collect();
    keys.sort();
    assert_eq!(
        keys,
        vec!["name", "posts"],
        "the join column must not appear in the response"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn embed_yields_an_empty_list_when_there_are_no_children() {
    let rows = get_rows("/api/users?select=id,posts(id)&order=id.desc&limit=10").await;

    let childless = rows
        .iter()
        .find(|r| r["posts"].as_array().map(|p| p.is_empty()).unwrap_or(false));

    assert!(
        childless.is_some(),
        "some fixture user has no posts; the embed should still be an empty array"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn unknown_relation_is_rejected() {
    let (status, body) = get("/api/users?select=id,not_a_relation(id)").await;

    assert!(
        status.is_client_error(),
        "expected a 4xx for an unknown relation, got {} -- body: {}",
        status,
        body
    );
}

/// Concurrent embeds must not exhaust the connection pool.
///
/// Embedding acquires a connection per relationship. While the request's own
/// connection was still held during that, each in-flight embed needed two
/// connections at once, so once every connection was held by a request waiting
/// for a second one, none could proceed and the pool deadlocked until acquire
/// timeouts fired.
///
/// The test pool holds two connections, so more concurrent embeds than that is
/// enough to reproduce it. Every sequential embed test passed throughout: only
/// concurrency above the pool size shows the problem, which is why this asserts
/// on a deadline rather than only on the response bodies.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires PostgreSQL"]
async fn concurrent_embeds_do_not_exhaust_the_pool() {
    const CONCURRENT: usize = 8;

    let app = build_app(None).await;

    let mut handles = Vec::with_capacity(CONCURRENT);
    for _ in 0..CONCURRENT {
        let app = app.clone();
        handles.push(tokio::spawn(async move {
            let request = Request::builder()
                .method("GET")
                .uri("/api/users?select=id,name,posts(id,title)&order=id.asc&limit=5")
                .body(Body::empty())
                .expect("failed to build request");

            let response = app.oneshot(request).await.expect("request failed");
            response.status()
        }));
    }

    // Comfortably above what these queries need, and far below the 30s acquire
    // timeout that the deadlock used to wait on.
    let statuses = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        let mut statuses = Vec::with_capacity(CONCURRENT);
        for handle in handles {
            statuses.push(handle.await.expect("task panicked"));
        }
        statuses
    })
    .await
    .expect("concurrent embeds did not finish in time -- the connection pool deadlocked");

    for status in statuses {
        assert_eq!(
            status,
            StatusCode::OK,
            "every concurrent embed should succeed"
        );
    }
}
