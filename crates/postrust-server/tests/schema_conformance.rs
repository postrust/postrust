//! Schema-cache and embedding fixes measured against PostgREST.
//!
//! Many-to-many embeds whose child shares a column name with the junction,
//! one candidate for a many-to-many through a view over an unexposed
//! junction, partition foreign keys that are not relationships, the key a
//! view inherits (`Location`, `Allow: PUT`), and a domain's own default.
//!
//! `DATABASE_URL` must name a superuser on a database with
//! `scripts/init-db.sql` loaded (for `web_anon`). Run with:
//!
//! ```text
//! cargo test --package postrust-server --test schema_conformance -- --ignored
//! ```

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::any;
use axum::Router;
use postrust_core::{AppConfig, SchemaCache};
use postrust_server::{handle_request, AppState};
use sqlx::postgres::PgPoolOptions;
use sqlx::AssertSqlSafe;
use sqlx::PgPool;
use std::sync::Arc;
use tower::ServiceExt;

/// Each test's own schema, so tests can run at the same time.
fn schema(test: &str) -> String {
    format!("schema_conf_{test}")
}

fn database_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/postrust_test".to_string())
}

async fn pool() -> PgPool {
    PgPoolOptions::new()
        .max_connections(2)
        .connect(&database_url())
        .await
        .expect("failed to connect to test database")
}

/// A fresh schema holding `statements`, with `{schema}` standing for it.
async fn setup(pool: &PgPool, test: &str, statements: &[&str]) {
    let schema = schema(test);
    let mut all = vec![
        format!("DROP SCHEMA IF EXISTS {schema} CASCADE"),
        format!("DROP SCHEMA IF EXISTS {schema}_priv CASCADE"),
        format!("CREATE SCHEMA {schema}"),
        format!("CREATE SCHEMA {schema}_priv"),
        format!("GRANT USAGE ON SCHEMA {schema} TO web_anon"),
    ];
    all.extend(statements.iter().map(|s| s.replace("{schema}", &schema)));
    all.push(format!(
        "GRANT ALL ON ALL TABLES IN SCHEMA {schema} TO web_anon"
    ));
    for statement in all {
        sqlx::query(AssertSqlSafe(statement))
            .execute(pool)
            .await
            .unwrap();
    }
}

async fn cache(pool: &PgPool, test: &str) -> SchemaCache {
    SchemaCache::load(pool, &[schema(test)])
        .await
        .expect("schema cache")
}

async fn app(pool: PgPool, test: &str) -> Router {
    let mut config = AppConfig::default();
    config.db_uri = database_url();
    config.db_schemas = vec![schema(test)];
    config.db_anon_role = Some("web_anon".to_string());
    let schema_cache = SchemaCache::load(&pool, &config.db_schemas)
        .await
        .expect("schema cache");
    let state = Arc::new({
        let mut state =
            AppState::new(pool, schema_cache, config).expect("the test's JWT secret parses");
        state.jwt_cache = None;
        state
    });
    let api: Router<Arc<AppState>> = Router::new()
        .route("/", any(handle_request))
        .route("/{*path}", any(handle_request));
    Router::new()
        .nest("/api", api)
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            postrust_server::app::options_allow,
        ))
        .with_state(state)
}

struct Answer {
    status: StatusCode,
    allow: Option<String>,
    location: Option<String>,
    body: String,
}

async fn send(app: &Router, method: &str, uri: &str, prefer: Option<&str>, body: &str) -> Answer {
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header("Content-Type", "application/json");
    if let Some(prefer) = prefer {
        request = request.header("Prefer", prefer);
    }
    let response = app
        .clone()
        .oneshot(request.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let header = |name: &str| {
        response
            .headers()
            .get(name)
            .map(|v| v.to_str().unwrap().to_string())
    };
    let allow = header("allow");
    let location = header("location");
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    Answer {
        status,
        allow,
        location,
        body: String::from_utf8_lossy(&body).into_owned(),
    }
}

async fn get(app: &Router, uri: &str) -> Answer {
    send(app, "GET", uri, None, "").await
}

fn json(answer: &Answer) -> serde_json::Value {
    serde_json::from_str(&answer.body).unwrap_or_else(|_| panic!("not JSON: {}", answer.body))
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_child_column_the_junction_shares_is_not_ambiguous() {
    let pool = pool().await;
    setup(
        &pool,
        "m2m",
        &[
            "CREATE TABLE {schema}.projects (id int PRIMARY KEY, name text)",
            "CREATE TABLE {schema}.jobs (id int PRIMARY KEY, name text)",
            // `name` is on the junction too.
            "CREATE TABLE {schema}.big_projects (
                 project_id int REFERENCES {schema}.projects,
                 job_id int REFERENCES {schema}.jobs,
                 name text,
                 PRIMARY KEY (project_id, job_id))",
            "INSERT INTO {schema}.projects VALUES (1, 'p1'), (2, 'p2')",
            "INSERT INTO {schema}.jobs VALUES (1, 'j1'), (2, 'j2')",
            "INSERT INTO {schema}.big_projects VALUES (1, 1, 'x'), (1, 2, 'y')",
        ],
    )
    .await;
    let app = app(pool, "m2m").await;

    // select
    let answer = get(&app, "/api/projects?select=name,jobs(name)&id=eq.1").await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);
    let jobs = &json(&answer)[0]["jobs"];
    assert_eq!(jobs.as_array().unwrap().len(), 2, "{}", answer.body);

    // filter on the embed
    let answer = get(
        &app,
        "/api/projects?select=name,jobs(name)&id=eq.1&jobs.name=eq.j2",
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);
    assert_eq!(
        json(&answer)[0]["jobs"],
        serde_json::json!([{"name": "j2"}])
    );

    // order inside the embed
    let answer = get(
        &app,
        "/api/projects?select=name,jobs(name)&id=eq.1&jobs.order=name.desc",
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);
    assert_eq!(
        json(&answer)[0]["jobs"],
        serde_json::json!([{"name": "j2"}, {"name": "j1"}])
    );

    // inner join with a filter,
    let answer = get(
        &app,
        "/api/projects?select=id,jobs!inner(name)&jobs.name=eq.j1",
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);
    assert_eq!(
        json(&answer).as_array().unwrap().len(),
        1,
        "{}",
        answer.body
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_junction_behind_a_view_is_one_candidate() {
    let pool = pool().await;
    setup(
        &pool,
        "dupe",
        &[
            "CREATE TABLE {schema}.actors (id int PRIMARY KEY)",
            "CREATE TABLE {schema}.roles (id int PRIMARY KEY)",
            // The junction is in a schema that is not exposed.
            "CREATE TABLE {schema}_priv.casting (
                 actor_id int REFERENCES {schema}.actors,
                 role_id int REFERENCES {schema}.roles,
                 PRIMARY KEY (actor_id, role_id))",
            "CREATE VIEW {schema}.casting AS SELECT * FROM {schema}_priv.casting",
            "INSERT INTO {schema}.actors VALUES (1)",
            "INSERT INTO {schema}.roles VALUES (1)",
            "INSERT INTO {schema}_priv.casting VALUES (1, 1)",
        ],
    )
    .await;
    let cache = cache(&pool, "dupe").await;
    let actors = postrust_core::api_request::QualifiedIdentifier::new(schema("dupe"), "actors");
    let to_roles = cache
        .get_relationships(&actors, &schema("dupe"))
        .unwrap()
        .iter()
        .filter(|rel| {
            rel.foreign_table().name == "roles" && rel.cardinality_name() == "many-to-many"
        })
        .count();
    assert_eq!(to_roles, 1);

    let app = app(pool, "dupe").await;
    let answer = get(&app, "/api/actors?select=id,roles(id)").await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_partitions_inherited_foreign_key_is_not_a_relationship() {
    let pool = pool().await;
    setup(
        &pool,
        "part",
        &[
            "CREATE TABLE {schema}.models (id int PRIMARY KEY, name text)",
            "CREATE TABLE {schema}.sales (id int, model int REFERENCES {schema}.models, d int)
                 PARTITION BY RANGE (d)",
            "CREATE TABLE {schema}.sales_p1 PARTITION OF {schema}.sales FOR VALUES FROM (0) TO (100)",
        ],
    )
    .await;
    let app = app(pool, "part").await;

    let answer = get(&app, "/api/models?select=id,sales(id)").await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);

    let answer = get(&app, "/api/models?select=id,sales_p1(id)").await;
    assert_eq!(answer.status, StatusCode::BAD_REQUEST, "{}", answer.body);
    let body = json(&answer);
    assert_eq!(body["code"], "PGRST200", "{}", answer.body);
    assert_eq!(
        body["hint"], "Perhaps you meant 'sales' instead of 'sales_p1'.",
        "{}",
        answer.body
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_view_has_the_key_of_the_tables_it_selects() {
    let pool = pool().await;
    setup(
        &pool,
        "viewpk",
        &[
            "CREATE TABLE {schema}.pk1 (pk1 int PRIMARY KEY)",
            "CREATE TABLE {schema}.pk2 (pk2 int PRIMARY KEY)",
            "CREATE TABLE {schema}.sponsors (id int PRIMARY KEY, sponsor_id int)",
            "CREATE VIEW {schema}.both_pks AS SELECT * FROM {schema}.pk1, {schema}.pk2",
            "CREATE FUNCTION {schema}.both_pks_insert() RETURNS trigger LANGUAGE plpgsql AS $$
             BEGIN
               INSERT INTO {schema}.pk1 VALUES (NEW.pk1);
               INSERT INTO {schema}.pk2 VALUES (NEW.pk2);
               RETURN NEW;
             END $$",
            "CREATE TRIGGER both_pks_insert INSTEAD OF INSERT ON {schema}.both_pks
                 FOR EACH ROW EXECUTE FUNCTION {schema}.both_pks_insert()",
            "CREATE VIEW {schema}.renamed AS SELECT id AS ident, sponsor_id FROM {schema}.sponsors",
            "CREATE VIEW {schema}.on_a_view AS SELECT * FROM {schema}.renamed",
            "CREATE VIEW {schema}.no_key AS SELECT sponsor_id FROM {schema}.sponsors",
        ],
    )
    .await;
    let cache = cache(&pool, "viewpk").await;
    let table = |name: &str| {
        cache
            .get_table(&postrust_core::api_request::QualifiedIdentifier::new(
                schema("viewpk"),
                name,
            ))
            .unwrap()
            .clone()
    };
    // Every base table's key, concatenated, in the order the view uses them.
    assert_eq!(table("both_pks").pk_cols, ["pk1", "pk2"]);
    // Through a rename, and through a view over a view.
    assert_eq!(table("renamed").pk_cols, ["ident"]);
    assert_eq!(table("on_a_view").pk_cols, ["ident"]);
    assert!(table("renamed").columns["ident"].is_pk);
    // A view that does not select the key has none.
    assert!(table("no_key").pk_cols.is_empty());

    // The created row's Location, from the derived key.
    sqlx::query(AssertSqlSafe(format!(
        "GRANT ALL ON ALL TABLES IN SCHEMA {} TO web_anon",
        schema("viewpk")
    )))
    .execute(&pool)
    .await
    .unwrap();
    let app = app(pool, "viewpk").await;
    let answer = send(
        &app,
        "POST",
        "/api/both_pks",
        Some("return=headers-only"),
        r#"{"pk1":1,"pk2":2}"#,
    )
    .await;
    assert_eq!(answer.status, StatusCode::CREATED, "{}", answer.body);
    assert_eq!(
        answer.location.as_deref(),
        Some("/both_pks?pk1=eq.1&pk2=eq.2")
    );

    // PUT is offered by an updatable view that has a key, and not otherwise.
    let allow = send(&app, "OPTIONS", "/api/renamed", None, "").await.allow;
    assert!(allow.unwrap().contains("PUT"));
    let allow = send(&app, "OPTIONS", "/api/no_key", None, "").await.allow;
    assert!(!allow.unwrap().contains("PUT"));
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_domains_default_is_the_columns_default() {
    let pool = pool().await;
    setup(
        &pool,
        "domain",
        &[
            "CREATE DOMAIN {schema}.devil_int AS int DEFAULT 666",
            "CREATE TABLE {schema}.evil_friends (id {schema}.devil_int PRIMARY KEY, name text)",
            "CREATE TABLE {schema}.plain (id serial PRIMARY KEY, n int)",
        ],
    )
    .await;
    let app = app(pool, "domain").await;
    let answer = send(
        &app,
        "POST",
        "/api/evil_friends?columns=id,name",
        Some("return=representation, missing=default"),
        r#"{"name":"Lu"}"#,
    )
    .await;
    assert_eq!(answer.status, StatusCode::CREATED, "{}", answer.body);
    assert_eq!(
        json(&answer),
        serde_json::json!([{"id": 666, "name": "Lu"}]),
        "{}",
        answer.body
    );
}
