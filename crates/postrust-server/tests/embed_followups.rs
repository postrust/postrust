//! Embed edge cases PostgREST answers and Postrust used to refuse: a bare
//! `count` inside an embed, a junction keyed on several columns whose parent
//! select leaves a key column out, and a missing table named before its
//! embeds are looked at.
//!
//! `DATABASE_URL` must name a superuser on a database with
//! `scripts/init-db.sql` loaded (for `web_anon`). Run with:
//!
//! ```text
//! cargo test --package postrust-server --test embed_followups -- --ignored
//! cargo test --package postrust-server --features pg-socket --test embed_followups -- --ignored
//! ```

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use postrust_core::AppConfig;
use postrust_server::{Builder, Host};
use serde_json::{json, Value};
use sqlx::postgres::PgPoolOptions;
use sqlx::AssertSqlSafe;
use tower::ServiceExt;

const SCHEMA: &str = "embed_followups";

fn database_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/postrust_test".to_string())
}

async fn schema() {
    let s = SCHEMA;
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url())
        .await
        .unwrap();
    let statements = [
        format!("DROP SCHEMA IF EXISTS {s} CASCADE"),
        format!("CREATE SCHEMA {s}"),
        format!("GRANT USAGE ON SCHEMA {s} TO web_anon"),
        // Bare `count` in an embed.
        format!("CREATE TABLE {s}.clients (id int PRIMARY KEY, name text)"),
        format!(
            "CREATE TABLE {s}.projects (id int PRIMARY KEY, name text, \
             client_id int REFERENCES {s}.clients(id))"
        ),
        format!("INSERT INTO {s}.clients VALUES (1, 'Microsoft'), (2, 'Apple')"),
        format!("INSERT INTO {s}.projects VALUES (1, 'a', 1), (2, 'b', 1), (3, 'c', 2), (4, 'd', 2)"),
        // A child with a real column called `count`: the column wins.
        format!("CREATE TABLE {s}.tallies (id int PRIMARY KEY, count int, client_id int REFERENCES {s}.clients(id))"),
        format!("INSERT INTO {s}.tallies VALUES (1, 7, 1)"),
        // A composite-keyed parent, a junction, and a child.
        format!(
            "CREATE TABLE {s}.users_tasks (user_id int NOT NULL, task_id int NOT NULL, \
             PRIMARY KEY (task_id, user_id))"
        ),
        format!(
            "CREATE TABLE {s}.files (project_id int NOT NULL, filename text NOT NULL, \
             content text NOT NULL, PRIMARY KEY (project_id, filename))"
        ),
        format!(
            "CREATE TABLE {s}.touched_files (user_id int NOT NULL, task_id int NOT NULL, \
             project_id int NOT NULL, filename text NOT NULL, \
             CONSTRAINT fk_users_tasks FOREIGN KEY (user_id, task_id) \
               REFERENCES {s}.users_tasks (user_id, task_id), \
             CONSTRAINT fk_upload FOREIGN KEY (project_id, filename) \
               REFERENCES {s}.files (project_id, filename), \
             PRIMARY KEY (user_id, task_id, project_id, filename))"
        ),
        format!("INSERT INTO {s}.users_tasks VALUES (1, 1), (1, 2)"),
        format!("INSERT INTO {s}.files VALUES (1, 'a.txt', 'A'), (1, 'b.txt', 'B'), (1, 'c.txt', 'C')"),
        format!(
            "INSERT INTO {s}.touched_files VALUES (1, 1, 1, 'a.txt'), (1, 1, 1, 'b.txt'), \
             (1, 2, 1, 'c.txt')"
        ),
        // A partition is not a table to the schema cache, though its parent is.
        format!("CREATE TABLE {s}.car_models (id int PRIMARY KEY, name text)"),
        format!(
            "CREATE TABLE {s}.car_model_sales (id int NOT NULL, name text, \
             model int REFERENCES {s}.car_models(id), date date NOT NULL) \
             PARTITION BY RANGE (date)"
        ),
        format!(
            "CREATE TABLE {s}.car_model_sales_202101 PARTITION OF {s}.car_model_sales \
             FOR VALUES FROM ('2021-01-01') TO ('2021-02-01')"
        ),
        format!("GRANT SELECT ON ALL TABLES IN SCHEMA {s} TO web_anon"),
    ];
    for statement in statements {
        sqlx::query(AssertSqlSafe(statement))
            .execute(&pool)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
    }
}

async fn router(compat_mode: bool) -> Router {
    let mut config = AppConfig::default();
    config.db_uri = database_url();
    config.db_schemas = vec![SCHEMA.to_string()];
    config.db_anon_role = Some("web_anon".to_string());
    config.db_pool_size = 2;
    config.compat_mode = compat_mode;
    Builder::new(config, Host::SERVER)
        .build()
        .await
        .unwrap()
        .router
}

async fn get(router: &Router, compat_mode: bool, path: &str) -> (StatusCode, Value) {
    let prefix = if compat_mode { "" } else { "/api" };
    let request = Request::get(format!("{prefix}{path}"))
        .body(Body::empty())
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn embed_followups_match_postgrest() {
    schema().await;

    for compat in [true, false] {
        let router = router(compat).await;
        let ask = |path: &'static str| {
            let router = router.clone();
            async move { get(&router, compat, path).await }
        };

        // G1: a bare `count` in an embed counts the embed's rows, and is not
        // refused for want of db-aggregates-enabled.
        let (status, body) = ask("/clients?select=name,projects(count)&order=id").await;
        assert_eq!(status, StatusCode::OK, "compat={compat} {body}");
        assert_eq!(
            body,
            json!([
                {"name": "Microsoft", "projects": [{"count": 2}]},
                {"name": "Apple", "projects": [{"count": 2}]}
            ]),
            "compat={compat}"
        );
        // A real column called `count` wins over the aggregate.
        let (status, body) = ask("/clients?select=name,tallies(count)&order=id").await;
        assert_eq!(status, StatusCode::OK, "compat={compat} {body}");
        assert_eq!(
            body,
            json!([
                {"name": "Microsoft", "tallies": [{"count": 7}]},
                {"name": "Apple", "tallies": []}
            ]),
            "compat={compat}"
        );

        // G2: the junction is correlated on both key columns, though the
        // root select names only one of them.
        for path in [
            "/users_tasks?user_id=eq.1&task_id=eq.1&select=user_id,files(filename,content)&files.order=filename",
            "/users_tasks?user_id=eq.1&task_id=eq.1&select=user_id,files!touched_files(filename,content)&files.order=filename",
        ] {
            let (status, body) = ask(path).await;
            assert_eq!(status, StatusCode::OK, "compat={compat} {path} {body}");
            assert_eq!(
                body,
                json!([{"user_id": 1, "files": [
                    {"filename": "a.txt", "content": "A"},
                    {"filename": "b.txt", "content": "B"}
                ]}]),
                "compat={compat} {path}"
            );
        }
        // Selecting every key column already worked and still does.
        let (status, body) =
            ask("/users_tasks?user_id=eq.1&task_id=eq.2&select=user_id,task_id,files(filename)")
                .await;
        assert_eq!(status, StatusCode::OK, "compat={compat} {body}");
        assert_eq!(
            body,
            json!([{"user_id": 1, "task_id": 2, "files": [{"filename": "c.txt"}]}]),
            "compat={compat}"
        );

        // G3: a partition is no table: the table is reported, not the embed.
        let (status, body) =
            ask("/car_model_sales_202101?select=id,name,car_models(id,name)").await;
        assert_eq!(status, StatusCode::NOT_FOUND, "compat={compat} {body}");
        assert_eq!(body["code"], "PGRST205", "compat={compat} {body}");
        assert_eq!(
            body["message"],
            format!(
                "Could not find the table '{SCHEMA}.car_model_sales_202101' in the schema cache"
            ),
            "compat={compat}"
        );
        assert_eq!(body["hint"], Value::Null, "compat={compat} {body}");
    }
}
