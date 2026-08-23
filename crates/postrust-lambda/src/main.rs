//! Postrust AWS Lambda handler.
//!
//! Deploys Postrust as an AWS Lambda function.

use lambda_http::{run, service_fn, Body, Error, Request, Response};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::sync::Arc;
use tokio::sync::{OnceCell, RwLock};
use tracing::{debug, error, info};

// Static pool for connection reuse across invocations
static POOL: OnceCell<PgPool> = OnceCell::const_new();
static SCHEMA_CACHE: OnceCell<Arc<RwLock<postrust_core::SchemaCache>>> = OnceCell::const_new();

#[tokio::main]
async fn main() -> Result<(), Error> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(false)
        .without_time()
        .init();

    info!("Starting Postrust Lambda handler");

    run(service_fn(handler)).await
}

async fn handler(event: Request) -> Result<Response<Body>, Error> {
    let config = postrust_core::AppConfig::from_env();

    // Get or create pool
    let pool = POOL
        .get_or_init(|| async {
            info!("Creating database pool");
            PgPoolOptions::new()
                .max_connections(1) // Single connection for Lambda
                .connect(&config.db_uri)
                .await
                .expect("Failed to connect to database")
        })
        .await;

    // Get or create schema cache
    let schema_cache = SCHEMA_CACHE
        .get_or_init(|| async {
            info!("Loading schema cache");
            let cache = postrust_core::SchemaCache::load_with_search_path(
                pool,
                &config.db_schemas,
                &config.db_extra_search_path,
            )
                .await
                .expect("Failed to load schema cache");
            Arc::new(RwLock::new(cache))
        })
        .await;

    // Process request
    match process_lambda_request(event, pool, schema_cache, &config).await {
        Ok(response) => Ok(response),
        Err(e) => {
            error!("Request error: {}", e);
            Ok(error_response(e))
        }
    }
}

async fn process_lambda_request(
    event: Request,
    pool: &PgPool,
    schema_cache: &Arc<RwLock<postrust_core::SchemaCache>>,
    config: &postrust_core::AppConfig,
) -> Result<Response<Body>, postrust_core::Error> {
    let jwt_config = postrust_auth::JwtConfig {
        secret: config.jwt_secret.clone(),
        secret_is_base64: config.jwt_secret_is_base64,
        audience: config.jwt_aud.clone(),
        role_claim_key: config.jwt_role_claim_key.clone(),
        anon_role: config.db_anon_role.clone(),
    };

    // Extract auth header
    let auth_header = event
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok());

    // Authenticate
    let auth_result = postrust_auth::authenticate(auth_header, &jwt_config)
        .map_err(|e| postrust_core::Error::InvalidJwt(e.to_string()))?;

    debug!("Authenticated as role: {}", auth_result.role);

    // Parse request body
    let body_bytes = match event.body() {
        Body::Empty => bytes::Bytes::new(),
        Body::Text(s) => bytes::Bytes::from(s.clone()),
        Body::Binary(b) => bytes::Bytes::from(b.clone()),
    };

    // Build HTTP request for parsing
    let mut builder = http::Request::builder()
        .method(event.method().clone())
        .uri(event.uri().clone());

    for (key, value) in event.headers() {
        builder = builder.header(key, value);
    }

    let http_request = builder
        .body(body_bytes.clone())
        .map_err(|e: http::Error| postrust_core::Error::Internal(e.to_string()))?;

    // Parse API request
    let mut api_request = postrust_core::parse_request(
        &http_request,
        config.default_schema(),
        &config.db_schemas,
        config.db_max_rows,
    )?;

    // Parse payload
    if !body_bytes.is_empty() {
        let payload = postrust_core::api_request::payload::parse_payload(
            body_bytes,
            &api_request.content_media_type,
        )?;
        api_request.payload = payload;
    }

    // Get schema cache
    let cache = schema_cache.read().await;

    // Create execution plan
    let plan = postrust_core::create_action_plan(&api_request, &cache)?;

    // Build and execute query
    let query = postrust_core::query::build_query(&plan, Some(&auth_result.role))?;

    if !query.has_main() {
        return Ok(Response::builder()
            .status(200)
            .body(Body::from("[]"))
            .unwrap());
    }

    let (sql, _params) = query.build_main();
    debug!("Executing SQL: {}", sql);

    // Execute query
    let rows = sqlx::query(&sql)
        .fetch_all(pool)
        .await
        .map_err(|e| postrust_core::Error::Internal(e.to_string()))?;

    // Convert to JSON using the shared typed conversion. Reading every column
    // as a `serde_json::Value` here (as this used to) only decodes json/jsonb
    // and silently turns integers, text and timestamps into null.
    //
    // `into_iter` frees each row as it is converted rather than holding every
    // row alongside every JSON value.
    let json_rows: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|row| postrust_core::row_json::row_to_json(&row))
        .collect();

    let body = serde_json::to_string(&json_rows).unwrap_or_else(|_| "[]".to_string());

    Ok(Response::builder()
        .status(200)
        .header("content-type", "application/json")
        .body(Body::from(body))
        .unwrap())
}

fn error_response(error: postrust_core::Error) -> Response<Body> {
    let status = error.status_code().as_u16();
    let body = serde_json::to_string(&error.to_json()).unwrap_or_else(|_| "{}".to_string());

    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(Body::from(body))
        .unwrap()
}
