//! Request handling.

use crate::state::AppState;
use axum::{
    body::Body,
    extract::{Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use postrust_auth::authenticate;
use postrust_core::{
    create_action_plan, parse_request, ActionPlan, ApiRequest, CallPlan, DbActionPlan,
};
use postrust_response::{format_response, QueryResult, Response as PgrstResponse};
use std::sync::Arc;
use tracing::{debug, error};

/// Main request handler.
pub async fn handle_request(State(state): State<Arc<AppState>>, request: Request) -> Response {
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
        state.config.db_max_rows,
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

    // Embedding joins on a column of the parent row, so that column has to be
    // selected even when the client did not ask for it. Any column added this
    // way is removed from the response once the embed is attached.
    let added_join_columns = add_embed_join_columns(&mut api_request, &schema_cache)?;

    // Create execution plan
    let plan = create_action_plan(&api_request, &schema_cache)?;

    // Execute plan
    let result = execute_plan(
        &state,
        &api_request,
        &plan,
        &auth_result,
        &added_join_columns,
    )
    .await?;

    // Format response
    let response = format_response(&api_request, &result)
        .map_err(|e| postrust_core::Error::Internal(e.to_string()))?;

    Ok(build_response(response))
}

/// Execute an action plan.
async fn execute_plan(
    state: &AppState,
    api_request: &ApiRequest,
    plan: &ActionPlan,
    auth: &postrust_auth::AuthResult,
    added_join_columns: &[String],
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

            let (mut sql, params) = query.build_main();

            // Embed related resources in the same query.
            //
            // Each relation becomes a correlated subselect in the SELECT list,
            // so PostgreSQL builds the nested JSON while it already has the
            // parent row -- one round trip for the whole tree instead of one per
            // relation per level, and no grouping or attaching in this process.
            //
            // Parent columns stay ordinary typed columns, so they are converted
            // to JSON by the same code as a request without embeds and a NUMERIC
            // or timestamp renders identically either way.
            let embed_expressions = match read_target(api_request) {
                Some(parent_qi)
                    if api_request.query_params.select.iter().any(|item| {
                        matches!(
                            item,
                            postrust_core::api_request::SelectItem::Relation { .. }
                        )
                    }) =>
                {
                    let schema_cache = state.schema_cache().await;
                    let mut counter = 0;
                    build_embed_expressions(
                        &schema_cache,
                        &parent_qi,
                        "src",
                        &api_request.query_params.select,
                        api_request.max_rows,
                        &mut counter,
                    )?
                }
                _ => Vec::new(),
            };

            if !embed_expressions.is_empty() {
                let mut projection = String::from("src.*");
                for (field_name, expression) in &embed_expressions {
                    projection.push_str(", ");
                    projection.push_str(expression);
                    projection.push_str(" AS ");
                    projection.push_str(&postrust_sql::escape_ident(field_name));
                }
                sql = format!("SELECT {} FROM ({}) AS src", projection, sql);
            }

            debug!("Executing SQL: {}", sql);
            debug!("With {} parameters", params.len());

            // Everything for this request runs in one transaction.
            //
            // `SET LOCAL ROLE` and `set_config(..., true)` are scoped to a
            // transaction. Sent on a bare pooled connection they apply to their
            // own implicit single-statement transaction and are gone before the
            // next statement runs, so the query executed as the pool's login
            // role with no JWT claims set -- row-level security and role grants
            // were not being applied at all. PostgreSQL says so in its log:
            // "SET LOCAL can only be used in transaction blocks".
            //
            // Embedding runs on this same transaction, which also means a
            // parent row and its children come from one snapshot rather than
            // from two separate reads.
            let mut tx = state
                .pool
                .begin()
                .await
                .map_err(|e| postrust_core::Error::ConnectionPool(e.to_string()))?;

            // Set role
            sqlx::query(&format!(
                "SET LOCAL ROLE {}",
                postrust_sql::escape_ident(&auth.role)
            ))
            .execute(&mut *tx)
            .await
            .map_err(|e| {
                postrust_core::Error::Database(postrust_core::error::DatabaseError {
                    code: "42501".into(),
                    message: e.to_string(),
                    details: None,
                    hint: None,
                    constraint: None,
                    table: None,
                    column: None,
                })
            })?;

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
                    .execute(&mut *tx)
                    .await
                    .ok(); // Ignore errors for individual claims
            }

            // Execute main query with bound parameters
            let rows = bind_params(sqlx::query(&sql), &params)
                .fetch_all(&mut *tx)
                .await
                .map_err(|e| {
                    error!("Query error: {}", e);
                    map_sqlx_error(e)
                })?;

            // Convert rows to JSON.
            //
            // `into_iter` matters for large result sets: each row's buffers are
            // freed as soon as it has been converted, rather than the whole
            // `Vec<PgRow>` staying alive alongside the whole `Vec<Value>`.
            let mut json_rows: Vec<serde_json::Value> = rows
                .into_iter()
                .map(|row| postrust_core::row_json::row_to_json(&row))
                .collect();

            // Embeds already came back with the parent query when the SELECT
            // list carried relations. This path remains for anything the
            // single-query form did not handle.
            if let Some(parent_qi) =
                read_target(api_request).filter(|_| embed_expressions.is_empty())
            {
                let schema_cache = state.schema_cache().await;
                embed_relations(
                    &mut tx,
                    &schema_cache,
                    &parent_qi,
                    &api_request.query_params.select,
                    &mut json_rows,
                    api_request.max_rows,
                )
                .await?;
            }

            // Reads take no locks worth holding and write plans commit their
            // work, so the transaction is committed either way.
            tx.commit()
                .await
                .map_err(|e| postrust_core::Error::ConnectionPool(e.to_string()))?;

            // Drop columns that were only selected to join the embeds.
            for column in added_join_columns {
                for row in json_rows.iter_mut() {
                    if let Some(object) = row.as_object_mut() {
                        object.remove(column);
                    }
                }
            }

            // In PostgREST-compatibility mode, reshape RPC responses to match
            // PostgREST: un-nest the function-name-keyed column and return a
            // bare value for non-set-returning functions.
            let (json_rows, singular) = if state.config.compat_mode {
                if let ActionPlan::Db(DbActionPlan::Call { call, .. }) = plan {
                    unwrap_rpc_rows(json_rows, call)
                } else {
                    (json_rows, false)
                }
            } else {
                (json_rows, false)
            };

            Ok(QueryResult {
                status: StatusCode::OK,
                rows: json_rows,
                singular,
                ..Default::default()
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

/// Reshape RPC rows for PostgREST-compatibility mode.
///
/// `SELECT * FROM func(...)` names its single output column after the function
/// (for scalar/`json` returns), which serializes to rows like
/// `[{"func": <value>}]`. PostgREST instead returns the bare value. This
/// un-nests that wrapper column and reports whether the result should be
/// rendered as a single (un-arrayed) value — i.e. when the function is not
/// set-returning.
///
/// The decision is driven by the plan's return-type metadata: composite and
/// `record` returns (`RETURNS TABLE`, row types) have real output columns and
/// are never un-nested, even when a single column happens to share the
/// function's name (e.g. `CREATE FUNCTION foo() RETURNS TABLE(foo int)`).
fn unwrap_rpc_rows(
    rows: Vec<serde_json::Value>,
    call: &CallPlan,
) -> (Vec<serde_json::Value>, bool) {
    let singular = !call.returns_set;

    if call.returns_composite {
        return (rows, singular);
    }

    let fname = call.function.name.as_str();
    let unwrapped = rows
        .into_iter()
        .map(|row| match row {
            serde_json::Value::Object(ref map) if map.len() == 1 && map.contains_key(fname) => {
                map.get(fname).cloned().unwrap_or(serde_json::Value::Null)
            }
            other => other,
        })
        .collect();

    (unwrapped, singular)
}

/// The table a read targets, if the request is a read.
fn read_target(
    api_request: &postrust_core::api_request::ApiRequest,
) -> Option<postrust_core::api_request::QualifiedIdentifier> {
    use postrust_core::api_request::{Action, DbAction};

    match &api_request.action {
        Action::Db(DbAction::RelationRead { qi, .. }) => Some(qi.clone()),
        _ => None,
    }
}

/// Ensure every embedded relation's join column is selected on the parent.
///
/// Returns the columns that were added purely to make the join possible, so
/// they can be removed from the response afterwards. An empty select list means
/// "all columns", in which case nothing needs adding.
#[allow(clippy::result_large_err)] // consistent with the crate's error type
fn add_embed_join_columns(
    api_request: &mut postrust_core::api_request::ApiRequest,
    schema_cache: &postrust_core::SchemaCache,
) -> Result<Vec<String>, postrust_core::Error> {
    use postrust_core::api_request::{Field, SelectItem};

    let Some(parent_qi) = read_target(api_request) else {
        return Ok(Vec::new());
    };

    let select = &api_request.query_params.select;
    if select.is_empty() {
        return Ok(Vec::new());
    }

    let selected: std::collections::HashSet<String> = select
        .iter()
        .filter_map(|item| match item {
            SelectItem::Field { field, .. } => Some(field.name.clone()),
            _ => None,
        })
        .collect();

    let mut added = Vec::new();

    for item in select.clone() {
        let SelectItem::Relation { relation, .. } = &item else {
            continue;
        };

        let rel = schema_cache
            .find_relationship(&parent_qi, relation, &parent_qi.schema)
            .ok_or_else(|| postrust_core::Error::RelationshipNotFound(relation.clone()))?;

        let plan = postrust_core::embed::EmbedPlan::resolve(rel, schema_cache)?;

        if !selected.contains(&plan.local_column) && !added.contains(&plan.local_column) {
            api_request.query_params.select.push(SelectItem::Field {
                field: Field::simple(&plan.local_column),
                aggregate: None,
                aggregate_cast: None,
                cast: None,
                alias: None,
            });
            added.push(plan.local_column.clone());
        }
    }

    Ok(added)
}

/// Build the SELECT-list expressions that embed relations in one query.
///
/// Returns one `(field_name, expression)` per requested relation. Each
/// expression is a correlated subselect yielding JSON, so the whole tree comes
/// back from the parent query rather than from a query per relation per level.
///
/// `alias_counter` hands out a distinct alias per level, so a self-referential
/// relationship stays unambiguous.
#[allow(clippy::result_large_err)] // consistent with the crate's error type
fn build_embed_expressions(
    schema_cache: &postrust_core::SchemaCache,
    parent_qi: &postrust_core::api_request::QualifiedIdentifier,
    parent_alias: &str,
    select: &[postrust_core::api_request::SelectItem],
    max_rows: Option<i64>,
    alias_counter: &mut usize,
) -> Result<Vec<(String, String)>, postrust_core::Error> {
    use postrust_core::api_request::SelectItem;

    let mut expressions = Vec::new();

    for item in select {
        let SelectItem::Relation {
            relation,
            alias,
            select: nested,
            ..
        } = item
        else {
            continue;
        };

        let rel = schema_cache
            .find_relationship(parent_qi, relation, &parent_qi.schema)
            .ok_or_else(|| postrust_core::Error::RelationshipNotFound(relation.clone()))?;
        let plan = postrust_core::embed::EmbedPlan::resolve(rel, schema_cache)?;

        *alias_counter += 1;
        let child_alias = format!("e{}", alias_counter);
        let child_qi = postrust_core::api_request::QualifiedIdentifier::new(
            &plan.foreign_schema,
            &plan.foreign_table,
        );

        // Deeper relations first: they become part of this level's SELECT list.
        let nested_expressions = build_embed_expressions(
            schema_cache,
            &child_qi,
            &child_alias,
            nested,
            max_rows,
            alias_counter,
        )?;

        // The child's own columns. Empty means every column, which is what a
        // relation with no explicit selection asks for.
        let mut parts: Vec<String> = Vec::new();
        let mut project_everything = nested.is_empty();
        for nested_item in nested {
            match nested_item {
                SelectItem::Field { field, alias, .. } => {
                    let column = postrust_sql::escape_ident(&field.name);
                    match alias {
                        Some(alias) => parts.push(format!(
                            "{} AS {}",
                            column,
                            postrust_sql::escape_ident(alias)
                        )),
                        None => parts.push(column),
                    }
                }
                // Handled as an expression, not a column.
                SelectItem::Relation { .. } => {}
                SelectItem::SpreadRelation { .. } => project_everything = true,
            }
        }
        if project_everything {
            parts.clear();
            parts.push(format!("{}.*", postrust_sql::escape_ident(&child_alias)));
        }
        for (field_name, expression) in nested_expressions {
            parts.push(format!(
                "{} AS {}",
                expression,
                postrust_sql::escape_ident(&field_name)
            ));
        }

        let inner_select = parts.join(", ");
        let expression =
            plan.embed_expression(parent_alias, &child_alias, &inner_select, max_rows)?;

        expressions.push((
            alias.clone().unwrap_or_else(|| relation.clone()),
            expression,
        ));
    }

    Ok(expressions)
}

type EmbedFuture<'f> = std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<(), postrust_core::Error>> + Send + 'f>,
>;

/// Attach the relations requested in `select` onto `rows`.
///
/// One query per relation per level: the parents' join keys are collected and
/// passed as a single array, so embedding across a page of parents does not
/// become a query per row. Recurses for nested embeds.
fn embed_relations<'f>(
    conn: &'f mut sqlx::PgConnection,
    schema_cache: &'f postrust_core::SchemaCache,
    parent_qi: &'f postrust_core::api_request::QualifiedIdentifier,
    select: &'f [postrust_core::api_request::SelectItem],
    rows: &'f mut [serde_json::Value],
    max_rows: Option<i64>,
) -> EmbedFuture<'f> {
    use postrust_core::api_request::SelectItem;

    Box::pin(async move {
        if rows.is_empty() {
            return Ok(());
        }

        for item in select {
            let SelectItem::Relation {
                relation,
                alias,
                select: nested,
                ..
            } = item
            else {
                continue;
            };

            let rel = schema_cache
                .find_relationship(parent_qi, relation, &parent_qi.schema)
                .ok_or_else(|| postrust_core::Error::RelationshipNotFound(relation.clone()))?;

            let plan = postrust_core::embed::EmbedPlan::resolve(rel, schema_cache)?;
            let keys = postrust_core::embed::parent_keys(rows, &plan.local_column);

            // Which of the child's columns the query needs to return.
            //
            // A column that was not asked for still costs a heap read, JSON
            // serialisation in PostgreSQL, socket bytes and a parse before
            // being discarded, so the projection is worked out before the query
            // rather than filtered afterwards.
            //
            // A nested embed joins on a column of the child row, so that column
            // has to survive the projection even when the client did not ask
            // for it. A spread relation falls back to every column: its shape
            // is decided further down and is not a plain projection.
            let child_qi = postrust_core::api_request::QualifiedIdentifier::new(
                &plan.foreign_schema,
                &plan.foreign_table,
            );

            let mut child_columns: Vec<String> = Vec::new();
            let mut project_everything = nested.is_empty();
            for nested_item in nested {
                match nested_item {
                    SelectItem::Field { field, .. } => child_columns.push(field.name.clone()),
                    SelectItem::Relation { relation, .. } => {
                        match schema_cache.find_relationship(&child_qi, relation, &child_qi.schema)
                        {
                            Some(nested_rel) => {
                                let nested_plan = postrust_core::embed::EmbedPlan::resolve(
                                    nested_rel,
                                    schema_cache,
                                )?;
                                child_columns.push(nested_plan.local_column);
                            }
                            // Leave it to the recursive call to report the
                            // unknown relation, rather than failing here with
                            // less context.
                            None => project_everything = true,
                        }
                    }
                    SelectItem::SpreadRelation { .. } => project_everything = true,
                }
            }
            if project_everything {
                child_columns.clear();
            }

            let mut grouped = if keys.is_empty() {
                std::collections::HashMap::new()
            } else {
                let sql = plan.children_grouped_sql(max_rows, &child_columns)?;

                let fetched = sqlx::query(&sql)
                    .bind(&keys)
                    .fetch_all(&mut *conn)
                    .await
                    .map_err(map_sqlx_error)?;

                // The query returns the join key and a JSON array of that
                // key's children, so the values are read straight out of the
                // columns -- the typed row converter would wrap them under the
                // expression name.
                use sqlx::Row;
                let pairs: Vec<(serde_json::Value, serde_json::Value)> = fetched
                    .into_iter()
                    .filter_map(|row| {
                        Some((
                            row.try_get::<serde_json::Value, _>(0).ok()?,
                            row.try_get::<serde_json::Value, _>(1).ok()?,
                        ))
                    })
                    .collect();

                postrust_core::embed::group_from_aggregated(pairs)
            };

            // Deeper embeds, if any were asked for.
            //
            // A nested embed issues one query for every child row at this
            // level, so the rows have to be contiguous for it. They arrive
            // grouped, so they are flattened for the recursive call and put
            // back afterwards. When nothing deeper was requested this is all
            // skipped, which is the common case.
            let has_deeper_embed = nested.iter().any(|item| {
                matches!(
                    item,
                    SelectItem::Relation { .. } | SelectItem::SpreadRelation { .. }
                )
            });

            if has_deeper_embed {
                let mut order: Vec<(String, usize)> = Vec::with_capacity(grouped.len());
                let mut flat: Vec<serde_json::Value> = Vec::new();
                for (key, children) in grouped.drain() {
                    order.push((key, children.len()));
                    flat.extend(children);
                }

                embed_relations(
                    &mut *conn,
                    schema_cache,
                    &child_qi,
                    nested,
                    &mut flat,
                    max_rows,
                )
                .await?;

                let mut rest = flat.into_iter();
                for (key, count) in order {
                    grouped.insert(key, rest.by_ref().take(count).collect());
                }
            }

            // Return only the requested columns of the related resource. An
            // empty nested select means every column.
            //
            // The projection in the query covers the columns; this covers what
            // is left over, which is the join column when the client did not
            // ask for it.
            let requested: Option<std::collections::HashSet<String>> = if nested.is_empty() {
                None
            } else {
                Some(
                    nested
                        .iter()
                        .filter_map(|nested_item| match nested_item {
                            SelectItem::Field { field, alias, .. } => {
                                Some(alias.clone().unwrap_or_else(|| field.name.clone()))
                            }
                            SelectItem::Relation {
                                relation, alias, ..
                            } => Some(alias.clone().unwrap_or_else(|| relation.clone())),
                            SelectItem::SpreadRelation { .. } => None,
                        })
                        .collect(),
                )
            };

            if let Some(requested) = requested {
                for group in grouped.values_mut() {
                    for child in group.iter_mut() {
                        if let Some(object) = child.as_object_mut() {
                            object.retain(|key, _| requested.contains(key));
                        }
                    }
                }
            }
            let field_name = alias.clone().unwrap_or_else(|| relation.clone());
            for row in rows.iter_mut() {
                postrust_core::embed::attach_to_parent(row, &field_name, &plan, &grouped);
            }
        }

        Ok(())
    })
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
                .map(|pg_err| {
                    (
                        pg_err.detail().map(String::from),
                        pg_err.hint().map(String::from),
                    )
                })
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

#[cfg(test)]
mod tests {
    use super::*;
    use postrust_core::plan::CallParams;
    use postrust_core::QualifiedIdentifier;
    use serde_json::json;

    fn call_plan(name: &str, returns_set: bool) -> CallPlan {
        CallPlan {
            function: QualifiedIdentifier::new("public", name),
            params: CallParams::None,
            returns_scalar: !returns_set,
            returns_set,
            returns_composite: false,
            volatility: "Volatile".into(),
        }
    }

    fn composite_call_plan(name: &str, returns_set: bool) -> CallPlan {
        CallPlan {
            returns_composite: true,
            returns_scalar: false,
            ..call_plan(name, returns_set)
        }
    }

    #[test]
    fn unwraps_json_return_to_bare_object() {
        // `SELECT * FROM sync(...)` on a json-returning function yields a single
        // column named after the function.
        let rows = vec![json!({"sync": {"ok": true, "count": 3}})];
        let (rows, singular) = unwrap_rpc_rows(rows, &call_plan("sync", false));
        assert!(singular, "non-set-returning function should be singular");
        assert_eq!(rows, vec![json!({"ok": true, "count": 3})]);
    }

    #[test]
    fn unwraps_scalar_return() {
        let rows = vec![json!({"add": 42})];
        let (rows, singular) = unwrap_rpc_rows(rows, &call_plan("add", false));
        assert!(singular);
        assert_eq!(rows, vec![json!(42)]);
    }

    #[test]
    fn unwraps_setof_scalar_to_array() {
        let rows = vec![json!({"gen": 1}), json!({"gen": 2})];
        let (rows, singular) = unwrap_rpc_rows(rows, &call_plan("gen", true));
        assert!(!singular, "set-returning function should not be singular");
        assert_eq!(rows, vec![json!(1), json!(2)]);
    }

    #[test]
    fn leaves_multi_column_rows_untouched() {
        // `RETURNS TABLE(...)` / composite set: rows already have real columns
        // and must not be un-nested.
        let rows = vec![json!({"id": 1, "name": "a"}), json!({"id": 2, "name": "b"})];
        let (out, singular) =
            unwrap_rpc_rows(rows.clone(), &composite_call_plan("list_users", true));
        assert!(!singular);
        assert_eq!(out, rows);
    }

    #[test]
    fn leaves_table_column_named_like_function_untouched() {
        // `CREATE FUNCTION foo() RETURNS TABLE(foo int)`: the single output
        // column legitimately shares the function's name. The composite
        // return-type metadata must prevent it from being mistaken for the
        // function-name wrapper.
        let rows = vec![json!({"foo": 1}), json!({"foo": 2})];
        let (out, singular) = unwrap_rpc_rows(rows.clone(), &composite_call_plan("foo", true));
        assert!(!singular);
        assert_eq!(out, rows);
    }

    #[test]
    fn single_composite_return_is_singular_but_not_unwrapped() {
        // A non-set function returning a row type expands to its columns;
        // nothing to un-nest, but the result still renders as a bare object.
        let rows = vec![json!({"id": 1, "name": "a"})];
        let (out, singular) =
            unwrap_rpc_rows(rows.clone(), &composite_call_plan("get_user", false));
        assert!(singular);
        assert_eq!(out, rows);
    }

    #[test]
    fn leaves_single_key_row_untouched_when_key_is_not_function_name() {
        // A single real column that happens to be the only column should not be
        // mistaken for the function-name wrapper.
        let rows = vec![json!({"id": 7})];
        let (out, _) = unwrap_rpc_rows(rows.clone(), &call_plan("get_thing", false));
        assert_eq!(out, rows);
    }
}
