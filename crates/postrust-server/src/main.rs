//! Postrust HTTP Server.
//!
//! A PostgREST-compatible REST API server for PostgreSQL.

use anyhow::Result;
use axum::{
    body::Body,
    extract::{Request, State},
    http::{Method, StatusCode},
    middleware,
    response::{IntoResponse, Response},
    routing::any,
    Router,
};
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use tokio::sync::RwLock;
use tower_http::cors::{Any, CorsLayer};
use tracing::{error, info};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod app;
mod state;

use app::handle_request;
use state::AppState;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "postrust=info".into()),
        ))
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Load configuration
    let config = postrust_core::AppConfig::from_env();
    info!("Starting Postrust server");
    info!("Database: {}", mask_db_uri(&config.db_uri));

    // Create database pool
    let pool = PgPoolOptions::new()
        .max_connections(config.db_pool_size)
        .connect(&config.db_uri)
        .await?;

    info!("Connected to database");

    // Load schema cache
    let schema_cache = postrust_core::SchemaCache::load(&pool, &config.db_schemas).await?;
    info!("{}", schema_cache.summary());

    // Create app state
    let state = Arc::new(AppState {
        pool,
        schema_cache: RwLock::new(schema_cache),
        config: config.clone(),
        jwt_config: postrust_auth::JwtConfig {
            secret: config.jwt_secret.clone(),
            secret_is_base64: config.jwt_secret_is_base64,
            audience: config.jwt_aud.clone(),
            role_claim_key: config.jwt_role_claim_key.clone(),
            anon_role: config.db_anon_role.clone(),
        },
    });

    // Build router
    let app = Router::new()
        .route("/", any(handle_request))
        .route("/{*path}", any(handle_request))
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods([
                    Method::GET,
                    Method::POST,
                    Method::PUT,
                    Method::PATCH,
                    Method::DELETE,
                    Method::OPTIONS,
                    Method::HEAD,
                ])
                .allow_headers(Any)
                .expose_headers(Any),
        )
        .with_state(state);

    // Start server
    let addr = format!("{}:{}", config.server_host, config.server_port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    info!("Listening on http://{}", addr);

    axum::serve(listener, app).await?;

    Ok(())
}

/// Mask database URI for logging.
fn mask_db_uri(uri: &str) -> String {
    if let Some(at_pos) = uri.find('@') {
        if let Some(proto_end) = uri.find("://") {
            return format!("{}://***@{}", &uri[..proto_end], &uri[at_pos + 1..]);
        }
    }
    uri.to_string()
}
