//! Postrust HTTP Server.
//!
//! A PostgREST-compatible REST API server for PostgreSQL.

use anyhow::Result;
use axum::{http::Method, response::Json, routing::any, Router};
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use tokio::sync::RwLock;
use tower_http::cors::{Any as CorsAny, CorsLayer};
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod app;
mod custom;
mod state;

#[cfg(feature = "admin-ui")]
mod admin;

#[cfg(feature = "admin-ui")]
use axum::routing::{get, post};

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

    // Build REST API router (under /api prefix)
    let api_router: Router<Arc<AppState>> = Router::new()
        .route("/", any(handle_request))
        .route("/{*path}", any(handle_request));

    // Build main router
    let mut app: Router<Arc<AppState>> = Router::new()
        .nest("/api", api_router);

    // Add custom routes (health checks, webhooks, etc.)
    app = app.nest("/_", custom::custom_router());
    info!("Custom routes enabled at /_");

    // Add admin routes and GraphQL endpoint if feature is enabled
    #[cfg(feature = "admin-ui")]
    {
        use async_graphql_axum::{GraphQLRequest as GqlRequest, GraphQLResponse as GqlResponse};
        use axum::extract::State as AxumState;
        use axum::http::HeaderMap;
        use postrust_graphql::handler::GraphQLState;
        use postrust_graphql::schema::SchemaConfig;

        info!("Admin UI enabled at /admin");
        app = app.nest("/admin", admin::admin_router());

        // Create GraphQL state with subscriptions enabled
        let schema_cache_snapshot = state.schema_cache.read().await.clone();
        let schema_cache_arc = Arc::new(schema_cache_snapshot);
        let graphql_config = SchemaConfig {
            enable_subscriptions: true,
            ..SchemaConfig::default()
        };
        let graphql_state = Arc::new(
            GraphQLState::new(
                state.pool.clone(),
                schema_cache_arc.clone(),
                graphql_config,
            )
            .expect("Failed to build GraphQL schema"),
        );

        // Initialize subscription broker
        if let Err(e) = graphql_state.init_subscriptions().await {
            tracing::warn!("Failed to initialize subscription broker: {}. Subscriptions may not work until triggers are created.", e);
        } else {
            info!("GraphQL subscriptions enabled");
        }

        info!("GraphQL endpoint enabled at /api/graphql");

        // Wrapper handler that creates context from request
        async fn handle_graphql(
            AxumState(gql_state): AxumState<Arc<GraphQLState>>,
            _headers: HeaderMap,
            req: GqlRequest,
        ) -> GqlResponse {
            // Use anonymous role for now (auth can be added later)
            let request = req.into_inner().data(gql_state.pool.clone());
            gql_state.schema.execute(request).await.into()
        }

        // Add GraphQL routes with WebSocket support for subscriptions
        let graphql_router = Router::new()
            .route("/", post(handle_graphql))
            .route("/", get(postrust_graphql::handler::graphql_playground))
            .route("/ws", get(postrust_graphql::handler::graphql_ws_handler))
            .with_state(graphql_state);

        app = app.nest("/api/graphql", graphql_router);
    }

    // Add root info endpoint
    app = app.route("/", axum::routing::get(|| async {
        Json(serde_json::json!({
            "name": "postrust",
            "version": env!("CARGO_PKG_VERSION"),
            "api": "/api",
            "custom": "/_",
            "health": "/_/health",
            "admin": "/admin",
            "docs": "/admin/swagger"
        }))
    }));

    // Apply CORS and state
    let app = app
        .layer(
            CorsLayer::new()
                .allow_origin(CorsAny)
                .allow_methods([
                    Method::GET,
                    Method::POST,
                    Method::PUT,
                    Method::PATCH,
                    Method::DELETE,
                    Method::OPTIONS,
                    Method::HEAD,
                ])
                .allow_headers(CorsAny)
                .expose_headers(CorsAny),
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
