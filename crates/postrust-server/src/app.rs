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

/// Column carrying the parent's whole row, for computed relationships.
///
/// A computed relationship is a function taking the parent row. By the time an
/// embed expression runs, the parent is a derived table, and a derived table's
/// alias has type `record` -- which PostgreSQL will not cast to the table's
/// composite type. So the row is captured one level down, where the real table
/// is still in scope and a bare reference to it yields the composite, and the
/// embed expression reads it from there. It is stripped from the response like
/// any other column added for embedding.
const PARENT_ROW_COLUMN: &str = "pgrst_parent_row";

/// Marks an embedded object whose columns belong to its parent.
///
/// A spread has no key of its own in the response -- its columns land in the
/// parent object. In SQL it is still one JSON column like any other embed, so
/// it is given a name nothing can collide with and dissolved into its parent
/// once the rows are JSON. Doing it there rather than in SQL is what lets a
/// spread nest inside another, and lets it spread a computed relationship.
const SPREAD_KEY_PREFIX: &str = "pgrst_spread_";
const PARENT_ROW_COLUMN_REF: &str = "\"src\".\"pgrst_parent_row\"";

/// Main request handler.
pub async fn handle_request(State(state): State<Arc<AppState>>, request: Request) -> Response {
    let method = request.method().clone();
    let path = request.uri().path().to_string();

    debug!("{} {}", method, path);

    let verbatim_db_errors = state.config.compat_mode;

    match process_request(state, request).await {
        Ok(response) => response.into_response(),
        Err(e) => error_response(e, verbatim_db_errors).into_response(),
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
    let auth_result = authenticate(auth_header, &state.jwt_config).map_err(jwt_error)?;

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

    // `application/vnd.pgrst.plan` asks for the query plan instead of the
    // result. It is off unless a server turns it on -- it says a great deal
    // about the schema and the data -- and a server that has not is required
    // to say so rather than answer with something else.
    if let Some(plan) = requested_plan(&api_request) {
        return Err(postrust_core::Error::InvalidMediaType(plan));
    }

    // A body that names different columns row by row cannot be written by one
    // statement. Skipped when `?columns=` says which columns to write, since
    // then the rows are free to differ.
    if api_request.query_params.columns.is_none() {
        if let Some(payload) = &api_request.payload {
            postrust_core::api_request::payload::validate_uniform_keys(payload)?;
        }
    }

    // A write with nothing to write is refused here rather than left to
    // produce an `UPDATE ... SET` with no assignments, which PostgreSQL
    // reports as a syntax error -- true, and no use at all to the client.
    if api_request.payload.is_none() && needs_payload(&api_request) {
        return Err(postrust_core::Error::InvalidBody(
            "Empty or invalid json".into(),
        ));
    }

    // Get schema cache
    let schema_cache = state.schema_cache().await;

    // Embedding joins on a column of the parent row, so that column has to be
    // selected even when the client did not ask for it. Any column added this
    // way is removed from the response once the embed is attached.
    let added_join_columns = add_embed_join_columns(&mut api_request, &schema_cache)?;

    if !state.config.db_aggregates_enabled && selects_an_aggregate(&api_request) {
        return Err(postrust_core::Error::AggregatesNotAllowed);
    }

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
    // A singular response the result cannot satisfy is the client's business,
    // not an internal failure: it asked for one object and the query answered
    // with some other number of rows.
    let response = format_response(&api_request, &result).map_err(|e| match e {
        postrust_response::FormatError::MultipleRows | postrust_response::FormatError::NotFound => {
            postrust_core::Error::NotSingular {
                rows: result.rows.len(),
            }
        }
        other => postrust_core::Error::Internal(other.to_string()),
    })?;

    Ok(build_response(response))
}

/// The plan media type a request asked for, when that is all it will accept.
///
/// A request that would also take JSON gets JSON; one that will take only the
/// plan is refused, and the message names what it asked for.
fn requested_plan(api_request: &ApiRequest) -> Option<String> {
    let accepted: Vec<&str> = api_request
        .accept_media_types
        .iter()
        .map(|media| media.content_type())
        .collect();

    match accepted
        .iter()
        .all(|media| media.starts_with("application/vnd.pgrst.plan"))
        && !accepted.is_empty()
    {
        true => Some(accepted.join(", ")),
        false => None,
    }
}

/// Which kind of mutation this request is, if it is one.
fn mutation_kind(api_request: &ApiRequest) -> Option<postrust_core::api_request::Mutation> {
    use postrust_core::api_request::{Action, DbAction};

    match &api_request.action {
        Action::Db(DbAction::RelationMut { mutation, .. }) => Some(mutation.clone()),
        _ => None,
    }
}

/// Split a leading `WITH ... AS (...)` off the front of a statement.
///
/// Returns the clause -- with its trailing space, or empty when there is none
/// -- and the statement that follows it. A `WITH` containing an `INSERT`,
/// `UPDATE` or `DELETE` may only appear at the top level, so anything that
/// wraps the statement has to wrap what follows the clause and put the clause
/// back in front.
fn split_leading_cte(sql: &str) -> (String, &str) {
    let Some(open) = sql.strip_prefix("WITH ").and_then(|rest| rest.find('(')) else {
        return (String::new(), sql);
    };
    let open = open + "WITH ".len();

    let mut depth = 0usize;
    let mut quoted = None::<char>;
    for (index, ch) in sql[open..].char_indices() {
        match (quoted, ch) {
            (Some(quote), ch) if ch == quote => quoted = None,
            (Some(_), _) => {}
            (None, '\'') | (None, '"') => quoted = Some(ch),
            (None, '(') => depth += 1,
            (None, ')') => {
                depth -= 1;
                if depth == 0 {
                    let end = open + index + 1;
                    return (format!("{} ", &sql[..end]), sql[end..].trim_start());
                }
            }
            _ => {}
        }
    }

    (String::new(), sql)
}

/// Render a map of strings as a JSON object.
///
/// PostgreSQL will not accept every header name as a setting name -- a `-` is
/// not allowed in one from version 14 -- so the whole map goes into a single
/// setting as JSON and a function picks out what it wants with `->>`.
fn json_object<'a>(entries: impl IntoIterator<Item = (&'a String, &'a String)>) -> String {
    serde_json::Value::Object(
        entries
            .into_iter()
            .map(|(name, value)| (name.clone(), serde_json::Value::String(value.clone())))
            .collect(),
    )
    .to_string()
}

/// The prefix given to primary key columns added only to build `Location`.
const LOCATION_KEY_PREFIX: &str = "pgrst_location_";

/// Whether this request writes rows of a table.
fn is_relation_mutation(api_request: &ApiRequest) -> bool {
    use postrust_core::api_request::{Action, DbAction};

    matches!(
        &api_request.action,
        Action::Db(DbAction::RelationMut { .. })
    )
}

/// Whether this request writes the row its URL names.
fn is_upsert(api_request: &ApiRequest) -> bool {
    use postrust_core::api_request::{Action, DbAction, Mutation};

    matches!(
        &api_request.action,
        Action::Db(DbAction::RelationMut {
            mutation: Mutation::SingleUpsert,
            ..
        })
    )
}

/// The `Location` of a newly created row, and removal of the key columns that
/// were added to find it.
///
/// `/projects?id=eq.7` -- the address the row can be read back from, which is
/// what a 201 is required to give. A key column that came back null is
/// addressed with `is.null`, since `eq.` would match nothing.
fn build_location(
    api_request: &ApiRequest,
    rows: &mut [serde_json::Value],
    keys: &[String],
) -> Option<String> {
    if keys.is_empty() {
        return None;
    }

    let table = api_request.path.rsplit('/').next()?;
    let mut conditions = Vec::with_capacity(keys.len());

    for key in keys {
        let value = rows
            .first()?
            .get(key)
            .or_else(|| rows.first()?.get(format!("{}{}", LOCATION_KEY_PREFIX, key)))?;
        conditions.push(match value {
            serde_json::Value::Null => format!("{}=is.null", urlencode(key)),
            serde_json::Value::String(text) => {
                format!("{}=eq.{}", urlencode(key), urlencode(text))
            }
            other => format!("{}=eq.{}", urlencode(key), urlencode(&other.to_string())),
        });
    }

    for row in rows.iter_mut() {
        if let Some(object) = row.as_object_mut() {
            for key in keys {
                object.remove(&format!("{}{}", LOCATION_KEY_PREFIX, key));
            }
        }
    }

    Some(format!("/{}?{}", table, conditions.join("&")))
}

/// Percent-encode everything a query-string value may not carry literally.
fn urlencode(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(*byte as char)
            }
            other => encoded.push_str(&format!("%{:02X}", other)),
        }
    }
    encoded
}

/// Whether this request must carry a body.
///
/// Insert, update and upsert all write values that can only come from one.
/// A delete names its rows in the query string, and a function call may take
/// no arguments at all.
fn needs_payload(api_request: &ApiRequest) -> bool {
    use postrust_core::api_request::{Action, DbAction, Mutation};

    matches!(
        &api_request.action,
        Action::Db(DbAction::RelationMut {
            mutation: Mutation::Create | Mutation::Update | Mutation::SingleUpsert,
            ..
        })
    )
}

/// Classify a JWT failure the way PostgREST reports it.
///
/// A token that could not be read at all and one that was read and found
/// wanting are different answers to the client: the first says the credential
/// is unusable, the second says what about it was unacceptable. PostgREST
/// gives them different codes, and clients branch on them.
fn jwt_error(error: postrust_auth::JwtError) -> postrust_core::Error {
    use postrust_auth::JwtError;

    match error {
        JwtError::MissingHeader => postrust_core::Error::MissingAuth,
        JwtError::Expired => postrust_core::Error::JwtClaim("JWT expired".into()),
        JwtError::NotYetValid => postrust_core::Error::JwtClaim("JWT not yet valid".into()),
        JwtError::InvalidAudience => postrust_core::Error::JwtClaim("JWT not in audience".into()),
        JwtError::MissingRole => postrust_core::Error::JwtClaim("Parsing claims failed".into()),
        JwtError::InvalidSignature => {
            postrust_core::Error::InvalidJwt("JWT cryptographic operation failed".into())
        }
        JwtError::InvalidHeaderFormat => {
            postrust_core::Error::InvalidJwt("Unsupported token type".into())
        }
        JwtError::InvalidToken(_) => {
            postrust_core::Error::InvalidJwt("No suitable key or wrong key type".into())
        }
    }
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
            // A media type the schema renders itself replaces the whole
            // response body: the aggregate is applied over the rows the
            // request selected, so it sees exactly what would otherwise have
            // been serialised as JSON.
            let media_handler = {
                let schema_cache = state.schema_cache().await;
                read_target(api_request, Some(db_plan), &schema_cache).and_then(|qi| {
                    api_request.accept_media_types.iter().find_map(|media| {
                        schema_cache
                            .media_handler(&api_request.schema, media.content_type(), &qi)
                            .map(|handler| {
                                (
                                    media.content_type().to_string(),
                                    format!(
                                        "{}.{}",
                                        postrust_sql::escape_ident(&handler.aggregate.schema),
                                        postrust_sql::escape_ident(&handler.aggregate.name)
                                    ),
                                    handler.table.is_some(),
                                )
                            })
                    })
                })
            };

            // Carry the parent's whole row out of the inner query when a
            // computed relationship needs it as an argument. It has to be
            // taken here, where the real table is still in scope: a bare
            // reference to the table yields its composite type, whereas the
            // derived table it becomes one level up yields a `record`.
            let needs_parent_row = added_join_columns.iter().any(|c| c == PARENT_ROW_COLUMN)
                || matches!(&media_handler, Some((_, _, true)));
            let db_plan = &if needs_parent_row {
                let mut adjusted = db_plan.clone();
                // A function's result is a relation too, named after the
                // function, so the row is reachable there by exactly the same
                // means -- which is what makes a computed relationship work on
                // `/rpc/getallvideogames` and not only on a table.
                let (tree, relation) = match &mut adjusted {
                    DbActionPlan::Read(tree) => {
                        let relation = tree.root.from.name.clone();
                        (Some(tree), relation)
                    }
                    DbActionPlan::Call { call, read } => {
                        let relation = call.function.name.clone();
                        (read.as_mut(), relation)
                    }
                    DbActionPlan::MutateRead { .. } => (None, String::new()),
                };

                if let Some(tree) = tree {
                    // An empty select means every column; naming the row would
                    // otherwise replace them rather than join them.
                    if tree.root.select.is_empty() {
                        tree.root
                            .select
                            .push(postrust_core::plan::CoercibleSelectField::simple("*", ""));
                    }

                    let mut field =
                        postrust_core::plan::CoercibleField::simple(relation, String::new());
                    field.full_row = true;
                    tree.root
                        .select
                        .push(postrust_core::plan::CoercibleSelectField {
                            field,
                            aggregate: None,
                            aggregate_cast: None,
                            cast: None,
                            alias: Some(PARENT_ROW_COLUMN.to_string()),
                        });
                }
                adjusted
            } else {
                db_plan.clone()
            };

            // The `Location` of a created row is built from its primary key,
            // which the client may not have selected. The columns are added
            // under names of our own so they cannot collide with anything the
            // request asked for, and are taken back out of the response once
            // the header is built.
            let location_keys: Vec<String> = match db_plan {
                DbActionPlan::MutateRead {
                    mutate: postrust_core::plan::MutatePlan::Insert { pk_cols, .. },
                    ..
                } if !is_upsert(api_request) => pk_cols.clone(),
                _ => Vec::new(),
            };
            // Whether the write created a row or merged into an existing one.
            // The statement is the only place that can say, so the answer is
            // carried out of it as a column and taken back out of the response.
            let reports_inserted = matches!(
                db_plan,
                DbActionPlan::MutateRead {
                    mutate: postrust_core::plan::MutatePlan::Insert {
                        on_conflict: Some(_),
                        reports_inserted: true,
                        ..
                    },
                    ..
                }
            );

            let db_plan = &if location_keys.is_empty() && !reports_inserted {
                db_plan.clone()
            } else {
                let mut adjusted = db_plan.clone();
                if let DbActionPlan::MutateRead {
                    read: Some(tree), ..
                } = &mut adjusted
                {
                    for key in &location_keys {
                        tree.root.select.push(
                            postrust_core::plan::CoercibleSelectField::with_alias(
                                key,
                                "",
                                &format!("{}{}", LOCATION_KEY_PREFIX, key),
                            ),
                        );
                    }
                    if reports_inserted {
                        tree.root
                            .select
                            .push(postrust_core::plan::CoercibleSelectField::simple(
                                postrust_core::query::INSERTED_COLUMN,
                                "boolean",
                            ));
                    }
                }
                adjusted
            };

            // Build SQL
            let query = postrust_core::query::build_query(
                &ActionPlan::Db(db_plan.clone()),
                Some(&auth.role),
            )?;

            if !query.has_main() {
                return Ok(QueryResult::default());
            }

            let (mut sql, params) = query.build_main();

            // A `Prefer: count` needs the same query without its page: the
            // total is what the filters match, not what this page returned.
            // LIMIT and OFFSET render as literals, so dropping them leaves the
            // placeholders and their numbering untouched and the count query
            // can be bound with the very same parameters.
            let mut count_sql = match api_request.preferences.count {
                Some(_) => unpaged_sql(db_plan, &auth.role)?,
                None => None,
            };

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
            let mut embed_filters = EmbedFilters {
                filters: &api_request.query_params.filters,
                orders: &api_request.query_params.order,
                ranges: &api_request.query_params.ranges,
                logic: &api_request.query_params.logic,
                params: Vec::new(),
                base: params.len(),
                max_rows: api_request.max_rows,
                alias_counter: 0,
            };

            // The single-query form cannot express a spread, whose columns have
            // to land in the parent object rather than under a key of their
            // own. One anywhere in the tree sends the whole request down the
            // two-query path, which can: taking the single-query path for the
            // plain relations would embed those and drop the spread, since the
            // two-query path only runs when the first produced nothing.
            let embed_parent = {
                let schema_cache = state.schema_cache().await;
                read_target(api_request, Some(db_plan), &schema_cache)
            };
            let embed_level = match embed_parent.clone() {
                Some(parent_qi)
                    if api_request.query_params.select.iter().any(|item| {
                        matches!(
                            item,
                            postrust_core::api_request::SelectItem::Relation { .. }
                                | postrust_core::api_request::SelectItem::SpreadRelation { .. }
                        )
                    }) =>
                {
                    let schema_cache = state.schema_cache().await;
                    // A computed relationship takes the parent's row, and a
                    // mutation's result is a CTE whose row type is anonymous
                    // -- there is nothing to pass it. Saying so here leaves
                    // the embed out rather than referring to a column the
                    // query does not have.
                    let parent_row = match is_mutation(db_plan) {
                        true => "",
                        false => PARENT_ROW_COLUMN_REF,
                    };
                    build_embed_expressions(
                        &schema_cache,
                        &parent_qi,
                        "src",
                        parent_row,
                        &api_request.query_params.select,
                        &mut embed_filters,
                        &[],
                        &[],
                    )?
                }
                _ => EmbedLevel::default(),
            };
            let mut embed_level = embed_level;

            // Ordering by an embedded column is resolved here rather than in
            // the read plan: the plan has no way to reach another table.
            if let (Some(parent_qi), DbActionPlan::Read(tree)) = (&embed_parent, db_plan) {
                let schema_cache = state.schema_cache().await;
                embed_level.orders = build_embed_orders(
                    &schema_cache,
                    parent_qi,
                    &tree.root.order,
                    &mut embed_filters,
                )?;
            }

            // Filter parameters are appended in the order the predicates were
            // renumbered, so placeholder N in the wrapped SQL lines up with
            // params[N - 1].
            let mut params = params;
            params.extend(embed_filters.params);

            // Built whenever there were relations at all, not only when they
            // produced expressions: `clients!inner()` contributes no column
            // but does narrow the parent, and its predicate binds parameters
            // that have already been appended -- left unapplied, the query is
            // sent with more parameters than it uses.
            if embed_level.saw_relations {
                // `!inner` removes parent rows, so it has to be applied before
                // the page is taken. Left where it is, the inner query's
                // LIMIT would run first and the join would then trim an
                // already-truncated page -- a request for one row could come
                // back empty. So the inner query is rebuilt unpaged and the
                // range moves out to the wrapper, after the join.
                //
                // Rebuilding is safe for parameter numbering: LIMIT and OFFSET
                // render as literals, so dropping them leaves the placeholders
                // and their count untouched.
                //
                // Ordering by an embedded resource's column needs the same
                // treatment for the same reason: the value being ordered on is
                // only reachable out here, so the page cannot be taken before
                // it is known.
                let orders_by_embed = matches!(db_plan, DbActionPlan::Read(tree)
                    if tree.root.order.iter().any(|t| t.relation.is_some()));

                let mut page = None;
                if !embed_level.inner_joins.is_empty() || orders_by_embed {
                    if let DbActionPlan::Read(tree) = db_plan {
                        let mut unpaged = tree.clone();
                        page = Some(std::mem::take(&mut unpaged.root.range));
                        sql = postrust_core::query::build_query(
                            &ActionPlan::Db(DbActionPlan::Read(unpaged)),
                            Some(&auth.role),
                        )?
                        .build_main()
                        .0;
                    }
                }

                let mut projection = String::from("src.*");
                for (field_name, expression) in &embed_level.expressions {
                    projection.push_str(", ");
                    projection.push_str(expression);
                    projection.push_str(" AS ");
                    projection.push_str(&postrust_sql::escape_ident(field_name));
                }
                // A data-modifying `WITH` has to stay at the top level:
                // PostgreSQL refuses one nested inside a subquery, which is
                // where wrapping the whole statement would put it. The clause
                // is lifted out and put back in front of the wrapper.
                let inner_sql = std::mem::take(&mut sql);
                let (with_clause, inner_sql) = split_leading_cte(&inner_sql);
                sql = format!(
                    "{}SELECT {} FROM ({}) AS src",
                    with_clause, projection, inner_sql
                );

                // `?clients=is.null` asks whether the embed matched anything,
                // not about a column. The embed's expression is the thing to
                // test, and it is only in scope out here -- referring to the
                // name it is given would be referring to a column of the very
                // select that defines it.
                let mut predicates = embed_level.inner_joins.clone();
                for filter in &api_request.query_params.filters_root {
                    let Some((_, exists)) = embed_level
                        .filterable
                        .iter()
                        .find(|(name, _)| name == &filter.field.name)
                    else {
                        continue;
                    };
                    let negated = match &filter.op_expr.operation {
                        postrust_core::api_request::Operation::Is(
                            postrust_core::api_request::IsValue::Null,
                        ) => filter.op_expr.negated,
                        postrust_core::api_request::Operation::Is(
                            postrust_core::api_request::IsValue::NotNull,
                        ) => !filter.op_expr.negated,
                        _ => continue,
                    };
                    // `is.null` asks for parents the embed did not match, so
                    // the existence test is the negation of it.
                    predicates.push(match negated {
                        true => exists.clone(),
                        false => format!("NOT {}", exists),
                    });
                }

                if !predicates.is_empty() {
                    sql.push_str(" WHERE ");
                    sql.push_str(&predicates.join(" AND "));
                }

                if !embed_level.orders.is_empty() {
                    sql.push_str(" ORDER BY ");
                    sql.push_str(&embed_level.orders.join(", "));
                }

                // `!inner` and `?rel=is.null` decide which parent rows survive,
                // so the count has to be taken after them -- the same wrapper,
                // over the unpaged query.
                count_sql = count_sql.map(|base| {
                    let (with_clause, base) = split_leading_cte(&base);
                    let mut counted = format!(
                        "{}SELECT {} FROM ({}) AS src",
                        with_clause, projection, base
                    );
                    if !predicates.is_empty() {
                        counted.push_str(" WHERE ");
                        counted.push_str(&predicates.join(" AND "));
                    }
                    counted
                });

                if let Some(range) = page {
                    if let Some(limit) = range.limit {
                        sql.push_str(&format!(" LIMIT {}", limit));
                    }
                    if range.offset > 0 {
                        sql.push_str(&format!(" OFFSET {}", range.offset));
                    }
                }
            }

            // `application/geo+json` has a rendering of its own even where no
            // schema declares one: a FeatureCollection whose geometry is the
            // table's geometry column and whose properties are the rest. A
            // schema-declared handler overrides it, which is why this only
            // runs when none matched.
            let geojson_column = if media_handler.is_none() {
                let schema_cache = state.schema_cache().await;
                read_target(api_request, Some(db_plan), &schema_cache)
                    .filter(|_| {
                        api_request
                            .accept_media_types
                            .iter()
                            .any(|m| matches!(m, postrust_core::api_request::MediaType::GeoJson))
                    })
                    .and_then(|qi| {
                        schema_cache.get_table(&qi).and_then(|table| {
                            table
                                .columns
                                .values()
                                .find(|c| {
                                    matches!(c.nominal_type.as_str(), "geometry" | "geography")
                                })
                                .map(|c| c.name.clone())
                        })
                    })
            } else {
                None
            };

            if let Some(column) = &geojson_column {
                // The geometry column already arrives as jsonb, since a
                // user-defined type is rendered by the database. For a
                // geometry that rendering is GeoJSON with a `crs` key, and
                // dropping that key is exactly `ST_AsGeoJSON` -- which also
                // means this needs no PostGIS function in scope.
                //
                // The name is a real column's, read from the catalogue rather
                // than from the request, but it is quoted both ways all the
                // same: as an identifier to read the column, and as a literal
                // to drop that key from the properties.
                sql = format!(
                    "SELECT json_build_object('type', 'FeatureCollection', 'features', \
                     COALESCE(json_agg(json_build_object('type', 'Feature', 'geometry', \
                     pgrst_geo.{column} - 'crs', 'properties', \
                     to_jsonb(pgrst_geo) - '{key}')), '[]'::json))::text \
                     AS pgrst_body FROM ({sql}) pgrst_geo",
                    column = postrust_sql::escape_ident(column),
                    key = column.replace('\'', "''"),
                    sql = sql
                );
            }

            if let Some((media_type, aggregate, over_row)) = &media_handler {
                // The aggregate takes the table's own row type. A derived
                // table yields `record`, which will not cast to it, so where
                // the handler names a table the aggregate is applied to the
                // column carrying the real row -- the same column a computed
                // relationship reads. A handler taking `anyelement` has no
                // such requirement and takes the derived row directly.
                let argument = if *over_row {
                    format!(
                        "pgrst_media.{}",
                        postrust_sql::escape_ident(PARENT_ROW_COLUMN)
                    )
                } else {
                    "pgrst_media".to_string()
                };
                sql = format!(
                    "SELECT {}({})::text AS pgrst_body FROM ({}) pgrst_media",
                    aggregate, argument, sql
                );
                debug!("Rendering {} via {}", media_type, aggregate);
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

            // A read request runs in a read-only transaction.
            //
            // Without this a GET can change data: `GET /rpc/some_volatile_fn`
            // happily executes whatever the function does. PostgreSQL is the
            // right place to enforce it, since it covers anything reachable
            // from the statement -- a volatile function, a trigger, a rule --
            // rather than only what the planner thought to look at.
            if is_read_only(api_request) {
                sqlx::query("SET TRANSACTION READ ONLY")
                    .execute(&mut *tx)
                    .await
                    .map_err(map_sqlx_error)?;
            }

            // Everything the request tells the database about itself, in
            // one statement.
            //
            // These are `SET LOCAL` by another spelling -- `set_config(k, v,
            // true)` -- and a function or an RLS policy reads them back with
            // `current_setting`. Sent as one `SELECT` rather than one
            // statement each, which is a round trip per setting otherwise, and
            // there are eight of them before a request has done any work.
            //
            // The order is PostgREST's: the settings a role carries, then the
            // role, then everything about the request. The role goes on after
            // its own settings so that a `GRANT SET ON PARAMETER` on the
            // authenticator still applies when they are set.
            {
                let mut path = vec![api_request.schema.clone()];
                path.extend(state.config.db_extra_search_path.iter().cloned());
                let search_path = path
                    .iter()
                    .map(|s| postrust_sql::escape_ident(s))
                    .collect::<Vec<_>>()
                    .join(", ");

                // The claims carry the role, as PostgREST's do: a policy
                // reading `request.jwt.claims` finds the role it is running as
                // whether or not the token named one.
                let mut claims: serde_json::Map<String, serde_json::Value> = auth
                    .claims
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
                claims.insert(
                    "role".to_string(),
                    serde_json::Value::String(auth.role.clone()),
                );

                let mut settings: Vec<(String, String)> = vec![
                    ("search_path".to_string(), search_path),
                    ("role".to_string(), auth.role.clone()),
                    (
                        "request.jwt.claims".to_string(),
                        serde_json::Value::Object(claims).to_string(),
                    ),
                    ("request.method".to_string(), api_request.method.clone()),
                    ("request.path".to_string(), api_request.path.clone()),
                    (
                        "request.headers".to_string(),
                        json_object(&api_request.headers),
                    ),
                    (
                        "request.cookies".to_string(),
                        json_object(&api_request.cookies),
                    ),
                ];

                // `Prefer: timezone` is applied to the session, so every value
                // the database renders -- and every `now()` a function calls --
                // sees it. An unknown zone is PostgreSQL's to reject, and it
                // does, with a message naming the value the client sent.
                if let Some(timezone) = &api_request.preferences.timezone {
                    settings.push(("timezone".to_string(), timezone.clone()));
                }

                let calls = (0..settings.len())
                    .map(|i| format!("set_config(${}, ${}, true)", i * 2 + 1, i * 2 + 2))
                    .collect::<Vec<_>>()
                    .join(", ");
                let sql = format!("SELECT {}", calls);
                let mut query = sqlx::query(&sql);
                for (name, value) in &settings {
                    query = query.bind(name).bind(value);
                }
                query.execute(&mut *tx).await.map_err(map_sqlx_error)?;
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
            // A rendered media type is a single value, not a row set: the
            // aggregate already produced the entire body.
            if let Some(media_type) = media_handler
                .as_ref()
                .map(|(m, _, _)| m.clone())
                .or_else(|| {
                    geojson_column
                        .as_ref()
                        .map(|_| "application/geo+json".to_string())
                })
                .or_else(|| declared_media_type(db_plan, api_request))
            {
                use sqlx::Row;
                // Read as bytes where the media type is not text: a value that
                // is a PNG is not a string, and asking for one back gets
                // nothing at all.
                let body = rows
                    .first()
                    .and_then(|row| {
                        row.try_get::<Option<String>, _>(0)
                            .map(|text| text.map(String::into_bytes))
                            .or_else(|_| row.try_get::<Option<Vec<u8>>, _>(0))
                            .ok()
                    })
                    .flatten()
                    .unwrap_or_default();
                tx.commit()
                    .await
                    .map_err(|e| postrust_core::Error::ConnectionPool(e.to_string()))?;
                return Ok(QueryResult {
                    status: StatusCode::OK,
                    raw_body: Some((media_type, body)),
                    ..QueryResult::default()
                });
            }

            let mut json_rows: Vec<serde_json::Value> = rows
                .into_iter()
                .map(|row| postrust_core::row_json::row_to_json(&row))
                .collect();

            // Embeds already came back with the parent query when the SELECT
            // list carried relations. This path remains for anything the
            // single-query form did not handle.
            let two_query_parent = {
                let schema_cache = state.schema_cache().await;
                read_target(api_request, Some(db_plan), &schema_cache)
            };
            if let Some(parent_qi) = two_query_parent.filter(|_| !embed_level.saw_relations) {
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

            // A function can set the response's status and headers, and only
            // it knows whether it did -- so the settings are read back after
            // it has run, on the same transaction. Reads cannot set them
            // without a function of their own having run, which is what a
            // call or a mutation is.
            let (guc_headers, guc_status) =
                match is_mutation(db_plan) || matches!(db_plan, DbActionPlan::Call { .. }) {
                    false => (None, None),
                    true => {
                        use sqlx::Row;
                        let row = sqlx::query(
                            "SELECT current_setting('response.headers', true), \
                         current_setting('response.status', true)",
                        )
                        .fetch_one(&mut *tx)
                        .await
                        .map_err(map_sqlx_error)?;
                        // Empty is absent. Once any transaction on this
                        // connection has defined a custom setting, PostgreSQL
                        // keeps the name for the rest of the session and
                        // reports it as `''` rather than null -- so a request
                        // that set nothing would otherwise inherit "the empty
                        // headers" from whichever request last set some.
                        let setting = |index: usize| {
                            row.try_get::<Option<String>, _>(index)
                                .ok()
                                .flatten()
                                .filter(|value| !value.trim().is_empty())
                        };
                        (setting(0), setting(1))
                    }
                };

            // Both are the function's own words, so both are checked before
            // they reach the response: a malformed one is a fault in the
            // schema, and saying so is more use than a header the client
            // cannot see or a status it cannot explain.
            if let Some(headers) = &guc_headers {
                if postrust_response::parse_guc_headers(headers).is_none() {
                    return Err(postrust_core::Error::InvalidGucHeaders);
                }
            }
            let guc_status = match guc_status {
                None => None,
                Some(status) => Some(
                    status
                        .trim()
                        .parse::<u16>()
                        .ok()
                        .and_then(|code| StatusCode::from_u16(code).ok())
                        .ok_or(postrust_core::Error::InvalidGucStatus)?,
                ),
            };

            // The count runs on the same transaction and so under the same
            // snapshot as the page it describes.
            let total = match (&api_request.preferences.count, &count_sql) {
                (Some(preference), Some(count_sql)) => {
                    resolve_count(
                        &mut tx,
                        preference,
                        count_sql,
                        &params,
                        api_request.max_rows,
                    )
                    .await?
                }
                _ => None,
            };

            // Reads take no locks worth holding and write plans commit their
            // work, so the transaction is committed either way.
            tx.commit()
                .await
                .map_err(|e| postrust_core::Error::ConnectionPool(e.to_string()))?;

            // Drop columns that were only selected to join the embeds.
            //
            // Except where the embed answers to that very name: embedding
            // through a column -- `?select=client_id(*)` -- keys the result by
            // the column it joined on, and the embed is what the client asked
            // for. Stripping by name would take it along with the column.
            let embed_keys: std::collections::HashSet<&str> = embed_level
                .expressions
                .iter()
                .map(|(key, _)| key.as_str())
                .collect();

            for column in added_join_columns
                .iter()
                .filter(|column| !embed_keys.contains(column.as_str()))
            {
                for row in json_rows.iter_mut() {
                    if let Some(object) = row.as_object_mut() {
                        object.remove(column);
                    }
                }
            }

            // A spread's columns belong to the object above it. This happens
            // after the join columns are dropped, not before: a spread may
            // legitimately produce a column of the same name as one added for
            // joining, and dropping it afterwards would take the wrong one.
            let spread_columns: std::collections::HashMap<String, Vec<String>> =
                embed_level.spread_columns.iter().cloned().collect();
            for row in json_rows.iter_mut() {
                flatten_spreads(row, &spread_columns);
            }

            // In PostgREST-compatibility mode, reshape RPC responses to match
            // PostgREST: un-nest the function-name-keyed column and return a
            // bare value for non-set-returning functions.
            let (mut json_rows, singular) = if state.config.compat_mode {
                if let ActionPlan::Db(DbActionPlan::Call { call, .. }) = plan {
                    unwrap_rpc_rows(json_rows, call)
                } else {
                    (json_rows, false)
                }
            } else {
                (json_rows, false)
            };

            // A function returning `void` has no result to report. PostgREST
            // answers 204 rather than a body of `null`, which is the honest
            // shape for something that returns nothing.
            let returns_void = matches!(
                db_plan,
                DbActionPlan::Call { call, .. } if call.returns_void
            );

            // `Prefer: max-affected` guards against a filter that turned out
            // to match more than the client meant. Nothing has been committed
            // yet, so refusing here undoes the whole statement.
            if let (Some(limit), postrust_core::api_request::PreferHandling::Strict) = (
                api_request.preferences.max_affected,
                &api_request.preferences.handling,
            ) {
                let affected = json_rows.len() as i64;
                let writes = is_mutation(db_plan) || matches!(db_plan, DbActionPlan::Call { .. });
                if writes && affected > limit {
                    return Err(postrust_core::Error::MaxAffectedExceeded(affected));
                }
            }

            // Any row this statement created rather than merged into. Read
            // before the column is taken back out of the response.
            let created_a_row = reports_inserted
                && json_rows.iter().any(|row| {
                    row.get(postrust_core::query::INSERTED_COLUMN)
                        == Some(&serde_json::Value::Bool(true))
                });
            if reports_inserted {
                for row in json_rows.iter_mut() {
                    if let Some(object) = row.as_object_mut() {
                        object.remove(postrust_core::query::INSERTED_COLUMN);
                    }
                }
            }

            // A `PUT` writes the row its URL names, and exactly that row. The
            // filters were applied to the body, so a body naming some other
            // row wrote nothing -- and one naming several wrote several. Both
            // are the same mistake, and PostgreSQL cannot report it because
            // neither is a database error.
            if is_upsert(api_request) && json_rows.len() != 1 {
                return Err(postrust_core::Error::PutMatchingPk);
            }

            // The created row's own address, taken from its key and then
            // removed from the body -- the client asked for a `?select=`, not
            // for the key.
            let location = build_location(api_request, &mut json_rows, &location_keys);

            // A mutation returns the affected rows only when the caller asked
            // for them; otherwise the body is empty whatever the status.
            let omit_body = is_mutation(db_plan) && !wants_representation(api_request);
            let rows = if omit_body { Vec::new() } else { json_rows };

            // PostgREST reports the returned window on every successful data
            // response -- reads, mutations and RPC alike, but not on errors or
            // OPTIONS. Without an exact count the total is unknown, which
            // renders as `*` and keeps the status at 200.
            // The window starts where the request asked it to, and `offset`
            // overrides the `Range` header exactly as it does when the plan
            // resolves the range -- otherwise the reported window says 0 for a
            // request that plainly skipped rows.
            let offset = api_request
                .query_params
                .ranges
                .get("")
                .map(|range| range.offset)
                .filter(|offset| *offset != 0)
                .unwrap_or(api_request.top_level_range.offset);

            // What a mutation reports is not a window on a result set. An
            // insert or a delete reports no window at all -- `*` -- because
            // the rows it wrote are not a page of anything; an update reports
            // how many it changed; and an upsert reports nothing, the client
            // having named the row itself. PostgREST draws exactly these
            // lines, and a client reading the header expects them.
            use postrust_core::api_request::Mutation;
            let content_range = match mutation_kind(api_request) {
                Some(Mutation::SingleUpsert) => None,
                Some(Mutation::Create) | Some(Mutation::Delete) => {
                    Some(postrust_response::ContentRange::new(1, 0, total))
                }
                Some(Mutation::Update) => Some(postrust_response::ContentRange::new(
                    0,
                    rows.len() as i64 - 1,
                    total,
                )),
                None => Some(postrust_response::ContentRange::from_pagination(
                    offset,
                    rows.len() as i64,
                    total,
                )),
            };

            // An offset past the end of the result is a range the server
            // cannot satisfy, and saying how many rows there actually are is
            // what lets the client correct it. Only reachable with a count
            // preference: without one the total is unknown and there is
            // nothing to be past the end of.
            if content_range
                .as_ref()
                .is_some_and(|range| range.status() == StatusCode::RANGE_NOT_SATISFIABLE)
                && mutation_kind(api_request).is_none()
            {
                return Err(postrust_core::Error::InvalidRange(format!(
                    "An offset of {} was requested, but there are only {} rows.",
                    offset,
                    total.unwrap_or(0)
                )));
            }

            Ok(QueryResult {
                // A status the function set outright wins: it knows things
                // about the outcome that the request's shape cannot say.
                status: match (guc_status, returns_void) {
                    (Some(status), _) => status,
                    (None, true) => StatusCode::NO_CONTENT,
                    (None, false) => {
                        mutation_status(db_plan, api_request, created_a_row, reports_inserted)
                            .unwrap_or_else(|| {
                                content_range
                                    .as_ref()
                                    .map(postrust_response::ContentRange::status)
                                    .unwrap_or(StatusCode::OK)
                            })
                    }
                },
                rows,
                singular,
                omit_body: omit_body || returns_void,
                location,
                content_range,
                guc_headers,
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
    db_plan: Option<&DbActionPlan>,
    schema_cache: &postrust_core::SchemaCache,
) -> Option<postrust_core::api_request::QualifiedIdentifier> {
    use postrust_core::api_request::{Action, DbAction};

    match &api_request.action {
        Action::Db(DbAction::RelationRead { qi, .. }) => Some(qi.clone()),
        // The rows a mutation affected are rows of that table, so `?select=`
        // reaches through them to related resources exactly as it does on a
        // read -- `DELETE /tasks?select=name,project:projects(id)` says what
        // was deleted and what it belonged to.
        Action::Db(DbAction::RelationMut { qi, .. }) => Some(qi.clone()),
        // A function returning a table's rows embeds, and renders, exactly as
        // that table does.
        //
        // Which table that is was settled when the overload was chosen: a
        // name may carry several signatures returning different things, and
        // asking the cache again by name answers for whichever comes first.
        // It also knows the answer for a return type this side cannot read
        // off the name at all, such as a domain over a table.
        Action::Db(DbAction::Routine { qi, .. }) => match db_plan {
            Some(DbActionPlan::Call { read, .. }) => {
                read.as_ref().map(|tree| tree.root.from.clone())
            }
            _ => schema_cache
                .routine_returned_table(qi)
                .map(|table| table.qualified_identifier()),
        },
        _ => None,
    }
}

/// Whether any part of the selection, at any depth, asks for an aggregate.
fn selects_an_aggregate(api_request: &ApiRequest) -> bool {
    use postrust_core::api_request::SelectItem;

    fn walk(items: &[SelectItem]) -> bool {
        items.iter().any(|item| match item {
            SelectItem::Field { aggregate, .. } => aggregate.is_some(),
            SelectItem::Relation { select, .. } => walk(select),
            SelectItem::SpreadRelation { select, .. } => walk(select),
        })
    }

    walk(&api_request.query_params.select)
}

/// Whether this request is a read, and so may run read-only.
///
/// It is the method that decides, not the plan: calling a function is a read
/// over GET and a write over POST, and the same function may be either. Asking
/// the plan instead would class every call as a read and refuse the writes a
/// POST is entitled to make.
fn is_read_only(api_request: &ApiRequest) -> bool {
    use postrust_core::api_request::{Action, DbAction, InvokeMethod};

    match &api_request.action {
        Action::Db(DbAction::RelationRead { .. }) | Action::Db(DbAction::SchemaRead { .. }) => true,
        Action::Db(DbAction::Routine { invoke_method, .. }) => {
            matches!(invoke_method, InvokeMethod::InvRead { .. })
        }
        Action::Db(DbAction::RelationMut { .. }) => false,
        // Metadata only; nothing is executed against the database.
        Action::RelationInfo(_) | Action::RoutineInfo { .. } | Action::SchemaInfo => true,
    }
}

/// The plan's SQL with its page removed, for counting what the filters match.
///
/// Only reads and calls are counted. A mutation's `Content-Range` reports the
/// rows it affected, which is the result set itself.
#[allow(clippy::result_large_err)] // consistent with the crate's error type
fn unpaged_sql(
    db_plan: &postrust_core::plan::DbActionPlan,
    role: &str,
) -> Result<Option<String>, postrust_core::Error> {
    use postrust_core::plan::{ActionPlan, DbActionPlan};

    let unpaged = match db_plan {
        DbActionPlan::Read(tree) => {
            let mut tree = tree.clone();
            tree.root.range = Default::default();
            DbActionPlan::Read(tree)
        }
        DbActionPlan::Call { call, read } => {
            let read = read.as_ref().map(|tree| {
                let mut tree = tree.clone();
                tree.root.range = Default::default();
                tree
            });
            DbActionPlan::Call {
                call: call.clone(),
                read,
            }
        }
        DbActionPlan::MutateRead { .. } => return Ok(None),
    };

    let query = postrust_core::query::build_query(&ActionPlan::Db(unpaged), Some(role))?;
    Ok(query.has_main().then(|| query.build_main().0))
}

/// The total the `Prefer: count` asked for, or `None` when it asked for none.
///
/// `exact` counts the rows the filters match. `planned` asks the query planner
/// what it expects without running anything, which is cheap on a large table
/// and correspondingly rough. `estimated` is the planner's guess only where it
/// matters: below the server's row ceiling the exact count is already paid for,
/// so it is used, and above it the larger of the two is reported -- an estimate
/// that came in under a count we have actually taken would be plainly wrong.
async fn resolve_count(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    preference: &postrust_core::api_request::PreferCount,
    count_sql: &str,
    params: &[postrust_sql::SqlParam],
    max_rows: Option<i64>,
) -> Result<Option<i64>, postrust_core::Error> {
    use postrust_core::api_request::PreferCount;

    match preference {
        PreferCount::Exact => exact_count(tx, count_sql, params).await,
        PreferCount::Planned => planned_count(tx, count_sql, params).await,
        PreferCount::Estimated => match (exact_count(tx, count_sql, params).await?, max_rows) {
            (Some(exact), Some(ceiling)) if exact <= ceiling => Ok(Some(exact)),
            (Some(exact), _) => {
                let planned = planned_count(tx, count_sql, params).await?;
                Ok(Some(exact.max(planned.unwrap_or(exact))))
            }
            (None, _) => planned_count(tx, count_sql, params).await,
        },
    }
}

/// The number of rows the filters match.
async fn exact_count(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    count_sql: &str,
    params: &[postrust_sql::SqlParam],
) -> Result<Option<i64>, postrust_core::Error> {
    use sqlx::Row;

    let sql = format!("SELECT count(*) FROM ({}) AS pgrst_count", count_sql);
    let row = bind_params(sqlx::query(&sql), params)
        .fetch_one(&mut **tx)
        .await
        .map_err(map_sqlx_error)?;
    Ok(row.try_get::<i64, _>(0).ok())
}

/// The number of rows the query planner expects, without running the query.
async fn planned_count(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    count_sql: &str,
    params: &[postrust_sql::SqlParam],
) -> Result<Option<i64>, postrust_core::Error> {
    use sqlx::Row;

    let sql = format!("EXPLAIN (FORMAT JSON) {}", count_sql);
    let row = bind_params(sqlx::query(&sql), params)
        .fetch_one(&mut **tx)
        .await
        .map_err(map_sqlx_error)?;
    let plan: serde_json::Value = row.try_get(0).unwrap_or(serde_json::Value::Null);
    Ok(plan
        .get(0)
        .and_then(|node| node.get("Plan"))
        .and_then(|node| node.get("Plan Rows"))
        .and_then(serde_json::Value::as_i64))
}

/// Whether this plan changes data.
fn is_mutation(db_plan: &postrust_core::plan::DbActionPlan) -> bool {
    matches!(
        db_plan,
        postrust_core::plan::DbActionPlan::MutateRead { .. }
    )
}

/// Whether the caller asked for the affected rows back.
fn wants_representation(api_request: &ApiRequest) -> bool {
    matches!(
        api_request.preferences.representation,
        postrust_core::api_request::PreferRepresentation::Full
    )
}

/// The status PostgREST gives a successful mutation, or `None` for anything
/// that isn't one.
///
/// An insert reports 201 Created. An update or delete reports 204 No Content
/// unless the caller asked for the rows back, in which case there is content
/// to report and it is a plain 200.
fn mutation_status(
    db_plan: &postrust_core::plan::DbActionPlan,
    api_request: &ApiRequest,
    created_a_row: bool,
    // Whether the statement was in a position to say. A write with no conflict
    // clause could only have created rows, and one on a relation that will not
    // give up `xmax` cannot tell either way -- in both cases there is nothing
    // to report but the creation.
    could_have_merged: bool,
) -> Option<StatusCode> {
    use postrust_core::api_request::PreferResolution;
    use postrust_core::plan::{DbActionPlan, MutatePlan};

    let DbActionPlan::MutateRead { mutate, .. } = db_plan else {
        return None;
    };

    let merging = matches!(
        api_request.preferences.resolution,
        Some(PreferResolution::MergeDuplicates)
    );

    Some(match mutate {
        // A `PUT` names the row it is writing, so from the client's side
        // nothing was discovered by the response -- it reports the outcome
        // only where it asked to see the row.
        MutatePlan::Insert { .. } if is_upsert(api_request) => {
            match (
                wants_representation(api_request),
                created_a_row || !could_have_merged,
            ) {
                (false, _) => StatusCode::NO_CONTENT,
                (true, true) => StatusCode::CREATED,
                (true, false) => StatusCode::OK,
            }
        }
        // A `POST` that merged into every row it touched created nothing, and
        // 201 would be a promise that something new is at the `Location`.
        MutatePlan::Insert { .. } if merging && could_have_merged && !created_a_row => {
            StatusCode::OK
        }
        MutatePlan::Insert { .. } => StatusCode::CREATED,
        _ => match wants_representation(api_request) {
            true => StatusCode::OK,
            false => StatusCode::NO_CONTENT,
        },
    })
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

    let Some(parent_qi) = read_target(api_request, None, schema_cache) else {
        return Ok(Vec::new());
    };

    let select = &api_request.query_params.select;
    if select.is_empty() {
        return Ok(Vec::new());
    }

    // `*` already names every column, so no join column needs adding -- and
    // adding one anyway selects it twice, which makes `src.*` ambiguous. The
    // relations are still walked: a computed relationship needs the parent's
    // row carried out, and `*` does not provide that.
    let selects_everything = select
        .iter()
        .any(|item| matches!(item, SelectItem::Field { field, .. } if field.name == "*"));

    // Only a column selected under its own name serves as a join key. An
    // aliased one -- `myId:id` -- arrives in the result as `myId`, so the
    // correlation would look for a column the inner query no longer has; the
    // same goes for one that was cast or reached into with a JSON path.
    let selected: std::collections::HashSet<String> = select
        .iter()
        .filter_map(|item| match item {
            SelectItem::Field {
                field,
                aggregate: None,
                cast: None,
                alias: None,
                ..
            } if field.json_path.is_empty() => Some(field.name.clone()),
            _ => None,
        })
        .collect();

    let mut added = Vec::new();

    for item in select.clone() {
        // A spread joins on a parent column exactly as a plain embed does, so
        // it needs that column selected just the same. Without it the parent
        // row has no key to match children against and every spread column
        // comes back null.
        let (relation, hint) = match &item {
            SelectItem::Relation { relation, hint, .. } => (relation, hint),
            SelectItem::SpreadRelation { relation, hint, .. } => (relation, hint),
            SelectItem::Field { .. } => continue,
        };

        let rel = schema_cache
            .find_relationship(&parent_qi, relation, hint.as_deref(), &parent_qi.schema)?
            .ok_or_else(|| {
                schema_cache.relationship_not_found(
                    &parent_qi,
                    relation,
                    hint.as_deref(),
                    &parent_qi.schema,
                )
            })?;

        let plan = postrust_core::embed::EmbedPlan::resolve(rel, schema_cache)?;

        // A computed relationship joins on nothing -- the parent row is the
        // function's argument. That row has to be carried out of the inner
        // query, which is a column of its own rather than a join key. A
        // mutation has no such row to carry, so the embed is left out there.
        if plan.function.is_some() {
            if is_relation_mutation(api_request) {
                continue;
            }
            if !added.iter().any(|c| c == PARENT_ROW_COLUMN) {
                added.push(PARENT_ROW_COLUMN.to_string());
            }
            continue;
        }

        // Every column the join is on, not just the first: a foreign key over
        // several columns correlates on all of them, and one left unselected
        // is a column the correlation cannot see.
        let join_columns: Vec<String> = match plan.columns.is_empty() {
            true => vec![plan.local_column.clone()],
            false => plan
                .columns
                .iter()
                .map(|(local, _)| local.clone())
                .collect(),
        };

        if selects_everything {
            continue;
        }

        for column in join_columns {
            if !selected.contains(&column) && !added.contains(&column) {
                api_request.query_params.select.push(SelectItem::Field {
                    field: Field::simple(&column),
                    aggregate: None,
                    aggregate_cast: None,
                    cast: None,
                    alias: None,
                });
                added.push(column);
            }
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
/// Filters addressed at embedded resources, and the parameters they bind.
///
/// The embed expressions are assembled as SQL text and wrapped around a main
/// query that already owns `$1..$n`, so any placeholder introduced here has to
/// be renumbered past that point and its value appended in the same order.
struct EmbedFilters<'a> {
    /// Every path-scoped filter on the request, e.g. `(["clients"], id=eq.1)`.
    filters: &'a [(
        postrust_core::api_request::EmbedPath,
        postrust_core::api_request::Filter,
    )],
    /// Values bound by the predicates built so far, in placeholder order.
    params: Vec<postrust_sql::SqlParam>,
    /// How many parameters the main query already uses.
    base: usize,
    /// Ordering asked of each embedded resource, by path.
    orders: &'a [(
        postrust_core::api_request::EmbedPath,
        Vec<postrust_core::api_request::OrderTerm>,
    )],
    /// Ranges asked of each embedded resource, by dotted path.
    ranges: &'a std::collections::HashMap<String, postrust_core::api_request::Range>,
    /// `and=`/`or=` groups asked of each embedded resource, by path.
    logic: &'a [(
        postrust_core::api_request::EmbedPath,
        postrust_core::api_request::LogicTree,
    )],
    /// Row cap applied to each embedded resource.
    max_rows: Option<i64>,
    /// Source of unique subquery aliases across the whole embed tree.
    alias_counter: usize,
}

/// Renumber the placeholders in `sql` so they start after `offset`.
///
/// Scans rather than string-replaces: a naive replacement of `$1` would also
/// corrupt the `$1` inside `$10`.
fn shift_placeholders(sql: &str, offset: usize) -> String {
    let mut out = String::with_capacity(sql.len());
    let mut chars = sql.char_indices().peekable();

    while let Some((i, ch)) = chars.next() {
        if ch != '$' {
            out.push(ch);
            continue;
        }
        let start = i + 1;
        let mut end = start;
        while let Some((j, d)) = chars.peek() {
            if d.is_ascii_digit() {
                end = j + d.len_utf8();
                chars.next();
            } else {
                break;
            }
        }
        match sql[start..end].parse::<usize>() {
            Ok(n) => out.push_str(&format!("${}", n + offset)),
            Err(_) => out.push('$'),
        }
    }

    out
}

impl EmbedFilters<'_> {
    /// A fresh alias, unique across the embed tree.
    fn next_alias(&mut self) -> String {
        self.alias_counter += 1;
        format!("e{}", self.alias_counter)
    }

    /// The `WHERE` fragment for one embedded resource, or `None` if unfiltered.
    #[allow(clippy::result_large_err)] // consistent with the crate's error type
    /// The `ORDER BY` an embedded resource was asked for, if any.
    ///
    /// `clients.order=name.desc` orders the rows inside the embed, which is a
    /// property of the child's own subselect rather than of the parent.
    fn order_for(&self, path: &[String], names: &[String]) -> Option<String> {
        let terms: Vec<String> = self
            .orders
            .iter()
            .filter(|(p, _)| p.as_slice() == path || p.as_slice() == names)
            .flat_map(|(_, terms)| terms.iter())
            .map(|term| {
                use postrust_core::api_request::{OrderDirection, OrderNulls, OrderTerm};
                let (field, direction, nulls) = match term {
                    OrderTerm::Field {
                        field,
                        direction,
                        nulls,
                    }
                    | OrderTerm::Relation {
                        field,
                        direction,
                        nulls,
                        ..
                    } => (field, direction, nulls),
                };

                let mut rendered = postrust_sql::escape_ident(&field.name);
                match direction {
                    Some(OrderDirection::Desc) => rendered.push_str(" DESC"),
                    Some(OrderDirection::Asc) => rendered.push_str(" ASC"),
                    None => {}
                }
                match nulls {
                    Some(OrderNulls::First) => rendered.push_str(" NULLS FIRST"),
                    Some(OrderNulls::Last) => rendered.push_str(" NULLS LAST"),
                    None => {}
                }
                rendered
            })
            .collect();

        match terms.is_empty() {
            true => None,
            false => Some(terms.join(", ")),
        }
    }

    /// The row window an embedded resource was asked for.
    ///
    /// The server's own cap still applies, so an embed cannot be asked for
    /// more rows than the server is willing to return.
    fn range_for(&self, path: &[String], names: &[String]) -> (Option<i64>, i64) {
        let range = self
            .ranges
            .get(&path.join("."))
            .or_else(|| self.ranges.get(&names.join(".")));
        let requested = range.and_then(|r| r.limit);
        let offset = range.map(|r| r.offset).unwrap_or(0);
        let limit = match (requested, self.max_rows) {
            (Some(limit), Some(cap)) => Some(limit.min(cap)),
            (Some(limit), None) => Some(limit),
            (None, cap) => cap,
        };
        (limit, offset)
    }

    #[allow(clippy::result_large_err)] // consistent with the crate's error type
    fn predicate_for(
        &mut self,
        path: &[String],
        names: &[String],
        child_qi: &postrust_core::api_request::QualifiedIdentifier,
        schema_cache: &postrust_core::SchemaCache,
    ) -> Result<Option<String>, postrust_core::Error> {
        let addresses = |candidate: &[String]| candidate == path || candidate == names;

        let matching: Vec<_> = self
            .filters
            .iter()
            .filter(|(filter_path, _)| addresses(filter_path))
            .map(|(_, filter)| filter.clone())
            .collect();

        let matching_logic: Vec<_> = self
            .logic
            .iter()
            .filter(|(logic_path, _)| addresses(logic_path))
            .map(|(_, tree)| tree.clone())
            .collect();

        if matching.is_empty() && matching_logic.is_empty() {
            return Ok(None);
        }

        let table = schema_cache.get_table(child_qi);
        let mut parts = Vec::with_capacity(matching.len());

        for filter in &matching {
            // Inside the child's subselect an unqualified name binds to the
            // child, but only because the child has that column: an unknown
            // name would resolve outward to the correlated parent and filter
            // the wrong table silently. Refuse instead.
            let column = table
                .and_then(|t| t.columns.get(&filter.field.name))
                .ok_or_else(|| {
                    postrust_core::Error::ColumnNotFound(format!(
                        "{}.{}",
                        child_qi.name, filter.field.name
                    ))
                })?;

            let frag =
                postrust_core::query::QueryBuilder::filter_sql(filter, &column.nominal_type)?;
            parts.push(shift_placeholders(
                frag.sql(),
                self.base + self.params.len(),
            ));
            self.params.extend(frag.params().iter().cloned());
        }

        for tree in matching_logic {
            // A name the child does not have would bind outward to the
            // correlated parent and filter the wrong table, exactly as a plain
            // filter would, so the tree is checked before it is rendered.
            let mut unknown = None;
            check_logic_columns(&tree, table, &mut unknown);
            if let Some(name) = unknown {
                return Err(postrust_core::Error::ColumnNotFound(format!(
                    "{}.{}",
                    child_qi.name, name
                )));
            }

            let resolver = |name: &str| -> String {
                table
                    .and_then(|t| t.get_column(name))
                    .map(|c| c.nominal_type.clone())
                    .unwrap_or_else(|| "text".to_string())
            };
            let frag = postrust_core::query::QueryBuilder::logic_sql(&tree, resolver)?;
            parts.push(shift_placeholders(
                frag.sql(),
                self.base + self.params.len(),
            ));
            self.params.extend(frag.params().iter().cloned());
        }

        Ok(Some(format!("({})", parts.join(" AND "))))
    }
}

/// Record the first column a logic tree names that the table does not have.
fn check_logic_columns(
    tree: &postrust_core::api_request::LogicTree,
    table: Option<&postrust_core::schema_cache::Table>,
    unknown: &mut Option<String>,
) {
    use postrust_core::api_request::LogicTree;

    match tree {
        LogicTree::Expr { children, .. } => {
            for child in children {
                check_logic_columns(child, table, unknown);
            }
        }
        LogicTree::Stmt(filter) => {
            if unknown.is_none()
                && table
                    .map(|t| t.get_column(&filter.field.name).is_none())
                    .unwrap_or(true)
            {
                *unknown = Some(filter.field.name.clone());
            }
        }
    }
}

/// One level of the embed tree.
#[derive(Default)]
struct EmbedLevel {
    /// `(response key, SQL expression)` for each relation at this level.
    expressions: Vec<(String, String)>,
    /// `EXISTS` predicates from `!inner` on these relations. They reference
    /// the alias of the level *above*, so the caller applies them.
    inner_joins: Vec<String>,
    /// `ORDER BY` expressions for terms naming an embedded resource.
    orders: Vec<String>,
    /// Each embed under the name the client used for it, with its
    /// expression. A spread's response key is internal, so this is what a
    /// filter naming the embed -- `?clients=is.null` -- is matched against.
    filterable: Vec<(String, String)>,
    /// For each spread, the keys it contributes to its parent.
    ///
    /// Needed because a spread that matched nothing still carries them --
    /// as nulls, or as empty arrays for a to-many -- and by then there is no
    /// row to read the names off.
    spread_columns: Vec<(String, Vec<String>)>,
    /// Whether this level had any relation to embed at all.
    ///
    /// Distinct from having produced an expression: `clients()` is a relation
    /// that deliberately contributes nothing, and the fallback path must not
    /// take an absent expression for a relation the single-query form could
    /// not handle.
    saw_relations: bool,
}

/// The keys a spread's selection contributes to the object above it.
///
/// `*` names every column of the related table, an alias renames the column it
/// precedes, and a nested spread contributes its own keys, having already been
/// flattened into this one.
fn spread_output_names(
    schema_cache: &postrust_core::SchemaCache,
    child_qi: &postrust_core::api_request::QualifiedIdentifier,
    select: &[postrust_core::api_request::SelectItem],
) -> Vec<String> {
    use postrust_core::api_request::SelectItem;

    let mut names = Vec::new();
    for item in select {
        match item {
            SelectItem::Field { field, .. } if field.name == "*" => {
                if let Some(table) = schema_cache.get_table(child_qi) {
                    names.extend(table.columns.keys().cloned());
                }
            }
            SelectItem::Field { field, alias, .. } => {
                names.push(alias.clone().unwrap_or_else(|| field.name.clone()));
            }
            SelectItem::Relation {
                relation, alias, ..
            } => names.push(alias.clone().unwrap_or_else(|| relation.clone())),
            SelectItem::SpreadRelation {
                relation, select, ..
            } => {
                if let Some(rel) = schema_cache
                    .find_relationship(child_qi, relation, None, &child_qi.schema)
                    .ok()
                    .flatten()
                {
                    let target = rel.foreign_table().clone();
                    names.extend(spread_output_names(schema_cache, &target, select));
                }
            }
        }
    }
    names
}

/// Dissolve spread embeds into the objects that contain them.
///
/// A spread arrives as an ordinary JSON column under a reserved name. This
/// walks the response and, wherever it finds one, moves its columns up into
/// the surrounding object and drops the name.
///
/// It recurses first, so a spread nested inside another is already flattened
/// by the time its parent is -- which is what carries a grandchild's columns
/// all the way up.
///
/// Spreading a to-many gives each column an array of that column's values
/// across the matched rows, rather than an array of objects.
fn flatten_spreads(
    value: &mut serde_json::Value,
    columns: &std::collections::HashMap<String, Vec<String>>,
) {
    match value {
        serde_json::Value::Array(items) => {
            for item in items {
                flatten_spreads(item, columns);
            }
        }
        serde_json::Value::Object(object) => {
            for (_, child) in object.iter_mut() {
                flatten_spreads(child, columns);
            }

            let spread_keys: Vec<String> = object
                .keys()
                .filter(|key| key.starts_with(SPREAD_KEY_PREFIX))
                .cloned()
                .collect();

            for key in spread_keys {
                let Some(spread) = object.remove(&key) else {
                    continue;
                };
                let names = columns.get(&key).cloned().unwrap_or_default();
                match spread {
                    serde_json::Value::Object(columns) => {
                        for (name, column) in columns {
                            object.insert(name, column);
                        }
                    }
                    // A to-many: one array per column rather than an array of
                    // objects, so the rows are transposed. With no rows the
                    // columns are still named, each holding an empty array.
                    serde_json::Value::Array(rows) => {
                        for name in names {
                            let column = rows
                                .iter()
                                .map(|row| {
                                    row.get(&name).cloned().unwrap_or(serde_json::Value::Null)
                                })
                                .collect();
                            object.insert(name, serde_json::Value::Array(column));
                        }
                    }
                    // The relationship matched nothing. The keys are still
                    // the client's to expect, so they are there, holding null.
                    _ => {
                        for name in names {
                            object.insert(name, serde_json::Value::Null);
                        }
                    }
                }
            }
        }
        _ => {}
    }
}

/// Build the `ORDER BY` expressions for terms naming an embedded resource.
///
/// `order=clients(name)` orders by a column the parent does not have, so each
/// term becomes a scalar subselect fetched per parent row -- the same shape as
/// the embed itself, reduced to one column.
#[allow(clippy::result_large_err)] // consistent with the crate's error type
fn build_embed_orders(
    schema_cache: &postrust_core::SchemaCache,
    parent_qi: &postrust_core::api_request::QualifiedIdentifier,
    order: &[postrust_core::plan::CoercibleOrderTerm],
    ctx: &mut EmbedFilters<'_>,
) -> Result<Vec<String>, postrust_core::Error> {
    let mut orders = Vec::new();

    for term in order.iter() {
        let Some(relation) = &term.relation else {
            continue;
        };

        let rel = schema_cache
            .find_relationship(parent_qi, relation, None, &parent_qi.schema)?
            .ok_or_else(|| {
                schema_cache.relationship_not_found(parent_qi, relation, None, &parent_qi.schema)
            })?;

        // A resource that yields many rows per parent has no single value to
        // order on, so there is nothing the request could mean.
        if !rel.is_to_one() {
            return Err(postrust_core::Error::RelatedOrderNotPossible {
                origin: parent_qi.name.clone(),
                relation: relation.clone(),
            });
        }

        let plan = postrust_core::embed::EmbedPlan::resolve(rel, schema_cache)?;

        let child_alias = ctx.next_alias();

        // The column belongs to the related table, so its type has to be read
        // from there. Resolved against the parent it would be unknown, and an
        // unknown type is one this process asks the database to render -- for
        // a `jsonb` column that would wrap it in `to_jsonb` and then reach
        // into the wrapper.
        let child_qi = postrust_core::api_request::QualifiedIdentifier::new(
            &plan.foreign_schema,
            &plan.foreign_table,
        );
        let mut field = term.field.clone();
        field.to_json = !field.json_path.is_empty()
            && !schema_cache
                .get_table(&child_qi)
                .and_then(|table| table.get_column(&field.name))
                .is_some_and(|column| matches!(column.data_type.as_str(), "json" | "jsonb"));

        let column =
            postrust_core::query::QueryBuilder::qualified_column_sql(Some(&child_alias), &field);
        let mut expression =
            plan.order_expression("src", PARENT_ROW_COLUMN_REF, &child_alias, &column);

        match term.direction {
            Some(postrust_core::api_request::OrderDirection::Desc) => expression.push_str(" DESC"),
            Some(postrust_core::api_request::OrderDirection::Asc) => expression.push_str(" ASC"),
            None => {}
        }
        match term.nulls {
            Some(postrust_core::api_request::OrderNulls::First) => {
                expression.push_str(" NULLS FIRST")
            }
            Some(postrust_core::api_request::OrderNulls::Last) => {
                expression.push_str(" NULLS LAST")
            }
            None => {}
        }

        orders.push(expression);
    }

    Ok(orders)
}

#[allow(clippy::result_large_err)] // consistent with the crate's error type
#[allow(clippy::too_many_arguments)] // each names one part of where the embed sits
fn build_embed_expressions(
    schema_cache: &postrust_core::SchemaCache,
    parent_qi: &postrust_core::api_request::QualifiedIdentifier,
    parent_alias: &str,
    // The SQL expression yielding the parent's whole row, which a computed
    // relationship takes as its argument. At a nested level the parent is a
    // real table alias and so is the row; at the top level the parent is a
    // derived table, whose alias is a `record` rather than the table's own
    // composite type, so a column carrying the row is passed instead.
    parent_row: &str,
    select: &[postrust_core::api_request::SelectItem],
    ctx: &mut EmbedFilters<'_>,
    path: &[String],
    // The same path spelled with each relation's own name rather than the
    // alias the request gave it.
    names: &[String],
) -> Result<EmbedLevel, postrust_core::Error> {
    use postrust_core::api_request::SelectItem;

    let mut level = EmbedLevel::default();

    for item in select {
        // A spread is embedded exactly like a plain relation. The two differ
        // only in where the child's columns end up, and that is settled once
        // the rows are JSON -- so here the spread is marked by the name its
        // expression is given, and flattened afterwards.
        let (relation, alias, child_select, join_type, hint, is_spread) = match item {
            SelectItem::Relation {
                relation,
                alias,
                select,
                join_type,
                hint,
            } => (relation, alias.clone(), select, join_type, hint, false),
            SelectItem::SpreadRelation {
                relation,
                select,
                join_type,
                hint,
            } => (relation, None, select, join_type, hint, true),
            SelectItem::Field { .. } => continue,
        };

        level.saw_relations = true;

        let rel = schema_cache
            .find_relationship(parent_qi, relation, hint.as_deref(), &parent_qi.schema)?
            .ok_or_else(|| {
                schema_cache.relationship_not_found(
                    parent_qi,
                    relation,
                    hint.as_deref(),
                    &parent_qi.schema,
                )
            })?;
        let plan = postrust_core::embed::EmbedPlan::resolve(rel, schema_cache)?;

        // No row to pass: the caller said the parent's is not expressible
        // here, and a computed relationship is a function of it.
        if plan.function.is_some() && parent_row.is_empty() {
            continue;
        }

        let child_alias = ctx.next_alias();
        let child_qi = postrust_core::api_request::QualifiedIdentifier::new(
            &plan.foreign_schema,
            &plan.foreign_table,
        );

        // Filters are addressed by the name the client used, which may be
        // either the alias or the relation itself: `c:clients(*)&c.id=eq.1`
        // and `the_tasks:tasks(*)&tasks.id=eq.1` both name the same embed.
        let mut child_path = path.to_vec();
        child_path.push(alias.clone().unwrap_or_else(|| relation.clone()));
        let mut child_names = names.to_vec();
        child_names.push(relation.clone());
        let child_where = ctx.predicate_for(&child_path, &child_names, &child_qi, schema_cache)?;

        // Deeper relations first: they become part of this level's SELECT list.
        let child_row = postrust_sql::escape_ident(&child_alias);
        let nested = build_embed_expressions(
            schema_cache,
            &child_qi,
            &child_alias,
            &child_row,
            child_select,
            ctx,
            &child_path,
            &child_names,
        )?;

        // A nested `!inner` is written against this child's alias, which is
        // only in scope inside this child's own subselect -- so it narrows the
        // child here rather than travelling up to the top-level wrapper.
        let child_where = match (child_where, nested.inner_joins.is_empty()) {
            (existing, true) => existing,
            (Some(existing), false) => Some(format!(
                "{} AND {}",
                existing,
                nested.inner_joins.join(" AND ")
            )),
            (None, false) => Some(nested.inner_joins.join(" AND ")),
        };

        // `!inner` on this relation restricts the parent rows. It is emitted
        // against `parent_alias`, so it belongs to the caller's level; reusing
        // the same predicate string lets both places share placeholders.
        // The child keeps the same alias it has in the embed subselect. The
        // two are sibling scopes, never nested, so there is no collision --
        // and any nested predicate folded into `child_where` already names
        // that alias, so it has to be the one in scope here too.
        if matches!(join_type, Some(postrust_core::api_request::JoinType::Inner)) {
            level.inner_joins.push(plan.inner_join_predicate(
                parent_alias,
                parent_row,
                &child_alias,
                child_where.as_deref(),
            ));
        }

        level.spread_columns.extend(nested.spread_columns);
        let nested_expressions = nested.expressions;

        // The child's own columns. Empty means every column, which is what a
        // relation with no explicit selection asks for.
        let mut parts: Vec<String> = Vec::new();
        let mut project_everything = child_select.is_empty();
        for nested_item in child_select {
            match nested_item {
                SelectItem::Field { field, .. } if field.name == "*" => {
                    project_everything = true;
                }
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
                SelectItem::Relation { .. } | SelectItem::SpreadRelation { .. } => {}
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
        let child_order = ctx.order_for(&child_path, &child_names);
        let (child_limit, child_offset) = ctx.range_for(&child_path, &child_names);
        let expression = plan.embed_expression(
            parent_alias,
            parent_row,
            &child_alias,
            &inner_select,
            child_limit,
            child_offset,
            child_where.as_deref(),
            child_order.as_deref(),
        )?;

        if is_spread {
            let names = spread_output_names(schema_cache, &child_qi, child_select);
            if names.is_empty() {
                // `...clients()` spreads no columns, so there is nothing to
                // fetch and nothing to merge.
                continue;
            }
            level
                .spread_columns
                .push((format!("{}{}", SPREAD_KEY_PREFIX, child_alias), names));
        }

        let key = match is_spread {
            // A name nothing can collide with, since the object it names is
            // dissolved into its parent before anyone sees it.
            true => format!("{}{}", SPREAD_KEY_PREFIX, child_alias),
            false => alias.clone().unwrap_or_else(|| relation.clone()),
        };

        // Under the name the client used, whatever the response key is: a
        // filter naming the embed is written against that name.
        //
        // What it records is an existence test, not the expression. Asking
        // whether a to-many embed is null would never be true -- it renders as
        // `[]`, never as null -- and "did this match anything" is the question
        // either way.
        level.filterable.push((
            alias.clone().unwrap_or_else(|| relation.clone()),
            plan.inner_join_predicate(
                parent_alias,
                parent_row,
                &child_alias,
                child_where.as_deref(),
            ),
        ));

        // `clients()` asks for none of the related resource's columns. It is
        // written to narrow the parent -- with `!inner`, or with a filter on
        // the embed -- and the resource itself is not part of the answer, so
        // it gets no key at all rather than an empty object. The test is
        // recursive: an embed whose only content is such an embed contributes
        // nothing either.
        if !yields_columns(child_select) {
            continue;
        }

        level.expressions.push((key, expression));
    }

    Ok(level)
}

/// The media type a function's own return type declares, where the client
/// asked for it.
///
/// A function returning a domain named `text/plain` returns text, and the
/// value is the whole response. It is still only returned that way to a client
/// that asked: the same value serialised as JSON is a perfectly good answer to
/// a request that did not.
fn declared_media_type(
    db_plan: &DbActionPlan,
    api_request: &postrust_core::api_request::ApiRequest,
) -> Option<String> {
    let DbActionPlan::Call { call, .. } = db_plan else {
        return None;
    };
    let declared = call.media_type.as_deref()?;
    api_request
        .accept_media_types
        .iter()
        .any(|accepted| accepted.content_type() == declared)
        .then(|| declared.to_string())
}

/// Whether a selection produces any keys in the response.
///
/// A relation contributes only what its own selection does, so `tasks()` and
/// `tasks(projects())` are alike empty, while `tasks(name)` is not.
fn yields_columns(select: &[postrust_core::api_request::SelectItem]) -> bool {
    use postrust_core::api_request::SelectItem;

    select.iter().any(|item| match item {
        SelectItem::Field { .. } => true,
        SelectItem::Relation { select, .. } | SelectItem::SpreadRelation { select, .. } => {
            yields_columns(select)
        }
    })
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
            // A spread embed is fetched exactly like a plain one -- the two
            // differ only in where the child's columns end up, which is
            // settled once the rows are in hand.
            let (relation, alias, nested, is_spread, hint) = match item {
                SelectItem::Relation {
                    relation,
                    alias,
                    select: nested,
                    hint,
                    ..
                } => (relation, alias.clone(), nested, false, hint),
                SelectItem::SpreadRelation {
                    relation,
                    select: nested,
                    hint,
                    ..
                } => (relation, None, nested, true, hint),
                SelectItem::Field { .. } => continue,
            };

            let rel = schema_cache
                .find_relationship(parent_qi, relation, hint.as_deref(), &parent_qi.schema)?
                .ok_or_else(|| {
                    schema_cache.relationship_not_found(
                        parent_qi,
                        relation,
                        hint.as_deref(),
                        &parent_qi.schema,
                    )
                })?;

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
                        match schema_cache
                            .find_relationship(&child_qi, relation, None, &child_qi.schema)
                            .ok()
                            .flatten()
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
            // A spread picks its own columns out when it merges them, and does
            // it by the child's own column names, so pruning here would only
            // get in the way.
            let requested: Option<std::collections::HashSet<String>> =
                if nested.is_empty() || is_spread {
                    None
                } else {
                    Some(
                        nested
                            .iter()
                            .flat_map(|nested_item| match nested_item {
                                SelectItem::Field { field, alias, .. } => {
                                    vec![alias.clone().unwrap_or_else(|| field.name.clone())]
                                }
                                SelectItem::Relation {
                                    relation, alias, ..
                                } => vec![alias.clone().unwrap_or_else(|| relation.clone())],
                                // A spread one level down has already merged
                                // its columns into these rows, so they are
                                // part of what was asked for.
                                SelectItem::SpreadRelation { select, .. } => select
                                    .iter()
                                    .filter_map(|column| match column {
                                        SelectItem::Field { field, .. } => Some(field.name.clone()),
                                        _ => None,
                                    })
                                    .collect(),
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
            if is_spread {
                // The child rows carry the table's own column names, so a
                // column is read by its real name and written under its alias.
                let spread_columns: Vec<(String, String)> = nested
                    .iter()
                    .filter_map(|nested_item| match nested_item {
                        SelectItem::Field { field, alias, .. } => Some((
                            field.name.clone(),
                            alias.clone().unwrap_or_else(|| field.name.clone()),
                        )),
                        _ => None,
                    })
                    .collect();
                for row in rows.iter_mut() {
                    postrust_core::embed::spread_into_parent(row, &plan, &grouped, &spread_columns);
                }
            } else {
                let field_name = alias.clone().unwrap_or_else(|| relation.clone());
                for row in rows.iter_mut() {
                    postrust_core::embed::attach_to_parent(row, &field_name, &plan, &grouped);
                }
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
/// Build the response for a failed request.
///
/// `verbatim_db_errors` passes a database failure through as PostgreSQL
/// reported it -- its SQLSTATE, message, detail and hint -- which is what
/// PostgREST does and what a client written against PostgREST branches on.
/// It is off unless compatibility mode is on, because those fields describe
/// the schema rather than the request: a constraint's name, a column that was
/// not selected, the text of a failing query. Everything else is reported the
/// same either way.
fn error_response(error: postrust_core::Error, verbatim_db_errors: bool) -> Response {
    let status = error.status_code();

    // Check if debug mode is enabled
    let debug_mode = std::env::var("PGRST_DEBUG")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false);

    // A function that raised `sqlstate 'PGRST'` supplied the whole response:
    // its status, its headers and its body. Nothing here is the API layer's to
    // decide, including whether to sanitise it -- the schema wrote it.
    if let postrust_core::Error::Database(db) = &error {
        match db.raised_response() {
            Some(Ok(raised)) => {
                let mut builder = Response::builder()
                    .status(status)
                    .header("content-type", "application/json");
                for (name, value) in &raised.headers {
                    builder = builder.header(name, value);
                }
                return builder
                    .body(Body::from(
                        serde_json::to_vec(&raised.body).unwrap_or_default(),
                    ))
                    .unwrap_or_else(|_| Response::new(Body::empty()));
            }
            // The schema meant to write a response and wrote something else.
            // Reported as the fault it is, and reported the same way whether
            // or not database errors are passed through, since this one is
            // about the schema's own text rather than about the request.
            Some(Err(fault)) => {
                return error_response(
                    postrust_core::Error::RaiseNotUnderstood(fault),
                    verbatim_db_errors,
                )
            }
            None => {}
        }
    }

    let body = if debug_mode {
        // Full error details in debug mode
        serde_json::to_vec(&error.to_json()).unwrap_or_default()
    } else if let (true, postrust_core::Error::Database(db)) = (verbatim_db_errors, &error) {
        serde_json::to_vec(&serde_json::json!({
            "code": db.code,
            "message": db.message,
            "details": db.details,
            "hint": db.hint,
        }))
        .unwrap_or_default()
    } else {
        // Sanitized error in production
        let sanitized = serde_json::json!({
            "code": error.code(),
            "message": sanitize_error_message(&error),
            "details": error.details(),
            // A hint says how to correct the request. It is carried by the
            // error itself, so it never has to reach for anything the
            // sanitised message deliberately left out.
            "hint": error.hint(),
        });
        serde_json::to_vec(&sanitized).unwrap_or_default()
    };

    let mut builder = Response::builder()
        .status(status)
        .header("content-type", "application/json");

    // A 401 has to say what would satisfy it, or a client has no way to know
    // it should be sending a token at all.
    if status == StatusCode::UNAUTHORIZED {
        builder = builder.header("www-authenticate", "Bearer");
    }

    builder
        .body(Body::from(body))
        .unwrap_or_else(|_| Response::new(Body::empty()))
}

/// Sanitize error messages for production.
fn sanitize_error_message(error: &postrust_core::Error) -> String {
    use postrust_core::Error;

    // A schema-cache miss names what the client asked for, and the client
    // already knows that -- it is what it sent. Saying so is what makes the
    // difference between "Resource not found" and a message the client can act
    // on, and it discloses nothing it did not supply. What stays sanitised is
    // the database's own account of a failure, which is about the schema
    // rather than about the request.
    match error {
        Error::TableNotFound { name, .. } | Error::NotFound(name) => {
            return format!("Could not find the table '{}' in the schema cache", name)
        }
        Error::FunctionNotFound {
            name,
            params,
            single_param,
            ..
        } => {
            // A body that is one value carries no argument names, so there is
            // no argument list to report -- naming the function is the whole
            // of what was looked for.
            let looked_for =
                match postrust_core::error::names_only_one_param(single_param.as_deref()) {
                    true => name.clone(),
                    false => postrust_core::error::function_signature(name, params),
                };
            return format!(
                "Could not find the function {} in the schema cache",
                looked_for
            );
        }
        Error::UnacceptableSchema { requested, .. } => {
            return format!("Invalid schema: {}", requested)
        }
        // The candidate list is the whole point of the message: it names the
        // signatures that could not be told apart.
        Error::AmbiguousFunction { .. } | Error::AmbiguousRelationship { .. } => {
            // The candidates are the whole point of these messages: they name
            // what could not be told apart.
            return error.to_string();
        }
        _ => {}
    }

    (match error {
        Error::ColumnNotFound(_) => "Column not found",
        // The column is the client's own word and the relation is the one it
        // addressed, so naming both says nothing it did not send.
        Error::UnknownColumn { .. } => return error.to_string(),
        // Each of these says only what the request itself said: the resource
        // it named, the preference it sent, the shape it asked for. Repeating
        // that back tells the client nothing it did not already know, and is
        // the difference between a message it can act on and one it cannot.
        Error::RelationshipNotFound { .. }
        | Error::NotAnEmbeddedResource(_)
        | Error::RelatedOrderNotPossible { .. }
        | Error::InvalidPreferences(_)
        | Error::NotSingular { .. }
        | Error::InvalidRange(_)
        | Error::InvalidBody(_)
        | Error::UnsupportedMethod(_)
        | Error::InvalidRpcMethod(_)
        | Error::InvalidPutFilters
        | Error::PutLimitNotAllowed
        | Error::PutMatchingPk
        | Error::MaxAffectedExceeded(_)
        | Error::InvalidGucHeaders
        | Error::InvalidGucStatus
        | Error::InvalidMediaType(_)
        // Quotes the client's own URL back at it, and names the grammar the
        // API publishes. Neither is the schema's.
        | Error::UnparsableQuery { .. }
        // Says nothing about the schema beyond that one RAISE is malformed,
        // which is what the author has to fix.
        | Error::RaiseNotUnderstood(_)
        | Error::InvalidResourcePath => return error.to_string(),
        Error::InvalidPath(_) => "Invalid request path",
        // What the token failed on is the client's own token, so naming it
        // costs nothing and is the only way it can tell a bad signature from
        // an expired one.
        Error::InvalidJwt(_) | Error::JwtClaim(_) | Error::MissingAuth => return error.to_string(),
        Error::InsufficientPermissions(_) => "Forbidden",

        // A fixed policy message that reveals nothing about the request or the
        // schema, so it is passed through verbatim -- and it is what PostgREST
        // says, which is what a client branching on it will be matching.
        Error::AggregatesNotAllowed => "Use of aggregate functions is not allowed",
        Error::InvalidHeader(_) | Error::InvalidQueryParam(_) => "Invalid request",
        Error::Database(_) => "Database error",
        Error::ConnectionPool(_) => "Service temporarily unavailable",
        Error::Internal(_) => "Internal server error",
        _ => "An error occurred",
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    /// A data-modifying `WITH` may only appear at the top level, so anything
    /// that wraps the statement has to leave the clause where it is.
    #[test]
    fn a_leading_cte_is_lifted_off_the_statement() {
        let (clause, body) = super::split_leading_cte(
            "WITH pgrst_mutation_result AS (DELETE FROM t WHERE n = ')' RETURNING t.id) \
             SELECT \"id\" FROM \"pgrst_mutation_result\"",
        );

        assert_eq!(
            clause,
            "WITH pgrst_mutation_result AS (DELETE FROM t WHERE n = ')' RETURNING t.id) "
        );
        assert_eq!(body, "SELECT \"id\" FROM \"pgrst_mutation_result\"");
    }

    /// A plain read has no clause to lift.
    #[test]
    fn a_statement_without_a_cte_is_left_alone() {
        let sql = "SELECT \"id\" FROM \"test\".\"items\"";
        let (clause, body) = super::split_leading_cte(sql);

        assert!(clause.is_empty());
        assert_eq!(body, sql);
    }

    use super::*;
    use postrust_core::plan::CallParams;
    use postrust_core::QualifiedIdentifier;
    use serde_json::json;

    fn call_plan(name: &str, returns_set: bool) -> CallPlan {
        CallPlan {
            output_columns: Vec::new(),
            media_type: None,
            function: QualifiedIdentifier::new("public", name),
            params: CallParams::None,
            returns_scalar: !returns_set,
            return_type: None,
            returns_set,
            returns_composite: false,
            volatility: "Volatile".into(),
            param_types: Vec::new(),
            returns_void: false,
            variadic_params: Vec::new(),
        }
    }

    fn composite_call_plan(name: &str, returns_set: bool) -> CallPlan {
        CallPlan {
            returns_composite: true,
            returns_scalar: false,
            return_type: None,
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
