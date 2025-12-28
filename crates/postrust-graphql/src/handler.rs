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
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info};

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
        let field_type = graphql_type_ref(&field.type_string());
        let mut gql_field = Field::new(&field.name, field_type, |_| {
            FieldFuture::new(async move { Ok(None::<FieldValue>) })
        });

        if let Some(desc) = &field.description {
            gql_field = gql_field.description(desc);
        }

        object = object.field(gql_field);
    }

    object
}

/// Create the Query type with all table query fields.
fn create_query_type(generated: &GeneratedSchema) -> Object {
    let mut query = Object::new("Query");

    for field in &generated.query_fields {
        let table_name = field.table_name.clone();
        let is_by_pk = field.is_by_pk;
        let return_type = graphql_type_ref(&field.return_type);

        let mut gql_field = Field::new(&field.name, return_type, move |ctx| {
            let table_name = table_name.clone();
            FieldFuture::new(async move {
                resolve_query(&ctx, &table_name, is_by_pk).await
            })
        });

        // Add standard query arguments
        if !is_by_pk {
            gql_field = gql_field
                .argument(InputValue::new("filter", TypeRef::named("JSON")))
                .argument(InputValue::new("orderBy", TypeRef::named_list("String")))
                .argument(InputValue::new("limit", TypeRef::named("Int")))
                .argument(InputValue::new("offset", TypeRef::named("Int")));
        } else {
            // Add PK arguments
            gql_field = gql_field.argument(InputValue::new("id", TypeRef::named_nn("Int")));
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
                let value_stream = stream.filter_map(|notification| async move {
                    match TableChangePayload::from_payload(&notification.payload) {
                        Ok(payload) => {
                            if let Some(data) = payload.data() {
                                Some(Ok(FieldValue::value(json_to_value(data.clone()))))
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

/// Resolve a query field.
async fn resolve_query(
    ctx: &ResolverContext<'_>,
    table_name: &str,
    is_by_pk: bool,
) -> Result<Option<Value>, async_graphql::Error> {
    let pool = ctx.data::<PgPool>()?;
    let gql_ctx = ctx.data::<GraphQLContext>()?;

    debug!("Resolving query for table: {}", table_name);

    // Extract pagination arguments
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

    // Build simple query
    let mut sql = format!(
        "SELECT row_to_json(t) FROM (SELECT * FROM public.{}) t",
        table_name
    );

    if let Some(limit) = limit {
        sql.push_str(&format!(" LIMIT {}", limit));
    }

    if let Some(offset) = offset {
        sql.push_str(&format!(" OFFSET {}", offset));
    }

    // Execute query
    let result = execute_query(pool, &sql, gql_ctx.role()).await?;

    if is_by_pk {
        Ok(result.first().cloned())
    } else {
        Ok(Some(Value::List(result)))
    }
}

/// Resolve a mutation field.
async fn resolve_mutation(
    ctx: &ResolverContext<'_>,
    table_name: &str,
    mutation_type: MutationType,
) -> Result<Option<Value>, async_graphql::Error> {
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

            execute_insert(pool, table_name, gql_ctx.role(), objects).await?
        }
        MutationType::Update | MutationType::UpdateByPk => {
            let set_value = ctx
                .args
                .try_get("set")
                .ok()
                .map(|v| accessor_to_json(&v))
                .unwrap_or_else(|| serde_json::json!({}));

            execute_update(pool, table_name, gql_ctx.role(), set_value).await?
        }
        MutationType::Delete | MutationType::DeleteByPk => {
            execute_delete(pool, table_name, gql_ctx.role()).await?
        }
    };

    Ok(Some(result))
}

/// Execute a SQL query and return results.
async fn execute_query(
    pool: &PgPool,
    sql: &str,
    role: &str,
) -> Result<Vec<Value>, async_graphql::Error> {
    use sqlx::Row;

    debug!("Executing SQL: {}", sql);

    let mut conn = pool.acquire().await?;

    // Set role
    sqlx::query(&format!("SET LOCAL ROLE {}", postrust_sql::escape_ident(role)))
        .execute(&mut *conn)
        .await?;

    // Execute query
    let rows = sqlx::query(sql).fetch_all(&mut *conn).await?;

    // Convert to GraphQL values
    let results: Vec<Value> = rows
        .iter()
        .filter_map(|row| {
            row.try_get::<serde_json::Value, _>(0)
                .ok()
                .map(json_to_value)
        })
        .collect();

    Ok(results)
}

/// Execute an insert mutation.
async fn execute_insert(
    _pool: &PgPool,
    table_name: &str,
    _role: &str,
    objects: serde_json::Value,
) -> Result<Value, async_graphql::Error> {
    // For now, return empty array - full implementation would execute INSERT
    debug!("Insert mutation for {}: {:?}", table_name, objects);
    Ok(Value::List(vec![]))
}

/// Execute an update mutation.
async fn execute_update(
    _pool: &PgPool,
    table_name: &str,
    _role: &str,
    set_value: serde_json::Value,
) -> Result<Value, async_graphql::Error> {
    // For now, return empty array - full implementation would execute UPDATE
    debug!("Update mutation for {}: {:?}", table_name, set_value);
    Ok(Value::List(vec![]))
}

/// Execute a delete mutation.
async fn execute_delete(
    _pool: &PgPool,
    table_name: &str,
    _role: &str,
) -> Result<Value, async_graphql::Error> {
    // For now, return empty array - full implementation would execute DELETE
    debug!("Delete mutation for {}", table_name);
    Ok(Value::List(vec![]))
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
}
