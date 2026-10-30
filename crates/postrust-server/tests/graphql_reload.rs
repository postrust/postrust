//! `NOTIFY pgrst, 'reload schema'` reaches GraphQL as well as REST.
//!
//! The listener reloaded the REST schema cache and nothing else: the GraphQL
//! schema had been built from a snapshot taken at start-up, so a table a
//! migration added was served over REST and was "not found in type
//! query_root" over GraphQL until the server was restarted.
//!
//! The database must have `scripts/init-db.sql` loaded and `DATABASE_URL` must
//! name a role that can create a table and become `web_anon`:
//!
//! ```text
//! cargo test --package postrust-server --features graphql --test graphql_reload -- --ignored
//! ```

#![cfg(feature = "graphql")]

use axum::body::Body;
use axum::http::Request;
use axum::Router;
use postrust_core::AppConfig;
use postrust_server::{build, Host};
use serde_json::{json, Value};
use sqlx::AssertSqlSafe;
use tower::ServiceExt;

const TABLE: &str = "graphql_reload_probe";
const CHANNEL: &str = "graphql_reload_test";

fn database_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/postrust_test".into())
}

async fn query(app: &Router, document: &str) -> Value {
    let response = app
        .clone()
        .oneshot(
            Request::post("/v1/graphql")
                .header("content-type", "application/json")
                .body(Body::from(json!({ "query": document }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&body).unwrap()
}

/// Announce a migration. Sent on each poll rather than once: the reloader
/// starts listening on a background task, and a notification sent before it
/// has is heard by no one.
async fn reload(admin: &sqlx::PgPool, channel: &str) {
    sqlx::query(AssertSqlSafe(format!("NOTIFY {channel}, 'reload schema'")))
        .execute(admin)
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_table_added_after_start_up_is_served_once_notified() {
    let admin = sqlx::PgPool::connect(&database_url()).await.unwrap();
    sqlx::query(AssertSqlSafe(format!(
        "DROP TABLE IF EXISTS public.{TABLE}"
    )))
    .execute(&admin)
    .await
    .unwrap();

    let mut config = AppConfig::default();
    config.db_uri = database_url();
    config.db_schemas = vec!["public".to_string()];
    config.db_anon_role = Some("web_anon".to_string());
    config.db_channel_enabled = true;
    config.db_channel = CHANNEL.to_string();
    let app = build(config, Host::SERVER).await.expect("build").router;

    let document = format!("{{ {TABLE} {{ id }} }}");
    let before = query(&app, &document).await;
    assert!(before["data"].is_null(), "{before}");

    for statement in [
        format!("CREATE TABLE public.{TABLE} (id int PRIMARY KEY)"),
        format!("INSERT INTO public.{TABLE} VALUES (1)"),
        format!("GRANT SELECT ON public.{TABLE} TO web_anon"),
    ] {
        sqlx::query(AssertSqlSafe(statement))
            .execute(&admin)
            .await
            .unwrap();
    }

    let mut after = Value::Null;
    for _ in 0..50 {
        reload(&admin, CHANNEL).await;
        after = query(&app, &document).await;
        if after["data"][TABLE] == json!([{ "id": 1 }]) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(after["data"][TABLE], json!([{ "id": 1 }]), "{after}");

    sqlx::query(AssertSqlSafe(format!("DROP TABLE public.{TABLE}")))
        .execute(&admin)
        .await
        .unwrap();
}

/// A schema that could not be built at start-up comes up on the reload after
/// the migration that fixes it, rather than only on a restart.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_schema_that_failed_at_start_up_is_built_on_reload() {
    const SCHEMA: &str = "graphql_reload_broken";
    const CHANNEL: &str = "graphql_reload_broken_test";
    let admin = sqlx::PgPool::connect(&database_url()).await.unwrap();
    for statement in [
        format!("DROP SCHEMA IF EXISTS {SCHEMA} CASCADE"),
        format!("CREATE SCHEMA {SCHEMA}"),
        format!("GRANT USAGE ON SCHEMA {SCHEMA} TO web_anon"),
        format!("CREATE TABLE {SCHEMA}.probe (id int PRIMARY KEY)"),
        format!("INSERT INTO {SCHEMA}.probe VALUES (1)"),
        format!("GRANT SELECT ON {SCHEMA}.probe TO web_anon"),
        // Its object type would take the name of the built-in Int scalar.
        format!("CREATE TABLE {SCHEMA}.\"Int\" (id int PRIMARY KEY)"),
    ] {
        sqlx::query(AssertSqlSafe(statement))
            .execute(&admin)
            .await
            .unwrap();
    }

    let mut config = AppConfig::default();
    config.db_uri = database_url();
    config.db_schemas = vec![SCHEMA.to_string()];
    config.db_anon_role = Some("web_anon".to_string());
    config.db_channel_enabled = true;
    config.db_channel = CHANNEL.to_string();
    let app = build(config, Host::SERVER).await.expect("build").router;

    let before = query(&app, "{ probe { id } }").await;
    assert_eq!(
        before["errors"][0]["extensions"]["code"], "unexpected",
        "{before}"
    );

    sqlx::query(AssertSqlSafe(format!("DROP TABLE {SCHEMA}.\"Int\"")))
        .execute(&admin)
        .await
        .unwrap();

    let mut after = Value::Null;
    for _ in 0..50 {
        reload(&admin, CHANNEL).await;
        after = query(&app, "{ probe { id } }").await;
        if after["data"]["probe"] == json!([{ "id": 1 }]) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(after["data"]["probe"], json!([{ "id": 1 }]), "{after}");

    sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {SCHEMA} CASCADE")))
        .execute(&admin)
        .await
        .unwrap();
}
