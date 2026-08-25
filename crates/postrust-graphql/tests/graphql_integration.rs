//! Integration tests for the GraphQL surface: reads, writes, and the schema
//! shape for subscriptions.
//!
//! These execute real GraphQL documents against a real PostgreSQL database,
//! which is the only place resolver behaviour can be verified -- the SQL the
//! resolvers build is not observable from a unit test.
//!
//! Each test creates its own PostgreSQL schema containing a single `widgets`
//! table and exposes only that schema, so tests cannot disturb each other and
//! the generated field names are predictable. That also exercises GraphQL
//! against a non-`public` schema. Run with:
//!
//! ```text
//! DATABASE_URL="postgres://postgres:postgres@localhost:5432/postrust_test" \
//!   cargo test --package postrust-graphql --test graphql_integration -- --ignored
//! ```

use async_graphql::Request;
use postrust_auth::AuthResult;
use postrust_core::schema_cache::{SchemaCache, SchemaCacheRef};
use postrust_graphql::context::GraphQLContext;
use postrust_graphql::handler::GraphQLState;
use postrust_graphql::schema::SchemaConfig;
use sqlx::postgres::PgPoolOptions;
use sqlx::{Executor, PgPool};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

/// Connect as a role that can create and mutate the test tables.
const TEST_ROLE: &str = "postgres";

static TABLE_COUNTER: AtomicU32 = AtomicU32::new(0);

fn database_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/postrust_test".to_string())
}

/// Name for a throwaway schema dedicated to one test.
fn unique_schema_name(prefix: &str) -> String {
    let id = TABLE_COUNTER.fetch_add(1, Ordering::SeqCst);
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();
    format!("gql_{}_{}_{}", prefix, stamp, id)
}

async fn connect() -> PgPool {
    PgPoolOptions::new()
        .max_connections(2)
        .connect(&database_url())
        .await
        .expect("failed to connect to test database")
}

/// Create a dedicated schema holding a `widgets` table with a mix of column
/// types, so type coercion is exercised too.
async fn create_widgets_schema(pool: &PgPool, schema: &str) {
    pool.execute(format!("DROP SCHEMA IF EXISTS {} CASCADE", schema).as_str())
        .await
        .expect("drop schema failed");
    pool.execute(format!("CREATE SCHEMA {}", schema).as_str())
        .await
        .expect("create schema failed");

    pool.execute(
        format!(
            r#"
            CREATE TABLE {}.widgets (
                id SERIAL PRIMARY KEY,
                name TEXT NOT NULL,
                category TEXT NOT NULL,
                price NUMERIC(10,2) NOT NULL,
                stock INTEGER NOT NULL,
                is_active BOOLEAN NOT NULL DEFAULT true
            )
            "#,
            schema
        )
        .as_str(),
    )
    .await
    .expect("create failed");

    pool.execute(
        format!(
            r#"
            INSERT INTO {}.widgets (name, category, price, stock, is_active) VALUES
                ('alpha', 'books', 10.50, 5, true),
                ('bravo', 'tools', 20.00, 0, true),
                ('charlie', 'books', 30.25, 12, false),
                ('delta', 'tools', 40.75, 7, true)
            "#,
            schema
        )
        .as_str(),
    )
    .await
    .expect("seed failed");
}

async fn drop_schema(pool: &PgPool, schema: &str) {
    let _ = pool
        .execute(format!("DROP SCHEMA IF EXISTS {} CASCADE", schema).as_str())
        .await;
}

/// Build GraphQL state over the current database schema.
async fn build_state(
    pool: &PgPool,
    schema: &str,
    max_rows: Option<i64>,
    subscriptions: bool,
) -> Arc<GraphQLState> {
    let schemas = vec![schema.to_string()];
    let cache = SchemaCache::load(pool, &schemas)
        .await
        .expect("failed to load schema cache");

    let config = SchemaConfig {
        exposed_schemas: schemas.clone(),
        enable_mutations: true,
        enable_subscriptions: subscriptions,
        max_rows,
        ..SchemaConfig::default()
    };

    Arc::new(
        GraphQLState::new(pool.clone(), Arc::new(cache), config)
            .expect("failed to build GraphQL schema"),
    )
}

/// Execute a GraphQL document and return the whole response.
async fn execute(
    state: &Arc<GraphQLState>,
    pool: &PgPool,
    schema: &str,
    query: &str,
) -> async_graphql::Response {
    let cache = SchemaCache::load(pool, &[schema.to_string()])
        .await
        .expect("failed to load schema cache");

    // Every write in one operation shares a transaction, and the caller is what
    // settles it -- so a test that only executes and never settles would leave
    // its rows uncommitted, exactly as the server would.
    let write: postrust_graphql::context::SharedWrite = Default::default();
    let ctx = GraphQLContext::new(
        pool.clone(),
        SchemaCacheRef::from_static(cache),
        AuthResult {
            role: TEST_ROLE.to_string(),
            claims: HashMap::new(),
        },
    )
    .with_write(std::sync::Arc::clone(&write));

    let request = Request::new(query).data(ctx).data(pool.clone());
    let response = state.schema.execute(request).await;
    if let Some(tx) = write.lock().await.take() {
        if response.errors.is_empty() {
            tx.commit().await.expect("the write could not be committed");
        } else {
            tx.rollback().await.expect("the write could not be undone");
        }
    }
    response
}

/// Execute and require success, returning the `data` payload as JSON.
async fn execute_ok(
    state: &Arc<GraphQLState>,
    pool: &PgPool,
    schema: &str,
    query: &str,
) -> serde_json::Value {
    let response = execute(state, pool, schema, query).await;
    assert!(
        response.errors.is_empty(),
        "expected no GraphQL errors for {} -- got: {:?}",
        query,
        response.errors
    );
    serde_json::to_value(&response.data).expect("data was not serialisable")
}

/// Execute and require failure, returning the joined error messages.
async fn execute_err(
    state: &Arc<GraphQLState>,
    pool: &PgPool,
    schema: &str,
    query: &str,
) -> String {
    let response = execute(state, pool, schema, query).await;
    assert!(
        !response.errors.is_empty(),
        "expected a GraphQL error for {} -- got data: {:?}",
        query,
        response.data
    );
    response
        .errors
        .iter()
        .map(|e| e.message.clone())
        .collect::<Vec<_>>()
        .join("; ")
}

fn ids_of(rows: &serde_json::Value, field: &str) -> Vec<i64> {
    rows.get(field)
        .and_then(|v| v.as_array())
        .unwrap_or_else(|| panic!("expected a list at {} -- got {}", field, rows))
        .iter()
        .map(|row| {
            row.get("id")
                .and_then(|v| v.as_i64())
                .unwrap_or_else(|| panic!("row missing integer id: {}", row))
        })
        .collect()
}

// ===========================================================================
// Reads
// ===========================================================================

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn list_query_returns_rows_with_typed_columns() {
    let pool = connect().await;
    let schema = unique_schema_name("list");
    create_widgets_schema(&pool, &schema).await;

    let state = build_state(&pool, &schema, None, false).await;
    let data = execute_ok(
        &state,
        &pool,
        &schema,
        "{ widgets(order_by: [{id: asc}]) { id name stock is_active } }",
    )
    .await;

    let rows = data
        .get("widgets")
        .and_then(|v| v.as_array())
        .unwrap()
        .clone();
    assert_eq!(rows.len(), 4);

    // Columns must come back with their real types, not stringified or null.
    assert_eq!(rows[0].get("id").and_then(|v| v.as_i64()), Some(1));
    assert_eq!(rows[0].get("name").and_then(|v| v.as_str()), Some("alpha"));
    assert_eq!(rows[0].get("stock").and_then(|v| v.as_i64()), Some(5));
    assert_eq!(
        rows[0].get("is_active").and_then(|v| v.as_bool()),
        Some(true)
    );

    drop_schema(&pool, &schema).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn by_pk_query_returns_the_requested_row() {
    // Regression: the by-PK resolver built no WHERE clause at all, fetched the
    // whole table and returned whichever row came back first.
    let pool = connect().await;
    let schema = unique_schema_name("bypk");
    create_widgets_schema(&pool, &schema).await;

    let state = build_state(&pool, &schema, None, false).await;
    let data = execute_ok(
        &state,
        &pool,
        &schema,
        "{ widgets_by_pk(id: 3) { id name } }",
    )
    .await;

    let row = data.get("widgets_by_pk").expect("missing by-pk field");
    assert_eq!(row.get("id").and_then(|v| v.as_i64()), Some(3));
    assert_eq!(row.get("name").and_then(|v| v.as_str()), Some("charlie"));

    drop_schema(&pool, &schema).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn by_pk_query_returns_null_for_a_missing_key() {
    let pool = connect().await;
    let schema = unique_schema_name("bypknull");
    create_widgets_schema(&pool, &schema).await;

    let state = build_state(&pool, &schema, None, false).await;
    let data = execute_ok(
        &state,
        &pool,
        &schema,
        "{ widgets_by_pk(id: 9999) { id name } }",
    )
    .await;

    assert_eq!(
        data.get("widgets_by_pk"),
        Some(&serde_json::Value::Null),
        "a key that matches nothing must resolve to null"
    );

    drop_schema(&pool, &schema).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn filter_argument_narrows_results() {
    // Regression: `filter` was a declared argument that the resolver ignored,
    // so a filtered query silently returned the entire table.
    let pool = connect().await;
    let schema = unique_schema_name("filter");
    create_widgets_schema(&pool, &schema).await;

    let state = build_state(&pool, &schema, None, false).await;
    let data = execute_ok(
        &state,
        &pool,
        &schema,
        "{ widgets(where: {category: {_eq: \"books\"}}, order_by: [{id: asc}]) { id } }",
    )
    .await;

    assert_eq!(ids_of(&data, "widgets"), vec![1, 3]);

    drop_schema(&pool, &schema).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn filter_supports_comparison_operators() {
    let pool = connect().await;
    let schema = unique_schema_name("filtercmp");
    create_widgets_schema(&pool, &schema).await;

    let state = build_state(&pool, &schema, None, false).await;
    let data = execute_ok(
        &state,
        &pool,
        &schema,
        "{ widgets(where: {stock: {_gt: 5}}, order_by: [{id: asc}]) { id } }",
    )
    .await;

    assert_eq!(ids_of(&data, "widgets"), vec![3, 4]);

    drop_schema(&pool, &schema).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn order_by_sorts_ascending_and_descending() {
    // Regression: `orderBy` was declared and ignored.
    let pool = connect().await;
    let schema = unique_schema_name("order");
    create_widgets_schema(&pool, &schema).await;

    let state = build_state(&pool, &schema, None, false).await;

    let ascending = execute_ok(
        &state,
        &pool,
        &schema,
        "{ widgets(order_by: [{id: asc}]) { id } }",
    )
    .await;
    let descending = execute_ok(
        &state,
        &pool,
        &schema,
        "{ widgets(order_by: [{id: desc}]) { id } }",
    )
    .await;

    assert_eq!(ids_of(&ascending, "widgets"), vec![1, 2, 3, 4]);
    assert_eq!(ids_of(&descending, "widgets"), vec![4, 3, 2, 1]);

    drop_schema(&pool, &schema).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn order_by_sorts_on_a_non_key_column() {
    let pool = connect().await;
    let schema = unique_schema_name("ordercol");
    create_widgets_schema(&pool, &schema).await;

    let state = build_state(&pool, &schema, None, false).await;
    let data = execute_ok(
        &state,
        &pool,
        &schema,
        "{ widgets(order_by: [{stock: asc}]) { id stock } }",
    )
    .await;

    // stock: bravo 0, alpha 5, delta 7, charlie 12
    assert_eq!(ids_of(&data, "widgets"), vec![2, 1, 4, 3]);

    drop_schema(&pool, &schema).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn order_by_rejects_an_unknown_column() {
    // A column name reaches SQL, so it has to be one the table has. It is
    // refused by validation now rather than by the builder -- `<table>_order_by`
    // is a generated input, so a name that is not a column is not a field --
    // which is a better place to refuse it: the client is told before the
    // request runs, and nothing crafted can be written there at all.
    let pool = connect().await;
    let schema = unique_schema_name("orderbad");
    create_widgets_schema(&pool, &schema).await;

    let state = build_state(&pool, &schema, None, false).await;
    let errors = execute_err(
        &state,
        &pool,
        &schema,
        "{ widgets(order_by: [{no_such_column: asc}]) { id } }",
    )
    .await;
    assert!(
        errors.contains("no_such_column"),
        "expected the unknown column to be named, got: {}",
        errors
    );

    // The table must still be intact.
    let remaining: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM {}.widgets", schema))
        .fetch_one(&pool)
        .await
        .expect("count failed");
    assert_eq!(remaining, 4);

    drop_schema(&pool, &schema).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn order_by_rejects_an_invalid_direction() {
    let pool = connect().await;
    let schema = unique_schema_name("orderdir");
    create_widgets_schema(&pool, &schema).await;

    let state = build_state(&pool, &schema, None, false).await;
    let errors = execute_err(
        &state,
        &pool,
        &schema,
        "{ widgets(order_by: [{id: sideways}]) { id } }",
    )
    .await;
    assert!(
        errors.contains("sideways"),
        "expected the direction to be named, got: {}",
        errors
    );

    drop_schema(&pool, &schema).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn limit_and_offset_paginate() {
    let pool = connect().await;
    let schema = unique_schema_name("page");
    create_widgets_schema(&pool, &schema).await;

    let state = build_state(&pool, &schema, None, false).await;
    let data = execute_ok(
        &state,
        &pool,
        &schema,
        "{ widgets(order_by: [{id: asc}], limit: 2, offset: 1) { id } }",
    )
    .await;

    assert_eq!(ids_of(&data, "widgets"), vec![2, 3]);

    drop_schema(&pool, &schema).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn max_rows_caps_a_query_with_no_limit() {
    // Regression: a GraphQL query with no `limit` selected the whole table.
    let pool = connect().await;
    let schema = unique_schema_name("maxrows");
    create_widgets_schema(&pool, &schema).await;

    let state = build_state(&pool, &schema, Some(2), false).await;
    let data = execute_ok(
        &state,
        &pool,
        &schema,
        "{ widgets(order_by: [{id: asc}]) { id } }",
    )
    .await;

    assert_eq!(ids_of(&data, "widgets"), vec![1, 2]);

    drop_schema(&pool, &schema).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn max_rows_bounds_a_larger_requested_limit() {
    let pool = connect().await;
    let schema = unique_schema_name("maxrowslim");
    create_widgets_schema(&pool, &schema).await;

    let state = build_state(&pool, &schema, Some(2), false).await;
    let data = execute_ok(
        &state,
        &pool,
        &schema,
        "{ widgets(order_by: [{id: asc}], limit: 100) { id } }",
    )
    .await;

    assert_eq!(ids_of(&data, "widgets").len(), 2);

    drop_schema(&pool, &schema).await;
}

// ===========================================================================
// Writes
// ===========================================================================

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn insert_mutation_creates_a_row() {
    let pool = connect().await;
    let schema = unique_schema_name("insert");
    create_widgets_schema(&pool, &schema).await;

    let state = build_state(&pool, &schema, None, false).await;
    let data = execute_ok(
        &state,
        &pool,
        &schema,
        "mutation { insert_widgets(objects: [{name: \"echo\", category: \"tools\", price: 5.5, stock: 3}]) \
          { affected_rows returning { id name } } }",
    )
    .await;

    let response = data.get("insert_widgets").expect("insert returned nothing");
    assert_eq!(
        response.get("affected_rows").and_then(|v| v.as_i64()),
        Some(1)
    );
    let rows = response
        .get("returning")
        .and_then(|v| v.as_array())
        .expect("insert returned no rows");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get("name").and_then(|v| v.as_str()), Some("echo"));

    let total: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM {}.widgets", schema))
        .fetch_one(&pool)
        .await
        .expect("count failed");
    assert_eq!(total, 5);

    drop_schema(&pool, &schema).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn update_mutation_with_where_changes_only_matching_rows() {
    let pool = connect().await;
    let schema = unique_schema_name("update");
    create_widgets_schema(&pool, &schema).await;

    let state = build_state(&pool, &schema, None, false).await;
    execute_ok(
        &state,
        &pool,
        &schema,
        "mutation { update_widgets(where: {id: {_eq: 2}}, _set: {name: \"renamed\"}) \
          { affected_rows returning { id name } } }",
    )
    .await;

    let renamed: i64 = sqlx::query_scalar(&format!(
        "SELECT count(*) FROM {}.widgets WHERE name = 'renamed'",
        schema
    ))
    .fetch_one(&pool)
    .await
    .expect("count failed");
    assert_eq!(renamed, 1, "exactly one row should have been updated");

    drop_schema(&pool, &schema).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn update_mutation_without_where_is_refused() {
    // Regression: an absent `where` produced `UPDATE <table> SET ...` with no
    // WHERE clause, rewriting every row.
    let pool = connect().await;
    let schema = unique_schema_name("updateall");
    create_widgets_schema(&pool, &schema).await;

    let state = build_state(&pool, &schema, None, false).await;
    let errors = execute_err(
        &state,
        &pool,
        &schema,
        "mutation { update_widgets(_set: {name: \"clobbered\"}) { returning { id } } }",
    )
    .await;
    // Refused by validation now rather than by the resolver: `where` is
    // non-null on a bulk write, so a request without one never runs.
    assert!(
        errors.contains("where"),
        "expected a refusal naming `where`, got: {}",
        errors
    );

    let clobbered: i64 = sqlx::query_scalar(&format!(
        "SELECT count(*) FROM {}.widgets WHERE name = 'clobbered'",
        schema
    ))
    .fetch_one(&pool)
    .await
    .expect("count failed");
    assert_eq!(clobbered, 0, "no row should have been updated");

    drop_schema(&pool, &schema).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn delete_mutation_with_where_removes_only_matching_rows() {
    let pool = connect().await;
    let schema = unique_schema_name("delete");
    create_widgets_schema(&pool, &schema).await;

    let state = build_state(&pool, &schema, None, false).await;
    execute_ok(
        &state,
        &pool,
        &schema,
        "mutation { delete_widgets(where: {category: {_eq: \"books\"}}) \
          { affected_rows returning { id } } }",
    )
    .await;

    let remaining: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM {}.widgets", schema))
        .fetch_one(&pool)
        .await
        .expect("count failed");
    assert_eq!(remaining, 2, "only the two 'books' rows should be gone");

    drop_schema(&pool, &schema).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn delete_mutation_without_where_is_refused() {
    // Regression: this emitted `DELETE FROM <table> RETURNING ...` and emptied
    // the table.
    let pool = connect().await;
    let schema = unique_schema_name("deleteall");
    create_widgets_schema(&pool, &schema).await;

    let state = build_state(&pool, &schema, None, false).await;
    let errors = execute_err(
        &state,
        &pool,
        &schema,
        "mutation { delete_widgets { returning { id } } }",
    )
    .await;
    // Refused by validation now rather than by the resolver: `where` is
    // non-null on a bulk write, so a request without one never runs.
    assert!(
        errors.contains("where"),
        "expected a refusal naming `where`, got: {}",
        errors
    );

    let remaining: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM {}.widgets", schema))
        .fetch_one(&pool)
        .await
        .expect("count failed");
    assert_eq!(remaining, 4, "the table must be untouched");

    drop_schema(&pool, &schema).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn mutation_value_is_not_interpreted_as_sql() {
    let pool = connect().await;
    let schema = unique_schema_name("inject");
    create_widgets_schema(&pool, &schema).await;

    let state = build_state(&pool, &schema, None, false).await;
    execute_ok(
        &state,
        &pool,
        &schema,
        "mutation { update_widgets(where: {name: {_eq: \"alpha\"}}, _set: {name: \"x'); DROP TABLE widgets;--\"}) \
          { returning { id } } }",
    )
    .await;

    let remaining: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM {}.widgets", schema))
        .fetch_one(&pool)
        .await
        .expect("table should still exist");
    assert_eq!(remaining, 4);

    drop_schema(&pool, &schema).await;
}

// ===========================================================================
// Schema shape: subscriptions, queries, mutations
// ===========================================================================

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn subscription_type_is_present_when_enabled() {
    let pool = connect().await;
    let schema = unique_schema_name("sub");
    create_widgets_schema(&pool, &schema).await;

    let state = build_state(&pool, &schema, None, true).await;
    let sdl = state.schema.sdl();
    assert!(
        sdl.contains("type subscription_root"),
        "subscriptions enabled but no Subscription type in the schema"
    );
    assert!(
        !state.subscription_fields.is_empty(),
        "no subscription fields were generated"
    );

    drop_schema(&pool, &schema).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn subscription_type_is_absent_when_disabled() {
    let pool = connect().await;
    let schema = unique_schema_name("nosub");
    create_widgets_schema(&pool, &schema).await;

    let state = build_state(&pool, &schema, None, false).await;
    assert!(
        !state.schema.sdl().contains("type Subscription"),
        "subscriptions are disabled but a Subscription type was generated"
    );

    drop_schema(&pool, &schema).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn schema_exposes_read_and_write_fields_for_each_table() {
    let pool = connect().await;
    let schema = unique_schema_name("shape");
    create_widgets_schema(&pool, &schema).await;

    let state = build_state(&pool, &schema, None, false).await;
    let sdl = state.schema.sdl();

    for field in [
        "widgets",
        "widgets_by_pk",
        "insert_widgets",
        "update_widgets",
        "delete_widgets",
    ] {
        assert!(sdl.contains(field), "schema is missing field {}", field);
    }

    drop_schema(&pool, &schema).await;
}

// ===========================================================================
// Multi-schema naming
// ===========================================================================

/// Create a schema holding a `widgets` table with a single identifying row.
async fn create_marker_schema(pool: &PgPool, schema: &str, marker: &str) {
    let _ = pool
        .execute(format!("DROP SCHEMA IF EXISTS {} CASCADE", schema).as_str())
        .await;
    pool.execute(format!("CREATE SCHEMA {}", schema).as_str())
        .await
        .expect("create schema failed");
    pool.execute(
        format!(
            "CREATE TABLE {}.widgets (id SERIAL PRIMARY KEY, marker TEXT NOT NULL)",
            schema
        )
        .as_str(),
    )
    .await
    .expect("create table failed");
    pool.execute(
        format!(
            "INSERT INTO {}.widgets (marker) VALUES ('{}')",
            schema, marker
        )
        .as_str(),
    )
    .await
    .expect("seed failed");
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn same_table_name_in_two_schemas_gets_distinct_fields() {
    // Regression: type and field names were derived from the table name alone,
    // so a `widgets` table in two exposed schemas produced identical names and
    // one silently replaced the other.
    let pool = connect().await;
    let default_schema = unique_schema_name("multi_default");
    let other_schema = unique_schema_name("multi_other");

    create_marker_schema(&pool, &default_schema, "from-default").await;
    create_marker_schema(&pool, &other_schema, "from-other").await;

    let schemas = vec![default_schema.clone(), other_schema.clone()];
    let cache = SchemaCache::load(&pool, &schemas)
        .await
        .expect("failed to load schema cache");
    let config = SchemaConfig {
        exposed_schemas: schemas.clone(),
        enable_mutations: true,
        max_rows: None,
        ..SchemaConfig::default()
    };
    let state = Arc::new(
        GraphQLState::new(pool.clone(), Arc::new(cache), config)
            .expect("failed to build GraphQL schema"),
    );

    // Both tables must be reachable: the default schema keeps the bare name,
    // the other is prefixed with its schema.
    let other_field = format!("{}_widgets", other_schema);
    let sdl = state.schema.sdl();
    assert!(
        sdl.contains("widgets"),
        "default-schema table missing from the schema"
    );
    assert!(
        sdl.contains(&other_field),
        "second-schema table missing; expected a field named {} in:\n{}",
        other_field,
        sdl.lines()
            .filter(|l| l.contains("idgets"))
            .collect::<Vec<_>>()
            .join("\n")
    );

    // And each must read from its own table, not the same one twice.
    let ctx_schema = default_schema.clone();
    let default_rows = execute_ok(&state, &pool, &ctx_schema, "{ widgets { id marker } }").await;
    assert_eq!(
        default_rows["widgets"][0]["marker"].as_str(),
        Some("from-default")
    );

    let other_rows = execute_ok(
        &state,
        &pool,
        &ctx_schema,
        &format!("{{ {} {{ id marker }} }}", other_field),
    )
    .await;
    assert_eq!(
        other_rows[other_field.as_str()][0]["marker"].as_str(),
        Some("from-other"),
        "the prefixed field must read the other schema's table"
    );

    drop_schema(&pool, &default_schema).await;
    drop_schema(&pool, &other_schema).await;
}

// ===========================================================================
// Filter operators
//
// Regression: `build_where_clause` silently skipped any operator it did not
// recognise, so an advertised-but-unimplemented operator (`in`, `isNull`)
// produced no condition at all and the query returned every row.
// ===========================================================================

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn filter_in_operator_matches_a_set() {
    let pool = connect().await;
    let schema = unique_schema_name("filterin");
    create_widgets_schema(&pool, &schema).await;

    let state = build_state(&pool, &schema, None, false).await;
    let data = execute_ok(
        &state,
        &pool,
        &schema,
        "{ widgets(where: {id: {_in: [1, 3]}}, order_by: [{id: asc}]) { id } }",
    )
    .await;

    assert_eq!(ids_of(&data, "widgets"), vec![1, 3]);

    drop_schema(&pool, &schema).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn filter_in_operator_with_an_empty_list_matches_nothing() {
    let pool = connect().await;
    let schema = unique_schema_name("filterinempty");
    create_widgets_schema(&pool, &schema).await;

    let state = build_state(&pool, &schema, None, false).await;
    let data = execute_ok(
        &state,
        &pool,
        &schema,
        "{ widgets(where: {id: {_in: []}}) { id } }",
    )
    .await;

    assert!(
        ids_of(&data, "widgets").is_empty(),
        "an empty `in` set must match nothing, not everything"
    );

    drop_schema(&pool, &schema).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn filter_is_null_operator_accepts_camel_case() {
    // The schema advertises `isNull`; the executor only understood `is_null`.
    let pool = connect().await;
    let schema = unique_schema_name("filterisnull");
    create_widgets_schema(&pool, &schema).await;
    // Two statements, issued separately: sqlx prepares each query, and a
    // prepared statement cannot carry multiple commands.
    pool.execute(format!("ALTER TABLE {}.widgets ADD COLUMN note TEXT", schema).as_str())
        .await
        .expect("alter failed");
    pool.execute(format!("UPDATE {}.widgets SET note = 'x' WHERE id = 1", schema).as_str())
        .await
        .expect("update failed");

    let state = build_state(&pool, &schema, None, false).await;

    let null_rows = execute_ok(
        &state,
        &pool,
        &schema,
        "{ widgets(where: {note: {_is_null: true}}, order_by: [{id: asc}]) { id } }",
    )
    .await;
    assert_eq!(ids_of(&null_rows, "widgets"), vec![2, 3, 4]);

    let non_null_rows = execute_ok(
        &state,
        &pool,
        &schema,
        "{ widgets(where: {note: {_is_null: false}}) { id } }",
    )
    .await;
    assert_eq!(ids_of(&non_null_rows, "widgets"), vec![1]);

    drop_schema(&pool, &schema).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn filter_rejects_an_unsupported_operator() {
    // Silently ignoring it would return every row.
    let pool = connect().await;
    let schema = unique_schema_name("filterbadop");
    create_widgets_schema(&pool, &schema).await;

    let state = build_state(&pool, &schema, None, false).await;
    let errors = execute_err(
        &state,
        &pool,
        &schema,
        "{ widgets(where: {stock: {_between: 5}}) { id } }",
    )
    .await;

    assert!(
        errors.contains("_between"),
        "expected a rejection, got: {}",
        errors
    );

    drop_schema(&pool, &schema).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn mutation_with_unsupported_where_operator_is_refused() {
    // Before the operator check, an unrecognised operator produced an empty
    // WHERE clause -- which for a delete meant every row.
    let pool = connect().await;
    let schema = unique_schema_name("mutbadop");
    create_widgets_schema(&pool, &schema).await;

    let state = build_state(&pool, &schema, None, false).await;
    let errors = execute_err(
        &state,
        &pool,
        &schema,
        "mutation { delete_widgets(where: {stock: {_between: 5}}) { returning { id } } }",
    )
    .await;
    assert!(
        errors.contains("_between"),
        "expected a rejection, got: {}",
        errors
    );

    let remaining: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM {}.widgets", schema))
        .fetch_one(&pool)
        .await
        .expect("count failed");
    assert_eq!(remaining, 4, "the table must be untouched");

    drop_schema(&pool, &schema).await;
}

// ===========================================================================
// By-PK mutations
//
// Regression: `updateXByPk` / `deleteXByPk` declared a free-form `where` and
// simply returned the first affected row, so they were bulk mutations wearing
// a by-key name.
// ===========================================================================

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn update_by_pk_targets_exactly_one_row() {
    let pool = connect().await;
    let schema = unique_schema_name("updbypk");
    create_widgets_schema(&pool, &schema).await;

    let state = build_state(&pool, &schema, None, false).await;
    let data = execute_ok(
        &state,
        &pool,
        &schema,
        "mutation { update_widgets_by_pk(pk_columns: {id: 2}, _set: {name: \"renamed\"}) { id name } }",
    )
    .await;

    let row = data.get("update_widgets_by_pk").expect("missing result");
    assert_eq!(row.get("id").and_then(|v| v.as_i64()), Some(2));
    assert_eq!(row.get("name").and_then(|v| v.as_str()), Some("renamed"));

    let renamed: i64 = sqlx::query_scalar(&format!(
        "SELECT count(*) FROM {}.widgets WHERE name = 'renamed'",
        schema
    ))
    .fetch_one(&pool)
    .await
    .expect("count failed");
    assert_eq!(renamed, 1, "a by-PK update must touch exactly one row");

    drop_schema(&pool, &schema).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn delete_by_pk_removes_exactly_one_row() {
    let pool = connect().await;
    let schema = unique_schema_name("delbypk");
    create_widgets_schema(&pool, &schema).await;

    let state = build_state(&pool, &schema, None, false).await;
    let data = execute_ok(
        &state,
        &pool,
        &schema,
        "mutation { delete_widgets_by_pk(id: 3) { id name } }",
    )
    .await;

    let row = data.get("delete_widgets_by_pk").expect("missing result");
    assert_eq!(row.get("id").and_then(|v| v.as_i64()), Some(3));

    let remaining: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM {}.widgets", schema))
        .fetch_one(&pool)
        .await
        .expect("count failed");
    assert_eq!(remaining, 3, "a by-PK delete must remove exactly one row");

    drop_schema(&pool, &schema).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn by_pk_mutations_require_the_key_and_reject_where() {
    let pool = connect().await;
    let schema = unique_schema_name("bypkargs");
    create_widgets_schema(&pool, &schema).await;

    let state = build_state(&pool, &schema, None, false).await;

    // The key argument is required.
    let missing = execute_err(
        &state,
        &pool,
        &schema,
        "mutation { delete_widgets_by_pk { id } }",
    )
    .await;
    assert!(
        !missing.is_empty(),
        "a by-PK delete without its key must fail"
    );

    // `where` is no longer part of a by-PK mutation's signature.
    let rejected = execute_err(
        &state,
        &pool,
        &schema,
        "mutation { delete_widgets_by_pk(where: {id: {_eq: 1}}) { id } }",
    )
    .await;
    assert!(
        rejected.contains("where"),
        "expected `where` to be rejected on a by-PK mutation, got: {}",
        rejected
    );

    let remaining: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM {}.widgets", schema))
        .fetch_one(&pool)
        .await
        .expect("count failed");
    assert_eq!(remaining, 4, "nothing should have been deleted");

    drop_schema(&pool, &schema).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn by_pk_mutation_with_an_unknown_key_affects_nothing() {
    let pool = connect().await;
    let schema = unique_schema_name("bypkmiss");
    create_widgets_schema(&pool, &schema).await;

    let state = build_state(&pool, &schema, None, false).await;
    let data = execute_ok(
        &state,
        &pool,
        &schema,
        "mutation { delete_widgets_by_pk(id: 9999) { id } }",
    )
    .await;

    assert_eq!(
        data.get("delete_widgets_by_pk"),
        Some(&serde_json::Value::Null),
        "a key that matches nothing must resolve to null"
    );

    let remaining: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM {}.widgets", schema))
        .fetch_one(&pool)
        .await
        .expect("count failed");
    assert_eq!(remaining, 4);

    drop_schema(&pool, &schema).await;
}

// ===========================================================================
// Relationship embedding
//
// Relationship metadata was generated but never wired into the schema, so
// nested fields did not exist at all.
// ===========================================================================

/// A schema with a parent/child pair joined by a foreign key.
async fn create_related_schema(pool: &PgPool, schema: &str) {
    let _ = pool
        .execute(format!("DROP SCHEMA IF EXISTS {} CASCADE", schema).as_str())
        .await;
    pool.execute(format!("CREATE SCHEMA {}", schema).as_str())
        .await
        .expect("create schema failed");

    for stmt in [
        format!(
            "CREATE TABLE {}.authors (id SERIAL PRIMARY KEY, name TEXT NOT NULL)",
            schema
        ),
        format!(
            "CREATE TABLE {}.books (id SERIAL PRIMARY KEY, title TEXT NOT NULL, \
             author_id INTEGER NOT NULL REFERENCES {}.authors(id))",
            schema, schema
        ),
        format!(
            "CREATE TABLE {}.chapters (id SERIAL PRIMARY KEY, heading TEXT NOT NULL, \
             book_id INTEGER NOT NULL REFERENCES {}.books(id))",
            schema, schema
        ),
        format!(
            "INSERT INTO {}.authors (name) VALUES ('ada'), ('grace'), ('lonely')",
            schema
        ),
        format!(
            "INSERT INTO {}.books (title, author_id) VALUES \
             ('a-one', 1), ('a-two', 1), ('g-one', 2)",
            schema
        ),
        format!(
            "INSERT INTO {}.chapters (heading, book_id) VALUES \
             ('a-one-c1', 1), ('a-one-c2', 1), ('g-one-c1', 3)",
            schema
        ),
    ] {
        pool.execute(stmt.as_str()).await.expect("setup failed");
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn to_many_relationship_is_embedded() {
    let pool = connect().await;
    let schema = unique_schema_name("relmany");
    create_related_schema(&pool, &schema).await;

    let state = build_state(&pool, &schema, None, false).await;
    let data = execute_ok(
        &state,
        &pool,
        &schema,
        "{ authors(order_by: [{id: asc}]) { id name books { id title } } }",
    )
    .await;

    let authors = data["authors"].as_array().expect("expected a list");
    assert_eq!(authors.len(), 3);

    let ada_books = authors[0]["books"]
        .as_array()
        .expect("books must be a list");
    assert_eq!(ada_books.len(), 2, "ada has two books");
    assert_eq!(ada_books[0]["title"].as_str(), Some("a-one"));

    assert_eq!(
        authors[2]["books"],
        serde_json::json!([]),
        "an author with no books must get an empty list, not null"
    );

    drop_schema(&pool, &schema).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn to_one_relationship_is_embedded() {
    let pool = connect().await;
    let schema = unique_schema_name("relone");
    create_related_schema(&pool, &schema).await;

    let state = build_state(&pool, &schema, None, false).await;
    let data = execute_ok(
        &state,
        &pool,
        &schema,
        "{ books(order_by: [{id: asc}]) { id title author { id name } } }",
    )
    .await;

    let books = data["books"].as_array().expect("expected a list");
    assert_eq!(books.len(), 3);
    assert_eq!(
        books[0]["author"]["name"].as_str(),
        Some("ada"),
        "a to-one relationship must resolve to its single parent"
    );
    assert_eq!(books[2]["author"]["name"].as_str(), Some("grace"));

    drop_schema(&pool, &schema).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn nested_relationships_recurse_two_levels() {
    let pool = connect().await;
    let schema = unique_schema_name("relnest");
    create_related_schema(&pool, &schema).await;

    let state = build_state(&pool, &schema, None, false).await;
    let data = execute_ok(
        &state,
        &pool,
        &schema,
        "{ authors(order_by: [{id: asc}]) { id books { id chapters { id heading } } } }",
    )
    .await;

    let authors = data["authors"].as_array().unwrap();
    let ada_books = authors[0]["books"].as_array().unwrap();
    let first_book_chapters = ada_books[0]["chapters"].as_array().expect("chapters list");

    assert_eq!(first_book_chapters.len(), 2, "a-one has two chapters");
    assert_eq!(first_book_chapters[0]["heading"].as_str(), Some("a-one-c1"));
    assert_eq!(
        ada_books[1]["chapters"],
        serde_json::json!([]),
        "a-two has no chapters"
    );

    drop_schema(&pool, &schema).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn embedding_works_from_a_by_pk_query() {
    let pool = connect().await;
    let schema = unique_schema_name("relbypk");
    create_related_schema(&pool, &schema).await;

    let state = build_state(&pool, &schema, None, false).await;
    let data = execute_ok(
        &state,
        &pool,
        &schema,
        "{ authors_by_pk(id: 1) { id name books { title } } }",
    )
    .await;

    let books = data["authors_by_pk"]["books"]
        .as_array()
        .expect("books list");
    assert_eq!(books.len(), 2);

    drop_schema(&pool, &schema).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn relationship_fields_appear_in_the_schema() {
    let pool = connect().await;
    let schema = unique_schema_name("relsdl");
    create_related_schema(&pool, &schema).await;

    let state = build_state(&pool, &schema, None, false).await;
    let sdl = state.schema.sdl();

    assert!(
        // The field carries the four arguments an embedded list takes, so it
        // is matched by what it answers with rather than by its whole
        // signature.
        sdl.contains("): [books!]!"),
        "expected a to-many relationship field on authors, got:\n{}",
        sdl.lines()
            .filter(|l| l.contains("book") || l.contains("author"))
            .collect::<Vec<_>>()
            .join("\n")
    );

    drop_schema(&pool, &schema).await;
}
