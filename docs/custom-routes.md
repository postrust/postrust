# Custom Routes

Postrust lets you add API endpoints of your own alongside the automatic
PostgREST-style routes, without forking it. This is useful for:

- Webhooks (Stripe, GitHub, etc.)
- Custom business logic that doesn't fit the REST pattern
- Integrations with external services
- Background job triggers

You write a small binary crate that depends on `postrust-server`, builds the
application with `postrust_server::Builder`, and nests your own
[axum](https://docs.rs/axum/0.8) routers into it. Everything the `postrust`
binary serves is still served, configured from the same `PGRST_*` environment
variables. A complete, compiling example is in
[`examples/custom-routes`](../examples/custom-routes).

## Architecture Overview

```
/              → Server info (the REST root, in compatibility mode)
/api/*         → PostgREST-style auto-generated routes
/_/health      → Liveness
/_/ready       → Readiness (checks the database)
/healthz       → Liveness, under Hasura's name
/v1/version    → Version
/v1/graphql    → GraphQL (graphql feature)
/admin/*       → Admin UI (admin-ui feature)
/<yours>/*     → Your routes, via Builder::nest
/_/<yours>/*   → Your routes beneath /_, beside the health checks
```

## Quick Start

`Cargo.toml`:

```toml
[package]
name = "my-api"
version = "0.1.0"
edition = "2021"

[dependencies]
postrust-server = "3"                                  # features = ["graphql"] or ["admin-ui"] as needed
postrust-core = "3"
axum = "0.8"
tokio = { version = "1", features = ["full"] }
sqlx = { version = "0.8", default-features = false, features = ["postgres"] }
serde_json = "1"
anyhow = "1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
```

`src/main.rs`:

```rust
use axum::{extract::State, http::StatusCode, routing::post, Router};
use postrust_server::{AppState, Builder, Host};
use std::sync::Arc;

fn webhooks() -> Router<Arc<AppState>> {
    Router::new().route("/stripe", post(handle_stripe_webhook))
}

async fn handle_stripe_webhook(State(state): State<Arc<AppState>>, body: String) -> StatusCode {
    // Your webhook logic here, with `state.pool` for the database.
    StatusCode::OK
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new("info"))
        .init();

    let config = postrust_core::AppConfig::try_from_env()?;
    let addr = format!("{}:{}", config.server_host, config.server_port);

    let app = Builder::new(config, Host::SERVER)
        .nest("/webhooks", webhooks())     // POST /webhooks/stripe
        .build()
        .await?;

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app.router).await?;
    Ok(())
}
```

`postrust_server::build(config, host)` is the same as
`Builder::new(config, host).build()`: the application with nothing added.

### What your routes get

Your routers are added before the server's outer layers, so they are served
exactly as the API is:

- the CORS policy (`PGRST_SERVER_CORS_ORIGINS`), including preflight answers;
- the request body limit (`max_body_size`), for handlers that take a body
  extractor such as `Json` or `String`;
- `OPTIONS` handling;
- the shared `Arc<AppState>`: the pool, the schema cache and the configuration.

### Prefixes

`nest` may be called as many times as you like. A prefix is a path: the
leading slash is added if missing and a trailing one dropped, so `"hooks"`,
`"/hooks"` and `"/hooks/"` are one prefix. `build()` returns an error -- it
never panics -- before connecting to anything, when a prefix:

- is empty or `/`;
- contains a parameter (`{id}`), a wildcard (`*`) or an empty segment;
- overlaps a path the server serves itself: `/api`, `/healthz`,
  `/v1/version`, `/v1/graphql`, `/v1alpha1/graphql`, `/admin`, `/_/health`,
  `/_/ready`, or `/_` itself (the list is
  `postrust_server::RESERVED_PREFIXES`). These are reserved whether or not the
  feature serving them is compiled in, so turning a feature on never breaks a
  binary that built before;
- overlaps another prefix you nested: `/hooks` and `/hooks/stripe` clash.

"Overlaps" is by whole segments: `/v1` clashes with `/v1/version`, while
`/v1/other` and `/apis` are fine.

Beneath `/_` is yours: `.nest("/_/hooks", router)` serves next to the built-in
`/_/health` and `/_/ready`.

### Compatibility mode

With `PGRST_COMPAT_MODE` on, the REST surface is also served at the root. Your
routes are explicit routes and are matched first, so a table or function that
shares a prefix's name is shadowed at the root -- `/hooks` answers from your
router -- and stays reachable under `/api` (`/api/hooks`). Every path no route
of yours claims still falls through to REST.

## AWS Lambda

`postrust-lambda` is a library as well as a binary. A Lambda function with
your routes is a binary of its own:

```rust
use postrust_lambda::postrust_server::{Builder, Host};

#[tokio::main]
async fn main() -> Result<(), postrust_lambda::Error> {
    postrust_lambda::init_tracing();
    // The environment's configuration, with a pool sized for one sandbox.
    let config = postrust_lambda::config_from_env()?;
    let app = Builder::new(config, Host::LAMBDA)
        .nest("/webhooks", webhooks())
        .build()
        .await?;
    postrust_lambda::run(app).await
}
```

Build it with `cargo lambda build --release` as you would the stock function.

## Handler Patterns

In each of these, `AppState` is `postrust_server::AppState`.

### Basic Handler

```rust
async fn hello() -> &'static str {
    "Hello, World!"
}
```

### JSON Response

```rust
use axum::response::Json;
use serde::Serialize;

#[derive(Serialize)]
struct ApiResponse {
    success: bool,
    message: String,
}

async fn json_handler() -> Json<ApiResponse> {
    Json(ApiResponse {
        success: true,
        message: "Operation completed".into(),
    })
}
```

### With State (Database Access)

```rust
use axum::{extract::State, response::{IntoResponse, Json}};
use postrust_server::AppState;
use std::sync::Arc;

async fn db_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users")
        .fetch_one(&state.pool)
        .await
        .unwrap_or((0,));

    Json(serde_json::json!({ "user_count": count.0 }))
}
```

**Row-level security.** The pool connects as the authenticator role, and a
query sent on it runs as that role: with every grant the authenticator has,
and with no `request.jwt.claims` for a policy to read, so RLS policies written
for the API's roles do not apply. Use the pool directly only for what the
authenticator may see. For a query that should run as the caller, see
[Running as the Caller](#running-as-the-caller).

### With JSON Body

```rust
use axum::Json;
use serde::Deserialize;
use sqlx::Row;

#[derive(Deserialize)]
struct CreateUserRequest {
    email: String,
    name: String,
}

async fn create_user(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<CreateUserRequest>,
) -> impl IntoResponse {
    let result = sqlx::query("INSERT INTO users (email, name) VALUES ($1, $2) RETURNING id")
        .bind(&payload.email)
        .bind(&payload.name)
        .fetch_one(&state.pool)
        .await;

    match result {
        Ok(row) => {
            let id: uuid::Uuid = row.get("id");
            (StatusCode::CREATED, Json(serde_json::json!({ "id": id })))
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}
```

### With Path Parameters

axum 0.8 writes a parameter as `{name}`.

```rust
use axum::extract::Path;

async fn get_user(
    State(state): State<Arc<AppState>>,
    Path(user_id): Path<uuid::Uuid>,
) -> impl IntoResponse {
    let user = sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_optional(&state.pool)
        .await;

    match user {
        Ok(Some(u)) => (StatusCode::OK, Json(u)).into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

// Register with path parameter
.route("/users/{id}", get(get_user))
```

The parameter goes in a route, not in the prefix given to `nest`.

### With Query Parameters

```rust
use axum::extract::Query;
use serde::Deserialize;

#[derive(Deserialize)]
struct Pagination {
    page: Option<u32>,
    limit: Option<u32>,
}

async fn list_users(
    State(state): State<Arc<AppState>>,
    Query(params): Query<Pagination>,
) -> impl IntoResponse {
    let page = params.page.unwrap_or(1).max(1);
    let limit = params.limit.unwrap_or(20).min(100);
    let offset = (page - 1) * limit;

    let users = sqlx::query_as::<_, User>(
        "SELECT * FROM users ORDER BY created_at DESC LIMIT $1 OFFSET $2",
    )
    .bind(limit as i64)
    .bind(offset as i64)
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();

    Json(users)
}
```

### With Headers

```rust
use axum::http::HeaderMap;

async fn with_headers(headers: HeaderMap) -> impl IntoResponse {
    let auth = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("none");

    Json(serde_json::json!({ "auth_header": auth }))
}
```

### Error Handling

`postrust_server::Error` answers in PostgREST's error shape -- the `code`,
`message`, `details` and `hint` body and the status that goes with it -- so a
client of the API reads a route's failures the way it reads the API's. `?`
converts a `sqlx::Error` into one:

```rust
async fn fallible_handler(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, postrust_server::Error> {
    sqlx::query("SELECT 1").execute(&state.pool).await?;
    Ok(Json(serde_json::json!({ "success": true })))
}
```

A database error is mapped by its SQLSTATE with the table REST uses: a unique
or foreign-key violation is 409, a missing grant (`42501`) 401, a bare
`RAISE EXCEPTION` 400, a function's `RAISE sqlstate 'PGRST'` the status,
headers and body it wrote; a connection that broke mid-statement is 503, and
one that could not be had from the pool in time is 500 (`PGRST500`). The database's own message, detail and hint are sanitised as
REST sanitises them, and passed through in compatibility mode -- for the
routers given to `Builder::nest`, which is where the server can see the
setting.

## Accessing Application State

`postrust_server::AppState` provides:

```rust
pub struct AppState {
    /// PostgreSQL connection pool
    pub pool: sqlx::PgPool,
    /// Cached database schema (tables, columns, relations)
    pub schema_cache: RwLock<SchemaCache>,
    /// Application configuration
    pub config: AppConfig,
    /// JWT configuration for auth
    pub jwt_config: JwtConfig,
    /// Cache of validated tokens, when `jwt_cache_enabled` asks for one
    pub jwt_cache: Option<JwtCache>,
}
```

### Database Pool

```rust
async fn handler(State(state): State<Arc<AppState>>) -> Result<(), sqlx::Error> {
    // Execute raw SQL
    let rows = sqlx::query("SELECT * FROM users")
        .fetch_all(&state.pool)
        .await?;

    // With parameters
    let user = sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(&state.pool)
        .await?;

    // Execute mutation
    sqlx::query("UPDATE users SET name = $1 WHERE id = $2")
        .bind(&new_name)
        .bind(&user_id)
        .execute(&state.pool)
        .await?;
    Ok(())
}
```

### Schema Cache

The cache is reloaded on `NOTIFY` when `db_channel_enabled` is on, so take
the read lock for as long as you need it and no longer.

```rust
use postrust_core::api_request::QualifiedIdentifier;

async fn handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let cache = state.schema_cache().await;

    // Every table and view, keyed by schema and name
    let count = cache.tables.len();

    // One table's columns
    if let Some(table) = cache.get_table(&QualifiedIdentifier::new("public", "users")) {
        for column in table.columns.values() {
            println!("{}: {}", column.name, column.data_type);
        }
    }
    Json(serde_json::json!({ "tables": count }))
}
```

### Configuration

```rust
async fn handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let config = &state.config;

    let schemas = &config.db_schemas;
    let anon_role = &config.db_anon_role;
    let max_rows = config.db_max_rows;
}
```

## Webhook Examples

### Stripe Webhook

```rust
use axum::http::HeaderMap;

async fn stripe_webhook(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    let Some(signature) = headers.get("stripe-signature").and_then(|v| v.to_str().ok()) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Missing signature" })),
        );
    };

    // Verify the signature against STRIPE_WEBHOOK_SECRET before trusting the
    // body -- use the stripe crate, or HMAC-SHA256 as Stripe documents.

    let event: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
    match event["type"].as_str().unwrap_or("unknown") {
        "checkout.session.completed" => {
            let session = &event["data"]["object"];
            let customer_id = session["customer"].as_str();
            // Update database...
        }
        "customer.subscription.updated" => {
            // Handle subscription update
        }
        other => tracing::info!("Unhandled event type: {}", other),
    }

    (StatusCode::OK, Json(serde_json::json!({ "received": true })))
}
```

### GitHub Webhook

```rust
async fn github_webhook(headers: HeaderMap, body: String) -> impl IntoResponse {
    let event_type = headers
        .get("x-github-event")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown");

    let payload: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();

    match event_type {
        "push" => {
            let repo = payload["repository"]["full_name"].as_str();
            let branch = payload["ref"].as_str();
            tracing::info!("Push to {:?} on {:?}", repo, branch);
        }
        "pull_request" => {
            let action = payload["action"].as_str();
            let pr_number = payload["number"].as_u64();
            tracing::info!("PR #{:?} - {:?}", pr_number, action);
        }
        _ => {}
    }

    StatusCode::OK
}
```

## Middleware

A layer on your router applies to your routes only, inside the server's own
layers (so CORS has already been handled by the time it runs).

A layer given to `Builder::layer` wraps the whole application instead -- the
API, GraphQL, your routes, and the server's own layers too -- so it sees every
request first, including a CORS preflight and a path nothing serves. Use it for
what has to cover everything, such as a rate limit or a header every response
carries. Of several, the one given last is the outermost:

```rust
let app = Builder::new(config, Host::SERVER)
    .nest("/webhooks", webhooks())
    .layer(middleware::map_response(add_server_header))
    .build()
    .await?;
```

### Logging

```rust
use axum::{extract::Request, middleware::{self, Next}, response::Response};

async fn logging_middleware(request: Request, next: Next) -> Response {
    let method = request.method().clone();
    let uri = request.uri().clone();

    let start = std::time::Instant::now();
    let response = next.run(request).await;

    tracing::info!("{} {} - {} - {:?}", method, uri, response.status(), start.elapsed());
    response
}

fn webhooks() -> Router<Arc<AppState>> {
    Router::new()
        .route("/stripe", post(stripe_webhook))
        .layer(middleware::from_fn(logging_middleware))
}
```

### Authentication

There is no middleware to write: take the caller as a
[`User`](#running-as-the-caller) in the handler. Because `User` is an
extractor, a request it refuses never reaches the handler's body.

## Running as the Caller

A route of yours is reached by the API's clients, with the API's tokens, and
the policies on your tables were written for the role and claims the API runs
a request as. `postrust_server::User` is the API's answer to who the caller
is, and `AppState::begin_as` is a transaction that runs as them:

```rust
use axum::{extract::State, Json};
use postrust_server::{AppState, User};
use serde_json::{json, Value};
use std::sync::Arc;

async fn summary(
    user: User,
    State(state): State<Arc<AppState>>,
) -> Result<Json<Value>, postrust_server::Error> {
    let mut tx = state.begin_as(&user).await?;
    let n: i64 = sqlx::query_scalar("select count(*) from todos")
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Json(json!({ "todos": n })))
}
```

Under a policy such as

```sql
CREATE POLICY own_todos ON api.todos TO web_user
    USING (owner = current_setting('request.jwt.claims', true)::json->>'sub');
```

each caller counts their own rows, exactly as `GET /api/todos` would show
them. [`examples/custom-routes`](../examples/custom-routes) has this route,
and the SQL it expects in `todos.sql`.

### `User`

`User` checks the `Authorization` header with the function the REST API checks
it with: the same JWT configuration, the same token cache, the same anonymous
role. A request with no token is `PGRST_DB_ANON_ROLE`, or refused with 401
when none is configured. A token that does not verify is refused with the
status, body and `WWW-Authenticate` challenge the API gives it, as a
`postrust_server::Error`.

`user.role()` is the role, `user.claims()` every claim of the token (and
`user.claim("sub")` one), `user.is_anonymous()` whether the request sent no
token at all.

`Option<User>` is for a route that treats a caller who has not signed in
differently, whatever the anonymous role: it is `None` when the request has no
`Authorization` header, and the token's user when it has one that verifies. A
token that does not verify is still refused, rather than read as `None`, so a
client with an expired token is told so instead of being answered as someone
else.

### `begin_as`

`state.begin_as(&user)` takes a connection from the pool, begins a
transaction, and sends what REST sends before a request's own query: the role,
the claims (`request.jwt.claims`), the facts about the request, the
`search_path` (the default schema, then `db_extra_search_path`), the
`app.settings.*`, the role's `statement_timeout` and isolation level, and
`db_pre_request`. What comes back is a plain
`sqlx::Transaction<'static, Postgres>`; run anything on it with `&mut *tx`.

The request's settings read back as they do in REST:
`current_setting('request.method')`, `request.path` (the path the client sent,
prefix included), `request.headers` and `request.cookies` (JSON objects, by
lowercase header name) and `request.trace_id`. Not read from your request:
the schema an `Accept-Profile` names and a `Prefer: timezone`, which are REST's
own request syntax.

Every setting is local to the transaction. Commit it, or drop it -- a dropped
transaction is rolled back -- and the connection goes back to the pool as the
authenticator, with nothing set.

`begin_as_with` takes `TxOptions` for what a route may want to differ:

```rust
use postrust_core::IsolationLevel;
use postrust_server::TxOptions;

let options = TxOptions::default()
    .read_only(true)                            // a write is refused: 405, as REST
    .isolation(IsolationLevel::RepeatableRead)  // over the role's and the server's
    .run_pre_request(false);                    // skip db_pre_request
let mut tx = state.begin_as_with(&user, options).await?;
```

`run_pre_request` is on by default. Turn it off only knowing what the
deployment's pre-request function checks: it is commonly where a caller is
refused.

A connection that cannot be had from the pool in time is logged at error and
answered 500 (`PGRST500`), as REST answers it; a session the database refuses -- a
pre-request function that raises, a role that does not exist -- is answered
with the database's error.

### What this does and does not protect

`begin_as` keeps what a request *sends* inside the policy: every statement you
run on the transaction runs as the caller, so the values a client supplies can
reach only the rows its role and claims allow.

It does not make your handler untrusted code. The handler runs as the
authenticator, holds the pool, and could leave the caller's role at any
time -- `RESET ROLE` on the transaction is enough. Treat a route of yours as
you would a `SECURITY DEFINER` function: code you wrote and reviewed, never
SQL a client chose.

## Organizing Routes

For larger applications, split routes into modules of your crate:

```
my-api/
├── Cargo.toml
└── src/
    ├── main.rs
    └── webhooks/
        ├── mod.rs
        ├── stripe.rs
        └── github.rs
```

```rust
// src/webhooks/mod.rs
mod github;
mod stripe;

use axum::{routing::post, Router};
use postrust_server::AppState;
use std::sync::Arc;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/stripe", post(stripe::handle))
        .route("/github", post(github::handle))
}
```

```rust
// src/main.rs
let app = Builder::new(config, Host::SERVER)
    .nest("/webhooks", webhooks::router())
    .nest("/_/jobs", jobs::router())
    .build()
    .await?;
```

## Testing Custom Routes

`app.router` is a plain `axum::Router`, so a test sends it requests with
`tower::ServiceExt::oneshot`. Building it connects to the database named by
the configuration.

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    #[tokio::test]
    async fn stripe_webhook_is_served() {
        let mut config = postrust_core::AppConfig::default();
        config.db_uri = std::env::var("DATABASE_URL").unwrap();
        let app = Builder::new(config, Host::SERVER)
            .nest("/webhooks", webhooks())
            .build()
            .await
            .unwrap();

        let response = app
            .router
            .oneshot(
                Request::post("/webhooks/stripe")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST); // no signature
    }
}
```

## Environment Variables for Custom Routes

Add custom environment variables in your deployment:

```env
# Webhook secrets
STRIPE_WEBHOOK_SECRET=whsec_...
GITHUB_WEBHOOK_SECRET=...

# External service URLs
EMAIL_SERVICE_URL=https://api.sendgrid.com
SLACK_WEBHOOK_URL=https://hooks.slack.com/...
```

Read them once at startup and keep them in your own state, or per request:

```rust
async fn handler() -> impl IntoResponse {
    let secret = std::env::var("STRIPE_WEBHOOK_SECRET")
        .expect("STRIPE_WEBHOOK_SECRET required");
    // ...
}
```

## Next Steps

- See [Deployment](./deployment.md) for building and deploying your own binary
- See [Authentication](./authentication.md) for JWT configuration
- See [Configuration](./configuration.md) for all available options
