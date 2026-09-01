//! Postrust Cloudflare Workers adapter.
//!
//! Deploys Postrust as a Cloudflare Worker.
//!
//! **This is a stub.** The fetch handler answers with a JSON body saying so;
//! it does not parse requests, reach a database, or return data. A real
//! implementation needs Cloudflare-specific pieces that are not here yet:
//! Hyperdrive for database connections, and KV or Durable Objects for the
//! schema cache.
//!
//! Because of that the crate is on its own 0.x version line rather than the
//! workspace's, and carries no stability promise. See docs/stability.md.

use worker::*;

#[event(fetch)]
async fn main(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    // Log request
    console_log!("{} {}", req.method().to_string(), req.path());

    // Get configuration from environment
    let _db_url = env.var("DATABASE_URL").map(|v| v.to_string()).ok();
    let _jwt_secret = env.secret("JWT_SECRET").map(|v| v.to_string()).ok();
    let _schemas = env
        .var("PGRST_DB_SCHEMAS")
        .map(|v| v.to_string())
        .unwrap_or_else(|_| "public".to_string());

    // For now, return a placeholder response
    // Full implementation would:
    // 1. Use Hyperdrive for database connections
    // 2. Cache schema in KV or Durable Objects
    // 3. Process requests using postrust-core

    let response_body = serde_json::json!({
        "message": "Postrust Worker is running",
        "status": "stub",
        "note": "Full implementation requires Hyperdrive configuration"
    });

    Response::from_json(&response_body)
}

/// Process a request using Postrust core.
///
/// This is a placeholder for the full implementation.
#[allow(dead_code)]
async fn process_request(_req: Request, _env: &Env) -> Result<Response> {
    // In a full implementation:
    // 1. Get Hyperdrive binding: env.hyperdrive("DB")?
    // 2. Parse request using postrust-core
    // 3. Execute query
    // 4. Format response

    Response::error("Not implemented", 501)
}

/// Get or load schema cache from KV.
#[allow(dead_code)]
async fn get_schema_cache(_env: &Env) -> Result<Option<String>> {
    // In a full implementation:
    // 1. Check KV for cached schema
    // 2. If not found, load from database
    // 3. Cache in KV with TTL

    Ok(None)
}
