//! A schema cache survives being written out and read back.
//!
//! A host that cannot load the cache on every cold start -- a Cloudflare
//! Worker -- keeps a serialized copy and reads it back (issue #50), so what
//! comes back has to be what went in. Checked on a cache loaded from a real
//! database, because a hand-built one only exercises what its author thought
//! of.
//!
//! The database must have `scripts/init-db.sql` and `scripts/test-fixtures.sql`
//! loaded. Set `DATABASE_URL` to point at it, then run:
//!
//! ```text
//! cargo test --package postrust-server --test schema_cache_serde -- --ignored
//! ```

use postrust_core::SchemaCache;
use serde_json::Value;

fn database_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/postrust_test".to_string())
}

/// A JSON value with every array sorted, so two serializations of one cache
/// compare equal however their hash maps happened to iterate.
fn canonical(value: Value) -> Value {
    match value {
        Value::Array(items) => {
            let mut items: Vec<Value> = items.into_iter().map(canonical).collect();
            items.sort_by_key(|item| item.to_string());
            Value::Array(items)
        }
        Value::Object(map) => {
            Value::Object(map.into_iter().map(|(k, v)| (k, canonical(v))).collect())
        }
        other => other,
    }
}

#[tokio::test]
#[ignore = "needs a PostgreSQL database with the test fixtures loaded"]
async fn a_loaded_cache_round_trips_through_json() {
    let pool = sqlx::PgPool::connect(&database_url())
        .await
        .expect("failed to connect to test database");
    let schemas = vec!["public".to_string(), "api".to_string()];
    let cache = SchemaCache::load(&pool, &schemas).await.unwrap();
    assert!(!cache.tables.is_empty(), "the fixtures should have tables");
    assert!(
        !cache.relationships.is_empty(),
        "the fixtures should have relationships"
    );

    let written = serde_json::to_string(&cache).unwrap();
    let read: SchemaCache = serde_json::from_str(&written).unwrap();
    let rewritten = serde_json::to_string(&read).unwrap();

    assert_eq!(
        canonical(serde_json::from_str(&written).unwrap()),
        canonical(serde_json::from_str(&rewritten).unwrap()),
    );

    // Sorting arrays hides order, and a table's column order is part of what
    // it means -- it is the order `select=*` answers in. So that is compared
    // as it is.
    for (qi, table) in &cache.tables {
        let back = &read.tables[qi];
        assert!(
            table.columns.keys().eq(back.columns.keys()),
            "column order of {qi} changed"
        );
    }
    assert_eq!(read.pg_version, cache.pg_version);
    assert_eq!(read.timezones, cache.timezones);
}
