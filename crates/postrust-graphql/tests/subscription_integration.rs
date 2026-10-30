//! Integration tests for PostgreSQL LISTEN/NOTIFY subscriptions.
//!
//! These tests require a running PostgreSQL database.
//! Run with: `cargo test --package postrust-graphql --test subscription_integration -- --ignored`
//!
//! Set DATABASE_URL environment variable to your test database connection string.

use futures::StreamExt;
use postrust_graphql::subscription::{
    create_notify_trigger_sql, drop_notify_trigger_sql, table_channel_name, NotifyBroker,
    TableChangePayload,
};
use sqlx::postgres::PgPoolOptions;
use sqlx::AssertSqlSafe;
use sqlx::Executor;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;
use tokio::time::timeout;

/// Get database URL from environment or use default.
fn get_database_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/postrust_test".to_string())
}

/// Test schema name.
const TEST_SCHEMA: &str = "public";

/// Counter for unique table names to avoid test conflicts.
static TABLE_COUNTER: AtomicU32 = AtomicU32::new(0);

/// Generate a unique table name for each test.
fn unique_table_name() -> String {
    let id = TABLE_COUNTER.fetch_add(1, Ordering::SeqCst);
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();
    format!("postrust_sub_test_{}_{}", timestamp, id)
}

/// Setup test table and trigger.
async fn setup_test_table(pool: &sqlx::PgPool, table_name: &str) -> Result<(), sqlx::Error> {
    // Drop existing table if exists
    pool.execute(AssertSqlSafe(format!(
        "DROP TABLE IF EXISTS {}.{} CASCADE",
        TEST_SCHEMA, table_name
    )))
    .await?;

    // Create test table
    pool.execute(AssertSqlSafe(format!(
        r#"
            CREATE TABLE {}.{} (
                id SERIAL PRIMARY KEY,
                name TEXT NOT NULL,
                value INTEGER DEFAULT 0,
                created_at TIMESTAMPTZ DEFAULT NOW()
            )
            "#,
        TEST_SCHEMA, table_name
    )))
    .await?;

    // Create notify trigger
    let trigger_sql = create_notify_trigger_sql(TEST_SCHEMA, table_name);
    pool.execute(AssertSqlSafe(trigger_sql)).await?;

    Ok(())
}

/// Cleanup test table and trigger.
async fn cleanup_test_table(pool: &sqlx::PgPool, table_name: &str) -> Result<(), sqlx::Error> {
    let drop_trigger_sql = drop_notify_trigger_sql(TEST_SCHEMA, table_name);
    pool.execute(AssertSqlSafe(drop_trigger_sql)).await?;

    pool.execute(AssertSqlSafe(format!(
        "DROP TABLE IF EXISTS {}.{} CASCADE",
        TEST_SCHEMA, table_name
    )))
    .await?;

    Ok(())
}

#[tokio::test]
#[ignore] // Requires running PostgreSQL database
async fn test_notify_broker_receives_insert() {
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&get_database_url())
        .await
        .expect("Failed to connect to database");

    let test_table = unique_table_name();

    // Setup
    setup_test_table(&pool, &test_table)
        .await
        .expect("Failed to setup test table");

    // Create and start broker
    let broker = NotifyBroker::new(pool.clone());
    let channel = table_channel_name(TEST_SCHEMA, &test_table);

    broker
        .start(vec![channel.clone()])
        .await
        .expect("Failed to start broker");

    // Give broker time to start listening
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Subscribe to notifications
    let mut stream = broker
        .subscribe(&channel)
        .await
        .expect("Failed to subscribe");

    // Insert a row (this should trigger a notification)
    sqlx::query(AssertSqlSafe(format!(
        "INSERT INTO {}.{} (name, value) VALUES ($1, $2)",
        TEST_SCHEMA, test_table
    )))
    .bind("test_user")
    .bind(42)
    .execute(&pool)
    .await
    .expect("Failed to insert row");

    // Wait for notification with timeout
    let notification = timeout(Duration::from_secs(5), stream.next())
        .await
        .expect("Timeout waiting for notification")
        .expect("Stream ended unexpectedly");

    // Verify notification
    assert_eq!(notification.channel, channel);

    let payload =
        TableChangePayload::from_payload(&notification.payload).expect("Failed to parse payload");

    assert_eq!(payload.operation, "INSERT");
    assert_eq!(payload.table, test_table);
    assert_eq!(payload.schema, TEST_SCHEMA);
    assert!(payload.new.is_some());
    assert!(payload.old.is_none());

    let new_data = payload.new.unwrap();
    assert_eq!(new_data["name"], "test_user");
    assert_eq!(new_data["value"], 42);

    // Cleanup
    broker.stop().await;
    cleanup_test_table(&pool, &test_table)
        .await
        .expect("Failed to cleanup");
}

#[tokio::test]
#[ignore] // Requires running PostgreSQL database
async fn test_notify_broker_receives_update() {
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&get_database_url())
        .await
        .expect("Failed to connect to database");

    let test_table = unique_table_name();
    setup_test_table(&pool, &test_table)
        .await
        .expect("Failed to setup test table");

    let broker = NotifyBroker::new(pool.clone());
    let channel = table_channel_name(TEST_SCHEMA, &test_table);

    broker
        .start(vec![channel.clone()])
        .await
        .expect("Failed to start broker");

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Subscribe first to catch all notifications
    let mut stream = broker
        .subscribe(&channel)
        .await
        .expect("Failed to subscribe");

    // Insert a row
    let row: (i32,) = sqlx::query_as(AssertSqlSafe(format!(
        "INSERT INTO {}.{} (name, value) VALUES ($1, $2) RETURNING id",
        TEST_SCHEMA, test_table
    )))
    .bind("update_test")
    .bind(10)
    .fetch_one(&pool)
    .await
    .expect("Failed to insert row");

    let inserted_id = row.0;

    // Consume the INSERT notification
    let insert_notification = timeout(Duration::from_secs(5), stream.next())
        .await
        .expect("Timeout waiting for INSERT notification")
        .expect("Stream ended unexpectedly");
    let insert_payload = TableChangePayload::from_payload(&insert_notification.payload).unwrap();
    assert_eq!(insert_payload.operation, "INSERT");

    // Update the row
    sqlx::query(AssertSqlSafe(format!(
        "UPDATE {}.{} SET value = $1 WHERE id = $2",
        TEST_SCHEMA, test_table
    )))
    .bind(100)
    .bind(inserted_id)
    .execute(&pool)
    .await
    .expect("Failed to update row");

    // Wait for notification
    let notification = timeout(Duration::from_secs(5), stream.next())
        .await
        .expect("Timeout waiting for notification")
        .expect("Stream ended unexpectedly");

    let payload =
        TableChangePayload::from_payload(&notification.payload).expect("Failed to parse payload");

    assert_eq!(payload.operation, "UPDATE");
    assert!(payload.old.is_some());
    assert!(payload.new.is_some());

    let old_data = payload.old.unwrap();
    let new_data = payload.new.unwrap();

    assert_eq!(old_data["value"], 10);
    assert_eq!(new_data["value"], 100);

    broker.stop().await;
    cleanup_test_table(&pool, &test_table)
        .await
        .expect("Failed to cleanup");
}

#[tokio::test]
#[ignore] // Requires running PostgreSQL database
async fn test_notify_broker_receives_delete() {
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&get_database_url())
        .await
        .expect("Failed to connect to database");

    let test_table = unique_table_name();
    setup_test_table(&pool, &test_table)
        .await
        .expect("Failed to setup test table");

    let broker = NotifyBroker::new(pool.clone());
    let channel = table_channel_name(TEST_SCHEMA, &test_table);

    broker
        .start(vec![channel.clone()])
        .await
        .expect("Failed to start broker");

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Subscribe first to catch all notifications
    let mut stream = broker
        .subscribe(&channel)
        .await
        .expect("Failed to subscribe");

    // Insert a row
    let row: (i32,) = sqlx::query_as(AssertSqlSafe(format!(
        "INSERT INTO {}.{} (name, value) VALUES ($1, $2) RETURNING id",
        TEST_SCHEMA, test_table
    )))
    .bind("delete_test")
    .bind(999)
    .fetch_one(&pool)
    .await
    .expect("Failed to insert row");

    let inserted_id = row.0;

    // Consume the INSERT notification
    let insert_notification = timeout(Duration::from_secs(5), stream.next())
        .await
        .expect("Timeout waiting for INSERT notification")
        .expect("Stream ended unexpectedly");
    let insert_payload = TableChangePayload::from_payload(&insert_notification.payload).unwrap();
    assert_eq!(insert_payload.operation, "INSERT");

    // Delete the row
    sqlx::query(AssertSqlSafe(format!(
        "DELETE FROM {}.{} WHERE id = $1",
        TEST_SCHEMA, test_table
    )))
    .bind(inserted_id)
    .execute(&pool)
    .await
    .expect("Failed to delete row");

    // Wait for notification
    let notification = timeout(Duration::from_secs(5), stream.next())
        .await
        .expect("Timeout waiting for notification")
        .expect("Stream ended unexpectedly");

    let payload =
        TableChangePayload::from_payload(&notification.payload).expect("Failed to parse payload");

    assert_eq!(payload.operation, "DELETE");
    assert!(payload.old.is_some());
    assert!(payload.new.is_none());

    // Verify data() returns old for DELETE
    let data = payload.data().expect("Should have data for DELETE");
    assert_eq!(data["name"], "delete_test");
    assert_eq!(data["value"], 999);

    broker.stop().await;
    cleanup_test_table(&pool, &test_table)
        .await
        .expect("Failed to cleanup");
}

#[tokio::test]
#[ignore] // Requires running PostgreSQL database
async fn test_notify_broker_multiple_subscribers() {
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&get_database_url())
        .await
        .expect("Failed to connect to database");

    let test_table = unique_table_name();
    setup_test_table(&pool, &test_table)
        .await
        .expect("Failed to setup test table");

    let broker = NotifyBroker::new(pool.clone());
    let channel = table_channel_name(TEST_SCHEMA, &test_table);

    broker
        .start(vec![channel.clone()])
        .await
        .expect("Failed to start broker");

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Create multiple subscribers
    let mut stream1 = broker
        .subscribe(&channel)
        .await
        .expect("Failed to subscribe 1");
    let mut stream2 = broker
        .subscribe(&channel)
        .await
        .expect("Failed to subscribe 2");

    // Insert a row
    sqlx::query(AssertSqlSafe(format!(
        "INSERT INTO {}.{} (name, value) VALUES ($1, $2)",
        TEST_SCHEMA, test_table
    )))
    .bind("multi_sub_test")
    .bind(123)
    .execute(&pool)
    .await
    .expect("Failed to insert row");

    // Both subscribers should receive the notification
    let notification1 = timeout(Duration::from_secs(5), stream1.next())
        .await
        .expect("Timeout waiting for notification 1")
        .expect("Stream 1 ended unexpectedly");

    let notification2 = timeout(Duration::from_secs(5), stream2.next())
        .await
        .expect("Timeout waiting for notification 2")
        .expect("Stream 2 ended unexpectedly");

    // Both should have the same payload
    assert_eq!(notification1.payload, notification2.payload);

    let payload = TableChangePayload::from_payload(&notification1.payload).unwrap();
    assert_eq!(payload.operation, "INSERT");
    assert_eq!(payload.new.unwrap()["name"], "multi_sub_test");

    broker.stop().await;
    cleanup_test_table(&pool, &test_table)
        .await
        .expect("Failed to cleanup");
}

#[tokio::test]
#[ignore] // Requires running PostgreSQL database
async fn test_notify_broker_dynamic_channel() {
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&get_database_url())
        .await
        .expect("Failed to connect to database");

    let test_table = unique_table_name();
    setup_test_table(&pool, &test_table)
        .await
        .expect("Failed to setup test table");

    let broker = NotifyBroker::new(pool.clone());
    let channel = table_channel_name(TEST_SCHEMA, &test_table);

    // Start broker with empty channels, then add dynamically
    broker.start(vec![]).await.expect("Failed to start broker");

    // Add channel dynamically
    broker
        .listen_channel(&channel)
        .await
        .expect("Failed to add dynamic channel");

    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut stream = broker.subscribe_or_create(&channel).await;

    // Insert a row
    sqlx::query(AssertSqlSafe(format!(
        "INSERT INTO {}.{} (name, value) VALUES ($1, $2)",
        TEST_SCHEMA, test_table
    )))
    .bind("dynamic_test")
    .bind(456)
    .execute(&pool)
    .await
    .expect("Failed to insert row");

    // Should receive notification on dynamically added channel
    let notification = timeout(Duration::from_secs(5), stream.next())
        .await
        .expect("Timeout waiting for notification")
        .expect("Stream ended unexpectedly");

    let payload = TableChangePayload::from_payload(&notification.payload).unwrap();
    assert_eq!(payload.operation, "INSERT");
    assert_eq!(payload.new.unwrap()["name"], "dynamic_test");

    broker.stop().await;
    cleanup_test_table(&pool, &test_table)
        .await
        .expect("Failed to cleanup");
}

#[tokio::test]
#[ignore] // Requires running PostgreSQL database
async fn test_trigger_sql_is_valid() {
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&get_database_url())
        .await
        .expect("Failed to connect to database");

    // Create a simple test table
    pool.execute(AssertSqlSafe(format!(
        "DROP TABLE IF EXISTS {}.trigger_test CASCADE",
        TEST_SCHEMA
    )))
    .await
    .expect("Failed to drop table");

    pool.execute(AssertSqlSafe(format!(
        "CREATE TABLE {}.trigger_test (id SERIAL PRIMARY KEY, data TEXT)",
        TEST_SCHEMA
    )))
    .await
    .expect("Failed to create table");

    // Apply trigger SQL - this verifies the SQL is syntactically correct
    let trigger_sql = create_notify_trigger_sql(TEST_SCHEMA, "trigger_test");
    pool.execute(AssertSqlSafe(trigger_sql))
        .await
        .expect("Failed to create trigger - SQL is invalid");

    // Verify trigger exists
    let row: (i64,) = sqlx::query_as(
        r#"
        SELECT COUNT(*) FROM pg_trigger
        WHERE tgname = 'postrust_notify_public_trigger_test'
        "#,
    )
    .fetch_one(&pool)
    .await
    .expect("Failed to query triggers");

    assert_eq!(row.0, 1, "Trigger should exist");

    // Verify function exists
    let row: (i64,) = sqlx::query_as(
        r#"
        SELECT COUNT(*) FROM pg_proc
        WHERE proname = 'postrust_notify_public_trigger_test_fn'
        "#,
    )
    .fetch_one(&pool)
    .await
    .expect("Failed to query functions");

    assert_eq!(row.0, 1, "Function should exist");

    // Cleanup
    let drop_sql = drop_notify_trigger_sql(TEST_SCHEMA, "trigger_test");
    pool.execute(AssertSqlSafe(drop_sql))
        .await
        .expect("Failed to drop trigger");

    pool.execute(AssertSqlSafe(format!(
        "DROP TABLE {}.trigger_test CASCADE",
        TEST_SCHEMA
    )))
    .await
    .expect("Failed to drop table");
}

/// The listening connections still open on `channel`, other than this one.
async fn listeners_on(pool: &sqlx::PgPool, channel: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM pg_stat_activity \
         WHERE pid <> pg_backend_pid() AND query ILIKE 'LISTEN%' AND query LIKE '%' || $1 || '%'",
    )
    .bind(channel)
    .fetch_one(pool)
    .await
    .unwrap()
}

#[tokio::test]
#[ignore] // Requires running PostgreSQL database
async fn test_stop_closes_a_quiet_listener() {
    // A stopped broker used to keep its connection until the next
    // notification arrived, which on a quiet channel is never.
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&get_database_url())
        .await
        .expect("Failed to connect to database");
    let channel = unique_table_name();

    let broker = NotifyBroker::new(pool.clone());
    broker.start(vec![channel.clone()]).await.unwrap();
    assert_eq!(listeners_on(&pool, &channel).await, 1);

    broker.stop().await;
    let mut open = 1;
    for _ in 0..50 {
        open = listeners_on(&pool, &channel).await;
        if open == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(open, 0, "the listener outlived stop()");
}

#[tokio::test]
#[ignore] // Requires running PostgreSQL database
async fn test_listen_adds_channels_without_doubling_existing_ones() {
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&get_database_url())
        .await
        .expect("Failed to connect to database");
    let first = unique_table_name();
    let second = unique_table_name();

    let broker = NotifyBroker::new(pool.clone());
    broker.start(vec![first.clone()]).await.unwrap();
    let mut on_first = broker.subscribe_or_create(&first).await;

    broker
        .listen(vec![first.clone(), second.clone()])
        .await
        .unwrap();
    assert_eq!(listeners_on(&pool, &first).await, 1);
    let mut on_second = broker.subscribe_or_create(&second).await;

    for channel in [&first, &second] {
        sqlx::query("SELECT pg_notify($1, 'ping')")
            .bind(channel)
            .execute(&pool)
            .await
            .unwrap();
    }
    for stream in [&mut on_first, &mut on_second] {
        let notification = timeout(Duration::from_secs(5), stream.next())
            .await
            .expect("Timeout waiting for notification")
            .unwrap();
        assert_eq!(notification.payload, "ping");
    }
    // One listener per channel, so one delivery per notification.
    assert!(
        timeout(Duration::from_millis(300), on_first.next())
            .await
            .is_err(),
        "a notification was delivered twice"
    );

    broker.stop().await;
}

#[tokio::test]
#[ignore] // Requires running PostgreSQL database
async fn test_a_rebuilt_state_keeps_the_broker_and_listens_on_new_tables() {
    use postrust_core::schema_cache::SchemaCache;
    use postrust_graphql::handler::GraphQLState;
    use postrust_graphql::schema::SchemaConfig;
    use std::sync::Arc;

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&get_database_url())
        .await
        .expect("Failed to connect to database");
    let schema = unique_table_name();
    pool.execute(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .await
        .unwrap();
    let table = |name: &str| {
        format!(
            "CREATE TABLE {schema}.{name} (id serial PRIMARY KEY, name text); {}",
            create_notify_trigger_sql(&schema, name)
        )
    };
    let state_for = |pool: sqlx::PgPool, schema: String| async move {
        let cache = SchemaCache::load(&pool, std::slice::from_ref(&schema))
            .await
            .unwrap();
        let mut config = SchemaConfig::default();
        config.exposed_schemas = vec![schema];
        config.enable_subscriptions = true;
        GraphQLState::new(pool, Arc::new(cache), config).unwrap()
    };

    pool.execute(AssertSqlSafe(table("before"))).await.unwrap();
    let mut first = state_for(pool.clone(), schema.clone()).await;
    first.continue_subscriptions(None).await.unwrap();
    let mut open = first
        .broker
        .read()
        .await
        .as_ref()
        .expect("a broker")
        .subscribe_or_create(&table_channel_name(&schema, "before"))
        .await;

    // A migration adds a table; the schema is rebuilt from the new cache.
    pool.execute(AssertSqlSafe(table("after"))).await.unwrap();
    let mut rebuilt = state_for(pool.clone(), schema.clone()).await;
    rebuilt.continue_subscriptions(Some(&first)).await.unwrap();
    assert!(Arc::ptr_eq(&first.broker, &rebuilt.broker));
    drop(first);

    let mut added = rebuilt
        .broker
        .read()
        .await
        .as_ref()
        .unwrap()
        .subscribe_or_create(&table_channel_name(&schema, "after"))
        .await;
    for name in ["before", "after"] {
        pool.execute(AssertSqlSafe(format!(
            "INSERT INTO {schema}.{name} (name) VALUES ('x')"
        )))
        .await
        .unwrap();
    }
    // The subscription opened before the rebuild still hears its table, and
    // the table the rebuild added is listened to.
    for stream in [&mut open, &mut added] {
        timeout(Duration::from_secs(5), stream.next())
            .await
            .expect("Timeout waiting for notification")
            .unwrap();
    }

    rebuilt.stop_subscriptions().await;
    pool.execute(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
        .await
        .unwrap();
}
