//! Axum handler for the /graphql endpoint.
//!
//! Provides GraphQL request handling using async-graphql with dynamic schema
//! generation from the PostgreSQL schema cache.

use crate::context::GraphQLContext;
use crate::error::GraphQLError;
use crate::schema::object::TableObjectType;
use crate::schema::relationship::RelationshipField;
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
            config.max_rows,
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
            self.config.max_rows,
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
    max_rows: Option<i64>,
) -> Result<Schema, GraphQLError> {
    // Create object types for each table
    let mut object_types: HashMap<String, Object> = HashMap::new();

    for (type_name, obj) in &generated.object_types {
        let relationships = generated
            .relationship_fields
            .get(type_name)
            .map(|r| r.as_slice())
            .unwrap_or(&[]);
        let table_obj = create_object_type(obj, relationships);
        object_types.insert(type_name.clone(), table_obj);
    }

    // Create query type. Resolvers need the relationship map to embed related
    // rows, so it is shared into each closure.
    let relationships = Arc::new(generated.relationship_fields.clone());
    let query = create_query_type(generated, max_rows, Arc::clone(&relationships));

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
fn create_object_type(obj: &TableObjectType, relationships: &[RelationshipField]) -> Object {
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

    // Relationship fields. The query resolver embeds related rows into the
    // parent JSON before returning it, so these read from the parent value the
    // same way column fields do.
    for rel in relationships {
        let field_name = rel.name.clone();
        let field_type = if rel.is_list {
            TypeRef::named_nn_list_nn(&rel.target_type)
        } else {
            TypeRef::named(&rel.target_type)
        };

        let gql_field = Field::new(&rel.name, field_type, move |ctx| {
            let field_name = field_name.clone();
            FieldFuture::new(async move {
                if let Some(Value::Object(map)) = ctx.parent_value.as_value() {
                    let key = async_graphql::Name::new(&field_name);
                    if let Some(val) = map.get(&key) {
                        return Ok(Some(FieldValue::value(val.clone())));
                    }
                }
                Ok(None)
            })
        });

        let gql_field = if let Some(desc) = &rel.description {
            gql_field.description(desc)
        } else {
            gql_field
        };

        object = object.field(gql_field);
    }

    object
}

/// Create the Query type with all table query fields.
fn create_query_type(
    generated: &GeneratedSchema,
    max_rows: Option<i64>,
    relationships: Arc<HashMap<String, Vec<RelationshipField>>>,
) -> Object {
    let mut query = Object::new("Query");

    for field in &generated.query_fields {
        let table_name = field.table_name.clone();
        let schema_name = field.schema_name.clone();
        let type_name = field.type_name.clone();
        let is_by_pk = field.is_by_pk;
        let pk_columns = field.pk_columns.clone();
        let return_type = graphql_type_ref(&field.return_type);

        let spec = Arc::new(QueryFieldSpec {
            schema_name,
            table_name,
            type_name,
            is_by_pk,
            pk_columns: pk_columns.clone(),
            max_rows,
            relationships: Arc::clone(&relationships),
        });

        let mut gql_field = Field::new(&field.name, return_type, move |ctx| {
            let spec = Arc::clone(&spec);
            FieldFuture::new(async move { resolve_query(&ctx, &spec).await })
        });

        // Add standard query arguments
        if !is_by_pk {
            gql_field = gql_field
                .argument(InputValue::new("filter", TypeRef::named("JSON")))
                .argument(InputValue::new("orderBy", TypeRef::named_list("String")))
                .argument(InputValue::new("limit", TypeRef::named("Int")))
                .argument(InputValue::new("offset", TypeRef::named("Int")));
        } else {
            // One required argument per primary key column, named and typed
            // after the column itself rather than assuming an integer `id`.
            for (col_name, pg_type) in &pk_columns {
                gql_field = gql_field.argument(InputValue::new(
                    col_name,
                    TypeRef::named_nn(pk_argument_type(pg_type)),
                ));
            }
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
        let schema_name = field.schema_name.clone();
        let mutation_type = field.mutation_type;
        let pk_columns = field.pk_columns.clone();
        let return_type = graphql_type_ref(&field.return_type);

        let resolver_pk_columns = pk_columns.clone();
        let mut gql_field = Field::new(&field.name, return_type, move |ctx| {
            let table_name = table_name.clone();
            let schema_name = schema_name.clone();
            let pk_columns = resolver_pk_columns.clone();
            FieldFuture::new(async move {
                resolve_mutation(&ctx, &schema_name, &table_name, mutation_type, &pk_columns).await
            })
        });

        // Add mutation-specific arguments.
        //
        // A by-PK mutation takes the key columns rather than a `where` object:
        // it is meant to address exactly one row, and accepting `where` made it
        // an ordinary bulk mutation that happened to return the first result.
        match mutation_type {
            MutationType::Insert | MutationType::InsertOne => {
                gql_field =
                    gql_field.argument(InputValue::new("objects", TypeRef::named_nn_list("JSON")));
            }
            MutationType::UpdateByPk => {
                gql_field = gql_field.argument(InputValue::new("set", TypeRef::named_nn("JSON")));
                for (col_name, pg_type) in &pk_columns {
                    gql_field = gql_field.argument(InputValue::new(
                        col_name,
                        TypeRef::named_nn(pk_argument_type(pg_type)),
                    ));
                }
            }
            MutationType::Update => {
                gql_field = gql_field
                    .argument(InputValue::new("where", TypeRef::named("JSON")))
                    .argument(InputValue::new("set", TypeRef::named_nn("JSON")));
            }
            MutationType::DeleteByPk => {
                for (col_name, pg_type) in &pk_columns {
                    gql_field = gql_field.argument(InputValue::new(
                        col_name,
                        TypeRef::named_nn(pk_argument_type(pg_type)),
                    ));
                }
            }
            MutationType::Delete => {
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

                let broker = broker_guard.as_ref().ok_or_else(|| {
                    async_graphql::Error::new("Subscription broker not initialized")
                })?;

                let stream = broker
                    .subscribe(&channel_name)
                    .await
                    .map_err(|e| async_graphql::Error::new(format!("Subscription error: {}", e)))?;

                // Transform notification stream to GraphQL values
                // Use FieldValue::value() so field resolvers can use as_value()
                let value_stream = stream.filter_map(|notification| async move {
                    match TableChangePayload::from_payload(&notification.payload) {
                        Ok(payload) => payload
                            .data()
                            .map(|data| Ok(FieldValue::value(json_to_value(data.clone())))),
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

/// Everything a query field's resolver needs about the field it serves.
struct QueryFieldSpec {
    schema_name: String,
    table_name: String,
    type_name: String,
    is_by_pk: bool,
    pk_columns: Vec<(String, String)>,
    max_rows: Option<i64>,
    relationships: Arc<HashMap<String, Vec<RelationshipField>>>,
}

/// Resolve a query field.
async fn resolve_query<'a>(
    ctx: &ResolverContext<'a>,
    spec: &QueryFieldSpec,
) -> Result<Option<FieldValue<'a>>, async_graphql::Error> {
    let schema_name = spec.schema_name.as_str();
    let table_name = spec.table_name.as_str();
    let type_name = spec.type_name.as_str();
    let is_by_pk = spec.is_by_pk;
    let pk_columns = spec.pk_columns.as_slice();
    let max_rows = spec.max_rows;
    let relationships = spec.relationships.as_ref();

    let pool = ctx.data::<PgPool>()?;
    let gql_ctx = ctx.data::<GraphQLContext>()?;

    debug!("Resolving query for table: {}", table_name);

    // Extract pagination arguments
    let requested_limit: Option<i64> = ctx.args.try_get("limit").ok().and_then(|v| v.i64().ok());

    let offset: Option<i64> = ctx.args.try_get("offset").ok().and_then(|v| v.i64().ok());

    // A query that names no limit would otherwise select the whole table, so
    // the configured ceiling is applied as the limit in that case, and as an
    // upper bound when the query asks for more than it. A by-PK query resolves
    // to at most one row.
    let limit: Option<i64> = if is_by_pk {
        Some(1)
    } else {
        match (requested_limit, max_rows) {
            (Some(requested), Some(ceiling)) => Some(requested.min(ceiling)),
            (Some(requested), None) => Some(requested),
            (None, ceiling) => ceiling,
        }
    };

    // Build the WHERE clause.
    //
    // A by-PK query filters on the table's key columns; each value is bound as
    // a parameter and cast to the column's type, since GraphQL scalars and
    // PostgreSQL types do not line up (a `uuid` key arrives as a String). A
    // list query filters on the `filter` argument, which takes the same shape
    // as a mutation's `where`.
    let mut where_sql = String::new();
    let mut bound_values: Vec<serde_json::Value> = Vec::new();

    if is_by_pk {
        if pk_columns.is_empty() {
            return Err(async_graphql::Error::new(format!(
                "\"{}\" has no primary key, so it cannot be queried by key",
                table_name
            )));
        }

        let mut conditions = Vec::with_capacity(pk_columns.len());
        for (idx, (col_name, pg_type)) in pk_columns.iter().enumerate() {
            let value = ctx.args.try_get(col_name).map_err(|_| {
                async_graphql::Error::new(format!(
                    "missing required primary key argument \"{}\"",
                    col_name
                ))
            })?;

            conditions.push(format!(
                "{} = ${}::{}",
                postrust_sql::escape_ident(col_name),
                idx + 1,
                pg_type
            ));
            bound_values.push(accessor_to_json(&value));
        }
        where_sql = format!(" WHERE {}", conditions.join(" AND "));
    } else if let Some(filter) = ctx
        .args
        .try_get("filter")
        .ok()
        .map(|v| accessor_to_json(&v))
    {
        let (filter_sql, filter_values) = build_where_clause(Some(&filter), 1)?;
        if !filter_sql.is_empty() {
            where_sql = format!(" {}", filter_sql);
            bound_values = filter_values;
        }
    }

    // Build the ORDER BY clause from the `orderBy` argument. Entries are
    // `column`, `column.asc` or `column.desc`; the column is validated against
    // the table so an unknown or crafted name cannot reach the SQL.
    let order_sql = if is_by_pk {
        String::new()
    } else {
        build_order_by_clause(ctx, &gql_ctx.schema_cache, schema_name, table_name).await?
    };

    // ORDER BY, LIMIT and OFFSET belong inside the subquery: applying them to
    // the outer `row_to_json` projection would leave the ordering of the rows
    // that survive the limit unspecified.
    let mut inner = format!(
        "SELECT * FROM {}.{}{}{}",
        postrust_sql::escape_ident(schema_name),
        postrust_sql::escape_ident(table_name),
        where_sql,
        order_sql
    );

    if let Some(limit) = limit {
        inner.push_str(&format!(" LIMIT {}", limit));
    }

    if let Some(offset) = offset {
        inner.push_str(&format!(" OFFSET {}", offset));
    }

    // Embed the requested relationships in this same query, as correlated
    // subselects in the SELECT list, so the whole selection is one round trip.
    let embed_expressions = {
        let guard = gql_ctx
            .schema_cache
            .get()
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;
        match guard.as_ref() {
            Some(cache) => build_embed_expressions(
                cache,
                relationships,
                type_name,
                "src",
                ctx.field(),
                max_rows,
                &mut 0,
            )?,
            None => Vec::new(),
        }
    };

    let inner = if embed_expressions.is_empty() {
        inner
    } else {
        let mut projection = String::from("src.*");
        for (field_name, expression) in &embed_expressions {
            projection.push_str(", ");
            projection.push_str(expression);
            projection.push_str(" AS ");
            projection.push_str(&postrust_sql::escape_ident(field_name));
        }
        format!("SELECT {} FROM ({}) AS src", projection, inner)
    };

    let sql = format!("SELECT row_to_json(t) FROM ({}) t", inner);

    // One transaction for the query and any embeds hanging off it, so the role
    // applies to all of them and the parent and child rows come from a single
    // snapshot.
    let mut tx = begin_with_role(pool, gql_ctx.role()).await?;

    // Execute query - returns Vec<serde_json::Value>
    let mut result = execute_query_on(&mut tx, &sql, &bound_values).await?;

    // Anything the single-query form did not cover.
    if embed_expressions.is_empty() {
        let guard = gql_ctx
            .schema_cache
            .get()
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;
        let cache = guard
            .as_ref()
            .ok_or_else(|| async_graphql::Error::new("schema cache is not loaded"))?;

        let embed_ctx = EmbedContext {
            schema_cache: cache,
            relationships,
            max_rows,
        };

        embed_relationships(&mut tx, &embed_ctx, type_name, ctx.field(), &mut result).await?;
    }

    tx.commit().await?;

    if is_by_pk {
        // Return single item as Value::Object
        // json_to_value converts serde_json to async_graphql Value
        Ok(result
            .into_iter()
            .next()
            .map(|v| FieldValue::value(json_to_value(v))))
    } else {
        // Return list with each item as Value::Object
        let items: Vec<FieldValue> = result
            .into_iter()
            .map(|v| FieldValue::value(json_to_value(v)))
            .collect();
        Ok(Some(FieldValue::list(items)))
    }
}

/// Resolve a mutation field.
async fn resolve_mutation<'a>(
    ctx: &ResolverContext<'a>,
    schema_name: &str,
    table_name: &str,
    mutation_type: MutationType,
    pk_columns: &[(String, String)],
) -> Result<Option<FieldValue<'a>>, async_graphql::Error> {
    let pool = ctx.data::<PgPool>()?;
    let gql_ctx = ctx.data::<GraphQLContext>()?;

    debug!(
        "Resolving mutation for table: {} type: {:?}",
        table_name, mutation_type
    );

    let result = match mutation_type {
        MutationType::Insert | MutationType::InsertOne => {
            let objects = ctx
                .args
                .try_get("objects")
                .ok()
                .map(|v| accessor_to_json(&v))
                .unwrap_or_else(|| serde_json::Value::Array(vec![]));

            execute_insert(
                pool,
                schema_name,
                table_name,
                gql_ctx.role(),
                objects,
                mutation_type,
            )
            .await?
        }
        MutationType::Update | MutationType::UpdateByPk => {
            let set_value = ctx
                .args
                .try_get("set")
                .ok()
                .map(|v| accessor_to_json(&v))
                .unwrap_or_else(|| serde_json::json!({}));

            let where_clause = if mutation_type == MutationType::UpdateByPk {
                Some(pk_where_from_args(ctx, table_name, pk_columns)?)
            } else {
                ctx.args.try_get("where").ok().map(|v| accessor_to_json(&v))
            };

            execute_update(
                pool,
                schema_name,
                table_name,
                gql_ctx.role(),
                set_value,
                where_clause,
                mutation_type,
            )
            .await?
        }
        MutationType::Delete | MutationType::DeleteByPk => {
            let where_clause = if mutation_type == MutationType::DeleteByPk {
                Some(pk_where_from_args(ctx, table_name, pk_columns)?)
            } else {
                ctx.args.try_get("where").ok().map(|v| accessor_to_json(&v))
            };

            execute_delete(
                pool,
                schema_name,
                table_name,
                gql_ctx.role(),
                where_clause,
                mutation_type,
            )
            .await?
        }
    };

    Ok(result)
}

/// Begin a transaction with the request's role applied.
///
/// The role has to be set inside a transaction. `SET LOCAL` sent on a bare
/// pooled connection applies to its own implicit single-statement transaction
/// and is discarded before the next statement runs, so the query would execute
/// as the pool's login role -- row-level security and role grants bypassed.
/// PostgreSQL logs "SET LOCAL can only be used in transaction blocks" every
/// time it happens.
async fn begin_with_role(
    pool: &PgPool,
    role: &str,
) -> Result<sqlx::Transaction<'static, sqlx::Postgres>, async_graphql::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query(&format!(
        "SET LOCAL ROLE {}",
        postrust_sql::escape_ident(role)
    ))
    .execute(&mut *tx)
    .await?;
    Ok(tx)
}

/// Execute a SQL query and return results as serde_json::Value.
/// We keep data as serde_json::Value so field resolvers can use try_downcast_ref.
async fn execute_query_on(
    conn: &mut sqlx::PgConnection,
    sql: &str,
    params: &[serde_json::Value],
) -> Result<Vec<serde_json::Value>, async_graphql::Error> {
    use sqlx::Row;

    trace!("Executing SQL: {}", sql);

    // Execute query
    let mut query = sqlx::query(sql);
    for param in params {
        query = bind_json_value(query, param);
    }
    let rows = query.fetch_all(&mut *conn).await?;

    // Return raw JSON values - don't convert to async_graphql::Value
    // This allows field resolvers to use try_downcast_ref::<serde_json::Value>()
    let results: Vec<serde_json::Value> = rows
        .into_iter()
        .filter_map(|row| row.try_get::<serde_json::Value, _>(0).ok())
        .collect();

    Ok(results)
}

/// Execute an insert mutation.
async fn execute_insert<'a>(
    pool: &PgPool,
    schema_name: &str,
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
        _ => {
            return Err(async_graphql::Error::new(
                "objects must be an array or object",
            ))
        }
    };

    if objects_array.is_empty() {
        return Err(async_graphql::Error::new("objects cannot be empty"));
    }

    let mut conn = begin_with_role(pool, role).await?;

    let mut inserted: Vec<FieldValue> = Vec::new();

    for obj in objects_array {
        if let serde_json::Value::Object(map) = obj {
            // Build INSERT query
            let columns: Vec<&str> = map.keys().map(|k| k.as_str()).collect();
            let placeholders: Vec<String> =
                (1..=columns.len()).map(|i| format!("${}", i)).collect();

            let sql = format!(
                "INSERT INTO {}.{} ({}) VALUES ({}) RETURNING row_to_json({}.{}.*)",
                postrust_sql::escape_ident(schema_name),
                postrust_sql::escape_ident(table_name),
                columns
                    .iter()
                    .map(|c| postrust_sql::escape_ident(c))
                    .collect::<Vec<_>>()
                    .join(", "),
                placeholders.join(", "),
                postrust_sql::escape_ident(schema_name),
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

    // Commit once every object has been inserted: committing inside the loop
    // would end the transaction, and the role set on it, after the first row.
    conn.commit().await?;

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
        serde_json::Value::String(s) => query.bind(s.clone()),
        _ => query.bind(value.to_string()),
    }
}

/// Execute an update mutation.
async fn execute_update<'a>(
    pool: &PgPool,
    schema_name: &str,
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

    let mut conn = begin_with_role(pool, role).await?;

    // Build SET clause
    let mut set_parts: Vec<String> = Vec::new();
    let mut param_idx = 1;
    for key in set_map.keys() {
        set_parts.push(format!(
            "{} = ${}",
            postrust_sql::escape_ident(key),
            param_idx
        ));
        param_idx += 1;
    }

    // Build WHERE clause
    let (where_sql, where_values) = build_where_clause(where_clause.as_ref(), param_idx)?;

    // An absent or unrecognised `where` argument yields an empty clause, which
    // would update every row in the table. Refuse instead.
    if where_sql.is_empty() {
        return Err(async_graphql::Error::new(format!(
            "update on \"{}\" requires a `where` argument with at least one \
             recognised condition; refusing to update every row",
            table_name
        )));
    }

    let sql = format!(
        "UPDATE {}.{} SET {} {} RETURNING row_to_json({}.{}.*)",
        postrust_sql::escape_ident(schema_name),
        postrust_sql::escape_ident(table_name),
        set_parts.join(", "),
        where_sql,
        postrust_sql::escape_ident(schema_name),
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
    conn.commit().await?;

    match mutation_type {
        MutationType::UpdateByPk => Ok(updated.into_iter().next()),
        _ => Ok(Some(FieldValue::list(updated))),
    }
}

/// Execute a delete mutation.
async fn execute_delete<'a>(
    pool: &PgPool,
    schema_name: &str,
    table_name: &str,
    role: &str,
    where_clause: Option<serde_json::Value>,
    mutation_type: MutationType,
) -> Result<Option<FieldValue<'a>>, async_graphql::Error> {
    use sqlx::Row;

    trace!("Delete mutation for {}", table_name);

    let mut conn = begin_with_role(pool, role).await?;

    // Build WHERE clause
    let (where_sql, where_values) = build_where_clause(where_clause.as_ref(), 1)?;

    // An absent or unrecognised `where` argument yields an empty clause, which
    // would delete every row in the table. Refuse instead.
    if where_sql.is_empty() {
        return Err(async_graphql::Error::new(format!(
            "delete on \"{}\" requires a `where` argument with at least one \
             recognised condition; refusing to delete every row",
            table_name
        )));
    }

    let sql = format!(
        "DELETE FROM {}.{} {} RETURNING row_to_json({}.{}.*)",
        postrust_sql::escape_ident(schema_name),
        postrust_sql::escape_ident(table_name),
        where_sql,
        postrust_sql::escape_ident(schema_name),
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
    conn.commit().await?;

    match mutation_type {
        MutationType::DeleteByPk => Ok(deleted.into_iter().next()),
        _ => Ok(Some(FieldValue::list(deleted))),
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
            let column = postrust_sql::escape_ident(key);

            match val {
                serde_json::Value::Object(op_map) => {
                    for (op, op_val) in op_map {
                        // Binary comparisons all bind exactly one parameter.
                        let binary_operator = match op.as_str() {
                            "eq" | "_eq" => Some("="),
                            "neq" | "_neq" => Some("!="),
                            "gt" | "_gt" => Some(">"),
                            "gte" | "_gte" => Some(">="),
                            "lt" | "_lt" => Some("<"),
                            "lte" | "_lte" => Some("<="),
                            "like" | "_like" => Some("LIKE"),
                            "ilike" | "_ilike" => Some("ILIKE"),
                            _ => None,
                        };

                        if let Some(sql_operator) = binary_operator {
                            conditions.push(format!("{} {} ${}", column, sql_operator, param_idx));
                            values.push(op_val.clone());
                            param_idx += 1;
                            continue;
                        }

                        match op.as_str() {
                            "is_null" | "_is_null" | "isNull" => {
                                if op_val.as_bool().unwrap_or(false) {
                                    conditions.push(format!("{} IS NULL", column));
                                } else {
                                    conditions.push(format!("{} IS NOT NULL", column));
                                }
                            }
                            "in" | "_in" => {
                                let items = op_val.as_array().ok_or_else(|| {
                                    async_graphql::Error::new(format!(
                                        "the `in` filter on \"{}\" requires a list of values",
                                        key
                                    ))
                                })?;

                                if items.is_empty() {
                                    // `IN ()` is not valid SQL, and an empty set
                                    // matches nothing.
                                    conditions.push("false".to_string());
                                    continue;
                                }

                                let mut placeholders = Vec::with_capacity(items.len());
                                for item in items {
                                    placeholders.push(format!("${}", param_idx));
                                    values.push(item.clone());
                                    param_idx += 1;
                                }
                                conditions.push(format!(
                                    "{} IN ({})",
                                    column,
                                    placeholders.join(", ")
                                ));
                            }
                            other => {
                                // Dropping an unrecognised operator would widen
                                // the result set -- returning every row for a
                                // query, or matching every row for a mutation.
                                // Fail loudly instead.
                                return Err(async_graphql::Error::new(format!(
                                    "unsupported filter operator \"{}\" on \"{}\"",
                                    other, key
                                )));
                            }
                        }
                    }
                }
                _ => {
                    // Direct equality: {field: value}
                    conditions.push(format!("{} = ${}", column, param_idx));
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

/// Build an `ORDER BY` clause from the `orderBy` argument.
///
/// Entries are `column`, `column.asc` or `column.desc`. Column names are
/// checked against the table in the schema cache and then quoted, so a name
/// that is unknown -- or crafted to inject SQL -- is rejected rather than
/// interpolated. Returns an empty string when no ordering was requested.
async fn build_order_by_clause(
    ctx: &ResolverContext<'_>,
    schema_cache: &postrust_core::schema_cache::SchemaCacheRef,
    schema_name: &str,
    table_name: &str,
) -> Result<String, async_graphql::Error> {
    let Ok(order_arg) = ctx.args.try_get("orderBy") else {
        return Ok(String::new());
    };

    let entries = match order_arg.list() {
        Ok(list) => list
            .iter()
            .map(|item| item.string().map(|s| s.to_string()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| async_graphql::Error::new("orderBy entries must be strings"))?,
        // A bare string is accepted as a single-column ordering.
        Err(_) => match order_arg.string() {
            Ok(single) => vec![single.to_string()],
            Err(_) => {
                return Err(async_graphql::Error::new(
                    "orderBy must be a string or a list of strings",
                ))
            }
        },
    };

    if entries.is_empty() {
        return Ok(String::new());
    }

    let guard = schema_cache
        .get()
        .await
        .map_err(|e| async_graphql::Error::new(e.to_string()))?;
    let cache = guard
        .as_ref()
        .ok_or_else(|| async_graphql::Error::new("schema cache is not loaded"))?;
    let qi = postrust_core::api_request::QualifiedIdentifier::new(schema_name, table_name);
    let table = cache
        .get_table(&qi)
        .ok_or_else(|| async_graphql::Error::new(format!("unknown table \"{}\"", table_name)))?;

    let mut terms = Vec::with_capacity(entries.len());
    for entry in entries {
        let (column, direction) = match entry.split_once('.') {
            Some((column, direction)) => (column, Some(direction)),
            None => (entry.as_str(), None),
        };

        if table.get_column(column).is_none() {
            return Err(async_graphql::Error::new(format!(
                "cannot order by unknown column \"{}\" on \"{}\"",
                column, table_name
            )));
        }

        let direction_sql = match direction.map(|d| d.to_ascii_lowercase()) {
            None => "",
            Some(d) if d == "asc" => " ASC",
            Some(d) if d == "desc" => " DESC",
            Some(other) => {
                return Err(async_graphql::Error::new(format!(
                    "invalid order direction \"{}\"; expected \"asc\" or \"desc\"",
                    other
                )))
            }
        };

        terms.push(format!(
            "{}{}",
            postrust_sql::escape_ident(column),
            direction_sql
        ));
    }

    Ok(format!(" ORDER BY {}", terms.join(", ")))
}

/// Build the SELECT-list expressions that embed relationships in one query.
///
/// The GraphQL mirror of the REST builder: each requested relationship becomes a
/// correlated subselect yielding JSON, so the whole selection comes back from
/// the parent query instead of one query per relationship per level.
fn build_embed_expressions(
    schema_cache: &SchemaCache,
    relationships: &HashMap<String, Vec<RelationshipField>>,
    type_name: &str,
    parent_alias: &str,
    selection: async_graphql::SelectionField<'_>,
    max_rows: Option<i64>,
    alias_counter: &mut usize,
) -> Result<Vec<(String, String)>, async_graphql::Error> {
    let Some(available) = relationships.get(type_name) else {
        return Ok(Vec::new());
    };

    let mut expressions = Vec::new();

    for field in selection.selection_set() {
        let Some(rel) = available.iter().find(|r| r.name == field.name()) else {
            continue;
        };

        let plan = postrust_core::embed::EmbedPlan::resolve(&rel.relationship, schema_cache)
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;

        *alias_counter += 1;
        let child_alias = format!("e{}", alias_counter);

        let nested = build_embed_expressions(
            schema_cache,
            relationships,
            &rel.target_type,
            &child_alias,
            field,
            max_rows,
            alias_counter,
        )?;

        // Leaf fields are columns; anything that resolved to a relationship is
        // an expression instead.
        let child_relationships = relationships.get(&rel.target_type);
        let mut parts: Vec<String> = Vec::new();
        for sub in field.selection_set() {
            let name = sub.name();
            let is_relationship = child_relationships
                .map(|rels| rels.iter().any(|r| r.name == name))
                .unwrap_or(false);
            if !is_relationship {
                parts.push(postrust_sql::escape_ident(name));
            }
        }
        if parts.is_empty() && nested.is_empty() {
            parts.push(format!("{}.*", postrust_sql::escape_ident(&child_alias)));
        }
        for (field_name, expression) in nested {
            parts.push(format!(
                "{} AS {}",
                expression,
                postrust_sql::escape_ident(&field_name)
            ));
        }

        let expression = plan
            .embed_expression(
                parent_alias,
                // GraphQL does not expose computed relationships, so the row
                // expression is never read; the alias is the honest value.
                &postrust_sql::escape_ident(parent_alias),
                &child_alias,
                &parts.join(", "),
                max_rows,
                None,
            )
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;

        expressions.push((rel.name.clone(), expression));
    }

    Ok(expressions)
}

/// What embedding a relationship needs, independent of the rows involved.
///
/// The connection is passed alongside rather than held here: embedding runs on
/// the request's transaction, and a shared reference to this struct could not
/// hand out the mutable borrow the queries need.
struct EmbedContext<'c> {
    schema_cache: &'c SchemaCache,
    relationships: &'c HashMap<String, Vec<RelationshipField>>,
    max_rows: Option<i64>,
}

/// Embed the relationship fields requested on `rows`.
///
/// One query per relationship per level, not one per row: the parents' join
/// keys are collected and passed as a single array. Recurses so a nested
/// selection costs one further query per relationship at each depth.
fn embed_relationships<'f>(
    conn: &'f mut sqlx::PgConnection,
    ctx: &'f EmbedContext<'f>,
    type_name: &'f str,
    selection: async_graphql::SelectionField<'f>,
    rows: &'f mut [serde_json::Value],
) -> futures::future::BoxFuture<'f, Result<(), async_graphql::Error>> {
    Box::pin(async move {
        if rows.is_empty() {
            return Ok(());
        }

        let Some(available) = ctx.relationships.get(type_name) else {
            return Ok(());
        };

        for requested in selection.selection_set() {
            let Some(rel) = available.iter().find(|r| r.name == requested.name()) else {
                continue;
            };

            let plan =
                postrust_core::embed::EmbedPlan::resolve(&rel.relationship, ctx.schema_cache)
                    .map_err(|e| async_graphql::Error::new(e.to_string()))?;

            let keys = postrust_core::embed::parent_keys(rows, &plan.local_column);

            // Project only what the selection asked for. A GraphQL selection
            // names its leaf fields, so the columns are known before the query
            // and an unrequested column need not be read, serialised, sent and
            // parsed just to be dropped.
            //
            // A nested relationship joins on a column of the child row, so that
            // column is added even when it was not selected; it is removed again
            // when the response is shaped. A selection whose sub-fields cannot
            // all be resolved to columns falls back to every column.
            let mut child_columns: Vec<String> = Vec::new();
            let mut project_everything = false;
            let child_relationships = ctx.relationships.get(&rel.target_type);
            for field in requested.selection_set() {
                let name = field.name();
                match child_relationships.and_then(|rels| rels.iter().find(|r| r.name == name)) {
                    Some(nested_rel) => {
                        match postrust_core::embed::EmbedPlan::resolve(
                            &nested_rel.relationship,
                            ctx.schema_cache,
                        ) {
                            Ok(nested_plan) => child_columns.push(nested_plan.local_column),
                            Err(_) => project_everything = true,
                        }
                    }
                    None => child_columns.push(name.to_string()),
                }
            }
            if project_everything || child_columns.is_empty() {
                child_columns.clear();
            }

            let mut grouped = if keys.is_empty() {
                std::collections::HashMap::new()
            } else {
                let sql = plan
                    .children_grouped_sql(ctx.max_rows, &child_columns)
                    .map_err(|e| async_graphql::Error::new(e.to_string()))?;

                let fetched = sqlx::query(&sql).bind(&keys).fetch_all(&mut *conn).await?;

                // The query returns the join key and a JSON array of that key's
                // children, grouped by PostgreSQL rather than row by row here.
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

            // Recurse before attaching, so nested embeds land in the values
            // copied onto the parents. One query serves every child row at this
            // level, so the rows are flattened for the call and put back
            // afterwards; skipped when the selection asks for nothing deeper.
            let has_deeper_embed = ctx
                .relationships
                .get(&rel.target_type)
                .map(|rels| {
                    requested
                        .selection_set()
                        .any(|field| rels.iter().any(|r| r.name == field.name()))
                })
                .unwrap_or(false);

            if has_deeper_embed {
                let mut order: Vec<(String, usize)> = Vec::with_capacity(grouped.len());
                let mut flat: Vec<serde_json::Value> = Vec::new();
                for (key, children) in grouped.drain() {
                    order.push((key, children.len()));
                    flat.extend(children);
                }

                embed_relationships(&mut *conn, ctx, &rel.target_type, requested, &mut flat)
                    .await?;

                let mut rest = flat.into_iter();
                for (key, count) in order {
                    grouped.insert(key, rest.by_ref().take(count).collect());
                }
            }

            for row in rows.iter_mut() {
                postrust_core::embed::attach_to_parent(row, &rel.name, &plan, &grouped);
            }
        }

        Ok(())
    })
}

/// Build a `where` document that addresses exactly one row by primary key.
///
/// Used by the by-PK mutations, which take the key columns as arguments instead
/// of a free-form `where`.
fn pk_where_from_args(
    ctx: &ResolverContext<'_>,
    table_name: &str,
    pk_columns: &[(String, String)],
) -> Result<serde_json::Value, async_graphql::Error> {
    if pk_columns.is_empty() {
        return Err(async_graphql::Error::new(format!(
            "\"{}\" has no primary key, so it cannot be mutated by key",
            table_name
        )));
    }

    let mut conditions = serde_json::Map::new();
    for (col_name, _) in pk_columns {
        let value = ctx.args.try_get(col_name).map_err(|_| {
            async_graphql::Error::new(format!(
                "missing required primary key argument \"{}\"",
                col_name
            ))
        })?;
        conditions.insert(
            col_name.clone(),
            serde_json::json!({ "eq": accessor_to_json(&value) }),
        );
    }

    Ok(serde_json::Value::Object(conditions))
}

/// GraphQL scalar name to use for a primary key argument of the given
/// PostgreSQL type.
///
/// Falls back to `String` for anything that does not map to a plain scalar --
/// a composite or array key cannot be expressed as a single named argument, and
/// the value is cast to the column's type in SQL anyway.
fn pk_argument_type(pg_type: &str) -> String {
    let rendered = crate::types::pg_type_to_graphql(pg_type).to_string();
    if rendered.starts_with('[') {
        "String".to_string()
    } else {
        rendered
    }
}

/// Convert a GraphQL type string to a TypeRef.
fn graphql_type_ref(type_str: &str) -> TypeRef {
    // Parse type string like "[Users!]!" or "String" or "Int!"
    let is_list = type_str.starts_with('[');
    let is_nn = type_str.ends_with('!');

    // Strip outer modifiers: first the trailing !, then the brackets
    let inner = if is_list {
        let stripped = type_str
            .trim_end_matches('!') // Remove outer !
            .trim_start_matches('[') // Remove [
            .trim_end_matches(']'); // Remove ]
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
        serde_json::Value::Array(list.iter().map(|v| accessor_to_json(&v)).collect())
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
        Value::List(arr) => serde_json::Value::Array(arr.iter().map(value_to_json).collect()),
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
        serde_json::Value::Array(arr) => Value::List(arr.into_iter().map(json_to_value).collect()),
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
    Scalar::new("BigDecimal").description("Arbitrary precision decimal number")
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
///
/// These are currently unreachable: the `filter` and `where` arguments are
/// declared as the `JSON` scalar, so no field references these input objects
/// and async-graphql prunes them from the published schema (introspecting
/// `IntFilterInput` returns null). They are kept as the shape to move to if
/// filters become typed per column; until then the operators a filter actually
/// supports are the ones `build_where_clause` implements, and it rejects
/// anything else rather than ignoring it.
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
            media_handlers: HashMap::new(),
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

        let result = build_dynamic_schema(&generated, &cache, None, None);
        if let Err(ref e) = result {
            eprintln!("Schema build error: {:?}", e);
        }
        assert!(result.is_ok(), "Schema build failed: {:?}", result.err());
    }

    #[test]
    fn test_create_object_type() {
        let table = create_test_table("users");
        let obj = TableObjectType::from_table(&table);
        let _gql_obj = create_object_type(&obj, &[]);
    }

    #[test]
    fn test_create_query_type() {
        let cache = create_test_schema_cache();
        let config = SchemaConfig::default();
        let generated = build_schema(&cache, &config);

        let _query = create_query_type(&generated, None, Arc::new(HashMap::new()));
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
        let query =
            Object::new("Query").field(Field::new("test", TypeRef::named("String"), |_| {
                FieldFuture::new(async { Ok(None::<FieldValue>) })
            }));

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
        let result = build_dynamic_schema(&generated, &cache, Some(&sub_fields), None);
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
