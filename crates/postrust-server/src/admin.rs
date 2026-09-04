//! Admin UI module.
//!
//! Provides administrative endpoints when the `admin-ui` feature is enabled:
//! - `/admin` - Admin dashboard
//! - `/admin/openapi.json` - OpenAPI 3.0 specification
//! - `/admin/swagger` - Swagger UI
//! - `/admin/scalar` - Scalar API documentation UI
//! - `/admin/graphql` - GraphQL Playground

// Many items here exist solely to feed the OpenAPI generator: `#[utoipa::path]`
// handler stubs and `ToSchema` request-body structs whose fields are read by the
// derive macro, not by Rust code. `dead_code` is therefore expected in this module.
#![allow(dead_code)]

use axum::{
    extract::State,
    http::StatusCode,
    response::{Html, IntoResponse, Response},
    routing::get,
    Json, Router,
};
use std::sync::Arc;
use utoipa::OpenApi;

use crate::state::AppState;
use postrust_core::OpenApiMode;

/// OpenAPI specification for the Postrust REST API.
#[derive(OpenApi)]
#[openapi(
    info(
        title = "Postrust API",
        version = "0.1.0",
        description = "A PostgREST-compatible REST API for PostgreSQL, with GraphQL support.",
        license(name = "MIT", url = "https://opensource.org/licenses/MIT"),
    ),
    servers(
        (url = "/", description = "Local server")
    ),
    tags(
        (name = "tables", description = "CRUD operations on database tables"),
        (name = "rpc", description = "Remote procedure call endpoints"),
        (name = "graphql", description = "GraphQL API"),
        (name = "admin", description = "Administrative endpoints"),
    ),
    paths(
        get_table,
        post_table,
        patch_table,
        delete_table,
        call_rpc,
        graphql_endpoint,
        admin_dashboard,
        openapi_spec,
    ),
    components(
        schemas(
            TableResponse,
            InsertBody,
            UpdateBody,
            RpcBody,
            GraphQLRequest,
            GraphQLResponse,
            ErrorResponse,
        )
    )
)]
pub struct ApiDoc;

// =============================================================================
// Schema Types for OpenAPI Documentation
// =============================================================================

/// Response from a table query.
#[derive(utoipa::ToSchema, serde::Serialize)]
pub struct TableResponse {
    /// Example: JSON array of rows
    #[schema(example = json!([{"id": 1, "name": "John"}]))]
    data: serde_json::Value,
}

/// Request body for inserting records.
#[derive(utoipa::ToSchema, serde::Deserialize)]
pub struct InsertBody {
    /// JSON object or array of objects to insert
    #[schema(example = json!({"name": "John", "email": "john@example.com"}))]
    #[serde(flatten)]
    data: serde_json::Value,
}

/// Request body for updating records.
#[derive(utoipa::ToSchema, serde::Deserialize)]
pub struct UpdateBody {
    /// JSON object with fields to update
    #[schema(example = json!({"status": "active"}))]
    #[serde(flatten)]
    data: serde_json::Value,
}

/// Request body for RPC calls.
#[derive(utoipa::ToSchema, serde::Deserialize)]
pub struct RpcBody {
    /// Function parameters as JSON object
    #[schema(example = json!({"param1": "value1"}))]
    #[serde(flatten)]
    params: serde_json::Value,
}

/// GraphQL request body.
#[derive(utoipa::ToSchema, serde::Deserialize)]
pub struct GraphQLRequest {
    /// GraphQL query string
    #[schema(example = "{ users { id name } }")]
    query: String,
    /// Optional operation name
    #[schema(example = "GetUsers")]
    operation_name: Option<String>,
    /// Optional variables
    variables: Option<serde_json::Value>,
}

/// GraphQL response body.
#[derive(utoipa::ToSchema, serde::Serialize)]
pub struct GraphQLResponse {
    /// Query result data
    data: Option<serde_json::Value>,
    /// Errors if any
    errors: Option<Vec<serde_json::Value>>,
}

/// Error response format.
#[derive(utoipa::ToSchema, serde::Serialize)]
pub struct ErrorResponse {
    /// Error code
    #[schema(example = "PGRST301")]
    code: String,
    /// Human-readable error message
    #[schema(example = "Table not found")]
    message: String,
    /// Additional details
    details: Option<String>,
    /// Hint for fixing the error
    hint: Option<String>,
}

// =============================================================================
// Path Documentation (for OpenAPI spec generation)
// =============================================================================

/// Query rows from a table.
///
/// Returns rows from the specified table, with optional filtering, ordering, and pagination.
#[utoipa::path(
    get,
    path = "/{table}",
    tag = "tables",
    params(
        ("table" = String, Path, description = "Table or view name"),
        ("select" = Option<String>, Query, description = "Columns to return"),
        ("order" = Option<String>, Query, description = "Column ordering"),
        ("limit" = Option<i32>, Query, description = "Maximum rows to return"),
        ("offset" = Option<i32>, Query, description = "Rows to skip"),
    ),
    responses(
        (status = 200, description = "Rows returned successfully", body = TableResponse),
        (status = 206, description = "Partial content (paginated)"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Table not found", body = ErrorResponse),
    )
)]
async fn get_table() {}

/// Insert rows into a table.
///
/// Creates one or more new rows in the specified table.
#[utoipa::path(
    post,
    path = "/{table}",
    tag = "tables",
    params(
        ("table" = String, Path, description = "Table name"),
    ),
    request_body = InsertBody,
    responses(
        (status = 201, description = "Rows created successfully", body = TableResponse),
        (status = 400, description = "Invalid request body", body = ErrorResponse),
        (status = 401, description = "Unauthorized"),
        (status = 409, description = "Constraint violation", body = ErrorResponse),
    )
)]
async fn post_table() {}

/// Update rows in a table.
///
/// Updates existing rows that match the filter conditions.
#[utoipa::path(
    patch,
    path = "/{table}",
    tag = "tables",
    params(
        ("table" = String, Path, description = "Table name"),
    ),
    request_body = UpdateBody,
    responses(
        (status = 200, description = "Rows updated successfully", body = TableResponse),
        (status = 400, description = "Invalid request body", body = ErrorResponse),
        (status = 401, description = "Unauthorized"),
    )
)]
async fn patch_table() {}

/// Delete rows from a table.
///
/// Deletes rows that match the filter conditions.
#[utoipa::path(
    delete,
    path = "/{table}",
    tag = "tables",
    params(
        ("table" = String, Path, description = "Table name"),
    ),
    responses(
        (status = 200, description = "Rows deleted successfully", body = TableResponse),
        (status = 204, description = "Rows deleted (no content)"),
        (status = 401, description = "Unauthorized"),
    )
)]
async fn delete_table() {}

/// Call a stored procedure.
///
/// Executes a PostgreSQL function and returns the result.
#[utoipa::path(
    post,
    path = "/rpc/{function}",
    tag = "rpc",
    params(
        ("function" = String, Path, description = "Function name"),
    ),
    request_body = RpcBody,
    responses(
        (status = 200, description = "Function executed successfully"),
        (status = 400, description = "Invalid parameters", body = ErrorResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Function not found", body = ErrorResponse),
    )
)]
async fn call_rpc() {}

/// Execute a GraphQL query or mutation.
///
/// Provides full GraphQL support for queries, mutations, and introspection.
#[utoipa::path(
    post,
    path = "/graphql",
    tag = "graphql",
    request_body = GraphQLRequest,
    responses(
        (status = 200, description = "Query executed successfully", body = GraphQLResponse),
        (status = 400, description = "Invalid query", body = GraphQLResponse),
        (status = 401, description = "Unauthorized"),
    )
)]
async fn graphql_endpoint() {}

/// Admin dashboard.
///
/// Landing page with links to API documentation and tools.
#[utoipa::path(
    get,
    path = "/admin",
    tag = "admin",
    responses(
        (status = 200, description = "Admin dashboard HTML"),
    )
)]
async fn admin_dashboard() {}

/// OpenAPI specification.
///
/// Returns the OpenAPI 3.0 specification as JSON.
#[utoipa::path(
    get,
    path = "/admin/openapi.json",
    tag = "admin",
    responses(
        (status = 200, description = "OpenAPI specification", content_type = "application/json"),
    )
)]
async fn openapi_spec() {}

// =============================================================================
// Route Handlers
// =============================================================================

/// Handler for the admin dashboard.
async fn dashboard_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let schema_cache = state.schema_cache.read().await;
    let table_count = schema_cache.tables.len();
    let routine_count = schema_cache.routines.len();
    let relationship_count = schema_cache.relationships.len();
    drop(schema_cache);

    Html(format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Postrust Admin</title>
    <style>
        :root {{
            --bg: #0d1117;
            --card-bg: #161b22;
            --border: #30363d;
            --text: #c9d1d9;
            --text-muted: #8b949e;
            --accent: #58a6ff;
            --accent-hover: #79c0ff;
            --success: #3fb950;
        }}
        * {{ box-sizing: border-box; margin: 0; padding: 0; }}
        body {{
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Helvetica, Arial, sans-serif;
            background: var(--bg);
            color: var(--text);
            line-height: 1.6;
            min-height: 100vh;
        }}
        .container {{
            max-width: 1200px;
            margin: 0 auto;
            padding: 2rem;
        }}
        header {{
            text-align: center;
            margin-bottom: 3rem;
            padding-bottom: 2rem;
            border-bottom: 1px solid var(--border);
        }}
        h1 {{
            font-size: 2.5rem;
            margin-bottom: 0.5rem;
            background: linear-gradient(135deg, var(--accent), var(--success));
            -webkit-background-clip: text;
            -webkit-text-fill-color: transparent;
        }}
        .subtitle {{ color: var(--text-muted); font-size: 1.1rem; }}
        .stats {{
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
            gap: 1.5rem;
            margin-bottom: 3rem;
        }}
        .stat-card {{
            background: var(--card-bg);
            border: 1px solid var(--border);
            border-radius: 12px;
            padding: 1.5rem;
            text-align: center;
        }}
        .stat-value {{
            font-size: 2.5rem;
            font-weight: bold;
            color: var(--accent);
        }}
        .stat-label {{ color: var(--text-muted); margin-top: 0.5rem; }}
        .cards {{
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(280px, 1fr));
            gap: 1.5rem;
        }}
        .card {{
            background: var(--card-bg);
            border: 1px solid var(--border);
            border-radius: 12px;
            padding: 1.5rem;
            transition: border-color 0.2s, transform 0.2s;
        }}
        .card:hover {{
            border-color: var(--accent);
            transform: translateY(-2px);
        }}
        .card h3 {{
            font-size: 1.25rem;
            margin-bottom: 0.75rem;
            display: flex;
            align-items: center;
            gap: 0.5rem;
        }}
        .card p {{ color: var(--text-muted); margin-bottom: 1rem; font-size: 0.9rem; }}
        .card a {{
            display: inline-block;
            background: var(--accent);
            color: #fff;
            text-decoration: none;
            padding: 0.5rem 1rem;
            border-radius: 6px;
            font-size: 0.9rem;
            transition: background 0.2s;
        }}
        .card a:hover {{ background: var(--accent-hover); }}
        .icon {{ font-size: 1.5rem; }}
        footer {{
            text-align: center;
            margin-top: 3rem;
            padding-top: 2rem;
            border-top: 1px solid var(--border);
            color: var(--text-muted);
        }}
        footer a {{ color: var(--accent); text-decoration: none; }}
        footer a:hover {{ text-decoration: underline; }}
    </style>
</head>
<body>
    <div class="container">
        <header>
            <h1>Postrust Admin</h1>
            <p class="subtitle">API Documentation &amp; Development Tools</p>
        </header>

        <div class="stats">
            <div class="stat-card">
                <div class="stat-value">{}</div>
                <div class="stat-label">Tables</div>
            </div>
            <div class="stat-card">
                <div class="stat-value">{}</div>
                <div class="stat-label">Functions</div>
            </div>
            <div class="stat-card">
                <div class="stat-value">{}</div>
                <div class="stat-label">Relationships</div>
            </div>
        </div>

        <div class="cards">
            <div class="card">
                <h3><span class="icon">📋</span> Swagger UI</h3>
                <p>Interactive API documentation with the ability to test endpoints directly.</p>
                <a href="/admin/swagger/">Open Swagger UI</a>
            </div>

            <div class="card">
                <h3><span class="icon">🎨</span> Scalar</h3>
                <p>Modern, beautiful API documentation with a clean interface.</p>
                <a href="/admin/scalar/">Open Scalar</a>
            </div>

            <div class="card">
                <h3><span class="icon">🔮</span> GraphQL Playground</h3>
                <p>Interactive GraphQL IDE for queries, mutations, and schema exploration.</p>
                <a href="/admin/graphql">Open Playground</a>
            </div>

            <div class="card">
                <h3><span class="icon">📄</span> OpenAPI Spec</h3>
                <p>Raw OpenAPI 3.0 specification in JSON format.</p>
                <a href="/admin/openapi.json">View JSON</a>
            </div>
        </div>

        <footer>
            <p>Powered by <a href="https://github.com/postrust/postrust">Postrust</a></p>
        </footer>
    </div>
</body>
</html>"#,
        table_count, routine_count, relationship_count
    ))
}

/// Handler for OpenAPI spec JSON.
///
/// The derived document describes the shape of the API; what is added here
/// describes *this* database -- a path per exposed table, with the operations
/// that table actually allows.
async fn openapi_json_handler(State(state): State<Arc<AppState>>) -> Response {
    // `disabled` means there is no specification to serve, not an empty one.
    if state.config.openapi_mode == OpenApiMode::Disabled {
        return (StatusCode::NOT_FOUND, "OpenAPI output is disabled").into_response();
    }

    let mut doc = ApiDoc::openapi();

    // Behind a reverse proxy the server's own address is not the one clients
    // use, and a "Try it" button that posts to the wrong host is worse than no
    // button. `openapi_server_proxy_uri` is that outside address.
    if let Some(proxy_uri) = &state.config.openapi_server_proxy_uri {
        doc.servers = Some(vec![utoipa::openapi::ServerBuilder::new()
            .url(proxy_uri.clone())
            .description(Some("Configured by openapi_server_proxy_uri"))
            .build()]);
    }

    add_table_paths(&mut doc, &state.schema_cache().await.tables, &state.config);
    Json(doc).into_response()
}

/// Add one path per exposed table.
///
/// Under `follow-privileges` a table contributes only the operations it
/// permits, which is what the schema cache already records from
/// `information_schema.table_privileges`. Under `ignore-privileges` every
/// table contributes all four, whether or not the request would be allowed --
/// which is the point of that mode: it documents the API, not the caller.
///
/// Note the privileges are the connecting role's, read when the cache was
/// loaded. They are not re-evaluated per request against the JWT's role.
fn add_table_paths(
    doc: &mut utoipa::openapi::OpenApi,
    tables: &postrust_core::schema_cache::TablesMap,
    config: &postrust_core::AppConfig,
) {
    use utoipa::openapi::path::{HttpMethod, OperationBuilder, PathItem};

    let ignore_privileges = config.openapi_mode == OpenApiMode::IgnorePrivileges;
    // In compatibility mode the REST surface is also at the root, which is the
    // path a PostgREST client will use.
    let prefix = if config.compat_mode { "" } else { "/api" };
    let default_schema = config.default_schema();

    // Sorted, and with the default schema first within a name.
    //
    // A URL carries no schema -- a table outside the default one is reached by
    // asking for it with `Accept-Profile` -- so two exposed schemas holding the
    // same table name produce the same path. Iterating a `HashMap` and
    // inserting blindly meant one of them silently overwrote the other, and
    // *which* one changed between runs. The default schema is the one served
    // without a header, so it is the one the bare path describes.
    let mut ordered: Vec<_> = tables.iter().collect();
    ordered.sort_by_key(|(qualified, _)| {
        (
            qualified.name.clone(),
            qualified.schema != default_schema,
            qualified.schema.clone(),
        )
    });

    for (qualified, table) in ordered {
        let path = format!("{prefix}/{}", qualified.name);

        // Already described by the default schema, or by an earlier one.
        // Recorded on the entry that won rather than dropped in silence.
        if let Some(existing) = doc.paths.paths.get_mut(&path) {
            let note = format!(
                "Also exposed by schema `{}`; select it with the \
                 `Accept-Profile` header.",
                qualified.schema
            );
            existing.description = Some(match existing.description.take() {
                Some(previous) => format!("{previous} {note}"),
                None => note,
            });
            continue;
        }

        let operation = |summary: &str| {
            OperationBuilder::new()
                .tag("tables")
                .summary(Some(summary.to_string()))
                .description(table.description.clone())
                .build()
        };

        let mut item = PathItem::new(HttpMethod::Get, operation("Read rows"));
        if ignore_privileges || table.insertable {
            item.post = Some(operation("Insert rows"));
        }
        if ignore_privileges || table.updatable {
            item.patch = Some(operation("Update rows"));
        }
        if ignore_privileges || table.deletable {
            item.delete = Some(operation("Delete rows"));
        }
        item.summary = Some(format!("{}.{}", qualified.schema, qualified.name));

        doc.paths.paths.insert(path, item);
    }
}

/// Handler for GraphQL Playground.
async fn graphql_playground_handler() -> impl IntoResponse {
    Html(async_graphql::http::playground_source(
        async_graphql::http::GraphQLPlaygroundConfig::new("/api/graphql"),
    ))
}

/// Handler for Swagger UI (CDN-based).
async fn swagger_ui_handler() -> impl IntoResponse {
    Html(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Postrust API - Swagger UI</title>
    <link rel="stylesheet" href="https://unpkg.com/swagger-ui-dist@5/swagger-ui.css">
    <style>
        html { box-sizing: border-box; overflow-y: scroll; }
        *, *:before, *:after { box-sizing: inherit; }
        body { margin: 0; background: #fafafa; }
        .swagger-ui .topbar { display: none; }
    </style>
</head>
<body>
    <div id="swagger-ui"></div>
    <script src="https://unpkg.com/swagger-ui-dist@5/swagger-ui-bundle.js"></script>
    <script src="https://unpkg.com/swagger-ui-dist@5/swagger-ui-standalone-preset.js"></script>
    <script>
        window.onload = function() {
            SwaggerUIBundle({
                url: "/admin/openapi.json",
                dom_id: '#swagger-ui',
                deepLinking: true,
                presets: [
                    SwaggerUIBundle.presets.apis,
                    SwaggerUIStandalonePreset
                ],
                layout: "StandaloneLayout"
            });
        };
    </script>
</body>
</html>"#,
    )
}

/// Handler for Scalar API docs (CDN-based).
async fn scalar_ui_handler() -> impl IntoResponse {
    Html(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Postrust API - Scalar</title>
    <style>
        body { margin: 0; }
    </style>
</head>
<body>
    <script id="api-reference" data-url="/admin/openapi.json"></script>
    <script src="https://cdn.jsdelivr.net/npm/@scalar/api-reference"></script>
</body>
</html>"#,
    )
}

// =============================================================================
// Router Builder
// =============================================================================

/// Build the admin router with all admin routes.
pub fn admin_router() -> Router<Arc<AppState>> {
    Router::new()
        // Admin dashboard
        .route("/", get(dashboard_handler))
        // OpenAPI spec
        .route("/openapi.json", get(openapi_json_handler))
        // Swagger UI
        .route("/swagger", get(swagger_ui_handler))
        .route("/swagger/", get(swagger_ui_handler))
        // Scalar UI
        .route("/scalar", get(scalar_ui_handler))
        .route("/scalar/", get(scalar_ui_handler))
        // GraphQL Playground
        .route("/graphql", get(graphql_playground_handler))
}

#[cfg(test)]
mod tests {
    use super::*;
    use postrust_core::schema_cache::{Table, TablesMap};

    fn table(schema: &str, name: &str) -> Table {
        Table {
            schema: schema.into(),
            name: name.into(),
            description: None,
            is_view: false,
            is_partitioned: false,
            insertable: true,
            updatable: true,
            deletable: true,
            pk_cols: vec!["id".into()],
            unique_constraints: Vec::new(),
            columns: Default::default(),
            computed_columns: Default::default(),
        }
    }

    fn tables_map(entries: Vec<Table>) -> TablesMap {
        entries
            .into_iter()
            .map(|t| (t.qualified_identifier(), t))
            .collect()
    }

    fn paths_for(tables: TablesMap, config: &postrust_core::AppConfig) -> Vec<String> {
        let mut doc = utoipa::openapi::OpenApiBuilder::new().build();
        add_table_paths(&mut doc, &tables, config);
        let mut paths: Vec<String> = doc.paths.paths.keys().cloned().collect();
        paths.sort();
        paths
    }

    #[test]
    fn a_table_gets_a_path_under_the_api_prefix() {
        let config = postrust_core::AppConfig::default();
        let paths = paths_for(tables_map(vec![table("public", "users")]), &config);
        assert_eq!(paths, vec!["/api/users".to_string()]);
    }

    #[test]
    fn compat_mode_puts_tables_at_the_root() {
        let config = postrust_core::AppConfig {
            compat_mode: true,
            ..Default::default()
        };
        let paths = paths_for(tables_map(vec![table("public", "users")]), &config);
        assert_eq!(paths, vec!["/users".to_string()]);
    }

    #[test]
    fn two_schemas_sharing_a_name_do_not_silently_lose_one() {
        // A URL carries no schema, so both tables want `/api/orders`.
        // Inserting blindly from a HashMap dropped one of them, and which one
        // varied between runs. The default schema wins the path, and the other
        // is recorded on it rather than vanishing.
        let config = postrust_core::AppConfig {
            db_schemas: vec!["public".into(), "api".into()],
            ..Default::default()
        };

        let tables = tables_map(vec![table("api", "orders"), table("public", "orders")]);
        let mut doc = utoipa::openapi::OpenApiBuilder::new().build();
        add_table_paths(&mut doc, &tables, &config);

        assert_eq!(doc.paths.paths.len(), 1);
        let item = doc.paths.paths.get("/api/orders").expect("path present");
        // `public` is the default schema, so it is what the bare path means.
        assert_eq!(item.summary.as_deref(), Some("public.orders"));
        let description = item.description.as_deref().unwrap_or_default();
        assert!(
            description.contains("`api`") && description.contains("Accept-Profile"),
            "the other schema must be named: {description:?}"
        );
    }

    #[test]
    fn the_default_schema_wins_the_path_whichever_way_the_map_iterates() {
        // Same inputs, built in both orders: the result must not depend on it.
        let config = postrust_core::AppConfig {
            db_schemas: vec!["public".into(), "api".into()],
            ..Default::default()
        };

        for order in [
            vec![table("api", "orders"), table("public", "orders")],
            vec![table("public", "orders"), table("api", "orders")],
        ] {
            let mut doc = utoipa::openapi::OpenApiBuilder::new().build();
            add_table_paths(&mut doc, &tables_map(order), &config);
            assert_eq!(
                doc.paths.paths["/api/orders"].summary.as_deref(),
                Some("public.orders")
            );
        }
    }

    #[test]
    fn follow_privileges_omits_what_a_table_does_not_allow() {
        let config = postrust_core::AppConfig::default();
        let mut read_only = table("public", "reports");
        read_only.insertable = false;
        read_only.updatable = false;
        read_only.deletable = false;

        let mut doc = utoipa::openapi::OpenApiBuilder::new().build();
        add_table_paths(&mut doc, &tables_map(vec![read_only]), &config);

        let item = &doc.paths.paths["/api/reports"];
        assert!(item.get.is_some());
        assert!(item.post.is_none());
        assert!(item.patch.is_none());
        assert!(item.delete.is_none());
    }

    #[test]
    fn ignore_privileges_lists_every_operation() {
        let config = postrust_core::AppConfig {
            openapi_mode: OpenApiMode::IgnorePrivileges,
            ..Default::default()
        };
        let mut read_only = table("public", "reports");
        read_only.insertable = false;
        read_only.updatable = false;
        read_only.deletable = false;

        let mut doc = utoipa::openapi::OpenApiBuilder::new().build();
        add_table_paths(&mut doc, &tables_map(vec![read_only]), &config);

        let item = &doc.paths.paths["/api/reports"];
        assert!(item.post.is_some());
        assert!(item.patch.is_some());
        assert!(item.delete.is_some());
    }

    #[test]
    fn test_openapi_spec_generation() {
        let spec = ApiDoc::openapi();
        assert_eq!(spec.info.title, "Postrust API");
        assert!(!spec.paths.paths.is_empty());
    }

    #[test]
    fn test_openapi_has_table_paths() {
        let spec = ApiDoc::openapi();
        // Check that table operations are documented
        assert!(spec.paths.paths.contains_key("/{table}"));
    }

    #[test]
    fn test_openapi_has_rpc_path() {
        let spec = ApiDoc::openapi();
        assert!(spec.paths.paths.contains_key("/rpc/{function}"));
    }

    #[test]
    fn test_openapi_has_graphql_path() {
        let spec = ApiDoc::openapi();
        assert!(spec.paths.paths.contains_key("/graphql"));
    }

    #[test]
    fn test_openapi_has_admin_paths() {
        let spec = ApiDoc::openapi();
        assert!(spec.paths.paths.contains_key("/admin"));
        assert!(spec.paths.paths.contains_key("/admin/openapi.json"));
    }

    #[test]
    fn test_openapi_has_tags() {
        let spec = ApiDoc::openapi();
        // Verify we have tags defined
        assert!(spec.tags.is_some(), "OpenAPI spec should have tags");
        // Serialize to JSON and check tag names
        let json = serde_json::to_value(&spec).unwrap();
        let tags = json["tags"].as_array().unwrap();
        let tag_names: Vec<&str> = tags.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(tag_names.contains(&"tables"));
        assert!(tag_names.contains(&"rpc"));
        assert!(tag_names.contains(&"graphql"));
        assert!(tag_names.contains(&"admin"));
    }
}
