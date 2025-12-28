//! Request handling.

use crate::state::AppState;
use axum::{
    body::Body,
    extract::{Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use bytes::Bytes;
use postrust_auth::authenticate;
use postrust_core::{create_action_plan, parse_request, ActionPlan, ApiRequest};
use postrust_response::{format_response, QueryResult, Response as PgrstResponse};
use sqlx::Row;
use std::sync::Arc;
use tracing::{debug, error, info, warn};

/// Main request handler.
pub async fn handle_request(
    State(state): State<Arc<AppState>>,
    request: Request,
) -> Response {
    let method = request.method().clone();
    let path = request.uri().path().to_string();

    debug!("{} {}", method, path);

    match process_request(state, request).await {
        Ok(response) => response.into_response(),
        Err(e) => error_response(e).into_response(),
    }
}

/// Process a request and return a response.
async fn process_request(
    state: Arc<AppState>,
    request: Request,
) -> Result<Response, postrust_core::Error> {
    // Extract auth header
    let auth_header = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok());

    // Authenticate
    let auth_result = authenticate(auth_header, &state.jwt_config)
        .map_err(|e| postrust_core::Error::InvalidJwt(e.to_string()))?;

    debug!("Authenticated as role: {}", auth_result.role);

    // Parse request
    let (parts, body) = request.into_parts();
    let body_bytes = axum::body::to_bytes(body, 10 * 1024 * 1024)
        .await
        .map_err(|e| postrust_core::Error::InvalidBody(e.to_string()))?;

    // Build HTTP request for parsing
    let mut builder = http::Request::builder()
        .method(parts.method.clone())
        .uri(parts.uri.clone());

    for (key, value) in &parts.headers {
        builder = builder.header(key, value);
    }

    let http_request = builder
        .body(body_bytes.clone())
        .map_err(|e| postrust_core::Error::Internal(e.to_string()))?;

    // Parse API request
    let mut api_request = parse_request(
        &http_request,
        state.default_schema(),
        state.schemas(),
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
    let schema_cache = state.schema_cache().await;

    // Create execution plan
    let plan = create_action_plan(&api_request, &schema_cache)?;

    // Execute plan
    let result = execute_plan(&state, &api_request, &plan, &auth_result).await?;

    // Format response
    let response = format_response(&api_request, &result)
        .map_err(|e| postrust_core::Error::Internal(e.to_string()))?;

    Ok(build_response(response))
}

/// Execute an action plan.
async fn execute_plan(
    state: &AppState,
    request: &ApiRequest,
    plan: &ActionPlan,
    auth: &postrust_auth::AuthResult,
) -> Result<QueryResult, postrust_core::Error> {
    match plan {
        ActionPlan::Db(db_plan) => {
            // Build SQL
            let query = postrust_core::query::build_query(
                &ActionPlan::Db(db_plan.clone()),
                Some(&auth.role),
            )?;

            if !query.has_main() {
                return Ok(QueryResult::default());
            }

            let (sql, params) = query.build_main();
            debug!("Executing SQL: {}", sql);
            debug!("With {} parameters", params.len());

            // Execute query
            let mut conn = state.pool.acquire().await
                .map_err(|e| postrust_core::Error::ConnectionPool(e.to_string()))?;

            // Set role
            sqlx::query(&format!(
                "SET LOCAL ROLE {}",
                postrust_sql::escape_ident(&auth.role)
            ))
            .execute(&mut *conn)
            .await
            .map_err(|e| postrust_core::Error::Database(postrust_core::error::DatabaseError {
                code: "42501".into(),
                message: e.to_string(),
                details: None,
                hint: None,
                constraint: None,
                table: None,
                column: None,
            }))?;

            // Set claims as GUC
            for (key, value) in &auth.claims {
                let guc_key = format!("request.jwt.claims.{}", key);
                let guc_value = match value {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };

                sqlx::query("SELECT set_config($1, $2, true)")
                    .bind(&guc_key)
                    .bind(&guc_value)
                    .execute(&mut *conn)
                    .await
                    .ok(); // Ignore errors for individual claims
            }

            // Execute main query with bound parameters
            let rows = bind_params(sqlx::query(&sql), &params)
                .fetch_all(&mut *conn)
                .await
                .map_err(|e| {
                    error!("Query error: {}", e);
                    map_sqlx_error(e)
                })?;

            // Convert rows to JSON
            let json_rows: Vec<serde_json::Value> = rows
                .iter()
                .map(|row| row_to_json(row))
                .collect();

            Ok(QueryResult {
                status: StatusCode::OK,
                rows: json_rows,
                total_count: None,
                content_range: None,
                location: None,
                guc_headers: None,
                guc_status: None,
            })
        }
        ActionPlan::Info(info_plan) => {
            use postrust_core::plan::InfoPlan;

            // Return appropriate metadata based on the info type
            let response_data = match info_plan {
                InfoPlan::OpenApiSpec => {
                    // Return basic server info for root endpoint
                    serde_json::json!({
                        "name": "postrust",
                        "version": env!("CARGO_PKG_VERSION"),
                        "description": "PostgREST-compatible REST API for PostgreSQL"
                    })
                }
                InfoPlan::RelationInfo(qi) => {
                    serde_json::json!({
                        "schema": qi.schema,
                        "name": qi.name,
                        "type": "relation"
                    })
                }
                InfoPlan::RoutineInfo(qi) => {
                    serde_json::json!({
                        "schema": qi.schema,
                        "name": qi.name,
                        "type": "routine"
                    })
                }
            };

            Ok(QueryResult {
                status: StatusCode::OK,
                rows: vec![response_data],
                ..Default::default()
            })
        }
    }
}

/// Convert a sqlx row to JSON.
fn row_to_json(row: &sqlx::postgres::PgRow) -> serde_json::Value {
    use sqlx::{Column, Row, TypeInfo};

    let mut map = serde_json::Map::new();

    for column in row.columns() {
        let name = column.name();
        let type_name = column.type_info().name();

        let value = match type_name {
            "INT2" | "SMALLINT" => row
                .try_get::<i16, _>(name)
                .ok()
                .map(|v| serde_json::Value::Number(v.into())),
            "INT4" | "INT" | "INTEGER" => row
                .try_get::<i32, _>(name)
                .ok()
                .map(|v| serde_json::Value::Number(v.into())),
            "INT8" | "BIGINT" => row
                .try_get::<i64, _>(name)
                .ok()
                .map(|v| serde_json::Value::Number(v.into())),
            "FLOAT4" | "REAL" => row
                .try_get::<f32, _>(name)
                .ok()
                .and_then(|v| serde_json::Number::from_f64(v as f64))
                .map(serde_json::Value::Number),
            "FLOAT8" | "DOUBLE PRECISION" => row
                .try_get::<f64, _>(name)
                .ok()
                .and_then(|v| serde_json::Number::from_f64(v))
                .map(serde_json::Value::Number),
            "NUMERIC" | "DECIMAL" => row
                .try_get::<sqlx::types::BigDecimal, _>(name)
                .ok()
                .map(|v| serde_json::Value::String(v.to_string())),
            "BOOL" | "BOOLEAN" => row
                .try_get::<bool, _>(name)
                .ok()
                .map(serde_json::Value::Bool),
            "JSON" | "JSONB" => row.try_get::<serde_json::Value, _>(name).ok(),
            "UUID" => row
                .try_get::<sqlx::types::Uuid, _>(name)
                .ok()
                .map(|v| serde_json::Value::String(v.to_string())),
            "TIMESTAMPTZ" | "TIMESTAMP WITH TIME ZONE" => row
                .try_get::<chrono::DateTime<chrono::Utc>, _>(name)
                .ok()
                .map(|v| serde_json::Value::String(v.to_rfc3339())),
            "TIMESTAMP" | "TIMESTAMP WITHOUT TIME ZONE" => row
                .try_get::<chrono::NaiveDateTime, _>(name)
                .ok()
                .map(|v| serde_json::Value::String(v.to_string())),
            "DATE" => row
                .try_get::<chrono::NaiveDate, _>(name)
                .ok()
                .map(|v| serde_json::Value::String(v.to_string())),
            "TIME" | "TIME WITHOUT TIME ZONE" => row
                .try_get::<chrono::NaiveTime, _>(name)
                .ok()
                .map(|v| serde_json::Value::String(v.to_string())),
            _ => row
                .try_get::<String, _>(name)
                .ok()
                .map(serde_json::Value::String),
        };

        map.insert(name.to_string(), value.unwrap_or(serde_json::Value::Null));
    }

    serde_json::Value::Object(map)
}

/// Bind SqlParam values to a sqlx query.
fn bind_params<'q>(
    mut query: sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments>,
    params: &'q [postrust_sql::SqlParam],
) -> sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments> {
    use postrust_sql::SqlParam;

    for param in params {
        query = match param {
            SqlParam::Null => query.bind(None::<String>),
            SqlParam::Bool(b) => query.bind(b),
            SqlParam::Int(n) => query.bind(n),
            SqlParam::Float(f) => query.bind(f),
            SqlParam::Text(s) => query.bind(s),
            SqlParam::Bytes(b) => query.bind(b),
            SqlParam::Json(j) => query.bind(j),
            SqlParam::Uuid(u) => query.bind(u),
            SqlParam::Timestamp(t) => query.bind(t),
            SqlParam::Array(arr) => {
                // Convert array to Vec<String> for text arrays
                let strings: Vec<String> = arr
                    .iter()
                    .map(|p| match p {
                        SqlParam::Text(s) => s.clone(),
                        SqlParam::Int(n) => n.to_string(),
                        SqlParam::Bool(b) => b.to_string(),
                        other => format!("{:?}", other),
                    })
                    .collect();
                query.bind(strings)
            }
        };
    }

    query
}

/// Map sqlx error to our error type.
fn map_sqlx_error(e: sqlx::Error) -> postrust_core::Error {
    match e {
        sqlx::Error::Database(db_err) => {
            // Try to downcast to Postgres-specific error for additional details
            let (details, hint) = db_err
                .try_downcast_ref::<sqlx::postgres::PgDatabaseError>()
                .map(|pg_err| (pg_err.detail().map(String::from), pg_err.hint().map(String::from)))
                .unwrap_or((None, None));

            postrust_core::Error::Database(postrust_core::error::DatabaseError {
                code: db_err.code().map(|c| c.to_string()).unwrap_or_default(),
                message: db_err.message().to_string(),
                details,
                hint,
                constraint: db_err.constraint().map(|s| s.to_string()),
                table: db_err.table().map(|s| s.to_string()),
                column: None,
            })
        }
        other => postrust_core::Error::Internal(other.to_string()),
    }
}

/// Build an HTTP response from our response type.
fn build_response(response: PgrstResponse) -> Response {
    let mut builder = Response::builder().status(response.status);

    for (key, value) in &response.headers {
        builder = builder.header(key, value);
    }

    builder
        .body(Body::from(response.body))
        .unwrap_or_else(|_| Response::new(Body::empty()))
}

/// Build an error response.
///
/// In production mode (PGRST_DEBUG=false or unset), sensitive error details
/// are hidden to prevent information leakage.
fn error_response(error: postrust_core::Error) -> Response {
    let status = error.status_code();

    // Check if debug mode is enabled
    let debug_mode = std::env::var("PGRST_DEBUG")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false);

    let body = if debug_mode {
        // Full error details in debug mode
        serde_json::to_vec(&error.to_json()).unwrap_or_default()
    } else {
        // Sanitized error in production
        let sanitized = serde_json::json!({
            "code": error.code(),
            "message": sanitize_error_message(&error),
            "details": null,
            "hint": null
        });
        serde_json::to_vec(&sanitized).unwrap_or_default()
    };

    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(Body::from(body))
        .unwrap_or_else(|_| Response::new(Body::empty()))
}

/// Sanitize error messages for production.
fn sanitize_error_message(error: &postrust_core::Error) -> &'static str {
    use postrust_core::Error;
    match error {
        Error::TableNotFound(_) | Error::NotFound(_) => "Resource not found",
        Error::FunctionNotFound(_) => "Function not found",
        Error::ColumnNotFound(_) | Error::UnknownColumn(_) => "Column not found",
        Error::RelationshipNotFound(_) => "Relationship not found",
        Error::InvalidPath(_) => "Invalid request path",
        Error::InvalidBody(_) => "Invalid request body",
        Error::InvalidJwt(_) | Error::JwtExpired | Error::MissingAuth => "Unauthorized",
        Error::InsufficientPermissions(_) => "Forbidden",
        Error::UnacceptableSchema(_) => "Invalid schema",
        Error::InvalidHeader(_) | Error::InvalidQueryParam(_) => "Invalid request",
        Error::Database(_) => "Database error",
        Error::ConnectionPool(_) => "Service temporarily unavailable",
        Error::Internal(_) => "Internal server error",
        _ => "An error occurred",
    }
}
