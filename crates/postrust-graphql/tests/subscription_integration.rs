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
    pool.execute(
        format!("DROP TABLE IF EXISTS {}.{} CASCADE", TEST_SCHEMA, table_name).as_str(),
    )
    .await?;

    // Create test table
    pool.execute(
        format!(
            r#"
            CREATE TABLE {}.{} (
                id SERIAL PRIMARY KEY,
                name TEXT NOT NULL,
                value INTEGER DEFAULT 0,
                created_at TIMESTAMPTZ DEFAULT NOW()
            )
            "#,
            TEST_SCHEMA, table_name
        )
        .as_str(),
    )
    .await?;

    // Create notify trigger
    let trigger_sql = create_notify_trigger_sql(TEST_SCHEMA, table_name);
    pool.execute(trigger_sql.as_str()).await?;

    Ok(())
}

/// Cleanup test table and trigger.
async fn cleanup_test_table(pool: &sqlx::PgPool, table_name: &str) -> Result<(), sqlx::Error> {
    let drop_trigger_sql = drop_notify_trigger_sql(TEST_SCHEMA, table_name);
    pool.execute(drop_trigger_sql.as_str()).await?;

    pool.execute(
        format!("DROP TABLE IF EXISTS {}.{} CASCADE", TEST_SCHEMA, table_name).as_str(),
    )
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
    setup_test_table(&pool, &test_table).await.expect("Failed to setup test table");

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
    sqlx::query(&format!(
        "INSERT INTO {}.{} (name, value) VALUES ($1, $2)",
        TEST_SCHEMA, test_table
    ))
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

    let payload = TableChangePayload::from_payload(&notification.payload)
        .expect("Failed to parse payload");

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
    cleanup_test_table(&pool, &test_table).await.expect("Failed to cleanup");
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
    setup_test_table(&pool, &test_table).await.expect("Failed to setup test table");

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
    let row: (i32,) = sqlx::query_as(&format!(
        "INSERT INTO {}.{} (name, value) VALUES ($1, $2) RETURNING id",
        TEST_SCHEMA, test_table
    ))
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
    sqlx::query(&format!(
        "UPDATE {}.{} SET value = $1 WHERE id = $2",
        TEST_SCHEMA, test_table
    ))
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

    let payload = TableChangePayload::from_payload(&notification.payload)
        .expect("Failed to parse payload");

    assert_eq!(payload.operation, "UPDATE");
    assert!(payload.old.is_some());
    assert!(payload.new.is_some());

    let old_data = payload.old.unwrap();
    let new_data = payload.new.unwrap();

    assert_eq!(old_data["value"], 10);
    assert_eq!(new_data["value"], 100);

    broker.stop().await;
    cleanup_test_table(&pool, &test_table).await.expect("Failed to cleanup");
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
    setup_test_table(&pool, &test_table).await.expect("Failed to setup test table");

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
    let row: (i32,) = sqlx::query_as(&format!(
        "INSERT INTO {}.{} (name, value) VALUES ($1, $2) RETURNING id",
        TEST_SCHEMA, test_table
    ))
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
    sqlx::query(&format!(
        "DELETE FROM {}.{} WHERE id = $1",
        TEST_SCHEMA, test_table
    ))
    .bind(inserted_id)
    .execute(&pool)
    .await
    .expect("Failed to delete row");

    // Wait for notification
    let notification = timeout(Duration::from_secs(5), stream.next())
        .await
        .expect("Timeout waiting for notification")
        .expect("Stream ended unexpectedly");

    let payload = TableChangePayload::from_payload(&notification.payload)
        .expect("Failed to parse payload");

    assert_eq!(payload.operation, "DELETE");
    assert!(payload.old.is_some());
    assert!(payload.new.is_none());

    // Verify data() returns old for DELETE
    let data = payload.data().expect("Should have data for DELETE");
    assert_eq!(data["name"], "delete_test");
    assert_eq!(data["value"], 999);

    broker.stop().await;
    cleanup_test_table(&pool, &test_table).await.expect("Failed to cleanup");
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
    setup_test_table(&pool, &test_table).await.expect("Failed to setup test table");

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
    sqlx::query(&format!(
        "INSERT INTO {}.{} (name, value) VALUES ($1, $2)",
        TEST_SCHEMA, test_table
    ))
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
    cleanup_test_table(&pool, &test_table).await.expect("Failed to cleanup");
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
    setup_test_table(&pool, &test_table).await.expect("Failed to setup test table");

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
    sqlx::query(&format!(
        "INSERT INTO {}.{} (name, value) VALUES ($1, $2)",
        TEST_SCHEMA, test_table
    ))
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
    cleanup_test_table(&pool, &test_table).await.expect("Failed to cleanup");
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
    pool.execute(
        format!("DROP TABLE IF EXISTS {}.trigger_test CASCADE", TEST_SCHEMA).as_str(),
    )
    .await
    .expect("Failed to drop table");

    pool.execute(
        format!(
            "CREATE TABLE {}.trigger_test (id SERIAL PRIMARY KEY, data TEXT)",
            TEST_SCHEMA
        )
        .as_str(),
    )
    .await
    .expect("Failed to create table");

    // Apply trigger SQL - this verifies the SQL is syntactically correct
    let trigger_sql = create_notify_trigger_sql(TEST_SCHEMA, "trigger_test");
    pool.execute(trigger_sql.as_str())
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
    pool.execute(drop_sql.as_str())
        .await
        .expect("Failed to drop trigger");

    pool.execute(format!("DROP TABLE {}.trigger_test CASCADE", TEST_SCHEMA).as_str())
        .await
        .expect("Failed to drop table");
}
