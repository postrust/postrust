# Custom Routes

Postrust allows you to add custom API endpoints alongside the automatic PostgREST-style routes. This is useful for:

- Webhooks (Stripe, GitHub, etc.)
- Health checks and readiness probes
- Custom business logic that doesn't fit the REST pattern
- Integrations with external services
- Background job triggers

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────┐
│                     Postrust Server                         │
├─────────────────────────────────────────────────────────────┤
│  /           → Server info (version, endpoints)             │
│  /api/*      → PostgREST-style auto-generated routes        │
│  /_/*        → Custom routes (health, webhooks, etc.)       │
│  /admin/*    → Admin UI (if admin-ui feature enabled)       │
│  /api/graphql → GraphQL endpoint (if admin-ui enabled)      │
└─────────────────────────────────────────────────────────────┘
```

## Quick Start

Custom routes are defined in `crates/postrust-server/src/custom.rs`. The module is already set up with health check endpoints.

### Adding a New Route

1. Open `crates/postrust-server/src/custom.rs`
2. Add your handler function
3. Register the route in `custom_router()`

```rust
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::{get, post},
    Router,
};
use std::sync::Arc;
use crate::state::AppState;

pub fn custom_router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/health", get(health_check))
        .route("/ready", get(readiness_check))
        // Add your custom routes here:
        .route("/webhooks/stripe", post(handle_stripe_webhook))
        .route("/notify", post(send_notification))
}

async fn handle_stripe_webhook(
    State(state): State<Arc<AppState>>,
    body: String,
) -> impl IntoResponse {
    // Your webhook logic here
    (StatusCode::OK, Json(serde_json::json!({ "received": true })))
}
```

## Handler Patterns

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
use axum::extract::State;
use std::sync::Arc;
use crate::state::AppState;

async fn db_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    // Access the database pool
    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users")
        .fetch_one(&state.pool)
        .await
        .unwrap_or((0,));

    Json(serde_json::json!({ "user_count": count.0 }))
}
```

### With JSON Body

```rust
use axum::Json;
use serde::Deserialize;

#[derive(Deserialize)]
struct CreateUserRequest {
    email: String,
    name: String,
}

async fn create_user(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<CreateUserRequest>,
) -> impl IntoResponse {
    let result = sqlx::query(
        "INSERT INTO users (email, name) VALUES ($1, $2) RETURNING id"
    )
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
            Json(serde_json::json!({ "error": e.to_string() }))
        ),
    }
}
```

### With Path Parameters

```rust
use axum::extract::Path;

async fn get_user(
    State(state): State<Arc<AppState>>,
    Path(user_id): Path<uuid::Uuid>,
) -> impl IntoResponse {
    let user = sqlx::query_as::<_, User>(
        "SELECT * FROM users WHERE id = $1"
    )
        .bind(user_id)
        .fetch_optional(&state.pool)
        .await;

    match user {
        Ok(Some(u)) => (StatusCode::OK, Json(u)).into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() }))
        ).into_response(),
    }
}

// Register with path parameter
.route("/users/:id", get(get_user))
```

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
    let page = params.page.unwrap_or(1);
    let limit = params.limit.unwrap_or(20).min(100);
    let offset = (page - 1) * limit;

    let users = sqlx::query_as::<_, User>(
        "SELECT * FROM users ORDER BY created_at DESC LIMIT $1 OFFSET $2"
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

```rust
use axum::http::StatusCode;

async fn fallible_handler(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let result = sqlx::query("SELECT 1")
        .execute(&state.pool)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() }))
            )
        })?;

    Ok(Json(serde_json::json!({ "success": true })))
}
```

## Accessing Application State

The `AppState` struct provides access to:

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
}
```

### Database Pool

```rust
async fn handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
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
}
```

### Schema Cache

```rust
async fn handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let cache = state.schema_cache.read().await;

    // Get all tables
    let tables = cache.tables();

    // Check if table exists
    if cache.has_table("users") {
        // ...
    }

    // Get table columns
    if let Some(table) = cache.get_table("users") {
        for column in &table.columns {
            println!("{}: {}", column.name, column.data_type);
        }
    }
}
```

### Configuration

```rust
async fn handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let config = &state.config;

    // Access config values
    let schemas = &config.db_schemas;
    let anon_role = &config.db_anon_role;
    let max_rows = config.max_rows;
}
```

## Webhook Examples

### Stripe Webhook

```rust
use axum::http::HeaderMap;
use hmac::{Hmac, Mac};
use sha2::Sha256;

async fn stripe_webhook(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    // Get Stripe signature
    let signature = headers
        .get("stripe-signature")
        .and_then(|v| v.to_str().ok());

    let Some(sig) = signature else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Missing signature" }))
        );
    };

    // Verify signature (simplified - use stripe crate in production)
    let webhook_secret = std::env::var("STRIPE_WEBHOOK_SECRET")
        .unwrap_or_default();

    // Parse and handle event
    let event: serde_json::Value = serde_json::from_str(&body)
        .unwrap_or_default();

    let event_type = event["type"].as_str().unwrap_or("unknown");

    match event_type {
        "checkout.session.completed" => {
            // Handle checkout completion
            let session = &event["data"]["object"];
            let customer_id = session["customer"].as_str();
            // Update database...
        }
        "customer.subscription.updated" => {
            // Handle subscription update
        }
        _ => {
            tracing::info!("Unhandled event type: {}", event_type);
        }
    }

    (StatusCode::OK, Json(serde_json::json!({ "received": true })))
}
```

### GitHub Webhook

```rust
async fn github_webhook(
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    let event_type = headers
        .get("x-github-event")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown");

    let payload: serde_json::Value = serde_json::from_str(&body)
        .unwrap_or_default();

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

### Adding Middleware to Custom Routes

```rust
use axum::middleware::{self, Next};
use axum::http::Request;

async fn logging_middleware<B>(
    request: Request<B>,
    next: Next<B>,
) -> impl IntoResponse {
    let method = request.method().clone();
    let uri = request.uri().clone();

    let start = std::time::Instant::now();
    let response = next.run(request).await;
    let duration = start.elapsed();

    tracing::info!(
        "{} {} - {:?} - {:?}",
        method, uri, response.status(), duration
    );

    response
}

pub fn custom_router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/health", get(health_check))
        .route("/webhooks/stripe", post(stripe_webhook))
        .layer(middleware::from_fn(logging_middleware))
}
```

### Authentication Middleware

```rust
use axum::http::Request;
use axum::middleware::Next;

async fn auth_middleware<B>(
    State(state): State<Arc<AppState>>,
    request: Request<B>,
    next: Next<B>,
) -> impl IntoResponse {
    let auth_header = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok());

    match auth_header {
        Some(token) if token.starts_with("Bearer ") => {
            let jwt = &token[7..];
            // Validate JWT using state.jwt_config
            // If valid, continue
            next.run(request).await
        }
        _ => StatusCode::UNAUTHORIZED.into_response(),
    }
}

// Apply to specific routes
pub fn custom_router() -> Router<Arc<AppState>> {
    let protected = Router::new()
        .route("/admin/stats", get(admin_stats))
        .layer(middleware::from_fn_with_state(state.clone(), auth_middleware));

    let public = Router::new()
        .route("/health", get(health_check));

    public.merge(protected)
}
```

## Organizing Routes

For larger applications, split routes into modules:

```
crates/postrust-server/src/
├── main.rs
├── state.rs
├── app.rs
├── custom/
│   ├── mod.rs
│   ├── health.rs
│   ├── webhooks/
│   │   ├── mod.rs
│   │   ├── stripe.rs
│   │   └── github.rs
│   └── admin.rs
```

```rust
// custom/mod.rs
mod health;
mod webhooks;
mod admin;

use axum::Router;
use std::sync::Arc;
use crate::state::AppState;

pub fn custom_router() -> Router<Arc<AppState>> {
    Router::new()
        .merge(health::router())
        .nest("/webhooks", webhooks::router())
        .nest("/admin", admin::router())
}
```

```rust
// custom/webhooks/mod.rs
mod stripe;
mod github;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/stripe", post(stripe::handle))
        .route("/github", post(github::handle))
}
```

## Testing Custom Routes

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    #[tokio::test]
    async fn test_health_check() {
        let app = custom_router().with_state(test_state());

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/_/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
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

# Feature flags
ENABLE_WEBHOOKS=true
ENABLE_ADMIN_API=true
```

Access in handlers:

```rust
async fn handler() -> impl IntoResponse {
    let secret = std::env::var("STRIPE_WEBHOOK_SECRET")
        .expect("STRIPE_WEBHOOK_SECRET required");
    // ...
}
```

## Next Steps

- See [Deployment](./deployment.md) for deploying your customized Postrust
- See [Authentication](./authentication.md) for JWT configuration
- See [Configuration](./configuration.md) for all available options
