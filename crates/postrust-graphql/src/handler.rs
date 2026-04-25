//! Axum handler for the /graphql endpoint.
//!
//! Provides GraphQL request handling using async-graphql with dynamic schema
//! generation from the PostgreSQL schema cache.

use crate::context::GraphQLContext;
use crate::error::GraphQLError;
use crate::schema::object::TableObjectType;
use crate::schema::{build_schema, GeneratedSchema, MutationType, SchemaConfig};
use crate::subscription::{
    generate_subscription_fields, NotifyBroker, SubscriptionField as SubField, TableChangePayload,
};
use async_graphql::dynamic::*;
use async_graphql::Value;
use async_graphql_axum::{GraphQLRequest, GraphQLResponse};
use axum::extract::State;
use axum::response::IntoResponse;
use futures::stream::StreamExt;
use postrust_core::schema_cache::SchemaCache;
use sqlx::PgPool;
use uuid::Uuid;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, trace};

/// GraphQL execution state shared across requests.
pub struct GraphQLState {
    /// Database connection pool
    pub pool: PgPool,
    /// Schema cache
    pub schema_cache: Arc<SchemaCache>,
    /// Generated GraphQL schema
    pub generated_schema: GeneratedSchema,
    /// async-graphql Schema (built dynamically)
    pub schema: Schema,
    /// Schema configuration
    pub config: SchemaConfig,
    /// Subscription fields
    pub subscription_fields: Vec<SubField>,
    /// Notification broker for subscriptions
    pub broker: Arc<RwLock<Option<NotifyBroker>>>,
}

impl GraphQLState {
    /// Create new GraphQL state from schema cache.
    pub fn new(
        pool: PgPool,
        schema_cache: Arc<SchemaCache>,
        config: SchemaConfig,
    ) -> Result<Self, GraphQLError> {
        let generated_schema = build_schema(&schema_cache, &config);
        let subscription_fields = if config.enable_subscriptions {
            generate_subscription_fields(&schema_cache, &generated_schema)
        } else {
            Vec::new()
        };
        let schema = build_dynamic_schema(
            &generated_schema,
            &schema_cache,
            if config.enable_subscriptions {
                Some(subscription_fields.as_slice())
            } else {
                None
            },
        )?;

        Ok(Self {
            pool: pool.clone(),
            schema_cache,
            generated_schema,
            schema,
            config,
            subscription_fields,
            broker: Arc::new(RwLock::new(None)),
        })
    }

    /// Rebuild the schema (e.g., after schema cache refresh).
    pub fn rebuild(&mut self) -> Result<(), GraphQLError> {
        self.generated_schema = build_schema(&self.schema_cache, &self.config);
        self.subscription_fields = if self.config.enable_subscriptions {
            generate_subscription_fields(&self.schema_cache, &self.generated_schema)
        } else {
            Vec::new()
        };
        self.schema = build_dynamic_schema(
            &self.generated_schema,
            &self.schema_cache,
            if self.config.enable_subscriptions {
                Some(self.subscription_fields.as_slice())
            } else {
                None
            },
        )?;
        Ok(())
    }

    /// Initialize the subscription broker.
    ///
    /// This should be called after creating the state to enable subscriptions.
    pub async fn init_subscriptions(&self) -> Result<(), crate::subscription::BrokerError> {
        if !self.config.enable_subscriptions {
            return Ok(());
        }

        let broker = NotifyBroker::new(self.pool.clone());

        // Collect all channels to listen on
        let channels: Vec<String> = self
            .subscription_fields
            .iter()
            .map(|f| f.channel_name())
            .collect();

        if !channels.is_empty() {
            broker.start(channels).await?;
            info!(
                "Subscription broker started with {} channels",
                self.subscription_fields.len()
            );
        }

        // Store the broker
        let mut broker_guard = self.broker.write().await;
        *broker_guard = Some(broker);

        Ok(())
    }

    /// Stop the subscription broker.
    pub async fn stop_subscriptions(&self) {
        let broker_guard = self.broker.read().await;
        if let Some(broker) = broker_guard.as_ref() {
            broker.stop().await;
        }
    }

    /// Get the notification broker.
    pub async fn get_broker(&self) -> Option<Arc<RwLock<Option<NotifyBroker>>>> {
        Some(Arc::clone(&self.broker))
    }
}

/// Handle a GraphQL request.
pub async fn graphql_handler(
    State(state): State<Arc<GraphQLState>>,
    ctx: GraphQLContext,
    req: GraphQLRequest,
) -> GraphQLResponse {
    let request = req
        .into_inner()
        .data(ctx)
        .data(state.pool.clone())
        .data(Arc::clone(&state.broker));
    state.schema.execute(request).await.into()
}

/// Handle GraphQL WebSocket subscription upgrade.
///
/// This should be called with a WebSocket upgrade request to enable
/// GraphQL subscriptions over WebSocket.
pub async fn graphql_ws_handler(
    State(state): State<Arc<GraphQLState>>,
    protocol: async_graphql_axum::GraphQLProtocol,
    ws: axum::extract::WebSocketUpgrade,
) -> impl IntoResponse {
    let schema = state.schema.clone();
    let pool = state.pool.clone();
    let broker = Arc::clone(&state.broker);

    ws.protocols(["graphql-transport-ws", "graphql-ws"])
        .on_upgrade(move |socket| async move {
            let mut data = async_graphql::Data::default();
            data.insert(pool);
            data.insert(broker);

            async_graphql_axum::GraphQLWebSocket::new(socket, schema, protocol)
                .with_data(data)
                .serve()
                .await
        })
}

/// Handle GraphQL playground request.
pub async fn graphql_playground() -> impl axum::response::IntoResponse {
    axum::response::Html(async_graphql::http::playground_source(
        async_graphql::http::GraphQLPlaygroundConfig::new("/graphql")
            .subscription_endpoint("/graphql/ws"),
    ))
}

/// Build the dynamic async-graphql schema from our generated schema.
fn build_dynamic_schema(
    generated: &GeneratedSchema,
    _schema_cache: &SchemaCache,
    subscription_fields: Option<&[SubField]>,
) -> Result<Schema, GraphQLError> {
    // Create object types for each table
    let mut object_types: HashMap<String, Object> = HashMap::new();

    for (type_name, obj) in &generated.object_types {
        let table_obj = create_object_type(obj);
        object_types.insert(type_name.clone(), table_obj);
    }

    // Create query type
    let query = create_query_type(generated);

    // Create mutation type
    let mutation = if !generated.mutation_fields.is_empty() {
        Some(create_mutation_type(generated))
    } else {
        None
    };

    // Create subscription type if enabled
    let subscription = subscription_fields.map(create_subscription_type);

    // Build schema
    let mut builder = Schema::build(
        "Query",
        mutation.as_ref().map(|_| "Mutation"),
        subscription.as_ref().map(|_| "Subscription"),
    );

    // Register all object types
    for (_, obj) in object_types {
        builder = builder.register(obj);
    }

    // Register query type
    builder = builder.register(query);

    // Register mutation type if present
    if let Some(mutation) = mutation {
        builder = builder.register(mutation);
    }

    // Register subscription type if present
    if let Some(subscription) = subscription {
        builder = builder.register(subscription);
    }

    // Register scalar types
    builder = builder.register(create_bigint_scalar());
    builder = builder.register(create_bigdecimal_scalar());
    builder = builder.register(create_json_scalar());
    builder = builder.register(create_uuid_scalar());
    builder = builder.register(create_date_scalar());
    builder = builder.register(create_datetime_scalar());
    builder = builder.register(create_time_scalar());

    // Register input types
    builder = register_filter_input_types(builder);

    builder
        .finish()
        .map_err(|e| GraphQLError::SchemaError(e.to_string()))
}

/// Create an object type from a TableObjectType.
fn create_object_type(obj: &TableObjectType) -> Object {
    let mut object = Object::new(&obj.name);

    if let Some(desc) = obj.description() {
        object = object.description(desc);
    }

    for field in &obj.fields {
        let field_name = field.name.clone();
        let field_type = graphql_type_ref(&field.type_string());

        // Create field with resolver that extracts from parent async_graphql::Value
        // The query resolver stores rows as FieldValue::value(Value::Object)
        // so we use as_value() to get the Value and extract fields from the Object
        let gql_field = Field::new(&field.name, field_type, move |ctx| {
            let field_name = field_name.clone();
            FieldFuture::new(async move {
                // Get the parent value as async_graphql::Value using as_value()
                if let Some(Value::Object(map)) = ctx.parent_value.as_value() {
                    // Convert field name to async_graphql::Name for lookup
                    let key = async_graphql::Name::new(&field_name);
                    if let Some(val) = map.get(&key) {
                        return Ok(Some(FieldValue::value(val.clone())));
                    }
                }

                // Field not found or parent not a Value::Object
                Ok(None)
            })
        });

        let gql_field = if let Some(desc) = &field.description {
            gql_field.description(desc)
        } else {
            gql_field
        };

        object = object.field(gql_field);
    }

    object
}

/// Create the Query type with all table query fields.
fn create_query_type(generated: &GeneratedSchema) -> Object {
    let mut query = Object::new("Query");

    for field in &generated.query_fields {
        let table_name = field.table_name.clone();
        let type_name = field.type_name.clone();
        let is_by_pk = field.is_by_pk;
        let is_count = field.is_count;
        let by_pk_id_type = field.by_pk_id_type.clone();
        let by_pk_column = field.by_pk_column.clone();
        let return_type = graphql_type_ref(&field.return_type);

        let mut gql_field = if is_count {
            let table_name_c = table_name.clone();
            Field::new(&field.name, return_type, move |ctx| {
                let table_name = table_name_c.clone();
                FieldFuture::new(async move {
                    resolve_count(&ctx, &table_name).await
                })
            })
        } else {
            let table_name_q = table_name.clone();
            let type_name_q = type_name.clone();
            let by_pk_id_type_q = by_pk_id_type.clone();
            let by_pk_column_q = by_pk_column.clone();
            Field::new(&field.name, return_type, move |ctx| {
                let table_name = table_name_q.clone();
                let type_name = type_name_q.clone();
                let by_pk_id_type = by_pk_id_type_q.clone();
                let by_pk_column = by_pk_column_q.clone();
                FieldFuture::new(async move {
                    resolve_query(
                        &ctx,
                        &table_name,
                        &type_name,
                        is_by_pk,
                        by_pk_id_type,
                        by_pk_column,
                    )
                    .await
                })
            })
        };

        // Add arguments
        if is_count {
            gql_field = gql_field
                .argument(InputValue::new("filter", TypeRef::named("JSON")));
        } else if !is_by_pk {
            gql_field = gql_field
                .argument(InputValue::new("filter", TypeRef::named("JSON")))
                .argument(InputValue::new("orderBy", TypeRef::named_list("String")))
                .argument(InputValue::new("limit", TypeRef::named("Int")))
                .argument(InputValue::new("offset", TypeRef::named("Int")));
        } else {
            // Single PK column, matching react-admin + ra-data-graphql getOne (variable `id`)
            let id_scalar = field
                .by_pk_id_type
                .as_deref()
                .unwrap_or("Int");
            gql_field = gql_field.argument(InputValue::new("id", TypeRef::named_nn(id_scalar)));
        }

        if let Some(desc) = &field.description {
            gql_field = gql_field.description(desc);
        }

        query = query.field(gql_field);
    }

    // Add introspection queries
    query = query.field(
        Field::new("_schema", TypeRef::named("String"), |_| {
            FieldFuture::new(async move {
                Ok(Some(Value::String("Postrust GraphQL Schema".to_string())))
            })
        })
        .description("Schema introspection"),
    );

    query
}

/// Create the Mutation type with all mutation fields.
fn create_mutation_type(generated: &GeneratedSchema) -> Object {
    let mut mutation = Object::new("Mutation");

    for field in &generated.mutation_fields {
        let table_name = field.table_name.clone();
        let mutation_type = field.mutation_type;
        let return_type = graphql_type_ref(&field.return_type);

        let mut gql_field = Field::new(&field.name, return_type, move |ctx| {
            let table_name = table_name.clone();
            FieldFuture::new(async move {
                resolve_mutation(&ctx, &table_name, mutation_type).await
            })
        });

        // Add mutation-specific arguments
        match mutation_type {
            MutationType::Insert | MutationType::InsertOne => {
                gql_field = gql_field
                    .argument(InputValue::new("objects", TypeRef::named_nn_list("JSON")));
            }
            MutationType::Update | MutationType::UpdateByPk => {
                gql_field = gql_field
                    .argument(InputValue::new("where", TypeRef::named("JSON")))
                    .argument(InputValue::new("set", TypeRef::named_nn("JSON")));
            }
            MutationType::Delete | MutationType::DeleteByPk => {
                gql_field = gql_field.argument(InputValue::new("where", TypeRef::named("JSON")));
            }
        }

        if let Some(desc) = &field.description {
            gql_field = gql_field.description(desc);
        }

        mutation = mutation.field(gql_field);
    }

    mutation
}

/// Create the Subscription type with all subscription fields.
fn create_subscription_type(fields: &[SubField]) -> Subscription {
    let mut subscription = Subscription::new("Subscription");

    for field in fields {
        let channel_name = field.channel_name();
        let return_type = TypeRef::named(&field.return_type);
        let field_name = field.name.clone();
        let description = field.description.clone();

        let gql_field = SubscriptionField::new(&field_name, return_type, move |ctx| {
            let channel_name = channel_name.clone();
            SubscriptionFieldFuture::new(async move {
                let broker_arc = ctx.data::<Arc<RwLock<Option<NotifyBroker>>>>()?;
                let broker_guard = broker_arc.read().await;

                let broker = broker_guard
                    .as_ref()
                    .ok_or_else(|| async_graphql::Error::new("Subscription broker not initialized"))?;

                let stream = broker
                    .subscribe(&channel_name)
                    .await
                    .map_err(|e| async_graphql::Error::new(format!("Subscription error: {}", e)))?;

                // Transform notification stream to GraphQL values
                // Use FieldValue::value() so field resolvers can use as_value()
                let value_stream = stream.filter_map(|notification| async move {
                    match TableChangePayload::from_payload(&notification.payload) {
                        Ok(payload) => {
                            if let Some(data) = subscription_event_value(&payload) {
                                // Convert to async_graphql::Value so field resolvers can extract fields
                                Some(Ok(FieldValue::value(json_to_value(data))))
                            } else {
                                None
                            }
                        }
                        Err(e) => {
                            debug!("Failed to parse notification payload: {}", e);
                            None
                        }
                    }
                });

                Ok(value_stream)
            })
        });

        let gql_field = if let Some(desc) = description {
            gql_field.description(desc)
        } else {
            gql_field
        };

        subscription = subscription.field(gql_field);
    }

    subscription
}

/// Single-column primary key value, matching the generated by-PK GraphQL `id` argument.
enum ByPkParam {
    I64(i64),
    Uuid(Uuid),
    String(String),
}

/// `SELECT * … WHERE <pk> = $1` with a typed parameter (Int / UUID / String scalars in the schema).
async fn execute_by_pk_one(
    pool: &PgPool,
    table_name: &str,
    pk_col: &str,
    value: ByPkParam,
    role: &str,
) -> Result<Vec<serde_json::Value>, async_graphql::Error> {
    use sqlx::Row;

    let t = postrust_sql::escape_ident(table_name);
    let c = postrust_sql::escape_ident(pk_col);
    let sql = format!(
        "SELECT row_to_json(s) FROM (SELECT * FROM public.{t} WHERE {c} = $1) s",
        t = t,
        c = c,
    );

    trace!("Executing by-PK SQL: {}, role={}", sql, role);

    let mut conn = pool
        .acquire()
        .await
        .map_err(|e| async_graphql::Error::new(e.to_string()))?;

    sqlx::query(&format!("SET LOCAL ROLE {}", postrust_sql::escape_ident(role)))
        .execute(&mut *conn)
        .await
        .map_err(|e| async_graphql::Error::new(e.to_string()))?;

    let rows = match value {
        ByPkParam::I64(n) => sqlx::query(&sql).bind(n).fetch_all(&mut *conn).await,
        ByPkParam::Uuid(u) => sqlx::query(&sql).bind(u).fetch_all(&mut *conn).await,
        ByPkParam::String(ref s) => {
            sqlx::query(&sql)
                .bind(s)
                .fetch_all(&mut *conn)
                .await
        }
    }
    .map_err(|e| async_graphql::Error::new(e.to_string()))?;

    let results: Vec<serde_json::Value> = rows
        .iter()
        .filter_map(|row| row.try_get::<serde_json::Value, _>(0).ok())
        .collect();

    Ok(results)
}

/// Resolve a query field.
async fn resolve_query<'a>(
    ctx: &ResolverContext<'a>,
    table_name: &str,
    _type_name: &str,
    is_by_pk: bool,
    by_pk_id_type: Option<String>,
    by_pk_column: Option<String>,
) -> Result<Option<FieldValue<'a>>, async_graphql::Error> {
    let pool = ctx.data::<PgPool>()?;
    let gql_ctx = ctx.data::<GraphQLContext>()?;

    debug!("Resolving query for table: {}", table_name);

    if is_by_pk {
        let id_type = by_pk_id_type.as_deref().unwrap_or("Int");
        let pk_col = by_pk_column.as_deref().unwrap_or("id");
        let v = ctx
            .args
            .try_get("id")
            .map_err(|_| async_graphql::Error::new("by-pk query requires an `id` argument"))?;
        let j = accessor_to_json(&v);
        let param = match id_type {
            "Int" => {
                let n = j
                    .as_i64()
                    .or_else(|| j.as_u64().and_then(|u| i64::try_from(u).ok()))
                    .ok_or_else(|| {
                        async_graphql::Error::new("by-pk `id` must be an integer")
                    })?;
                ByPkParam::I64(n)
            }
            "UUID" => {
                let s = j
                    .as_str()
                    .ok_or_else(|| {
                        async_graphql::Error::new("by-pk `id` must be a UUID string")
                    })?;
                let u = Uuid::parse_str(s).map_err(|e| {
                    async_graphql::Error::new(format!("by-pk `id` is not a valid UUID: {e}"))
                })?;
                ByPkParam::Uuid(u)
            }
            _ => {
                let s: String = if let Some(s) = j.as_str() {
                    s.to_string()
                } else if let Some(n) = j.as_i64() {
                    n.to_string()
                } else if let Some(n) = j.as_u64() {
                    n.to_string()
                } else {
                    return Err(async_graphql::Error::new(
                        "by-pk `id` value could not be interpreted for this scalar",
                    ));
                };
                ByPkParam::String(s)
            }
        };
        let result = execute_by_pk_one(pool, table_name, pk_col, param, gql_ctx.role()).await?;
        return Ok(result
            .into_iter()
            .next()
            .map(|v| FieldValue::value(json_to_value(v))));
    }

    let limit: Option<i64> = ctx
        .args
        .try_get("limit")
        .ok()
        .and_then(|v| v.i64().ok());

    let offset: Option<i64> = ctx
        .args
        .try_get("offset")
        .ok()
        .and_then(|v| v.i64().ok());

    let filter_value = ctx
        .args
        .try_get("filter")
        .ok()
        .map(|v| accessor_to_json(&v));

    let order_by: Option<Vec<String>> = ctx
        .args
        .try_get("orderBy")
        .ok()
        .and_then(|v| v.list().ok())
        .map(|list| {
            list.iter()
                .filter_map(|v| v.string().ok().map(|s| s.to_string()))
                .collect()
        });

    let (sql, where_values) = build_list_sql(table_name, filter_value.as_ref(), order_by.as_deref(), limit, offset)?;

    let mut conn = pool.acquire().await?;
    sqlx::query(&format!("SET LOCAL ROLE {}", postrust_sql::escape_ident(gql_ctx.role())))
        .execute(&mut *conn)
        .await?;

    let mut query = sqlx::query(&sql);
    for val in &where_values {
        query = bind_json_value(query, val);
    }

    let result: Vec<serde_json::Value> = {
        use sqlx::Row;
        let rows = query.fetch_all(&mut *conn).await?;
        rows.iter()
            .filter_map(|row| row.try_get::<serde_json::Value, _>(0).ok())
            .collect()
    };
    let items: Vec<FieldValue> = result
        .into_iter()
        .map(|v| FieldValue::value(json_to_value(v)))
        .collect();
    Ok(Some(FieldValue::list(items)))
}
/// Build the SQL for a list query with optional filter, ordering, limit, and offset.
fn build_list_sql(
    table_name: &str,
    filter_value: Option<&serde_json::Value>,
    order_by: Option<&[String]>,
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<(String, Vec<serde_json::Value>), async_graphql::Error> {
    let t = postrust_sql::escape_ident(table_name);
    let (where_sql, where_values) = build_where_clause(filter_value, 1)?;

    let order_sql = match order_by {
        Some(fields) if !fields.is_empty() => {
            let clauses: Vec<String> = fields
                .iter()
                .filter_map(|s| {
                    // Parse "field_ASC" or "field_DESC"
                    if let Some(field) = s.strip_suffix("_ASC") {
                        Some(format!("{} ASC", postrust_sql::escape_ident(field)))
                    } else if let Some(field) = s.strip_suffix("_DESC") {
                        Some(format!("{} DESC", postrust_sql::escape_ident(field)))
                    } else {
                        None
                    }
                })
                .collect();
            if clauses.is_empty() {
                String::new()
            } else {
                format!(" ORDER BY {}", clauses.join(", "))
            }
        }
        _ => String::new(),
    };

    let mut sql = format!(
        "SELECT row_to_json(t) FROM (SELECT * FROM public.{t} {where_sql}{order_sql}) t",
    );

    if let Some(limit) = limit {
        sql.push_str(&format!(" LIMIT {limit}"));
    }
    if let Some(offset) = offset {
        sql.push_str(&format!(" OFFSET {offset}"));
    }

    Ok((sql, where_values))
}

/// Resolve a count query field (e.g., usersCount).
async fn resolve_count<'a>(
    ctx: &ResolverContext<'a>,
    table_name: &str,
) -> Result<Option<FieldValue<'a>>, async_graphql::Error> {
    use sqlx::Row;

    let pool = ctx.data::<PgPool>()?;
    let gql_ctx = ctx.data::<GraphQLContext>()?;

    debug!("Resolving count for table: {}", table_name);

    let filter_value = ctx
        .args
        .try_get("filter")
        .ok()
        .map(|v| accessor_to_json(&v));

    let (where_sql, where_values) = build_where_clause(filter_value.as_ref(), 1)?;

    let sql = format!(
        "SELECT COUNT(*) AS cnt FROM public.{} {}",
        postrust_sql::escape_ident(table_name),
        where_sql,
    );

    trace!("Executing COUNT SQL: {}", sql);

    let mut conn = pool.acquire().await?;

    sqlx::query(&format!("SET LOCAL ROLE {}", postrust_sql::escape_ident(gql_ctx.role())))
        .execute(&mut *conn)
        .await?;

    let mut query = sqlx::query(&sql);
    for val in &where_values {
        query = bind_json_value(query, val);
    }

    let row = query.fetch_one(&mut *conn).await?;
    let count: i64 = row.try_get("cnt")?;

    Ok(Some(FieldValue::value(Value::Number(count.into()))))
}

/// Resolve a mutation field.
async fn resolve_mutation<'a>(
    ctx: &ResolverContext<'a>,
    table_name: &str,
    mutation_type: MutationType,
) -> Result<Option<FieldValue<'a>>, async_graphql::Error> {
    let pool = ctx.data::<PgPool>()?;
    let gql_ctx = ctx.data::<GraphQLContext>()?;

    debug!("Resolving mutation for table: {} type: {:?}", table_name, mutation_type);

    let result = match mutation_type {
        MutationType::Insert | MutationType::InsertOne => {
            let objects = ctx
                .args
                .try_get("objects")
                .ok()
                .map(|v| accessor_to_json(&v))
                .unwrap_or_else(|| serde_json::Value::Array(vec![]));

            execute_insert(pool, table_name, gql_ctx.role(), objects, mutation_type).await?
        }
        MutationType::Update | MutationType::UpdateByPk => {
            let set_value = ctx
                .args
                .try_get("set")
                .ok()
                .map(|v| accessor_to_json(&v))
                .unwrap_or_else(|| serde_json::json!({}));

            let where_clause = ctx
                .args
                .try_get("where")
                .ok()
                .map(|v| accessor_to_json(&v));

            execute_update(pool, table_name, gql_ctx.role(), set_value, where_clause, mutation_type).await?
        }
        MutationType::Delete | MutationType::DeleteByPk => {
            let where_clause = ctx
                .args
                .try_get("where")
                .ok()
                .map(|v| accessor_to_json(&v));

            execute_delete(pool, table_name, gql_ctx.role(), where_clause, mutation_type).await?
        }
    };

    Ok(result)
}


/// Execute an insert mutation.
async fn execute_insert<'a>(
    pool: &PgPool,
    table_name: &str,
    role: &str,
    objects: serde_json::Value,
    mutation_type: MutationType,
) -> Result<Option<FieldValue<'a>>, async_graphql::Error> {
    use sqlx::Row;

    trace!("Insert mutation for {}: {:?}", table_name, objects);

    // Handle both array and single object
    let objects_array = match objects {
        serde_json::Value::Array(arr) => arr,
        serde_json::Value::Object(obj) => vec![serde_json::Value::Object(obj)],
        _ => return Err(async_graphql::Error::new("objects must be an array or object")),
    };

    if objects_array.is_empty() {
        return Err(async_graphql::Error::new("objects cannot be empty"));
    }

    let mut conn = pool.acquire().await?;

    // Set role
    sqlx::query(&format!("SET LOCAL ROLE {}", postrust_sql::escape_ident(role)))
        .execute(&mut *conn)
        .await?;

    let mut inserted: Vec<FieldValue> = Vec::new();

    for obj in objects_array {
        if let serde_json::Value::Object(map) = obj {
            // Build INSERT query
            let columns: Vec<&str> = map.keys().map(|k| k.as_str()).collect();
            let placeholders: Vec<String> = (1..=columns.len()).map(|i| format!("${}", i)).collect();

            let sql = format!(
                "INSERT INTO public.{} ({}) VALUES ({}) RETURNING row_to_json(public.{}.*)",
                postrust_sql::escape_ident(table_name),
                columns.iter().map(|c| postrust_sql::escape_ident(c)).collect::<Vec<_>>().join(", "),
                placeholders.join(", "),
                postrust_sql::escape_ident(table_name)
            );

            trace!("Executing INSERT SQL: {}", sql);

            // Build query with parameters
            let mut query = sqlx::query(&sql);
            for col in &columns {
                if let Some(val) = map.get(*col) {
                    query = bind_json_value(query, val);
                }
            }

            let row = query.fetch_one(&mut *conn).await?;
            if let Ok(json_val) = row.try_get::<serde_json::Value, _>(0) {
                inserted.push(FieldValue::value(json_to_value(json_val)));
            }
        }
    }

    // Return based on mutation type
    match mutation_type {
        MutationType::InsertOne => {
            // Return single item
            Ok(inserted.into_iter().next())
        }
        _ => {
            // Return list
            Ok(Some(FieldValue::list(inserted)))
        }
    }
}

/// Bind a JSON value to a sqlx query.
fn bind_json_value<'q>(
    query: sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments>,
    value: &serde_json::Value,
) -> sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments> {
    match value {
        serde_json::Value::Null => query.bind(None::<String>),
        serde_json::Value::Bool(b) => query.bind(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                query.bind(i)
            } else if let Some(f) = n.as_f64() {
                query.bind(f)
            } else {
                query.bind(n.to_string())
            }
        }
        serde_json::Value::String(s) => {
            // Try to bind as UUID if the string looks like one, so Postgres
            // doesn't reject `uuid = text` comparisons.
            if let Ok(u) = uuid::Uuid::parse_str(s) {
                query.bind(u)
            } else {
                query.bind(s.clone())
            }
        }
        _ => query.bind(value.to_string()),
    }
}

/// Execute an update mutation.
async fn execute_update<'a>(
    pool: &PgPool,
    table_name: &str,
    role: &str,
    set_value: serde_json::Value,
    where_clause: Option<serde_json::Value>,
    mutation_type: MutationType,
) -> Result<Option<FieldValue<'a>>, async_graphql::Error> {
    use sqlx::Row;

    trace!("Update mutation for {}: {:?}", table_name, set_value);

    let set_map = match set_value {
        serde_json::Value::Object(map) => map,
        _ => return Err(async_graphql::Error::new("set must be an object")),
    };

    if set_map.is_empty() {
        return Err(async_graphql::Error::new("set cannot be empty"));
    }

    let mut conn = pool.acquire().await?;

    // Set role
    sqlx::query(&format!("SET LOCAL ROLE {}", postrust_sql::escape_ident(role)))
        .execute(&mut *conn)
        .await?;

    // Build SET clause
    let mut set_parts: Vec<String> = Vec::new();
    let mut param_idx = 1;
    for key in set_map.keys() {
        set_parts.push(format!("{} = ${}", postrust_sql::escape_ident(key), param_idx));
        param_idx += 1;
    }

    // Build WHERE clause
    let (where_sql, where_values) = build_where_clause(where_clause.as_ref(), param_idx)?;

    let sql = format!(
        "UPDATE public.{} SET {} {} RETURNING row_to_json(public.{}.*)",
        postrust_sql::escape_ident(table_name),
        set_parts.join(", "),
        where_sql,
        postrust_sql::escape_ident(table_name)
    );

    trace!("Executing UPDATE SQL: {}", sql);

    // Build query with parameters
    let mut query = sqlx::query(&sql);

    // Bind SET values
    for val in set_map.values() {
        query = bind_json_value(query, val);
    }

    // Bind WHERE values
    for val in &where_values {
        query = bind_json_value(query, val);
    }

    let rows = query.fetch_all(&mut *conn).await?;

    let updated: Vec<FieldValue> = rows
        .iter()
        .filter_map(|row| row.try_get::<serde_json::Value, _>(0).ok())
        .map(|v| FieldValue::value(json_to_value(v)))
        .collect();

    // Return based on mutation type
    match mutation_type {
        MutationType::UpdateByPk => {
            Ok(updated.into_iter().next())
        }
        _ => {
            Ok(Some(FieldValue::list(updated)))
        }
    }
}

/// Execute a delete mutation.
async fn execute_delete<'a>(
    pool: &PgPool,
    table_name: &str,
    role: &str,
    where_clause: Option<serde_json::Value>,
    mutation_type: MutationType,
) -> Result<Option<FieldValue<'a>>, async_graphql::Error> {
    use sqlx::Row;

    trace!("Delete mutation for {}", table_name);

    let mut conn = pool.acquire().await?;

    // Set role
    sqlx::query(&format!("SET LOCAL ROLE {}", postrust_sql::escape_ident(role)))
        .execute(&mut *conn)
        .await?;

    // Build WHERE clause
    let (where_sql, where_values) = build_where_clause(where_clause.as_ref(), 1)?;

    let sql = format!(
        "DELETE FROM public.{} {} RETURNING row_to_json(public.{}.*)",
        postrust_sql::escape_ident(table_name),
        where_sql,
        postrust_sql::escape_ident(table_name)
    );

    trace!("Executing DELETE SQL: {}", sql);

    // Build query with parameters
    let mut query = sqlx::query(&sql);

    // Bind WHERE values
    for val in &where_values {
        query = bind_json_value(query, val);
    }

    let rows = query.fetch_all(&mut *conn).await?;

    let deleted: Vec<FieldValue> = rows
        .iter()
        .filter_map(|row| row.try_get::<serde_json::Value, _>(0).ok())
        .map(|v| FieldValue::value(json_to_value(v)))
        .collect();

    // Return based on mutation type
    match mutation_type {
        MutationType::DeleteByPk => {
            Ok(deleted.into_iter().next())
        }
        _ => {
            Ok(Some(FieldValue::list(deleted)))
        }
    }
}

/// Build a WHERE clause from a JSON filter object.
fn build_where_clause(
    where_value: Option<&serde_json::Value>,
    start_param_idx: usize,
) -> Result<(String, Vec<serde_json::Value>), async_graphql::Error> {
    let mut conditions: Vec<String> = Vec::new();
    let mut values: Vec<serde_json::Value> = Vec::new();
    let mut param_idx = start_param_idx;

    if let Some(serde_json::Value::Object(map)) = where_value {
        for (key, val) in map {
            match val {
                serde_json::Value::Object(op_map) => {
                    // Handle operators like {eq: value}, {gt: value}, etc.
                    for (op, op_val) in op_map {
                        match op.as_str() {
                            "in" | "_in" => {
                                if let serde_json::Value::Array(arr) = op_val {
                                    if arr.is_empty() {
                                        conditions.push("FALSE".to_string());
                                    } else {
                                        let col = postrust_sql::escape_ident(key);
                                        let parts: Vec<String> = arr.iter().map(|v| {
                                            let placeholder = format!("${}", param_idx);
                                            values.push(v.clone());
                                            param_idx += 1;
                                            format!("{} = {}", col, placeholder)
                                        }).collect();
                                        if parts.len() == 1 {
                                            conditions.push(parts.into_iter().next().unwrap());
                                        } else {
                                            conditions.push(format!("({})", parts.join(" OR ")));
                                        }
                                    }
                                }
                            }
                            "is_null" | "_is_null" => {
                                if op_val.as_bool().unwrap_or(false) {
                                    conditions.push(format!("{} IS NULL", postrust_sql::escape_ident(key)));
                                } else {
                                    conditions.push(format!("{} IS NOT NULL", postrust_sql::escape_ident(key)));
                                }
                            }
                            _ => {
                                let condition = match op.as_str() {
                                    "eq" | "_eq" => format!("{} = ${}", postrust_sql::escape_ident(key), param_idx),
                                    "neq" | "_neq" => format!("{} != ${}", postrust_sql::escape_ident(key), param_idx),
                                    "gt" | "_gt" => format!("{} > ${}", postrust_sql::escape_ident(key), param_idx),
                                    "gte" | "_gte" => format!("{} >= ${}", postrust_sql::escape_ident(key), param_idx),
                                    "lt" | "_lt" => format!("{} < ${}", postrust_sql::escape_ident(key), param_idx),
                                    "lte" | "_lte" => format!("{} <= ${}", postrust_sql::escape_ident(key), param_idx),
                                    "like" | "_like" => format!("{} LIKE ${}", postrust_sql::escape_ident(key), param_idx),
                                    "ilike" | "_ilike" => format!("{} ILIKE ${}", postrust_sql::escape_ident(key), param_idx),
                                    _ => continue,
                                };
                                conditions.push(condition);
                                values.push(op_val.clone());
                                param_idx += 1;
                            }
                        }
                    }
                }
                _ => {
                    // Direct equality: {field: value}
                    conditions.push(format!("{} = ${}", postrust_sql::escape_ident(key), param_idx));
                    values.push(val.clone());
                    param_idx += 1;
                }
            }
        }
    }

    let where_sql = if conditions.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", conditions.join(" AND "))
    };

    Ok((where_sql, values))
}

/// Convert a GraphQL type string to a TypeRef.
fn graphql_type_ref(type_str: &str) -> TypeRef {
    // Parse type string like "[Users!]!" or "String" or "Int!"
    let is_list = type_str.starts_with('[');
    let is_nn = type_str.ends_with('!');

    // Strip outer modifiers: first the trailing !, then the brackets
    let inner = if is_list {
        let stripped = type_str
            .trim_end_matches('!')  // Remove outer !
            .trim_start_matches('[')  // Remove [
            .trim_end_matches(']');   // Remove ]
        stripped
    } else {
        type_str.trim_end_matches('!')
    };

    let inner_nn = inner.ends_with('!');
    let base_type = inner.trim_end_matches('!');

    if is_list {
        if is_nn {
            if inner_nn {
                TypeRef::named_nn_list_nn(base_type)
            } else {
                TypeRef::named_list_nn(base_type)
            }
        } else if inner_nn {
            TypeRef::named_nn_list(base_type)
        } else {
            TypeRef::named_list(base_type)
        }
    } else if is_nn {
        TypeRef::named_nn(base_type)
    } else {
        TypeRef::named(base_type)
    }
}

/// Convert ValueAccessor to JSON.
fn accessor_to_json(accessor: &ValueAccessor<'_>) -> serde_json::Value {
    // Use the deserialize method if available, or convert manually
    if accessor.is_null() {
        serde_json::Value::Null
    } else if let Ok(b) = accessor.boolean() {
        serde_json::Value::Bool(b)
    } else if let Ok(i) = accessor.i64() {
        serde_json::Value::Number(i.into())
    } else if let Ok(f) = accessor.f64() {
        serde_json::Number::from_f64(f)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null)
    } else if let Ok(s) = accessor.string() {
        serde_json::Value::String(s.to_string())
    } else if let Ok(list) = accessor.list() {
        serde_json::Value::Array(
            list.iter()
                .map(|v| accessor_to_json(&v))
                .collect()
        )
    } else if let Ok(obj) = accessor.object() {
        let map: serde_json::Map<String, serde_json::Value> = obj
            .iter()
            .map(|(k, v)| (k.to_string(), accessor_to_json(&v)))
            .collect();
        serde_json::Value::Object(map)
    } else {
        serde_json::Value::Null
    }
}

/// Convert async-graphql Value to JSON.
#[allow(dead_code)]
fn value_to_json(value: &Value) -> serde_json::Value {
    match value {
        Value::Null => serde_json::Value::Null,
        Value::Boolean(b) => serde_json::Value::Bool(*b),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                serde_json::Value::Number(i.into())
            } else if let Some(f) = n.as_f64() {
                serde_json::Value::Number(serde_json::Number::from_f64(f).unwrap())
            } else {
                serde_json::Value::Null
            }
        }
        Value::String(s) => serde_json::Value::String(s.clone()),
        Value::List(arr) => {
            serde_json::Value::Array(arr.iter().map(value_to_json).collect())
        }
        Value::Object(obj) => {
            let map: serde_json::Map<String, serde_json::Value> = obj
                .iter()
                .map(|(k, v)| (k.to_string(), value_to_json(v)))
                .collect();
            serde_json::Value::Object(map)
        }
        Value::Binary(b) => serde_json::Value::String(base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            b,
        )),
        Value::Enum(e) => serde_json::Value::String(e.to_string()),
    }
}

/// Convert JSON to async-graphql Value.
fn json_to_value(json: serde_json::Value) -> Value {
    match json {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(b) => Value::Boolean(b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Number(i.into())
            } else if let Some(f) = n.as_f64() {
                Value::Number(async_graphql::Number::from_f64(f).unwrap())
            } else {
                Value::Null
            }
        }
        serde_json::Value::String(s) => Value::String(s),
        serde_json::Value::Array(arr) => {
            Value::List(arr.into_iter().map(json_to_value).collect())
        }
        serde_json::Value::Object(obj) => {
            let map: indexmap::IndexMap<async_graphql::Name, Value> = obj
                .into_iter()
                .map(|(k, v)| (async_graphql::Name::new(k), json_to_value(v)))
                .collect();
            Value::Object(map)
        }
    }
}

fn subscription_event_value(payload: &TableChangePayload) -> Option<serde_json::Value> {
    match payload.data() {
        Some(value) => Some(value.clone()),
        None => None,
    }
}

/// Create BigInt scalar type.
fn create_bigint_scalar() -> Scalar {
    Scalar::new("BigInt")
        .description("64-bit integer")
        .specified_by_url("https://spec.graphql.org/draft/#sec-Int")
}

/// Create BigDecimal scalar type.
fn create_bigdecimal_scalar() -> Scalar {
    Scalar::new("BigDecimal")
        .description("Arbitrary precision decimal number")
}

/// Create JSON scalar type.
fn create_json_scalar() -> Scalar {
    Scalar::new("JSON")
        .description("Arbitrary JSON value")
        .specified_by_url("https://spec.graphql.org/draft/#sec-Scalars")
}

/// Create UUID scalar type.
fn create_uuid_scalar() -> Scalar {
    Scalar::new("UUID").description("UUID string")
}

/// Create Date scalar type.
fn create_date_scalar() -> Scalar {
    Scalar::new("Date").description("ISO 8601 date string (YYYY-MM-DD)")
}

/// Create DateTime scalar type.
fn create_datetime_scalar() -> Scalar {
    Scalar::new("DateTime").description("ISO 8601 datetime string")
}

/// Create Time scalar type.
fn create_time_scalar() -> Scalar {
    Scalar::new("Time").description("ISO 8601 time string (HH:MM:SS)")
}

/// Register filter input types.
fn register_filter_input_types(builder: SchemaBuilder) -> SchemaBuilder {
    let string_filter = InputObject::new("StringFilterInput")
        .field(InputValue::new("eq", TypeRef::named("String")))
        .field(InputValue::new("neq", TypeRef::named("String")))
        .field(InputValue::new("like", TypeRef::named("String")))
        .field(InputValue::new("ilike", TypeRef::named("String")))
        .field(InputValue::new("in", TypeRef::named_list("String")))
        .field(InputValue::new("isNull", TypeRef::named("Boolean")));

    let int_filter = InputObject::new("IntFilterInput")
        .field(InputValue::new("eq", TypeRef::named("Int")))
        .field(InputValue::new("neq", TypeRef::named("Int")))
        .field(InputValue::new("gt", TypeRef::named("Int")))
        .field(InputValue::new("gte", TypeRef::named("Int")))
        .field(InputValue::new("lt", TypeRef::named("Int")))
        .field(InputValue::new("lte", TypeRef::named("Int")))
        .field(InputValue::new("in", TypeRef::named_list("Int")));

    let boolean_filter = InputObject::new("BooleanFilterInput")
        .field(InputValue::new("eq", TypeRef::named("Boolean")));

    builder
        .register(string_filter)
        .register(int_filter)
        .register(boolean_filter)
}

#[cfg(test)]
mod tests {
    use super::*;
    use indexmap::IndexMap;
    use postrust_core::schema_cache::{Column, Table};
    use std::collections::{HashMap, HashSet};

    fn create_test_table(name: &str) -> Table {
        let mut columns = IndexMap::new();
        columns.insert(
            "id".into(),
            Column {
                name: "id".into(),
                description: None,
                nullable: false,
                data_type: "integer".into(),
                nominal_type: "int4".into(),
                max_len: None,
                default: Some("nextval('id_seq')".into()),
                enum_values: vec![],
                is_pk: true,
                position: 1,
            },
        );
        columns.insert(
            "name".into(),
            Column {
                name: "name".into(),
                description: None,
                nullable: false,
                data_type: "text".into(),
                nominal_type: "text".into(),
                max_len: None,
                default: None,
                enum_values: vec![],
                is_pk: false,
                position: 2,
            },
        );

        Table {
            schema: "public".into(),
            name: name.into(),
            description: None,
            is_view: false,
            insertable: true,
            updatable: true,
            deletable: true,
            pk_cols: vec!["id".into()],
            columns,
        }
    }

    fn create_test_schema_cache() -> SchemaCache {
        let mut tables = HashMap::new();
        let users = create_test_table("users");
        tables.insert(users.qualified_identifier(), users);

        SchemaCache {
            tables,
            relationships: HashMap::new(),
            routines: HashMap::new(),
            timezones: HashSet::new(),
            pg_version: 150000,
        }
    }

    // ============================================================================
    // Type Reference Tests
    // ============================================================================

    #[test]
    fn test_graphql_type_ref_simple() {
        let _type_ref = graphql_type_ref("String");
        // TypeRef doesn't implement PartialEq, so we just test it doesn't panic
    }

    #[test]
    fn test_graphql_type_ref_non_null() {
        let _type_ref = graphql_type_ref("String!");
    }

    #[test]
    fn test_graphql_type_ref_list() {
        let _type_ref = graphql_type_ref("[String]");
    }

    #[test]
    fn test_graphql_type_ref_list_non_null() {
        let _type_ref = graphql_type_ref("[String!]!");
    }

    // ============================================================================
    // Value Conversion Tests
    // ============================================================================

    #[test]
    fn test_value_to_json_null() {
        let value = Value::Null;
        let json = value_to_json(&value);
        assert_eq!(json, serde_json::Value::Null);
    }

    #[test]
    fn test_value_to_json_boolean() {
        let value = Value::Boolean(true);
        let json = value_to_json(&value);
        assert_eq!(json, serde_json::Value::Bool(true));
    }

    #[test]
    fn test_value_to_json_number() {
        let value = Value::Number(42.into());
        let json = value_to_json(&value);
        assert_eq!(json, serde_json::json!(42));
    }

    #[test]
    fn test_value_to_json_string() {
        let value = Value::String("hello".to_string());
        let json = value_to_json(&value);
        assert_eq!(json, serde_json::Value::String("hello".to_string()));
    }

    #[test]
    fn test_value_to_json_list() {
        let value = Value::List(vec![Value::Number(1.into()), Value::Number(2.into())]);
        let json = value_to_json(&value);
        assert_eq!(json, serde_json::json!([1, 2]));
    }

    #[test]
    fn test_json_to_value_null() {
        let json = serde_json::Value::Null;
        let value = json_to_value(json);
        assert!(matches!(value, Value::Null));
    }

    #[test]
    fn test_json_to_value_boolean() {
        let json = serde_json::Value::Bool(false);
        let value = json_to_value(json);
        assert!(matches!(value, Value::Boolean(false)));
    }

    #[test]
    fn test_json_to_value_number() {
        let json = serde_json::json!(123);
        let value = json_to_value(json);
        assert!(matches!(value, Value::Number(_)));
    }

    #[test]
    fn test_json_to_value_string() {
        let json = serde_json::Value::String("test".to_string());
        let value = json_to_value(json);
        assert!(matches!(value, Value::String(_)));
    }

    #[test]
    fn test_json_to_value_array() {
        let json = serde_json::json!([1, 2, 3]);
        let value = json_to_value(json);
        assert!(matches!(value, Value::List(_)));
    }

    #[test]
    fn test_json_to_value_object() {
        let json = serde_json::json!({"key": "value"});
        let value = json_to_value(json);
        assert!(matches!(value, Value::Object(_)));
    }

    #[test]
    fn test_subscription_event_value_returns_data() {
        let payload = TableChangePayload {
            operation: "DELETE".to_string(),
            table: "users".to_string(),
            schema: "public".to_string(),
            old: Some(serde_json::json!({
                "id": 6,
                "name": "Alice2"
            })),
            new: None,
        };

        let value = subscription_event_value(&payload).unwrap();
        assert_eq!(value["id"], 6);
        assert_eq!(value["name"], "Alice2");
    }

    // ============================================================================
    // Schema Building Tests
    // ============================================================================

    #[test]
    fn test_build_dynamic_schema() {
        let cache = create_test_schema_cache();
        let config = SchemaConfig::default();
        let generated = build_schema(&cache, &config);

        let result = build_dynamic_schema(&generated, &cache, None);
        if let Err(ref e) = result {
            eprintln!("Schema build error: {:?}", e);
        }
        assert!(result.is_ok(), "Schema build failed: {:?}", result.err());
    }

    #[test]
    fn test_create_object_type() {
        let table = create_test_table("users");
        let obj = TableObjectType::from_table(&table);
        let _gql_obj = create_object_type(&obj);
    }

    #[test]
    fn test_create_query_type() {
        let cache = create_test_schema_cache();
        let config = SchemaConfig::default();
        let generated = build_schema(&cache, &config);

        let _query = create_query_type(&generated);
    }

    #[test]
    fn test_create_mutation_type() {
        let cache = create_test_schema_cache();
        let config = SchemaConfig::default();
        let generated = build_schema(&cache, &config);

        let _mutation = create_mutation_type(&generated);
    }

    // ============================================================================
    // Scalar Tests
    // ============================================================================

    #[test]
    fn test_create_scalars() {
        let _bigint = create_bigint_scalar();
        let _json = create_json_scalar();
        let _uuid = create_uuid_scalar();
        let _datetime = create_datetime_scalar();
    }

    // ============================================================================
    // Filter Input Type Tests
    // ============================================================================

    #[test]
    fn test_register_filter_input_types() {
        let cache = create_test_schema_cache();
        let config = SchemaConfig::default();
        let _generated = build_schema(&cache, &config);

        // Build a minimal schema with filter types
        let query = Object::new("Query").field(Field::new(
            "test",
            TypeRef::named("String"),
            |_| FieldFuture::new(async { Ok(None::<FieldValue>) }),
        ));

        let mut builder = Schema::build("Query", None::<&str>, None);
        builder = builder.register(query);
        builder = register_filter_input_types(builder);

        let result = builder.finish();
        assert!(result.is_ok());
    }

    // ============================================================================
    // Subscription Tests
    // ============================================================================

    #[test]
    fn test_build_schema_with_subscriptions() {
        let cache = create_test_schema_cache();
        let config = SchemaConfig {
            enable_subscriptions: true,
            ..SchemaConfig::default()
        };
        let generated = build_schema(&cache, &config);

        // Generate subscription fields
        let sub_fields = generate_subscription_fields(&cache, &generated);
        assert!(!sub_fields.is_empty(), "Should have subscription fields");

        // Build schema with subscriptions
        let result = build_dynamic_schema(&generated, &cache, Some(&sub_fields));
        assert!(result.is_ok(), "Schema with subscriptions should build");
    }

    #[test]
    fn test_subscription_field_generation() {
        let cache = create_test_schema_cache();
        let config = SchemaConfig::default();
        let generated = build_schema(&cache, &config);

        let fields = generate_subscription_fields(&cache, &generated);

        // Should have one subscription field for the users table
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].name, "users");
        assert_eq!(fields[0].table_name, "users");
        assert_eq!(fields[0].channel_name(), "postrust_public_users");
    }

    #[test]
    fn test_create_subscription_type() {
        use crate::subscription::SubscriptionField as SubField;

        let fields = vec![
            SubField::for_table("public", "users", "Users"),
            SubField::for_table("public", "orders", "Orders"),
        ];

        let _subscription = create_subscription_type(&fields);
        // Just test that it doesn't panic
    }

    // ============================================================================
    // build_list_sql Tests
    // ============================================================================

    #[test]
    fn test_build_list_sql_no_args() {
        let (sql, values) = build_list_sql("users", None, None, None, None).unwrap();
        assert_eq!(
            sql,
            r#"SELECT row_to_json(t) FROM (SELECT * FROM public."users" ) t"#
        );
        assert!(values.is_empty());
    }

    #[test]
    fn test_build_list_sql_with_limit_and_offset() {
        let (sql, values) = build_list_sql("users", None, None, Some(10), Some(20)).unwrap();
        assert!(sql.contains("LIMIT 10"));
        assert!(sql.contains("OFFSET 20"));
        assert!(values.is_empty());
    }

    #[test]
    fn test_build_list_sql_with_order_by_asc() {
        let order = vec!["name_ASC".to_string()];
        let (sql, _) = build_list_sql("users", None, Some(&order), None, None).unwrap();
        assert!(
            sql.contains(r#"ORDER BY "name" ASC"#),
            "Expected ORDER BY clause in SQL: {}",
            sql
        );
    }

    #[test]
    fn test_build_list_sql_with_order_by_desc() {
        let order = vec!["createdAt_DESC".to_string()];
        let (sql, _) = build_list_sql("users", None, Some(&order), None, None).unwrap();
        assert!(
            sql.contains(r#"ORDER BY "createdAt" DESC"#),
            "Expected ORDER BY clause in SQL: {}",
            sql
        );
    }

    #[test]
    fn test_build_list_sql_with_multiple_order_by() {
        let order = vec!["name_ASC".to_string(), "id_DESC".to_string()];
        let (sql, _) = build_list_sql("users", None, Some(&order), None, None).unwrap();
        assert!(
            sql.contains(r#"ORDER BY "name" ASC, "id" DESC"#),
            "Expected multi-column ORDER BY in SQL: {}",
            sql
        );
    }

    #[test]
    fn test_build_list_sql_with_filter_eq() {
        let filter = serde_json::json!({ "status": { "eq": "active" } });
        let (sql, values) = build_list_sql("users", Some(&filter), None, None, None).unwrap();
        assert!(
            sql.contains("WHERE"),
            "Expected WHERE clause in SQL: {}",
            sql
        );
        assert!(
            sql.contains(r#""status" = $1"#),
            "Expected parameterized equality in SQL: {}",
            sql
        );
        assert_eq!(values.len(), 1);
        assert_eq!(values[0], serde_json::json!("active"));
    }

    #[test]
    fn test_build_list_sql_with_filter_in() {
        let filter = serde_json::json!({ "role": { "in": ["admin", "editor"] } });
        let (sql, values) = build_list_sql("users", Some(&filter), None, None, None).unwrap();
        assert!(
            sql.contains("WHERE"),
            "Expected WHERE clause in SQL: {}",
            sql
        );
        assert_eq!(values.len(), 2);
        assert_eq!(values[0], serde_json::json!("admin"));
        assert_eq!(values[1], serde_json::json!("editor"));
    }

    #[test]
    fn test_build_list_sql_with_filter_and_order_and_paging() {
        let filter = serde_json::json!({ "status": { "eq": "active" } });
        let order = vec!["name_ASC".to_string()];
        let (sql, values) = build_list_sql(
            "users",
            Some(&filter),
            Some(&order),
            Some(25),
            Some(50),
        )
        .unwrap();
        assert!(sql.contains("WHERE"), "Missing WHERE: {}", sql);
        assert!(sql.contains(r#"ORDER BY "name" ASC"#), "Missing ORDER BY: {}", sql);
        assert!(sql.contains("LIMIT 25"), "Missing LIMIT: {}", sql);
        assert!(sql.contains("OFFSET 50"), "Missing OFFSET: {}", sql);
        assert_eq!(values.len(), 1);
    }

    #[test]
    fn test_build_list_sql_escapes_table_name() {
        let (sql, _) = build_list_sql("user accounts", None, None, None, None).unwrap();
        assert!(
            sql.contains(r#"public."user accounts""#),
            "Table name not escaped: {}",
            sql
        );
    }

    // ============================================================================
    // build_where_clause Tests
    // ============================================================================

    #[test]
    fn test_build_where_clause_none() {
        let (sql, values) = build_where_clause(None, 1).unwrap();
        assert_eq!(sql, "");
        assert!(values.is_empty());
    }

    #[test]
    fn test_build_where_clause_eq() {
        let filter = serde_json::json!({ "name": { "eq": "Alice" } });
        let (sql, values) = build_where_clause(Some(&filter), 1).unwrap();
        assert_eq!(sql, r#"WHERE "name" = $1"#);
        assert_eq!(values, vec![serde_json::json!("Alice")]);
    }

    #[test]
    fn test_build_where_clause_neq() {
        let filter = serde_json::json!({ "status": { "neq": "inactive" } });
        let (sql, values) = build_where_clause(Some(&filter), 1).unwrap();
        assert_eq!(sql, r#"WHERE "status" != $1"#);
        assert_eq!(values, vec![serde_json::json!("inactive")]);
    }

    #[test]
    fn test_build_where_clause_gt_lt() {
        let filter = serde_json::json!({ "age": { "gt": 18, "lt": 65 } });
        let (sql, values) = build_where_clause(Some(&filter), 1).unwrap();
        assert!(sql.contains("WHERE"));
        assert_eq!(values.len(), 2);
    }

    #[test]
    fn test_build_where_clause_in_single() {
        let filter = serde_json::json!({ "id": { "in": [42] } });
        let (sql, values) = build_where_clause(Some(&filter), 1).unwrap();
        assert_eq!(sql, r#"WHERE "id" = $1"#);
        assert_eq!(values, vec![serde_json::json!(42)]);
    }

    #[test]
    fn test_build_where_clause_in_multiple() {
        let filter = serde_json::json!({ "id": { "in": [1, 2, 3] } });
        let (sql, values) = build_where_clause(Some(&filter), 1).unwrap();
        assert_eq!(sql, r#"WHERE ("id" = $1 OR "id" = $2 OR "id" = $3)"#);
        assert_eq!(values.len(), 3);
    }

    #[test]
    fn test_build_where_clause_in_empty() {
        let filter = serde_json::json!({ "id": { "in": [] } });
        let (sql, values) = build_where_clause(Some(&filter), 1).unwrap();
        assert_eq!(sql, "WHERE FALSE");
        assert!(values.is_empty());
    }

    #[test]
    fn test_build_where_clause_is_null() {
        let filter = serde_json::json!({ "email": { "is_null": true } });
        let (sql, values) = build_where_clause(Some(&filter), 1).unwrap();
        assert_eq!(sql, r#"WHERE "email" IS NULL"#);
        assert!(values.is_empty());
    }

    #[test]
    fn test_build_where_clause_direct_equality() {
        let filter = serde_json::json!({ "name": "Bob" });
        let (sql, values) = build_where_clause(Some(&filter), 1).unwrap();
        assert_eq!(sql, r#"WHERE "name" = $1"#);
        assert_eq!(values, vec![serde_json::json!("Bob")]);
    }

    #[test]
    fn test_build_where_clause_param_idx_offset() {
        let filter = serde_json::json!({ "name": { "eq": "Alice" } });
        let (sql, _) = build_where_clause(Some(&filter), 5).unwrap();
        assert_eq!(sql, r#"WHERE "name" = $5"#);
    }

    #[test]
    fn test_build_where_clause_like() {
        let filter = serde_json::json!({ "name": { "like": "%test%" } });
        let (sql, values) = build_where_clause(Some(&filter), 1).unwrap();
        assert_eq!(sql, r#"WHERE "name" LIKE $1"#);
        assert_eq!(values, vec![serde_json::json!("%test%")]);
    }

    #[test]
    fn test_build_where_clause_ilike() {
        let filter = serde_json::json!({ "name": { "ilike": "%TEST%" } });
        let (sql, values) = build_where_clause(Some(&filter), 1).unwrap();
        assert_eq!(sql, r#"WHERE "name" ILIKE $1"#);
        assert_eq!(values, vec![serde_json::json!("%TEST%")]);
    }
}
