//! A route of your own, run as its caller: `User` and `AppState::begin_as`.
//!
//! A route nested with `Builder::nest` takes the caller as a `User`,
//! authenticated as REST authenticates, and runs its SQL in a transaction
//! from `begin_as` -- where a row-level security policy written for the
//! API's roles applies as it does to REST, and the request's settings read
//! back as REST's do. When it fails, it fails with REST's status and body.
//!
//! Each test has a schema of its own, dropped at the end, since the tests in
//! a binary run at once. `DATABASE_URL` must name a superuser on a database
//! with `scripts/init-db.sql` loaded (for `web_anon` and `web_user`). Run
//! with:
//!
//! ```text
//! cargo test --package postrust-server --test begin_as -- --ignored
//! ```

use axum::body::Body;
use axum::extract::{Query, State};
use axum::http::{Request, StatusCode};
use axum::response::Response;
use axum::routing::get;
use axum::{Json, Router};
use jsonwebtoken::{encode, EncodingKey, Header};
use postrust_core::AppConfig;
use postrust_server::{AppState, Builder, Error, Host, TxOptions, User};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use tower::ServiceExt;

const SECRET: &str = "reallyreallyreallyreallyverysafe";

fn database_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/postrust_test".to_string())
}

/// What a route is asked to run: one statement yielding one `json` value.
#[derive(Deserialize)]
struct Ask {
    sql: String,
    #[serde(default)]
    read_only: bool,
    #[serde(default = "yes")]
    pre_request: bool,
}

fn yes() -> bool {
    true
}

/// The route under test, in the shape the documentation gives it.
async fn ask(
    user: User,
    State(state): State<Arc<AppState>>,
    Query(ask): Query<Ask>,
) -> Result<Json<Value>, Error> {
    let options = TxOptions::default()
        .read_only(ask.read_only)
        .run_pre_request(ask.pre_request);
    let mut tx = state.begin_as_with(&user, options).await?;
    let value: Option<Value> = sqlx::query_scalar(&ask.sql).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    Ok(Json(value.unwrap_or(Value::Null)))
}

/// Who `Option<User>` says the caller is.
async fn who(user: Option<User>) -> Json<Value> {
    Json(match user {
        Some(user) => json!({ "role": user.role(), "anonymous": user.is_anonymous() }),
        None => Value::Null,
    })
}

/// Set up `schema`: a table under a policy on the token's `sub`, a table only
/// the anonymous role may read, and a pre-request function that leaves a
/// mark. Then build the application with `/mine` nested in it.
async fn app(schema: &str, compat_mode: bool) -> (Router, Arc<AppState>) {
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url())
        .await
        .expect("failed to connect to test database");
    for statement in [
        format!("DROP SCHEMA IF EXISTS {schema} CASCADE"),
        format!("CREATE SCHEMA {schema}"),
        format!("GRANT USAGE ON SCHEMA {schema} TO web_anon, web_user"),
        format!(
            "CREATE TABLE {schema}.todos (id serial PRIMARY KEY, owner text NOT NULL, \
             title text NOT NULL UNIQUE)"
        ),
        format!(
            "INSERT INTO {schema}.todos (owner, title) VALUES \
             ('alice', 'a1'), ('alice', 'a2'), ('bob', 'b1')"
        ),
        format!("ALTER TABLE {schema}.todos ENABLE ROW LEVEL SECURITY"),
        format!(
            "CREATE POLICY mine ON {schema}.todos TO web_user \
             USING (owner = current_setting('request.jwt.claims', true)::json->>'sub') \
             WITH CHECK (owner = current_setting('request.jwt.claims', true)::json->>'sub')"
        ),
        format!("GRANT SELECT, INSERT ON {schema}.todos TO web_user"),
        format!("GRANT USAGE ON ALL SEQUENCES IN SCHEMA {schema} TO web_user"),
        format!("CREATE TABLE {schema}.notices (id int PRIMARY KEY, body text)"),
        format!("INSERT INTO {schema}.notices VALUES (1, 'hello')"),
        format!("GRANT SELECT ON {schema}.notices TO web_anon"),
        format!(
            "CREATE FUNCTION {schema}.pre() RETURNS void LANGUAGE plpgsql AS \
             $$ BEGIN PERFORM set_config('test.pre', 'ran', true); END $$"
        ),
        format!("GRANT EXECUTE ON FUNCTION {schema}.pre() TO web_anon, web_user"),
    ] {
        sqlx::query(&statement).execute(&pool).await.unwrap();
    }

    let mut config = AppConfig::default();
    config.db_uri = database_url();
    config.db_schemas = vec![schema.to_string()];
    config.db_anon_role = Some("web_anon".to_string());
    config.db_pool_size = 2;
    config.db_pre_request = Some(format!("{schema}.pre"));
    config.jwt_secret = Some(SECRET.to_string());
    config.compat_mode = compat_mode;

    let routes: Router<Arc<AppState>> = Router::new()
        .route("/ask", get(ask).post(ask))
        .route("/who", get(who));
    let app = Builder::new(config, Host::LAMBDA)
        .nest("/mine", routes)
        .build()
        .await
        .expect("build");
    (app.router, app.state)
}

async fn drop_schema(schema: &str) {
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url())
        .await
        .unwrap();
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&pool)
        .await
        .unwrap();
}

fn token(claims: Value) -> String {
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(SECRET.as_bytes()),
    )
    .unwrap()
}

fn bearer(sub: &str) -> String {
    format!(
        "Bearer {}",
        token(json!({ "role": "web_user", "sub": sub, "exp": 4_000_000_000_u64 }))
    )
}

/// Every byte that is not alphanumeric, percent-encoded.
fn encode_query(value: &str) -> String {
    value
        .bytes()
        .map(|b| match b.is_ascii_alphanumeric() {
            true => (b as char).to_string(),
            false => format!("%{b:02X}"),
        })
        .collect()
}

fn ask_uri(sql: &str, extra: &str) -> String {
    format!("/mine/ask?sql={}{extra}", encode_query(sql))
}

/// What a response said: its status, its `WWW-Authenticate`, and its body
/// as JSON.
#[derive(Debug, PartialEq)]
struct Answer {
    status: StatusCode,
    challenge: Option<String>,
    body: Value,
}

async fn answer(response: Response) -> Answer {
    let status = response.status();
    let challenge = response
        .headers()
        .get("www-authenticate")
        .map(|v| v.to_str().unwrap().to_string());
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()));
    Answer {
        status,
        challenge,
        body,
    }
}

async fn send(
    router: &Router,
    method: &str,
    uri: &str,
    headers: &[(&str, &str)],
    body: Option<Value>,
) -> Answer {
    let mut request = Request::builder().method(method).uri(uri);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let body = match body {
        Some(body) => {
            request = request.header("content-type", "application/json");
            Body::from(body.to_string())
        }
        None => Body::empty(),
    };
    answer(
        router
            .clone()
            .oneshot(request.body(body).unwrap())
            .await
            .unwrap(),
    )
    .await
}

const TITLES: &str = "SELECT json_agg(title ORDER BY id) FROM todos";

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn each_caller_sees_what_the_policy_and_grants_give_them() {
    const SCHEMA: &str = "begin_as_policy";
    let (router, _) = app(SCHEMA, false).await;
    let alice = bearer("alice");
    let bob = bearer("bob");

    // Each user only their own rows: the policy reads the claims REST sets.
    let mine = send(
        &router,
        "GET",
        &ask_uri(TITLES, ""),
        &[("authorization", &alice)],
        None,
    )
    .await;
    assert_eq!(mine.status, StatusCode::OK, "{mine:?}");
    assert_eq!(mine.body, json!(["a1", "a2"]));
    let theirs = send(
        &router,
        "GET",
        &ask_uri(TITLES, ""),
        &[("authorization", &bob)],
        None,
    )
    .await;
    assert_eq!(theirs.body, json!(["b1"]));

    // The anonymous role reads what it is granted ...
    let notices = send(
        &router,
        "GET",
        &ask_uri("SELECT json_agg(body) FROM notices", ""),
        &[],
        None,
    )
    .await;
    assert_eq!(notices.status, StatusCode::OK, "{notices:?}");
    assert_eq!(notices.body, json!(["hello"]));

    // ... and is refused what it is not, with the status and body REST
    // answers the same refusal with.
    let refused = send(&router, "GET", &ask_uri(TITLES, ""), &[], None).await;
    let rest = send(&router, "GET", "/api/todos", &[], None).await;
    assert_eq!(rest.status, StatusCode::UNAUTHORIZED, "{rest:?}");
    assert_eq!(refused, rest);

    // A token that does not verify: REST's 401, body and challenge.
    let forged = format!(
        "Bearer {}",
        encode(
            &Header::default(),
            &json!({ "role": "web_user", "sub": "alice" }),
            &EncodingKey::from_secret(b"not-the-servers-secret-not-at-all"),
        )
        .unwrap()
    );
    for authorization in [forged.as_str(), "Basic dXNlcjpwYXNz"] {
        let refused = send(
            &router,
            "GET",
            &ask_uri(TITLES, ""),
            &[("authorization", authorization)],
            None,
        )
        .await;
        let rest = send(
            &router,
            "GET",
            "/api/todos",
            &[("authorization", authorization)],
            None,
        )
        .await;
        assert_eq!(rest.status, StatusCode::UNAUTHORIZED, "{rest:?}");
        assert_eq!(refused, rest, "{authorization}");
    }

    // `Option<User>`: no header is `None`, a token is its user, and a token
    // that does not verify is still refused.
    let none = send(&router, "GET", "/mine/who", &[], None).await;
    assert_eq!(none.body, Value::Null);
    let some = send(
        &router,
        "GET",
        "/mine/who",
        &[("authorization", &alice)],
        None,
    )
    .await;
    assert_eq!(some.body, json!({ "role": "web_user", "anonymous": false }));
    let refused = send(
        &router,
        "GET",
        "/mine/who",
        &[("authorization", &forged)],
        None,
    )
    .await;
    assert_eq!(refused.status, StatusCode::UNAUTHORIZED);

    drop_schema(SCHEMA).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_request_reads_back_in_the_transaction_as_it_does_in_rest() {
    const SCHEMA: &str = "begin_as_request";
    let (router, _) = app(SCHEMA, false).await;
    let sql = "SELECT json_build_object(\
         'user', current_user, \
         'method', current_setting('request.method', true), \
         'path', current_setting('request.path', true), \
         'tenant', current_setting('request.headers', true)::json->>'x-tenant', \
         'cookie', current_setting('request.cookies', true)::json->>'session', \
         'trace', current_setting('request.trace_id', true), \
         'sub', current_setting('request.jwt.claims', true)::json->>'sub', \
         'search_path', current_setting('search_path'))";

    let seen = send(
        &router,
        "POST",
        &ask_uri(sql, ""),
        &[
            ("authorization", &bearer("alice")),
            ("x-tenant", "acme"),
            ("cookie", "session=abc; theme=dark"),
            (
                "traceparent",
                "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
            ),
        ],
        None,
    )
    .await;
    assert_eq!(seen.status, StatusCode::OK, "{seen:?}");
    assert_eq!(
        seen.body,
        json!({
            "user": "web_user",
            "method": "POST",
            // As the client sent it, with the prefix the route was nested at.
            "path": "/mine/ask",
            "tenant": "acme",
            "cookie": "abc",
            "trace": "4bf92f3577b34da6a3ce929d0e0e4736",
            "sub": "alice",
            "search_path": format!("\"{SCHEMA}\", \"public\""),
        })
    );

    drop_schema(SCHEMA).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_options_shape_the_transaction() {
    const SCHEMA: &str = "begin_as_options";
    let (router, _) = app(SCHEMA, false).await;
    let alice = bearer("alice");
    let write = "INSERT INTO todos (owner, title) VALUES ('alice', 'a3') RETURNING to_json(id)";

    // Read-only is PostgreSQL's to enforce, and a refused write is REST's
    // 405.
    let refused = send(
        &router,
        "POST",
        &ask_uri(write, "&read_only=true"),
        &[("authorization", &alice)],
        None,
    )
    .await;
    assert_eq!(
        refused.status,
        StatusCode::METHOD_NOT_ALLOWED,
        "{refused:?}"
    );
    let written = send(
        &router,
        "POST",
        &ask_uri(write, ""),
        &[("authorization", &alice)],
        None,
    )
    .await;
    assert_eq!(written.status, StatusCode::OK, "{written:?}");

    // `db_pre_request` runs unless it is asked not to.
    let mark = "SELECT to_json(nullif(current_setting('test.pre', true), ''))";
    let ran = send(
        &router,
        "GET",
        &ask_uri(mark, ""),
        &[("authorization", &alice)],
        None,
    )
    .await;
    assert_eq!(ran.body, json!("ran"), "{ran:?}");
    let skipped = send(
        &router,
        "GET",
        &ask_uri(mark, "&pre_request=false"),
        &[("authorization", &alice)],
        None,
    )
    .await;
    assert_eq!(
        (skipped.status, skipped.body),
        (StatusCode::OK, Value::Null)
    );

    drop_schema(SCHEMA).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_connection_goes_back_to_the_pool_as_it_came() {
    const SCHEMA: &str = "begin_as_leakage";
    let (router, state) = app(SCHEMA, false).await;
    let alice = bearer("alice");

    // A transaction committed, one rolled back by a refusal, and one dropped
    // when the statement failed -- each on one of the pool's connections.
    for uri in [
        ask_uri(TITLES, ""),
        ask_uri(
            "INSERT INTO todos (owner, title) VALUES ('alice', 'x') RETURNING to_json(id)",
            "&read_only=true",
        ),
        ask_uri("SELECT to_json(1 / 0)", ""),
    ] {
        for _ in 0..2 {
            send(&router, "GET", &uri, &[("authorization", &alice)], None).await;
        }
    }

    // Both connections at once, so neither is the other.
    let mut first = state.pool.acquire().await.unwrap();
    let mut second = state.pool.acquire().await.unwrap();
    for connection in [&mut first, &mut second] {
        let left: (String, Option<String>, Option<String>, Option<String>) = sqlx::query_as(
            "SELECT current_user::text, \
             nullif(current_setting('request.jwt.claims', true), ''), \
             nullif(current_setting('request.method', true), ''), \
             nullif(current_setting('test.pre', true), '')",
        )
        .fetch_one(&mut **connection)
        .await
        .unwrap();
        assert_eq!(left, ("postgres".to_string(), None, None, None));
    }
    drop((first, second));

    drop_schema(SCHEMA).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_unique_violation_is_answered_as_rest_answers_it() {
    for (schema, compat_mode) in [
        ("begin_as_conflict", false),
        ("begin_as_conflict_compat", true),
    ] {
        let (router, _) = app(schema, compat_mode).await;
        let alice = bearer("alice");
        let headers = [("authorization", alice.as_str())];

        let conflict = send(
            &router,
            "POST",
            &ask_uri(
                "INSERT INTO todos (owner, title) VALUES ('alice', 'a1') RETURNING to_json(id)",
                "",
            ),
            &headers,
            None,
        )
        .await;
        let rest = send(
            &router,
            "POST",
            "/api/todos",
            &headers,
            Some(json!({ "owner": "alice", "title": "a1" })),
        )
        .await;
        assert_eq!(conflict.status, StatusCode::CONFLICT, "{conflict:?}");
        assert_eq!(conflict, rest, "compat_mode: {compat_mode}");
        // Sanitised, as REST sanitises it; in compatibility mode, the
        // database's own SQLSTATE and words, as REST gives them there.
        let (code, message) = (&conflict.body["code"], &conflict.body["message"]);
        match compat_mode {
            false => assert_eq!(
                (code.as_str(), message.as_str()),
                (Some("PGRST503"), Some("Database error"))
            ),
            true => {
                assert_eq!(code, "23505");
                assert!(
                    message.as_str().unwrap().starts_with("duplicate key value"),
                    "{conflict:?}"
                );
            }
        }

        drop_schema(schema).await;
    }
}
