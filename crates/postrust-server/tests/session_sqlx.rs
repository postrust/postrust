//! A `Session` applied to a sqlx transaction of the caller's own.
//!
//! What GraphQL and a hand-written route do with a raw sqlx transaction: the
//! transaction runs as the session's role, with its settings readable, and
//! the connection goes back to the pool with none of it left behind.
//!
//! `DATABASE_URL` must name a superuser on a database with
//! `scripts/init-db.sql` loaded (for `web_anon`). Run with:
//!
//! ```text
//! cargo test --package postrust-server --test session_sqlx -- --ignored
//! ```

use postrust_core::{AppConfig, Session};
use sqlx::postgres::PgPoolOptions;

fn database_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/postrust_test".to_string())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_session_holds_for_the_transaction_and_no_longer() {
    // One connection, so the second transaction is on the first's connection.
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url())
        .await
        .expect("failed to connect to test database");
    let config = AppConfig::default();
    let claims = vec![(
        "request.jwt.claims".to_string(),
        r#"{"role":"web_anon","sub":"alice"}"#.to_string(),
    )];

    let mut tx = pool.begin().await.unwrap();
    Session::new(&config, "web_anon", claims)
        .request("POST", "/_/hooks")
        .apply_sqlx(&mut tx)
        .await
        .expect("the session applies");
    let (user, method, sub): (String, String, String) = sqlx::query_as(
        "SELECT current_user::text, current_setting('request.method'), \
         current_setting('request.jwt.claims')::json->>'sub'",
    )
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    assert_eq!(
        (user.as_str(), method.as_str(), sub.as_str()),
        ("web_anon", "POST", "alice")
    );
    tx.commit().await.unwrap();

    let (user, method): (String, Option<String>) = sqlx::query_as(
        "SELECT current_user::text, nullif(current_setting('request.method', true), '')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(user, "postgres", "the role ended with the transaction");
    assert_eq!(method, None, "and so did the settings");
}
