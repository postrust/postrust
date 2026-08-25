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
use std::collections::{HashMap, HashSet};
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
            Arc::new(config.names.clone()),
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
            Arc::new(self.config.names.clone()),
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

/// The scalar at the bottom of a type, past any list wrapping.
fn leaf_scalar_name(graphql_type: &crate::types::GraphQLType) -> String {
    match graphql_type {
        crate::types::GraphQLType::List(inner) => leaf_scalar_name(inner),
        other => other.to_string(),
    }
}

/// Build the dynamic async-graphql schema from our generated schema.
fn build_dynamic_schema(
    generated: &GeneratedSchema,
    _schema_cache: &SchemaCache,
    subscription_fields: Option<&[SubField]>,
    max_rows: Option<i64>,
    names: Arc<crate::names::NameOverrides>,
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

    // Aggregate types: every table gets them, because every table has a count
    // even when it has nothing to sum.
    for (type_name, obj) in &generated.object_types {
        for aggregate_type in create_aggregate_types(type_name, obj) {
            object_types.insert(aggregate_type.type_name().to_string(), aggregate_type);
        }
    }

    // One mutation response per table that has any mutation, and only those:
    // an unreferenced type is still a type, and a read-only view would
    // otherwise contribute a response nothing can return.
    let mutable: HashSet<&str> = generated
        .mutation_fields
        .iter()
        .map(|f| {
            f.return_type
                .trim_matches(|c| c == '[' || c == ']' || c == '!')
        })
        .collect();
    for base_name in mutable {
        object_types.insert(
            mutation_response_type_name(base_name),
            create_mutation_response_type(base_name),
        );
    }

    // Create query type. Resolvers need the relationship map to embed related
    // rows, so it is shared into each closure.
    let relationships = Arc::new(generated.relationship_fields.clone());
    // (schema, table) -> GraphQL type name. A nested insert is handed a table
    // by a relationship and has to find that table's relationships in turn,
    // and the map they live in is keyed by type name -- which is not always
    // the table's name, since a second schema prefixes it and a name may have
    // been given.
    let type_names: Arc<HashMap<(String, String), String>> = Arc::new(
        generated
            .object_types
            .iter()
            .map(|(type_name, object)| {
                (
                    (object.table.schema.clone(), object.table.name.clone()),
                    type_name.clone(),
                )
            })
            .collect(),
    );
    let query = create_query_type(
        generated,
        max_rows,
        Arc::clone(&relationships),
        Arc::clone(&names),
    );

    // Create mutation type
    let mutation = if !generated.mutation_fields.is_empty() {
        Some(create_mutation_type(
            generated,
            Arc::clone(&relationships),
            Arc::clone(&type_names),
            Arc::clone(&names),
            max_rows,
        ))
    } else {
        None
    };

    // Create subscription type if enabled
    let subscription = subscription_fields.map(create_subscription_type);

    // Build schema
    // `query_root`, not `Query`. The root type's name is not private to the
    // server: a fragment is declared `on query_root`, and a client that writes
    // one names the type it was generated against.
    let mut builder = Schema::build(
        "query_root",
        mutation.as_ref().map(|_| "mutation_root"),
        subscription.as_ref().map(|_| "subscription_root"),
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
    // Every scalar the generated types actually name, rather than a fixed
    // list. A `geometry` column, a `raster` column and a database enum each
    // become a scalar under their own PostgreSQL name, because that is the
    // name a client's query declares its variables with -- and a scalar the
    // schema mentions but never registers makes the whole schema unbuildable.
    let (bool_exp_inputs, bool_exp_scalars) = crate::input::bool_exp::build_inputs(
        &generated.object_types,
        &generated.relationship_fields,
    );
    for input in bool_exp_inputs {
        builder = builder.register(input);
    }

    let mut scalar_names: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for object in generated.object_types.values() {
        for field in &object.fields {
            scalar_names.insert(leaf_scalar_name(&field.graphql_type));
        }
    }
    // Used as an argument type in its own right, whether or not any column is
    // one: `objects`, `_set` and the mutation inputs are still JSON.
    scalar_names.insert("JSON".to_string());
    // Every scalar the boolean expressions name, which is more than the
    // scalars the columns are: a cast from a geometry names `geography`, and a
    // raster comparison names `geometry`. Taken from what was generated rather
    // than listed here, because a list here is a second place to keep in step
    // and it fell out of step twice.
    scalar_names.extend(bool_exp_scalars);

    for name in scalar_names {
        if matches!(name.as_str(), "Int" | "Float" | "String" | "Boolean" | "ID") {
            continue;
        }
        // An enum table's type is an enum, not a scalar. It reaches this list
        // because a column typed as one names it, and registering both would
        // define the same type twice.
        if generated.enum_types.contains_key(&name) {
            continue;
        }
        builder = builder.register(
            Scalar::new(&name).description(format!("The PostgreSQL `{}` type.", name)),
        );
    }

    // Register the boolean expression inputs -- one per table, plus one
    // comparison input per scalar the tables use.

    // A key as one object, for `update_x_by_pk(pk_columns: {...})`. Only the
    // by-key update spells its key this way; the by-key query and delete take
    // the columns as arguments, and both spellings are what a generated client
    // already sends.
    let mut key_inputs: HashMap<String, InputObject> = HashMap::new();
    for field in &generated.mutation_fields {
        if field.mutation_type != MutationType::UpdateByPk || field.pk_columns.is_empty() {
            continue;
        }
        let base_name = field
            .return_type
            .trim_matches(|c| c == '[' || c == ']' || c == '!')
            .trim_end_matches("_mutation_response");
        let type_name = format!("{}_pk_columns_input", base_name);
        let mut input = InputObject::new(&type_name)
            .description(format!("The primary key of one {} row.", base_name));
        for (column, pg_type) in &field.pk_columns {
            input = input.field(InputValue::new(
                column,
                TypeRef::named_nn(pk_argument_type(pg_type)),
            ));
        }
        key_inputs.insert(type_name, input);
    }
    for (_, input) in key_inputs {
        builder = builder.register(input);
    }

    // What a mutation writes. `objects` and `_set` were `JSON`, which accepts
    // anything and lets a client generate nothing: no completion, no codegen,
    // and a misspelled column reaching the database instead of being caught
    // before the request was sent. The same argument that made `where` a real
    // type applies unchanged.
    for (type_name, object) in &generated.object_types {
        let table = &object.table;
        // Only real columns are written. A computed field is a function of the
        // row and a relationship is handled separately below.
        let writable: Vec<&crate::schema::object::GraphQLField> = object
            .fields
            .iter()
            .filter(|field| table.get_column(&field.name).is_some())
            .collect();
        if writable.is_empty() {
            continue;
        }

        let relationships = generated
            .relationship_fields
            .get(type_name)
            .map(|r| r.as_slice())
            .unwrap_or(&[]);
        let conflict_type = match table.unique_constraints.is_empty() {
            true => None,
            false => Some(format!("{}_on_conflict", type_name)),
        };

        if crate::input::mutation::is_insertable(table) {
            let mut insert = InputObject::new(format!("{}_insert_input", type_name))
                .description(format!("The columns of a new {} row.", type_name));
            let mut taken: HashSet<&str> = HashSet::new();
            for field in &writable {
                // Every column optional: which ones the database insists on is
                // the database's answer, and a column that is NOT NULL with a
                // default does not have to be given.
                taken.insert(field.name.as_str());
                insert = insert.field(InputValue::new(
                    &field.name,
                    TypeRef::named(leaf_scalar_name(&field.graphql_type)),
                ));
            }
            // A nested write: the rows to insert beside this one.
            for relationship in relationships {
                if !taken.insert(relationship.name.as_str()) {
                    continue;
                }
                let suffix = match relationship.is_list {
                    true => "arr_rel_insert_input",
                    false => "obj_rel_insert_input",
                };
                insert = insert.field(InputValue::new(
                    &relationship.name,
                    TypeRef::named(format!("{}_{}", relationship.target_type, suffix)),
                ));
            }
            builder = builder.register(insert);

            // How this table is written as somebody else's nested row. Both
            // shapes carry an `on_conflict`, which is what makes a nested
            // upsert expressible.
            let data_type = format!("{}_insert_input", type_name);
            let mut object_rel = InputObject::new(format!("{}_obj_rel_insert_input", type_name))
                .description(format!("One {} row written beside its parent.", type_name))
                .field(InputValue::new("data", TypeRef::named_nn(&data_type)));
            let mut array_rel = InputObject::new(format!("{}_arr_rel_insert_input", type_name))
                .description(format!("{} rows written beside their parent.", type_name))
                .field(InputValue::new(
                    "data",
                    TypeRef::named_nn_list_nn(&data_type),
                ));
            if let Some(conflict) = &conflict_type {
                object_rel =
                    object_rel.field(InputValue::new("on_conflict", TypeRef::named(conflict)));
                array_rel =
                    array_rel.field(InputValue::new("on_conflict", TypeRef::named(conflict)));
            }
            builder = builder.register(object_rel).register(array_rel);
        }

        if crate::input::mutation::is_updatable(table) {
            let mut set = InputObject::new(format!("{}_set_input", type_name))
                .description(format!("Columns of {} to replace.", type_name));
            let mut numeric = InputObject::new(format!("{}_inc_input", type_name))
                .description(format!("Columns of {} to add to.", type_name));
            let mut any_numeric = false;
            for field in &writable {
                let scalar = leaf_scalar_name(&field.graphql_type);
                set = set.field(InputValue::new(&field.name, TypeRef::named(&scalar)));
                if crate::schema::aggregate::is_numeric(&field.graphql_type) {
                    any_numeric = true;
                    numeric = numeric.field(InputValue::new(&field.name, TypeRef::named(&scalar)));
                }
            }
            builder = builder.register(set);
            // A table with nothing to add to gets no type for adding to it.
            if any_numeric {
                builder = builder.register(numeric);
            }
        }
    }

    // Upserts. A table with no unique constraint has no conflict to resolve,
    // and a GraphQL enum may not be empty, so it gets none of these types
    // rather than an unusable set of them.
    for (type_name, object) in &generated.object_types {
        if object.table.unique_constraints.is_empty() || object.fields.is_empty() {
            continue;
        }

        let mut constraints = Enum::new(format!("{}_constraint", type_name)).description(format!(
            "A uniqueness of {} that an insert may conflict with.",
            type_name
        ));
        for (name, columns) in &object.table.unique_constraints {
            constraints = constraints.item(
                EnumItem::new(name).description(format!("unique ({})", columns.join(", "))),
            );
        }

        let mut updatable = Enum::new(format!("{}_update_column", type_name))
            .description(format!("A column of {} an upsert may write.", type_name));
        for field in &object.fields {
            updatable = updatable.item(EnumItem::new(&field.name));
        }

        let on_conflict = InputObject::new(format!("{}_on_conflict", type_name))
            .description(format!("What to do when an insert into {} conflicts.", type_name))
            .field(InputValue::new(
                "constraint",
                TypeRef::named_nn(format!("{}_constraint", type_name)),
            ))
            // An empty list is `DO NOTHING`, which is how Hasura spells "leave
            // the row that is already there alone".
            .field(InputValue::new(
                "update_columns",
                TypeRef::named_nn_list_nn(format!("{}_update_column", type_name)),
            ))
            .field(InputValue::new(
                "where",
                TypeRef::named(crate::input::bool_exp::bool_exp_type_name(type_name)),
            ));

        builder = builder
            .register(constraints)
            .register(updatable)
            .register(on_conflict);
    }

    // The enums generated from enum tables.
    for (type_name, members) in &generated.enum_types {
        let mut generated_enum = Enum::new(type_name)
            .description(format!("The values {} allows.", type_name));
        for (value, comment) in members {
            let item = EnumItem::new(value);
            generated_enum = generated_enum.item(match comment {
                Some(description) => item.description(description),
                None => item,
            });
        }
        builder = builder.register(generated_enum);
    }

    // Ordering: one input per table, one column enum per table, and the single
    // direction enum they all share.
    let (order_inputs, order_enums) =
        crate::input::order_by::build_inputs(&generated.object_types, &generated.relationship_fields);
    for input in order_inputs {
        builder = builder.register(input);
    }
    for enum_type in order_enums {
        builder = builder.register(enum_type);
    }

    builder
        .finish()
        .map_err(|e| GraphQLError::SchemaError(e.to_string()))
}

/// Create an object type from a TableObjectType.
/// The name of a table's mutation response type.
pub fn mutation_response_type_name(base_name: &str) -> String {
    format!("{}_mutation_response", base_name)
}

/// Build the `<table>_mutation_response` object.
///
/// Every bulk mutation answers with this rather than with the rows directly.
/// `affected_rows` is the count PostgreSQL reports, which is not the length of
/// `returning`: a client may ask for no rows back at all and still need to know
/// how many were touched, and that is the usual case for a delete.
fn create_mutation_response_type(base_name: &str) -> Object {
    let response_name = mutation_response_type_name(base_name);
    let row_type = base_name.to_string();

    Object::new(&response_name)
        .description(format!("The rows {} changed, and how many.", base_name))
        .field(Field::new(
            "affected_rows",
            TypeRef::named_nn(TypeRef::INT),
            |ctx| {
                FieldFuture::new(async move {
                    if let Some(Value::Object(map)) = ctx.parent_value.as_value() {
                        if let Some(value) = map.get(&async_graphql::Name::new("affected_rows")) {
                            return Ok(Some(FieldValue::value(value.clone())));
                        }
                    }
                    Ok(Some(FieldValue::value(Value::from(0))))
                })
            },
        ))
        .field(Field::new(
            "returning",
            TypeRef::named_nn_list_nn(row_type),
            |ctx| {
                FieldFuture::new(async move {
                    let rows = match ctx.parent_value.as_value() {
                        Some(Value::Object(map)) => match map
                            .get(&async_graphql::Name::new("returning"))
                        {
                            Some(Value::List(items)) => items.clone(),
                            _ => Vec::new(),
                        },
                        _ => Vec::new(),
                    };
                    Ok(Some(FieldValue::list(
                        rows.into_iter().map(FieldValue::value),
                    )))
                })
            },
        ))
}

/// Build every aggregate type for one table.
///
/// Four kinds, and they nest: `<t>_aggregate` holds `aggregate` and `nodes`,
/// `<t>_aggregate_fields` holds `count` and one field per function, and each
/// function has a type of its own holding the columns it applies to.
fn create_aggregate_types(base_name: &str, object: &TableObjectType) -> Vec<Object> {
    use crate::schema::aggregate as agg;

    let mut types = Vec::new();

    // `<t>_aggregate`: the rows, and the numbers about them.
    let fields_type = agg::aggregate_fields_type_name(base_name);
    types.push(
        Object::new(agg::aggregate_type_name(base_name))
            .description(format!("Aggregates over {}, with the rows themselves.", base_name))
            .field(Field::new(
                "aggregate",
                TypeRef::named(&fields_type),
                |ctx| FieldFuture::new(async move { Ok(child_of(&ctx, "aggregate")) }),
            ))
            .field(Field::new(
                "nodes",
                TypeRef::named_nn_list_nn(base_name.to_string()),
                |ctx| {
                    FieldFuture::new(async move {
                        let rows = match child_value(&ctx, "nodes") {
                            Some(Value::List(items)) => items,
                            _ => Vec::new(),
                        };
                        Ok(Some(FieldValue::list(rows.into_iter().map(FieldValue::value))))
                    })
                },
            )),
    );

    // `<t>_aggregate_fields`: count, and one field per function.
    let mut aggregate_fields = Object::new(&fields_type)
        .description(format!("Aggregate functions over {}.", base_name))
        .field(
            Field::new("count", TypeRef::named_nn(TypeRef::INT), |ctx| {
                FieldFuture::new(async move {
                    Ok(Some(
                        child_value(&ctx, "count").unwrap_or(Value::from(0)),
                    )
                    .map(FieldValue::value))
                })
            })
            // `count(columns:)` counts the rows where those columns are not
            // null, and `distinct` counts distinct values among them.
            .argument(InputValue::new(
                "columns",
                TypeRef::named_nn_list(agg_select_column(base_name)),
            ))
            .argument(InputValue::new("distinct", TypeRef::named("Boolean"))),
        );

    for (function, returns, columns) in agg::functions_for(object) {
        let function_type = agg::function_fields_type_name(base_name, function);
        let mut per_column = Object::new(&function_type)
            .description(format!("`{}` of each {} column it applies to.", function, base_name));
        for column in &columns {
            let column_name = column.clone();
            let type_name = agg::field_type_for(object, column, returns);
            per_column = per_column.field(Field::new(column, TypeRef::named(type_name), move |ctx| {
                let column_name = column_name.clone();
                FieldFuture::new(async move {
                    Ok(child_value(&ctx, &column_name).map(FieldValue::value))
                })
            }));
        }
        types.push(per_column);

        let owned = function.to_string();
        aggregate_fields = aggregate_fields.field(Field::new(
            function,
            TypeRef::named(&function_type),
            move |ctx| {
                let owned = owned.clone();
                FieldFuture::new(async move { Ok(child_of(&ctx, &owned)) })
            },
        ));
    }

    types.push(aggregate_fields);
    types
}

/// The column enum a `count(columns:)` draws from.
fn agg_select_column(base_name: &str) -> String {
    crate::input::order_by::select_column_type_name(base_name)
}

/// One key of the parent object, as a value.
fn child_value(ctx: &ResolverContext<'_>, key: &str) -> Option<Value> {
    match ctx.parent_value.as_value() {
        Some(Value::Object(map)) => map.get(&async_graphql::Name::new(key)).cloned(),
        _ => None,
    }
}

/// One key of the parent object, as a resolvable child.
///
/// `None` where the key is absent or null, which is what a client that did not
/// ask for that aggregate should see.
fn child_of<'a>(ctx: &ResolverContext<'_>, key: &str) -> Option<FieldValue<'a>> {
    match child_value(ctx, key) {
        None | Some(Value::Null) => None,
        Some(value) => Some(FieldValue::value(value)),
    }
}

fn create_object_type(obj: &TableObjectType, relationships: &[RelationshipField]) -> Object {
    let mut object = Object::new(&obj.name);

    if let Some(desc) = obj.description() {
        object = object.description(desc);
    }

    // A GraphQL object may not have two fields of one name, and a schema that
    // tries to build one aborts the process rather than returning an error.
    // Relationship names are derived from the table they point at, so a
    // foreign key column named after its own target -- `pizza.crust`
    // referencing `crust`, which is an ordinary way to write that schema --
    // produces exactly that clash.
    //
    // The column wins. It is the table's own data, it is what a client's
    // existing queries select, and dropping it to keep a derived name would
    // lose something the schema actually says. The relationship is left out
    // and said so, which is what Hasura does with the same clash: the
    // metadata is marked inconsistent and the field is simply not there.
    let mut taken: HashSet<String> = obj.fields.iter().map(|f| f.name.clone()).collect();

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
        if !taken.insert(rel.name.clone()) {
            tracing::warn!(
                "{}: not exposing the relationship to \"{}\" as `{}` -- the table \
                 already has a column of that name",
                obj.name,
                rel.target_type,
                rel.name
            );
            continue;
        }
        let field_name = rel.name.clone();
        let field_type = if rel.is_list {
            TypeRef::named_nn_list_nn(&rel.target_type)
        } else {
            TypeRef::named(&rel.target_type)
        };

        let mut gql_field = Field::new(&rel.name, field_type, move |ctx| {
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

        // The same arguments the root field takes. An embedded list is a list
        // like any other: a client showing the five most recent articles of an
        // author has nowhere else to say so, and without these the only way to
        // narrow one is to fetch all of it and discard the rest in the client.
        // A to-one relationship is left alone -- there is nothing to order or
        // page through when the answer is one row.
        if rel.is_list {
            gql_field = gql_field
                .argument(InputValue::new(
                    "where",
                    TypeRef::named(crate::input::bool_exp::bool_exp_type_name(&rel.target_type)),
                ))
                .argument(InputValue::new(
                    "order_by",
                    TypeRef::named_nn_list(crate::input::order_by::order_by_type_name(
                        &rel.target_type,
                    )),
                ))
                .argument(InputValue::new("limit", TypeRef::named("Int")))
                .argument(InputValue::new("offset", TypeRef::named("Int")));
        }

        let gql_field = if let Some(desc) = &rel.description {
            gql_field.description(desc)
        } else {
            gql_field
        };

        object = object.field(gql_field);

        // `author { articles_aggregate { aggregate { count } } }` -- the count
        // of a row's children without fetching them, which is the query behind
        // every "12 comments" beside a post. Only for a relationship to many:
        // counting one row is not a question anyone asks.
        if rel.is_list {
            let aggregate_field = format!("{}_aggregate", rel.name);
            if taken.insert(aggregate_field.clone()) {
                let key = aggregate_field.clone();
                object = object.field(
                    Field::new(
                        &aggregate_field,
                        TypeRef::named_nn(crate::schema::aggregate::aggregate_type_name(
                            &rel.target_type,
                        )),
                        move |ctx| {
                            let key = key.clone();
                            FieldFuture::new(async move {
                                // Absent where the parent has no children at
                                // all; the aggregate type's own resolvers read
                                // a missing key as zero and an empty list.
                                Ok(Some(FieldValue::value(
                                    child_value(&ctx, &key).unwrap_or(Value::Null),
                                )))
                            })
                        },
                    )
                    .description(format!("Aggregates over {}.", rel.name)),
                );
            }
        }
    }

    object
}

/// Create the Query type with all table query fields.
fn create_query_type(
    generated: &GeneratedSchema,
    max_rows: Option<i64>,
    relationships: Arc<HashMap<String, Vec<RelationshipField>>>,
    names: Arc<crate::names::NameOverrides>,
) -> Object {
    let mut query = Object::new("query_root");

    for field in &generated.query_fields {
        let table_name = field.table_name.clone();
        let schema_name = field.schema_name.clone();
        let type_name = field.type_name.clone();
        let is_by_pk = field.is_by_pk;
        let pk_columns = field.pk_columns.clone();
        let return_type = graphql_type_ref(&field.return_type);

        let spec_type_name = type_name.clone();
        let spec = Arc::new(QueryFieldSpec {
            schema_name,
            table_name,
            type_name,
            is_by_pk,
            pk_columns: pk_columns.clone(),
            max_rows,
            relationships: Arc::clone(&relationships),
            names: Arc::clone(&names),
        });

        let mut gql_field = Field::new(&field.name, return_type, move |ctx| {
            let spec = Arc::clone(&spec);
            FieldFuture::new(async move { resolve_query(&ctx, &spec).await })
        });

        // Add standard query arguments
        if !is_by_pk {
            gql_field = gql_field
                .argument(InputValue::new(
                    "where",
                    TypeRef::named(crate::input::bool_exp::bool_exp_type_name(
                        &spec_type_name,
                    )),
                ))
                .argument(InputValue::new(
                    "order_by",
                    TypeRef::named_nn_list(crate::input::order_by::order_by_type_name(
                        &spec_type_name,
                    )),
                ))
                .argument(InputValue::new(
                    "distinct_on",
                    TypeRef::named_nn_list(crate::input::order_by::select_column_type_name(
                        &spec_type_name,
                    )),
                ))
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

        // The same rows, with numbers about them. Same arguments as the list
        // field, because `author_aggregate(where: ...)` counts the set the
        // filter describes, not the whole table.
        if !is_by_pk {
            let agg_spec = Arc::new(AggregateSpec {
                schema_name: field.schema_name.clone(),
                table_name: field.table_name.clone(),
                type_name: field.type_name.clone(),
                max_rows,
                relationships: Arc::clone(&relationships),
                names: Arc::clone(&names),
            });
            let mut agg_field = Field::new(
                crate::schema::aggregate::aggregate_type_name(&field.type_name),
                TypeRef::named_nn(crate::schema::aggregate::aggregate_type_name(
                    &field.type_name,
                )),
                move |ctx| {
                    let agg_spec = Arc::clone(&agg_spec);
                    FieldFuture::new(async move { resolve_aggregate(&ctx, &agg_spec).await })
                },
            );
            agg_field = agg_field
                .argument(InputValue::new(
                    "where",
                    TypeRef::named(crate::input::bool_exp::bool_exp_type_name(&spec_type_name)),
                ))
                .argument(InputValue::new(
                    "order_by",
                    TypeRef::named_nn_list(crate::input::order_by::order_by_type_name(
                        &spec_type_name,
                    )),
                ))
                .argument(InputValue::new(
                    "distinct_on",
                    TypeRef::named_nn_list(crate::input::order_by::select_column_type_name(
                        &spec_type_name,
                    )),
                ))
                .argument(InputValue::new("limit", TypeRef::named("Int")))
                .argument(InputValue::new("offset", TypeRef::named("Int")))
                .description(format!("Aggregates over {}.", field.table_name));
            query = query.field(agg_field);
        }
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

/// Add the operators an update may be written with.
///
/// All optional, and at least one required -- which GraphQL cannot express, so
/// the resolver says so instead of the schema. Making `_set` non-null would
/// have been expressible and wrong: an update that only increments a counter
/// never sends one.
fn with_update_operators(field: Field, base_name: &str, has_numeric: bool) -> Field {
    let mut field = field.argument(InputValue::new(
        "_set",
        TypeRef::named(format!("{}_set_input", base_name)),
    ));
    if has_numeric {
        field = field.argument(InputValue::new(
            "_inc",
            TypeRef::named(format!("{}_inc_input", base_name)),
        ));
    }
    field
        // The jsonb operators keep an untyped argument: each takes a value of
        // a different shape per column -- a key, an index, a path -- and the
        // type that would say so is one per operator per table.
        .argument(InputValue::new("_append", TypeRef::named("JSON")))
        .argument(InputValue::new("_prepend", TypeRef::named("JSON")))
        .argument(InputValue::new("_delete_key", TypeRef::named("JSON")))
        .argument(InputValue::new("_delete_elem", TypeRef::named("JSON")))
        .argument(InputValue::new("_delete_at_path", TypeRef::named("JSON")))
}

/// Create the Mutation type with all mutation fields.
fn create_mutation_type(
    generated: &GeneratedSchema,
    relationships: Arc<HashMap<String, Vec<RelationshipField>>>,
    type_names: Arc<HashMap<(String, String), String>>,
    names: Arc<crate::names::NameOverrides>,
    max_rows: Option<i64>,
) -> Object {
    let mut mutation = Object::new("mutation_root");

    // Only a table with a unique constraint has a conflict to name, and only
    // those got the types for it.
    // Which tables have something to add to, since `_inc` is only offered
    // where there is.
    let has_numeric_column: HashSet<String> = generated
        .object_types
        .iter()
        .filter(|(_, object)| {
            object.fields.iter().any(|field| {
                object.table.get_column(&field.name).is_some()
                    && crate::schema::aggregate::is_numeric(&field.graphql_type)
            })
        })
        .map(|(type_name, _)| type_name.clone())
        .collect();

    let has_conflict_target: HashSet<String> = generated
        .object_types
        .iter()
        .filter(|(_, object)| !object.table.unique_constraints.is_empty())
        .map(|(type_name, _)| type_name.clone())
        .collect();

    for field in &generated.mutation_fields {
        let table_name = field.table_name.clone();
        let schema_name = field.schema_name.clone();
        let mutation_type = field.mutation_type;
        let pk_columns = field.pk_columns.clone();
        let return_type = graphql_type_ref(&field.return_type);

        // The table's own type name, which is what its boolean expression is
        // named after. A bulk mutation returns `[author!]!` and a by-key one
        // returns `author`; both name the same table.
        let where_type = field
            .return_type
            .trim_matches(|c| c == '[' || c == ']' || c == '!')
            .trim_end_matches("_mutation_response")
            .to_string();

        let resolver_pk_columns = pk_columns.clone();
        let field_relationships = Arc::clone(&relationships);
        let field_type_names = Arc::clone(&type_names);
        let field_names = Arc::clone(&names);
        let mut gql_field = Field::new(&field.name, return_type, move |ctx| {
            let table_name = table_name.clone();
            let schema_name = schema_name.clone();
            let pk_columns = resolver_pk_columns.clone();
            let relationships = Arc::clone(&field_relationships);
            let type_names = Arc::clone(&field_type_names);
            let names = Arc::clone(&field_names);
            FieldFuture::new(async move {
                resolve_mutation(
                    &ctx,
                    &schema_name,
                    &table_name,
                    mutation_type,
                    &pk_columns,
                    &relationships,
                    &type_names,
                    &names,
                    max_rows,
                )
                .await
            })
        });

        // Add mutation-specific arguments.
        //
        // A by-PK mutation takes the key columns rather than a `where` object:
        // it is meant to address exactly one row, and accepting `where` made it
        // an ordinary bulk mutation that happened to return the first result.
        match mutation_type {
            MutationType::Insert | MutationType::InsertOne => {
                let insert_input = format!("{}_insert_input", where_type);
                gql_field = if mutation_type == MutationType::Insert {
                    gql_field.argument(InputValue::new(
                        "objects",
                        TypeRef::named_nn_list_nn(&insert_input),
                    ))
                } else {
                    // A single insert takes `object`, not a one-element
                    // `objects`.
                    gql_field
                        .argument(InputValue::new("object", TypeRef::named_nn(&insert_input)))
                };
                if has_conflict_target.contains(&where_type) {
                    gql_field = gql_field.argument(InputValue::new(
                        "on_conflict",
                        TypeRef::named(format!("{}_on_conflict", where_type)),
                    ));
                }
            }
            MutationType::UpdateByPk => {
                gql_field = gql_field.argument(InputValue::new(
                    "pk_columns",
                    TypeRef::named_nn(format!("{}_pk_columns_input", where_type)),
                ));
                gql_field = with_update_operators(
                    gql_field,
                    &where_type,
                    has_numeric_column.contains(&where_type),
                );
            }
            MutationType::Update => {
                gql_field = gql_field
                    .argument(InputValue::new(
                        "where",
                        TypeRef::named_nn(crate::input::bool_exp::bool_exp_type_name(
                            &where_type,
                        )),
                    ));
                gql_field = with_update_operators(
                    gql_field,
                    &where_type,
                    has_numeric_column.contains(&where_type),
                );
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
                gql_field = gql_field.argument(InputValue::new(
                    "where",
                    TypeRef::named(crate::input::bool_exp::bool_exp_type_name(&where_type)),
                ));
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
    let mut subscription = Subscription::new("subscription_root");

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
    names: Arc<crate::names::NameOverrides>,
}

/// Everything an aggregate field's resolver needs.
struct AggregateSpec {
    schema_name: String,
    table_name: String,
    type_name: String,
    max_rows: Option<i64>,
    relationships: Arc<HashMap<String, Vec<RelationshipField>>>,
    names: Arc<crate::names::NameOverrides>,
}

/// Resolve an aggregate field.
///
/// One pass over the selection decides what SQL to write: a client asking only
/// for `count` should not pay for the rows, and one asking only for `nodes`
/// should not pay for a second scan. Both halves read from the same filtered
/// subquery, so `aggregate` describes exactly the set `nodes` came from.
async fn resolve_aggregate<'a>(
    ctx: &ResolverContext<'a>,
    spec: &AggregateSpec,
) -> Result<Option<FieldValue<'a>>, async_graphql::Error> {
    use crate::schema::aggregate as agg;

    let pool = ctx.data::<PgPool>()?;
    let gql_ctx = ctx.data::<GraphQLContext>()?;

    let mut bound_values: Vec<serde_json::Value> = Vec::new();
    let mut where_sql = String::new();
    if let Some(filter) = ctx.args.try_get("where").ok().map(|v| accessor_to_json(&v)) {
        let guard = gql_ctx
            .schema_cache
            .get()
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;
        let cache = guard
            .as_ref()
            .ok_or_else(|| async_graphql::Error::new("schema cache is not loaded"))?;
        let scope =
            WhereScope::table(&spec.schema_name, &spec.table_name, &spec.type_name)
                .with_resolution(cache, spec.relationships.as_ref());
        let (sql, values) = build_where_clause(Some(&filter), 1, &scope)?;
        if !sql.is_empty() {
            where_sql = format!(" {}", sql);
            bound_values = values;
        }
    }

    let order_sql = build_order_by_clause(
        ctx,
        &gql_ctx.schema_cache,
        &spec.schema_name,
        &spec.table_name,
        &spec.type_name,
        spec.relationships.as_ref(),
    )
    .await?;

    let requested_limit = ctx.args.try_get("limit").ok().and_then(|v| v.i64().ok());
    let offset = ctx.args.try_get("offset").ok().and_then(|v| v.i64().ok());
    let limit = match (requested_limit, spec.max_rows) {
        (Some(requested), Some(ceiling)) => Some(requested.min(ceiling)),
        (Some(requested), None) => Some(requested),
        (None, ceiling) => ceiling,
    };

    // A computed field asked for under `nodes` is a function of the row and so
    // is not in `*`, exactly as at the root.
    let computed_sql = {
        let guard = gql_ctx
            .schema_cache
            .get()
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;
        let nodes = ctx
            .field()
            .selection_set()
            .find(|selection| selection.name() == "nodes");
        match (guard.as_ref(), nodes) {
            (Some(cache), Some(nodes)) => {
                let qi = postrust_core::api_request::QualifiedIdentifier::new(
                    &spec.schema_name,
                    &spec.table_name,
                );
                match cache.get_table(&qi) {
                    Some(table) => {
                        let qualified = format!(
                            "{}.{}",
                            postrust_sql::escape_ident(&spec.schema_name),
                            postrust_sql::escape_ident(&spec.table_name)
                        );
                        let projections = computed_projections(
                            table,
                            nodes,
                            &format!("{}.*", qualified),
                            spec.names.as_ref(),
                        );
                        if projections.is_empty() {
                            String::new()
                        } else {
                            format!(", {}", projections.join(", "))
                        }
                    }
                    None => String::new(),
                }
            }
            _ => String::new(),
        }
    };

    let mut inner = format!(
        "SELECT *{} FROM {}.{}{}{}",
        computed_sql,
        postrust_sql::escape_ident(&spec.schema_name),
        postrust_sql::escape_ident(&spec.table_name),
        where_sql,
        order_sql
    );
    if let Some(limit) = limit {
        inner.push_str(&format!(" LIMIT {}", limit));
    }
    if let Some(offset) = offset {
        inner.push_str(&format!(" OFFSET {}", offset));
    }

    // What the client actually asked for.
    let mut wants_nodes = false;
    let mut wanted: Vec<(String, Vec<String>)> = Vec::new();
    let mut wants_count = false;
    for selection in ctx.field().selection_set() {
        match selection.name() {
            "nodes" => wants_nodes = true,
            "aggregate" => {
                for function in selection.selection_set() {
                    if function.name() == "count" {
                        wants_count = true;
                        continue;
                    }
                    let columns: Vec<String> = function
                        .selection_set()
                        .map(|c| c.name().to_string())
                        .collect();
                    if !columns.is_empty() {
                        wanted.push((function.name().to_string(), columns));
                    }
                }
            }
            _ => {}
        }
    }

    let mut result = async_graphql::indexmap::IndexMap::new();

    if wants_count || !wanted.is_empty() {
        let mut parts = vec!["'count', count(*)".to_string()];
        for (function, columns) in &wanted {
            // The function name comes from the schema, not from the request:
            // a selection can only name a field that was generated, so there
            // is no path from client text to this identifier.
            let sql_function = agg::NUMERIC_AGGREGATES
                .iter()
                .chain(agg::ORDERED_AGGREGATES.iter())
                .find(|(name, _)| name == function)
                .map(|(name, _)| *name)
                .ok_or_else(|| {
                    async_graphql::Error::new(format!("unknown aggregate \"{}\"", function))
                })?;
            let per_column: Vec<String> = columns
                .iter()
                .map(|column| {
                    format!(
                        "'{}', {}({})",
                        column.replace('\'', "''"),
                        sql_function,
                        postrust_sql::escape_ident(column)
                    )
                })
                .collect();
            parts.push(format!(
                "'{}', json_build_object({})",
                sql_function,
                per_column.join(", ")
            ));
        }

        let sql = format!(
            "SELECT json_build_object({}) FROM ({}) AS pgrst_agg",
            parts.join(", "),
            inner
        );
        let mut conn = begin_with_session(pool, gql_ctx.role(), &gql_ctx.session_settings()).await?;
        let rows = execute_query_on(&mut conn, &sql, &bound_values).await?;
        conn.commit().await?;
        if let Some(first) = rows.into_iter().next() {
            if let Value::Object(map) = json_to_value(first) {
                result.insert(
                    async_graphql::Name::new("aggregate"),
                    Value::Object(map),
                );
            }
        }
    }

    if wants_nodes {
        let sql = format!(
            "SELECT row_to_json(pgrst_nodes) FROM ({}) AS pgrst_nodes",
            inner
        );
        let mut conn = begin_with_session(pool, gql_ctx.role(), &gql_ctx.session_settings()).await?;
        let rows = execute_query_on(&mut conn, &sql, &bound_values).await?;
        conn.commit().await?;
        result.insert(
            async_graphql::Name::new("nodes"),
            Value::List(rows.into_iter().map(json_to_value).collect()),
        );
    }

    let _ = &spec.type_name;
    Ok(Some(FieldValue::value(Value::Object(result))))
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
    } else if let Some(filter) = ctx.args.try_get("where").ok().map(|v| accessor_to_json(&v)) {
        let guard = gql_ctx
            .schema_cache
            .get()
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;
        let cache = guard
            .as_ref()
            .ok_or_else(|| async_graphql::Error::new("schema cache is not loaded"))?;
        let scope = WhereScope::table(schema_name, table_name, type_name)
            .with_resolution(cache, relationships);
        let (filter_sql, filter_values) = build_where_clause(Some(&filter), 1, &scope)?;
        if !filter_sql.is_empty() {
            where_sql = format!(" {}", filter_sql);
            bound_values = filter_values;
        }
    }

    // A by-key query resolves to one row, so neither ordering nor distinct
    // has anything to do.
    let (order_sql, distinct_on) = if is_by_pk {
        (String::new(), Vec::new())
    } else {
        (
            build_order_by_clause(
                ctx,
                &gql_ctx.schema_cache,
                schema_name,
                table_name,
                type_name,
                relationships,
            )
            .await?,
            build_distinct_on(ctx, &gql_ctx.schema_cache, schema_name, table_name).await?,
        )
    };

    // PostgreSQL keeps the first row of each DISTINCT ON group in the query's
    // own order, and picks arbitrarily where the order does not begin with the
    // distinct columns. Prepending them keeps whatever the client asked for as
    // the tiebreak instead of discarding it.
    let (distinct_sql, order_sql) = if distinct_on.is_empty() {
        (String::new(), order_sql)
    } else {
        let mut terms: Vec<String> = distinct_on.clone();
        if let Some(rest) = order_sql.strip_prefix(" ORDER BY ") {
            for term in rest.split(", ") {
                let column = term.split_whitespace().next().unwrap_or(term);
                if !distinct_on.iter().any(|d| d == column) {
                    terms.push(term.to_string());
                }
            }
        }
        (
            format!("DISTINCT ON ({}) ", distinct_on.join(", ")),
            format!(" ORDER BY {}", terms.join(", ")),
        )
    };

    // A computed column is a function of the row rather than part of it, so
    // it is not in `*` and is named only when it was asked for.
    let computed = {
        let guard = gql_ctx
            .schema_cache
            .get()
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;
        let qualified = format!(
            "{}.{}",
            postrust_sql::escape_ident(schema_name),
            postrust_sql::escape_ident(table_name)
        );
        match guard.as_ref() {
            Some(cache) => {
                let qi =
                    postrust_core::api_request::QualifiedIdentifier::new(schema_name, table_name);
                match cache.get_table(&qi) {
                    Some(table) => {
                        let _ = &qualified;
                        computed_projections(table, ctx.field(), "src", spec.names.as_ref())
                    }
                    None => Vec::new(),
                }
            }
            None => Vec::new(),
        }
    };
    // ORDER BY, LIMIT and OFFSET belong inside the subquery: applying them to
    // the outer `row_to_json` projection would leave the ordering of the rows
    // that survive the limit unspecified.
    let mut inner = format!(
        "SELECT {}* FROM {}.{}{}{}",
        distinct_sql,
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
            Some(cache) => {
                // Parameters bound inside an embed continue the outer query's
                // numbering: sqlx binds by position, so the values only have
                // to be pushed in the order their placeholders were handed
                // out.
                let mut param_idx = bound_values.len() + 1;
                build_embed_expressions(
                    cache,
                    relationships,
                    type_name,
                    "src",
                    ctx.field(),
                    max_rows,
                    &mut 0,
                    &mut param_idx,
                    &mut bound_values,
                    spec.names.as_ref(),
                )?
            }
            None => Vec::new(),
        }
    };

    // A computed column is a function of the row, and the row it is a function
    // of is the table's -- not the subquery's. Projecting it inside the
    // subquery gave `src` an extra column, which is what made a computed
    // relationship beside it fail with `cannot cast type record to author`: a
    // subquery alias is only passable as a composite value while its columns
    // are exactly the table's. So both live out here, where `src` still is
    // one.
    let inner = if embed_expressions.is_empty() && computed.is_empty() {
        inner
    } else {
        let mut projection = String::from("src.*");
        for expression in &computed {
            projection.push_str(", ");
            projection.push_str(expression);
        }
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
    let mut tx = begin_with_session(pool, gql_ctx.role(), &gql_ctx.session_settings()).await?;

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
#[allow(clippy::too_many_arguments)]
async fn resolve_mutation<'a>(
    ctx: &ResolverContext<'a>,
    schema_name: &str,
    table_name: &str,
    mutation_type: MutationType,
    pk_columns: &[(String, String)],
    relationships: &Arc<HashMap<String, Vec<RelationshipField>>>,
    type_names: &Arc<HashMap<(String, String), String>>,
    names: &Arc<crate::names::NameOverrides>,
    max_rows: Option<i64>,
) -> Result<Option<FieldValue<'a>>, async_graphql::Error> {
    let pool = ctx.data::<PgPool>()?;
    let gql_ctx = ctx.data::<GraphQLContext>()?;

    debug!(
        "Resolving mutation for table: {} type: {:?}",
        table_name, mutation_type
    );

    let (result, affected) = match mutation_type {
        MutationType::Insert | MutationType::InsertOne => {
            let objects = ctx
                .args
                .try_get("objects")
                .ok()
                .map(|v| accessor_to_json(&v))
                .or_else(|| {
                    // `insert_x_one(object: {...})` is one row spelled without
                    // the list.
                    ctx.args
                        .try_get("object")
                        .ok()
                        .map(|v| serde_json::Value::Array(vec![accessor_to_json(&v)]))
                })
                .unwrap_or_else(|| serde_json::Value::Array(vec![]));

            let guard = gql_ctx
                .schema_cache
                .get()
                .await
                .map_err(|e| async_graphql::Error::new(e.to_string()))?;
            let cache = guard
                .as_ref()
                .ok_or_else(|| async_graphql::Error::new("schema cache is not loaded"))?;
            let context = InsertContext {
                on_conflict: ctx
                    .args
                    .try_get("on_conflict")
                    .ok()
                    .map(|v| accessor_to_json(&v))
                    .filter(|v| !v.is_null()),
                cache,
                relationships: relationships.as_ref(),
                type_names: type_names.as_ref(),
            };

            execute_insert(
                pool,
                schema_name,
                table_name,
                gql_ctx.role(),
                objects,
                &context,
            )
            .await?
        }
        MutationType::Update | MutationType::UpdateByPk => {
            // `_set` replaces, the others read the column they write. A client
            // may send more than one, so all of them are collected rather than
            // the first that happens to be present.
            const OPERATORS: [&str; 7] = [
                "_set",
                "_inc",
                "_append",
                "_prepend",
                "_delete_key",
                "_delete_elem",
                "_delete_at_path",
            ];
            let operators: Vec<(&'static str, serde_json::Value)> = OPERATORS
                .iter()
                .filter_map(|name| {
                    ctx.args
                        .try_get(name)
                        .ok()
                        .map(|v| (*name, accessor_to_json(&v)))
                })
                .filter(|(_, value)| !value.is_null())
                .collect();

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
                operators,
                column_types_of(&gql_ctx.schema_cache, schema_name, table_name).await,
                where_clause,
            )
            .await
            .map(|rows| {
                let count = rows.len();
                (rows, count)
            })?
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
            )
            .await
            .map(|rows| {
                let count = rows.len();
                (rows, count)
            })?
        }
    };

    // A relationship or a computed field asked for beside the written columns
    // is not in `RETURNING`, so the rows are read again through the projection
    // an ordinary query uses -- and only when the selection asks for something
    // that needs it.
    let type_name = type_names
        .get(&(schema_name.to_string(), table_name.to_string()))
        .cloned()
        .unwrap_or_else(|| table_name.to_string());

    let by_key = matches!(
        mutation_type,
        MutationType::InsertOne | MutationType::UpdateByPk | MutationType::DeleteByPk
    );
    let returning = if by_key {
        Some(ctx.field())
    } else {
        ctx.field()
            .selection_set()
            .find(|selection| selection.name() == "returning")
    };

    let result = match returning {
        Some(returning) if !result.is_empty() => {
            reread_returning(
                pool,
                gql_ctx,
                schema_name,
                table_name,
                &type_name,
                result,
                returning,
                relationships.as_ref(),
                names.as_ref(),
                max_rows,
            )
            .await?
        }
        _ => result,
    };

    Ok(mutation_result(result, affected, by_key))
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
    begin_with_session(pool, role, &[]).await
}

/// Begin a transaction with the request's role and session variables applied.
///
/// The settings go in before the role does. A role with fewer privileges may
/// not be allowed to call `set_config` at all, and a policy that reads a
/// setting the caller could then change is not a policy.
async fn begin_with_session(
    pool: &PgPool,
    role: &str,
    settings: &[(String, String)],
) -> Result<sqlx::Transaction<'static, sqlx::Postgres>, async_graphql::Error> {
    let mut tx = pool.begin().await?;

    for (name, value) in settings {
        sqlx::query("SELECT set_config($1, $2, true)")
            .bind(name)
            .bind(value)
            .execute(&mut *tx)
            .await?;
    }

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

/// The cast a written value needs to reach a column of this type.
///
/// A bound parameter arrives as text. PostgreSQL will coerce it to a numeric
/// or a date on assignment, but not to `jsonb`, an array, or a user-defined
/// type -- `column "details" is of type jsonb but expression is of type text`
/// is what an uncast insert answers. Naming the column's own type covers all
/// of them and changes nothing where the coercion would have worked anyway.
fn write_cast(column_types: &HashMap<String, String>, column: &str) -> String {
    match column_types.get(column) {
        Some(pg_type) => format!("::{}", pg_type),
        None => String::new(),
    }
}

/// Every column of a table with the type it is declared as.
async fn column_types_of(
    schema_cache: &postrust_core::schema_cache::SchemaCacheRef,
    schema_name: &str,
    table_name: &str,
) -> HashMap<String, String> {
    let Ok(guard) = schema_cache.get().await else {
        return HashMap::new();
    };
    let Some(cache) = guard.as_ref() else {
        return HashMap::new();
    };
    let qi = postrust_core::api_request::QualifiedIdentifier::new(schema_name, table_name);
    match cache.get_table(&qi) {
        Some(table) => table
            .columns
            .values()
            .map(|c| (c.name.clone(), c.nominal_type.clone()))
            .collect(),
        None => HashMap::new(),
    }
}

/// Insert one row, and any rows nested inside it.
///
/// Hasura writes a parent and its children in one mutation:
///
/// ```graphql
/// insert_article(objects: [{title: "x", author: {data: {name: "y"}}}])
/// ```
///
/// Which row goes first follows from which side holds the key. A relationship
/// to one row is reached through a column of *this* table, so the related row
/// is inserted first and its key is what this row's column is set to. A
/// relationship to many rows is reached through a column of *theirs*, so this
/// row goes first and each child's column is set from it.
///
/// Everything runs in the transaction the caller opened, so a child that
/// cannot be written takes the parent with it -- which is what writing them in
/// one mutation means.
fn insert_row<'life>(
    conn: &'life mut sqlx::PgConnection,
    schema_name: &'life str,
    table_name: &'life str,
    object: serde_json::Map<String, serde_json::Value>,
    context: &'life InsertContext<'life>,
    written_count: &'life mut usize,
) -> std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<serde_json::Value, async_graphql::Error>> + Send + 'life>,
> {
    Box::pin(async move {
        use sqlx::Row;

        let qi = postrust_core::api_request::QualifiedIdentifier::new(schema_name, table_name);
        let table = context
            .cache
            .get_table(&qi)
            .ok_or_else(|| async_graphql::Error::new(format!("unknown table \"{}\"", table_name)))?;
        let column_types: HashMap<String, String> = table
            .columns
            .values()
            .map(|c| (c.name.clone(), c.nominal_type.clone()))
            .collect();

        // A key that is a relationship rather than a column is a nested write.
        let type_name = context.type_names.get(&(schema_name.to_string(), table_name.to_string()));
        let relationships: &[RelationshipField] = type_name
            .and_then(|name| context.relationships.get(name))
            .map(|r| r.as_slice())
            .unwrap_or(&[]);

        let mut columns = serde_json::Map::new();
        type Nested<'r> = (&'r RelationshipField, serde_json::Value, Option<serde_json::Value>);
        let mut to_one: Vec<Nested> = Vec::new();
        let mut to_many: Vec<Nested> = Vec::new();

        for (key, value) in object {
            match relationships.iter().find(|r| r.name == key) {
                None => {
                    columns.insert(key, value);
                }
                Some(relationship) => {
                    // `{data: {...}}` for one row, `{data: [{...}]}` for many,
                    // and an `on_conflict` beside either -- a nested row is
                    // upserted the same way a top-level one is.
                    let (data, conflict) = match &value {
                        serde_json::Value::Object(map) => (
                            map.get("data").cloned().unwrap_or(value.clone()),
                            map.get("on_conflict").cloned().filter(|v| !v.is_null()),
                        ),
                        other => (other.clone(), None),
                    };
                    if relationship.is_list {
                        to_many.push((relationship, data, conflict));
                    } else {
                        to_one.push((relationship, data, conflict));
                    }
                }
            }
        }

        // The rows this one points at, first: this row's own column carries
        // their key.
        for (relationship, data, conflict) in to_one {
            let plan =
                postrust_core::embed::EmbedPlan::resolve(&relationship.relationship, context.cache)
                    .map_err(|e| async_graphql::Error::new(e.to_string()))?;
            let serde_json::Value::Object(child) = data else {
                return Err(async_graphql::Error::new(format!(
                    "\"{}\" takes an object to insert",
                    relationship.name
                )));
            };
            let nested = InsertContext {
                on_conflict: conflict,
                cache: context.cache,
                relationships: context.relationships,
                type_names: context.type_names,
            };
            let written = insert_row(
                conn,
                &plan.foreign_schema,
                &plan.foreign_table,
                child,
                &nested,
                written_count,
            )
            .await?;
            for (local, foreign) in &plan.columns {
                if let Some(value) = written.get(foreign) {
                    columns.insert(local.clone(), value.clone());
                }
            }
        }

        // `ON CONFLICT` says which uniqueness is being resolved against and
        // what to do about it. An empty `update_columns` is `DO NOTHING`,
        // which is how a client says "leave the row that is already there".
        let conflict_sql = match &context.on_conflict {
            Some(serde_json::Value::Object(spec)) => {
                let constraint = spec.get("constraint").and_then(|v| v.as_str());
                let Some(constraint) = constraint else {
                    return Err(async_graphql::Error::new(
                        "on_conflict needs a constraint to resolve against",
                    ));
                };
                if !table
                    .unique_constraints
                    .iter()
                    .any(|(name, _)| name == constraint)
                {
                    return Err(async_graphql::Error::new(format!(
                        "\"{}\" is not a unique constraint of \"{}\"",
                        constraint, table_name
                    )));
                }
                let updates: Vec<&str> = spec
                    .get("update_columns")
                    .and_then(|v| v.as_array())
                    .map(|items| items.iter().filter_map(|i| i.as_str()).collect())
                    .unwrap_or_default();

                if updates.is_empty() {
                    format!(
                        " ON CONFLICT ON CONSTRAINT {} DO NOTHING",
                        postrust_sql::escape_ident(constraint)
                    )
                } else {
                    let assignments: Vec<String> = updates
                        .iter()
                        .map(|column| {
                            format!(
                                "{} = EXCLUDED.{}",
                                postrust_sql::escape_ident(column),
                                postrust_sql::escape_ident(column)
                            )
                        })
                        .collect();
                    format!(
                        " ON CONFLICT ON CONSTRAINT {} DO UPDATE SET {}",
                        postrust_sql::escape_ident(constraint),
                        assignments.join(", ")
                    )
                }
            }
            _ => String::new(),
        };

        let names: Vec<&str> = columns.keys().map(|k| k.as_str()).collect();
        let written = if names.is_empty() {
            // Every column defaulted. `DEFAULT VALUES` is how SQL says that;
            // an empty column list is a syntax error.
            let sql = format!(
                "INSERT INTO {}.{} DEFAULT VALUES{} RETURNING row_to_json({}.{}.*)",
                postrust_sql::escape_ident(schema_name),
                postrust_sql::escape_ident(table_name),
                conflict_sql,
                postrust_sql::escape_ident(schema_name),
                postrust_sql::escape_ident(table_name)
            );
            sqlx::query(&sql).fetch_optional(&mut *conn).await?
        } else {
            let placeholders: Vec<String> = names
                .iter()
                .enumerate()
                .map(|(i, column)| format!("${}{}", i + 1, write_cast(&column_types, column)))
                .collect();
            let sql = format!(
                "INSERT INTO {}.{} ({}) VALUES ({}){} RETURNING row_to_json({}.{}.*)",
                postrust_sql::escape_ident(schema_name),
                postrust_sql::escape_ident(table_name),
                names
                    .iter()
                    .map(|c| postrust_sql::escape_ident(c))
                    .collect::<Vec<_>>()
                    .join(", "),
                placeholders.join(", "),
                conflict_sql,
                postrust_sql::escape_ident(schema_name),
                postrust_sql::escape_ident(table_name)
            );
            trace!("Executing INSERT SQL: {}", sql);
            let mut query = sqlx::query(&sql);
            for column in &names {
                if let Some(value) = columns.get(*column) {
                    query = bind_json_value(query, value);
                }
            }
            query.fetch_optional(&mut *conn).await?
        };

        // `DO NOTHING` writes nothing and returns nothing, which is the answer
        // rather than an error: the row that was already there stays.
        let Some(written) = written else {
            return Ok(serde_json::Value::Null);
        };
        let row: serde_json::Value = written.try_get(0).unwrap_or(serde_json::Value::Null);
        *written_count += 1;

        // Then the rows that point at this one, which need its key.
        for (relationship, data, conflict) in to_many {
            let plan =
                postrust_core::embed::EmbedPlan::resolve(&relationship.relationship, context.cache)
                    .map_err(|e| async_graphql::Error::new(e.to_string()))?;
            let children = match data {
                serde_json::Value::Array(items) => items,
                serde_json::Value::Object(map) => vec![serde_json::Value::Object(map)],
                serde_json::Value::Null => Vec::new(),
                _ => {
                    return Err(async_graphql::Error::new(format!(
                        "\"{}\" takes objects to insert",
                        relationship.name
                    )))
                }
            };
            for child in children {
                let serde_json::Value::Object(mut child) = child else {
                    continue;
                };
                for (local, foreign) in &plan.columns {
                    if let Some(value) = row.get(local) {
                        child.insert(foreign.clone(), value.clone());
                    }
                }
                let nested = InsertContext {
                    on_conflict: conflict.clone(),
                    cache: context.cache,
                    relationships: context.relationships,
                    type_names: context.type_names,
                };
                insert_row(
                    conn,
                    &plan.foreign_schema,
                    &plan.foreign_table,
                    child,
                    &nested,
                    written_count,
                )
                .await?;
            }
        }

        Ok(row)
    })
}

/// What a nested insert needs to follow a relationship.
struct InsertContext<'a> {
    /// What to do when this row conflicts. A nested row carries its own, from
    /// the `on_conflict` beside its `data`.
    on_conflict: Option<serde_json::Value>,
    cache: &'a SchemaCache,
    relationships: &'a HashMap<String, Vec<RelationshipField>>,
    /// (schema, table) -> the GraphQL type name, which keys the relationship
    /// map. A table's type is not always its name: a second schema prefixes
    /// it, and a name may have been given.
    type_names: &'a HashMap<(String, String), String>,
}

/// Re-read written rows so that a mutation's `returning` can carry
/// relationships and computed fields.
///
/// `RETURNING row_to_json(t.*)` gives the table's own columns and nothing
/// else, so a relationship asked for beside them had no value to resolve --
/// and a non-null list field with no value is an error, which meant a mutation
/// that had written its rows correctly answered as though it had failed. The
/// write is not in doubt; only the shape of the answer is.
///
/// So the rows are read again by their primary key, through the same
/// projection an ordinary query uses. One extra round trip, and only when the
/// selection actually asks for something `RETURNING` cannot give.
#[allow(clippy::too_many_arguments)]
async fn reread_returning(
    pool: &PgPool,
    gql_ctx: &GraphQLContext,
    schema_name: &str,
    table_name: &str,
    type_name: &str,
    rows: Vec<Value>,
    returning: async_graphql::SelectionField<'_>,
    relationships: &HashMap<String, Vec<RelationshipField>>,
    names: &crate::names::NameOverrides,
    max_rows: Option<i64>,
) -> Result<Vec<Value>, async_graphql::Error> {
    let guard = gql_ctx
        .schema_cache
        .get()
        .await
        .map_err(|e| async_graphql::Error::new(e.to_string()))?;
    let Some(cache) = guard.as_ref() else {
        return Ok(rows);
    };
    let qi = postrust_core::api_request::QualifiedIdentifier::new(schema_name, table_name);
    let Some(table) = cache.get_table(&qi) else {
        return Ok(rows);
    };
    if table.pk_cols.is_empty() {
        // Nothing to identify the rows by. The columns are still right.
        return Ok(rows);
    }

    let mut param_idx = 1usize;
    let mut values: Vec<serde_json::Value> = Vec::new();
    let mut alias_counter = 0usize;
    let embeds = build_embed_expressions(
        cache,
        relationships,
        type_name,
        "src",
        returning,
        max_rows,
        &mut alias_counter,
        &mut param_idx,
        &mut values,
        names,
    )?;
    let computed = computed_projections(table, returning, "src", names);
    if embeds.is_empty() && computed.is_empty() {
        return Ok(rows);
    }

    // One row per key written, in the order they were written.
    let mut conditions = Vec::with_capacity(rows.len());
    for row in &rows {
        let mut parts = Vec::with_capacity(table.pk_cols.len());
        for column in &table.pk_cols {
            let Value::Object(map) = row else {
                return Ok(rows);
            };
            let Some(value) = map.get(&async_graphql::Name::new(column)) else {
                return Ok(rows);
            };
            parts.push(format!(
                "{} = ${}",
                postrust_sql::escape_ident(column),
                param_idx
            ));
            values.push(value_to_json(value));
            param_idx += 1;
        }
        conditions.push(format!("({})", parts.join(" AND ")));
    }
    if conditions.is_empty() {
        return Ok(rows);
    }

    let mut projection = String::from("src.*");
    for expression in &computed {
        projection.push_str(", ");
        projection.push_str(expression);
    }
    for (name, expression) in &embeds {
        projection.push_str(", ");
        projection.push_str(expression);
        projection.push_str(" AS ");
        projection.push_str(&postrust_sql::escape_ident(name));
    }

    let sql = format!(
        "SELECT row_to_json(pgrst_r) FROM (SELECT {} FROM (SELECT * FROM {}.{} WHERE {}) AS src) AS pgrst_r",
        projection,
        postrust_sql::escape_ident(schema_name),
        postrust_sql::escape_ident(table_name),
        conditions.join(" OR ")
    );

    let mut conn = begin_with_session(pool, gql_ctx.role(), &gql_ctx.session_settings()).await?;
    let reread = execute_query_on(&mut conn, &sql, &values).await?;
    conn.commit().await?;

    if reread.is_empty() {
        return Ok(rows);
    }
    Ok(reread.into_iter().map(json_to_value).collect())
}

/// Execute an insert mutation.
async fn execute_insert(
    pool: &PgPool,
    schema_name: &str,
    table_name: &str,
    role: &str,
    objects: serde_json::Value,
    context: &InsertContext<'_>,
) -> Result<(Vec<Value>, usize), async_graphql::Error> {
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

    let mut inserted: Vec<Value> = Vec::new();
    let mut written = 0usize;

    for object in objects_array {
        let serde_json::Value::Object(map) = object else {
            return Err(async_graphql::Error::new("each object to insert is an object"));
        };
        let row = insert_row(&mut conn, schema_name, table_name, map, context, &mut written).await?;
        // A row `DO NOTHING` left alone is not in `returning` and is not in
        // `affected_rows` either: nothing was written, and the row that was
        // already there is not this mutation's to report.
        if !row.is_null() {
            inserted.push(json_to_value(row));
        }
    }

    // Commit once every object has been written, nested rows included:
    // committing inside the loop would end the transaction, and the role set
    // on it, after the first row -- and would leave a half-written parent
    // behind when a child fails.
    conn.commit().await?;

    Ok((inserted, written))
}

/// Shape a mutation's result for the field that asked for it.
///
/// A by-key mutation answers with the row, or with null where the key matched
/// nothing. Every other mutation answers with the table's mutation response,
/// which carries `affected_rows` alongside the rows themselves -- a client may
/// ask for no rows back and still need the count, which is the usual case for
/// a delete.
///
/// The count is passed in rather than taken from the rows, because a nested
/// insert writes more rows than it returns: two articles each carrying a new
/// author is four rows written and two returned, and four is the answer.
fn mutation_result<'a>(rows: Vec<Value>, affected: usize, by_key: bool) -> Option<FieldValue<'a>> {
    if by_key {
        return rows.into_iter().next().map(FieldValue::value);
    }
    let mut response = async_graphql::indexmap::IndexMap::new();
    response.insert(
        async_graphql::Name::new("affected_rows"),
        Value::from(affected),
    );
    response.insert(async_graphql::Name::new("returning"), Value::List(rows));
    Some(FieldValue::value(Value::Object(response)))
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
async fn execute_update(
    pool: &PgPool,
    schema_name: &str,
    table_name: &str,
    role: &str,
    operators: Vec<(&'static str, serde_json::Value)>,
    column_types: HashMap<String, String>,
    where_clause: Option<serde_json::Value>,
) -> Result<Vec<Value>, async_graphql::Error> {
    use sqlx::Row;

    trace!("Update mutation for {}: {:?}", table_name, operators);

    let mut conn = begin_with_role(pool, role).await?;

    // Build the SET clause. Each operator writes a column in terms of itself
    // except `_set`, which replaces it -- `_inc` adds, the jsonb operators
    // concatenate or remove, and a client may send several in one mutation as
    // long as they name different columns.
    let mut set_parts: Vec<String> = Vec::new();
    let mut set_values: Vec<serde_json::Value> = Vec::new();
    let mut param_idx = 1;
    let mut written: HashSet<String> = HashSet::new();

    for (operator, payload) in &operators {
        let serde_json::Value::Object(map) = payload else {
            return Err(async_graphql::Error::new(format!(
                "\"{}\" takes an object mapping columns to values",
                operator
            )));
        };
        for (column, value) in map {
            if !written.insert(column.clone()) {
                return Err(async_graphql::Error::new(format!(
                    "\"{}\" is written twice in one update; a column may be \
                     changed by one operator at a time",
                    column
                )));
            }
            let quoted = postrust_sql::escape_ident(column);
            // `_set` writes the column, so its value is cast to the column's
            // type. The others already say what they need -- `::jsonb` for a
            // concatenation, `::text[]` for a path -- and casting twice would
            // be wrong for `_delete_elem`, whose operand is an integer rather
            // than a value of the column.
            let placeholder = if *operator == "_set" {
                format!("${}{}", param_idx, write_cast(&column_types, column))
            } else {
                format!("${}", param_idx)
            };
            // The assignment PostgreSQL needs, which for everything but `_set`
            // reads the column's current value.
            let assignment = match *operator {
                "_set" => format!("{} = {}", quoted, placeholder),
                "_inc" => format!("{} = {} + {}", quoted, quoted, placeholder),
                "_append" => format!("{} = {} || {}::jsonb", quoted, quoted, placeholder),
                "_prepend" => format!("{} = {}::jsonb || {}", quoted, placeholder, quoted),
                "_delete_key" => format!("{} = {} - {}::text", quoted, quoted, placeholder),
                "_delete_elem" => format!("{} = {} - {}::int", quoted, quoted, placeholder),
                "_delete_at_path" => {
                    format!("{} = {} #- {}::text[]", quoted, quoted, placeholder)
                }
                other => {
                    return Err(async_graphql::Error::new(format!(
                        "unsupported update operator \"{}\"",
                        other
                    )))
                }
            };
            set_parts.push(assignment);
            set_values.push(value.clone());
            param_idx += 1;
        }
    }

    if set_parts.is_empty() {
        return Err(async_graphql::Error::new(format!(
            "update on \"{}\" changes nothing; give it _set, _inc or one of \
             the jsonb operators",
            table_name
        )));
    }

    // Build WHERE clause
    let scope = WhereScope::table(schema_name, table_name, table_name);
    let (where_sql, where_values) =
        build_where_clause(where_clause.as_ref(), param_idx, &scope)?;

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
    for val in &set_values {
        query = bind_json_value(query, val);
    }

    // Bind WHERE values
    for val in &where_values {
        query = bind_json_value(query, val);
    }

    let rows = query.fetch_all(&mut *conn).await?;

    let updated: Vec<Value> = rows
        .iter()
        .filter_map(|row| row.try_get::<serde_json::Value, _>(0).ok())
        .map(json_to_value)
        .collect();

    conn.commit().await?;

    Ok(updated)
}

/// Execute a delete mutation.
async fn execute_delete(
    pool: &PgPool,
    schema_name: &str,
    table_name: &str,
    role: &str,
    where_clause: Option<serde_json::Value>,
) -> Result<Vec<Value>, async_graphql::Error> {
    use sqlx::Row;

    trace!("Delete mutation for {}", table_name);

    let mut conn = begin_with_role(pool, role).await?;

    // Build WHERE clause
    let scope = WhereScope::table(schema_name, table_name, table_name);
    let (where_sql, where_values) = build_where_clause(where_clause.as_ref(), 1, &scope)?;

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

    let deleted: Vec<Value> = rows
        .iter()
        .filter_map(|row| row.try_get::<serde_json::Value, _>(0).ok())
        .map(json_to_value)
        .collect();

    // Return based on mutation type
    conn.commit().await?;

    Ok(deleted)
}

/// Build a WHERE clause from a boolean expression.
///
/// The expression is the JSON form of a `<table>_bool_exp`: column names
/// mapping to comparisons, and `_and`, `_or` and `_not` for structure. It
/// nests, so this recurses.
///
/// Nothing is dropped silently. An operator that is not recognised widens the
/// result set if it is ignored -- every row for a query, every row for a
/// mutation -- so it is an error instead.
fn build_where_clause(
    where_value: Option<&serde_json::Value>,
    start_param_idx: usize,
    scope: &WhereScope<'_>,
) -> Result<(String, Vec<serde_json::Value>), async_graphql::Error> {
    let mut values: Vec<serde_json::Value> = Vec::new();
    let mut param_idx = start_param_idx;
    let mut alias_counter = 0usize;

    let condition = match where_value {
        Some(value) => build_condition(value, scope, &mut param_idx, &mut values, &mut alias_counter)?,
        None => None,
    };

    Ok(match condition {
        Some(sql) => (format!("WHERE {}", sql), values),
        None => (String::new(), values),
    })
}

/// The table a boolean expression is being read against.
///
/// Column references are qualified with `sql_ref` so that a predicate over a
/// relationship -- which becomes a correlated `EXISTS` against another table --
/// can tell the two apart. Without the qualification a child column sharing a
/// parent column's name would silently resolve to whichever PostgreSQL
/// preferred.
///
/// `resolution` is absent where a caller has no schema cache to hand, which is
/// how the mutation paths are called. A relationship predicate then reports
/// that it cannot be resolved rather than being read as a column.
pub struct WhereScope<'a> {
    /// The table itself, for the comparisons that need a column's type.
    qualified: postrust_core::api_request::QualifiedIdentifier,
    /// How to refer to this table's columns in SQL.
    sql_ref: String,
    /// How to refer to this table's *row*, which is what a function taking the
    /// row is passed. A qualified name reads as a column reference there, so
    /// this is the bare alias.
    row_ref: String,
    /// The GraphQL type name, which is the key into the relationship map.
    type_name: String,
    resolution: Option<WhereResolution<'a>>,
}

/// What a scope needs to follow a relationship.
struct WhereResolution<'a> {
    cache: &'a SchemaCache,
    relationships: &'a HashMap<String, Vec<RelationshipField>>,
}

impl<'a> WhereScope<'a> {
    /// A scope over a table addressed by its qualified name.
    pub fn table(schema: &str, table: &str, type_name: &str) -> Self {
        Self {
            qualified: postrust_core::api_request::QualifiedIdentifier::new(schema, table),
            sql_ref: format!(
                "{}.{}",
                postrust_sql::escape_ident(schema),
                postrust_sql::escape_ident(table)
            ),
            row_ref: postrust_sql::escape_ident(table),
            type_name: type_name.to_string(),
            resolution: None,
        }
    }

    /// The same, able to follow relationships.
    fn with_resolution(
        mut self,
        cache: &'a SchemaCache,
        relationships: &'a HashMap<String, Vec<RelationshipField>>,
    ) -> Self {
        self.resolution = Some(WhereResolution {
            cache,
            relationships,
        });
        self
    }

    /// A scope over an aliased table, able to follow relationships.
    fn for_alias(
        alias: &str,
        type_name: &str,
        cache: &'a SchemaCache,
        relationships: &'a HashMap<String, Vec<RelationshipField>>,
    ) -> Self {
        Self {
            qualified: postrust_core::api_request::QualifiedIdentifier::new("", ""),
            sql_ref: postrust_sql::escape_ident(alias),
            row_ref: postrust_sql::escape_ident(alias),
            type_name: type_name.to_string(),
            resolution: Some(WhereResolution {
                cache,
                relationships,
            }),
        }
    }

    /// A scope over an aliased table, for the inside of an `EXISTS`.
    fn aliased(alias: &str, type_name: &str, from: &WhereScope<'a>) -> Self {
        Self {
            qualified: from.qualified.clone(),
            sql_ref: postrust_sql::escape_ident(alias),
            row_ref: postrust_sql::escape_ident(alias),
            type_name: type_name.to_string(),
            resolution: from.resolution.as_ref().map(|r| WhereResolution {
                cache: r.cache,
                relationships: r.relationships,
            }),
        }
    }

    fn column(&self, name: &str) -> String {
        format!("{}.{}", self.sql_ref, postrust_sql::escape_ident(name))
    }

    /// The PostgreSQL type of one of this table's columns, where it can be
    /// found. A spatial comparison needs it: the same function takes a
    /// geometry or a geography and the operand has to be cast to match.
    fn column_type(&self, name: &str) -> Option<String> {
        let cache = self.resolution.as_ref()?.cache;
        let table = cache.get_table(&self.qualified)?;
        table.get_column(name).map(|c| c.nominal_type.clone())
    }

    fn relationship(&self, name: &str) -> Option<&'a RelationshipField> {
        self.resolution
            .as_ref()?
            .relationships
            .get(&self.type_name)?
            .iter()
            .find(|r| r.name == name)
    }
}

/// One boolean expression, as SQL. `None` where it constrains nothing.
fn build_condition(
    value: &serde_json::Value,
    scope: &WhereScope<'_>,
    param_idx: &mut usize,
    values: &mut Vec<serde_json::Value>,
    alias_counter: &mut usize,
) -> Result<Option<String>, async_graphql::Error> {
    let serde_json::Value::Object(map) = value else {
        return Ok(None);
    };

    let mut conditions: Vec<String> = Vec::new();

    for (key, val) in map {
        match key.as_str() {
            // An empty `_and` is true and an empty `_or` is false, which is
            // what SQL's own identities give: nothing to AND constrains
            // nothing, nothing to OR matches nothing.
            "_and" | "_or" => {
                let members = val.as_array().ok_or_else(|| {
                    async_graphql::Error::new(format!("\"{}\" takes a list of expressions", key))
                })?;
                let mut parts = Vec::new();
                for member in members {
                    if let Some(sql) =
                        build_condition(member, scope, param_idx, values, alias_counter)?
                    {
                        parts.push(sql);
                    }
                }
                if parts.is_empty() {
                    if key == "_or" {
                        conditions.push("false".to_string());
                    }
                    continue;
                }
                let joiner = if key == "_and" { " AND " } else { " OR " };
                conditions.push(format!("({})", parts.join(joiner)));
            }
            "_not" => {
                if let Some(sql) = build_condition(val, scope, param_idx, values, alias_counter)? {
                    conditions.push(format!("NOT ({})", sql));
                }
            }
            // A relationship is filtered by filtering the rows at its other
            // end. `where: {articles: {title: {_eq: "x"}}}` keeps the authors
            // that have such an article -- an `EXISTS` correlated back to the
            // parent row, which is also what it means for a to-one
            // relationship, where the subquery simply cannot match twice.
            name if scope.relationship(name).is_some() => {
                let relationship = scope.relationship(name).expect("just checked");
                conditions.push(exists_sql(
                    relationship,
                    val,
                    scope,
                    param_idx,
                    values,
                    alias_counter,
                )?);
            }
            column => {
                let quoted = scope.column(column);
                match val {
                    serde_json::Value::Object(ops) => {
                        let column_type = scope.column_type(column);
                        for (op, operand) in ops {
                            conditions.push(comparison_sql(
                                &quoted,
                                column,
                                column_type.as_deref(),
                                op,
                                operand,
                                param_idx,
                                values,
                            )?);
                        }
                    }
                    // `{id: 1}` rather than `{id: {_eq: 1}}`. Not a spelling
                    // Hasura accepts, but one that costs nothing to read and
                    // that hand-written queries reach for.
                    other => {
                        conditions.push(format!("{} = ${}", quoted, param_idx));
                        values.push(other.clone());
                        *param_idx += 1;
                    }
                }
            }
        }
    }

    Ok(if conditions.is_empty() {
        None
    } else if conditions.len() == 1 {
        conditions.pop()
    } else {
        Some(conditions.join(" AND "))
    })
}

/// A predicate over a relationship, as a correlated `EXISTS`.
///
/// Only a plain key join is expressible this way. A many-to-many through a
/// junction and a computed relationship both need more than a pair of columns
/// to correlate on, and saying so is the only honest answer -- reading the
/// predicate as though it constrained nothing would return every parent row.
fn exists_sql(
    relationship: &RelationshipField,
    child_expression: &serde_json::Value,
    scope: &WhereScope<'_>,
    param_idx: &mut usize,
    values: &mut Vec<serde_json::Value>,
    alias_counter: &mut usize,
) -> Result<String, async_graphql::Error> {
    let cache = scope
        .resolution
        .as_ref()
        .map(|r| r.cache)
        .ok_or_else(|| {
            async_graphql::Error::new(format!(
                "filtering on the relationship \"{}\" is not available here",
                relationship.name
            ))
        })?;

    let plan = postrust_core::embed::EmbedPlan::resolve(&relationship.relationship, cache)
        .map_err(|e| async_graphql::Error::new(e.to_string()))?;

    if plan.junction.is_some() || (plan.function.is_none() && plan.columns.is_empty()) {
        return Err(async_graphql::Error::new(format!(
            "filtering on \"{}\" is not supported: it is reached through a junction \
             rather than by a key",
            relationship.name
        )));
    }

    *alias_counter += 1;
    let alias = format!("pgrst_rel_{}", alias_counter);
    let child_scope = WhereScope::aliased(&alias, &relationship.target_type, scope);

    // A computed relationship is correlated by argument -- the function takes
    // the parent row -- where a key relationship is correlated by columns.
    let (source, mut correlation) = match &plan.function {
        Some(function) => (
            format!(
                "{}.{}({})",
                postrust_sql::escape_ident(&function.schema),
                postrust_sql::escape_ident(&function.name),
                scope.row_ref
            ),
            Vec::new(),
        ),
        None => {
            let mut columns = Vec::with_capacity(plan.columns.len());
            for (parent_column, child_column) in &plan.columns {
                columns.push(format!(
                    "{} = {}",
                    scope.column(parent_column),
                    child_scope.column(child_column)
                ));
            }
            (
                format!(
                    "{}.{}",
                    postrust_sql::escape_ident(&plan.foreign_schema),
                    postrust_sql::escape_ident(&plan.foreign_table)
                ),
                columns,
            )
        }
    };

    let child_condition =
        build_condition(child_expression, &child_scope, param_idx, values, alias_counter)?;
    if let Some(sql) = child_condition {
        correlation.push(format!("({})", sql));
    }
    if correlation.is_empty() {
        correlation.push("true".to_string());
    }

    Ok(format!(
        "EXISTS (SELECT 1 FROM {} AS {} WHERE {})",
        source,
        postrust_sql::escape_ident(&alias),
        correlation.join(" AND ")
    ))
}

/// One spatial comparison, as a PostGIS call.
///
/// The operand arrives as GeoJSON, which is what Hasura accepts and what a
/// client sends, so it is parsed by `ST_GeomFromGeoJSON` rather than bound as
/// a shape. The cast that follows is the column's own type: the same function
/// takes a geometry or a geography and picking the wrong one is not an
/// overload PostGIS has.
#[allow(clippy::too_many_arguments)]
fn postgis_sql(
    quoted: &str,
    column_type: Option<&str>,
    function: &str,
    op: &str,
    operand: &serde_json::Value,
    param_idx: &mut usize,
    values: &mut Vec<serde_json::Value>,
) -> Result<String, async_graphql::Error> {
    let is_geography = column_type == Some("geography");
    let mut shape = |value: &serde_json::Value, param_idx: &mut usize| -> String {
        let placeholder = format!("${}", param_idx);
        *param_idx += 1;
        values.push(value.clone());
        match is_geography {
            true => format!("ST_GeomFromGeoJSON({})::geography", placeholder),
            false => format!("ST_GeomFromGeoJSON({})", placeholder),
        }
    };

    match op {
        // `{distance, from}`: the shape and how far from it.
        "_st_d_within" | "_st_3d_d_within" => {
            let serde_json::Value::Object(spec) = operand else {
                return Err(async_graphql::Error::new(format!(
                    "\"{}\" takes {{distance, from}}",
                    op
                )));
            };
            let Some(from) = spec.get("from") else {
                return Err(async_graphql::Error::new(format!(
                    "\"{}\" needs a shape to measure from",
                    op
                )));
            };
            let Some(distance) = spec.get("distance") else {
                return Err(async_graphql::Error::new(format!(
                    "\"{}\" needs a distance",
                    op
                )));
            };
            let from_sql = shape(from, param_idx);
            let distance_sql = format!("${}", param_idx);
            *param_idx += 1;
            values.push(distance.clone());
            Ok(format!(
                "{}({}, {}, {}::float8)",
                function, quoted, from_sql, distance_sql
            ))
        }
        // A raster against another raster, bound as one rather than parsed.
        "_st_intersects_rast" => {
            let placeholder = format!("${}", param_idx);
            *param_idx += 1;
            values.push(operand.clone());
            Ok(format!("{}({}, {}::raster)", function, quoted, placeholder))
        }
        // A raster against a shape, in one band or in any.
        "_st_intersects_geom_nband" | "_st_intersects_nband_geom" => {
            let serde_json::Value::Object(spec) = operand else {
                return Err(async_graphql::Error::new(format!(
                    "\"{}\" takes an object",
                    op
                )));
            };
            let Some(geometry) = spec.get("geommin") else {
                return Err(async_graphql::Error::new(format!(
                    "\"{}\" needs a shape",
                    op
                )));
            };
            let band = spec.get("nband").filter(|v| !v.is_null());
            let geometry_sql = shape(geometry, param_idx);
            Ok(match band {
                None => format!("{}({}, {})", function, quoted, geometry_sql),
                Some(band) => {
                    let placeholder = format!("${}", param_idx);
                    *param_idx += 1;
                    values.push(band.clone());
                    // `ST_Intersects(raster, nband, geometry)` is the spelling
                    // with a band; the argument order is PostGIS's, not the
                    // input's.
                    format!(
                        "{}({}, {}::int, {})",
                        function, quoted, placeholder, geometry_sql
                    )
                }
            })
        }
        // Everything else is the relation between this shape and one other.
        _ => {
            let other = shape(operand, param_idx);
            Ok(format!("{}({}, {})", function, quoted, other))
        }
    }
}

/// A list of values as an array PostgreSQL will read.
///
/// Bound one element at a time rather than as one parameter. A JSON array
/// arrives as the text `["a","b"]`, which is not an array literal -- PostgreSQL
/// answers `malformed array literal` and means it.
fn sql_array(
    items: &[serde_json::Value],
    cast: &str,
    param_idx: &mut usize,
    values: &mut Vec<serde_json::Value>,
) -> String {
    if items.is_empty() {
        return format!("ARRAY[]::{}", cast);
    }
    let placeholders: Vec<String> = items
        .iter()
        .map(|item| {
            let placeholder = format!("${}", param_idx);
            *param_idx += 1;
            values.push(item.clone());
            placeholder
        })
        .collect();
    format!("ARRAY[{}]::{}", placeholders.join(", "), cast)
}

/// One comparison against one column.
#[allow(clippy::too_many_arguments)]
fn comparison_sql(
    quoted: &str,
    column: &str,
    column_type: Option<&str>,
    op: &str,
    operand: &serde_json::Value,
    param_idx: &mut usize,
    values: &mut Vec<serde_json::Value>,
) -> Result<String, async_graphql::Error> {
    // `_cast` is not a comparison but a change of what is being compared: the
    // column becomes another type and the comparisons inside it apply to that.
    // A geometry and a geography answer different questions about the same
    // shape -- one on a plane, one on a sphere -- and this is how a client asks
    // for the other without the schema carrying two columns.
    if op == "_cast" {
        let serde_json::Value::Object(casts) = operand else {
            return Err(async_graphql::Error::new(
                "_cast takes an object naming the type to compare as",
            ));
        };
        let mut conditions = Vec::new();
        for (target, comparisons) in casts {
            if !matches!(target.as_str(), "geometry" | "geography") {
                return Err(async_graphql::Error::new(format!(
                    "cannot compare \"{}\" as \"{}\"",
                    column, target
                )));
            }
            let serde_json::Value::Object(ops) = comparisons else {
                continue;
            };
            let cast = format!("{}::{}", quoted, target);
            for (nested_op, nested_operand) in ops {
                conditions.push(comparison_sql(
                    &cast,
                    column,
                    Some(target),
                    nested_op,
                    nested_operand,
                    param_idx,
                    values,
                )?);
            }
        }
        let mut conditions = conditions;
        return Ok(match conditions.len() {
            0 => "true".to_string(),
            1 => conditions.pop().expect("just counted"),
            _ => format!("({})", conditions.join(" AND ")),
        });
    }

    // A spatial relation is a function of two shapes rather than an operator
    // between them, so it is written before the operator table is consulted.
    if let Some(function) = crate::input::bool_exp::postgis_function(op) {
        return postgis_sql(quoted, column_type, function, op, operand, param_idx, values);
    }

    // A tree comparison is an operator, but one whose operand has to be cast:
    // `?` is "any of these labels" for an ltree and "has this key" for a
    // jsonb, and PostgreSQL tells them apart by the operand's type alone.
    if let Some((operator, cast)) = crate::input::bool_exp::ltree_operator(op) {
        if cast.ends_with("[]") {
            let items = operand.as_array().ok_or_else(|| {
                async_graphql::Error::new(format!(
                    "the \"{}\" comparison on \"{}\" takes a list",
                    op, column
                ))
            })?;
            return Ok(format!(
                "{} {} {}",
                quoted,
                operator,
                sql_array(items, cast, param_idx, values)
            ));
        }
        let placeholder = format!("${}", param_idx);
        *param_idx += 1;
        values.push(operand.clone());
        return Ok(format!("{} {} {}::{}", quoted, operator, placeholder, cast));
    }

    // Comparisons binding exactly one parameter, by the SQL they become.
    let binary = match op {
        "_eq" => Some("="),
        "_neq" => Some("<>"),
        "_gt" => Some(">"),
        "_gte" => Some(">="),
        "_lt" => Some("<"),
        "_lte" => Some("<="),
        "_like" => Some("LIKE"),
        "_nlike" => Some("NOT LIKE"),
        "_ilike" => Some("ILIKE"),
        "_nilike" => Some("NOT ILIKE"),
        "_similar" => Some("SIMILAR TO"),
        "_nsimilar" => Some("NOT SIMILAR TO"),
        "_regex" => Some("~"),
        "_nregex" => Some("!~"),
        "_iregex" => Some("~*"),
        "_niregex" => Some("!~*"),
        "_contains" => Some("@>"),
        "_contained_in" => Some("<@"),
        "_has_key" => Some("?"),
        _ => None,
    };

    if let Some(sql_op) = binary {
        let placeholder = format!("${}", param_idx);
        *param_idx += 1;
        values.push(operand.clone());
        // A bound parameter arrives as text, and PostgreSQL will infer a type
        // for it from the operator only where one is unambiguous. `@>` is
        // defined over several pairs of types, so `jsonb @> text` is not an
        // operator at all and the comparison fails outright. The containment
        // and key operators say what they were given.
        let cast = match op {
            "_contains" | "_contained_in" => "::jsonb",
            "_has_key" => "::text",
            _ => "",
        };
        return Ok(format!("{} {} {}{}", quoted, sql_op, placeholder, cast));
    }

    match op {
        "_is_null" => Ok(if operand.as_bool().unwrap_or(false) {
            format!("{} IS NULL", quoted)
        } else {
            format!("{} IS NOT NULL", quoted)
        }),
        "_in" | "_nin" => {
            let items = operand.as_array().ok_or_else(|| {
                async_graphql::Error::new(format!(
                    "the \"{}\" comparison on \"{}\" requires a list of values",
                    op, column
                ))
            })?;
            if items.is_empty() {
                // `IN ()` is not valid SQL. An empty `_in` matches nothing and
                // an empty `_nin` matches everything.
                return Ok(if op == "_in" { "false" } else { "true" }.to_string());
            }
            let mut placeholders = Vec::with_capacity(items.len());
            for item in items {
                placeholders.push(format!("${}", param_idx));
                values.push(item.clone());
                *param_idx += 1;
            }
            Ok(format!(
                "{} {} ({})",
                quoted,
                if op == "_in" { "IN" } else { "NOT IN" },
                placeholders.join(", ")
            ))
        }
        "_has_keys_any" | "_has_keys_all" => {
            let items = operand.as_array().ok_or_else(|| {
                async_graphql::Error::new(format!(
                    "the \"{}\" comparison on \"{}\" requires a list of keys",
                    op, column
                ))
            })?;
            Ok(format!(
                "{} {} {}",
                quoted,
                if op == "_has_keys_any" { "?|" } else { "?&" },
                sql_array(items, "text[]", param_idx, values)
            ))
        }
        other => Err(async_graphql::Error::new(format!(
            "unsupported comparison \"{}\" on \"{}\"",
            other, column
        ))),
    }
}

/// Build an `ORDER BY` clause from the `order_by` argument.
///
/// The argument is a list of single-column objects -- `[{name: asc}, {id:
/// desc}]` -- and the list is ordered, which is why it is a list and not one
/// object with several keys. Column names are checked against the table in the
/// schema cache before being quoted, so a name that is unknown, or crafted to
/// inject SQL, is rejected rather than interpolated. Returns an empty string
/// when no ordering was requested.
async fn build_order_by_clause(
    ctx: &ResolverContext<'_>,
    schema_cache: &postrust_core::schema_cache::SchemaCacheRef,
    schema_name: &str,
    table_name: &str,
    type_name: &str,
    relationships: &HashMap<String, Vec<RelationshipField>>,
) -> Result<String, async_graphql::Error> {
    let Ok(order_arg) = ctx.args.try_get("order_by") else {
        return Ok(String::new());
    };

    let value = accessor_to_json(&order_arg);
    // A single object is accepted as a one-entry list, which is what a client
    // writing `order_by: {name: asc}` means and what Hasura reads it as.
    let entries: Vec<&serde_json::Value> = match &value {
        serde_json::Value::Array(items) => items.iter().collect(),
        serde_json::Value::Object(_) => vec![&value],
        serde_json::Value::Null => return Ok(String::new()),
        _ => {
            return Err(async_graphql::Error::new(
                "order_by takes an object or a list of them",
            ))
        }
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

    let mut terms = Vec::new();
    let mut alias_counter = 0usize;
    let reference = format!(
        "{}.{}",
        postrust_sql::escape_ident(schema_name),
        postrust_sql::escape_ident(table_name)
    );
    for entry in entries {
        order_terms_into(
            entry,
            cache,
            relationships,
            type_name,
            table,
            &reference,
            &mut alias_counter,
            &mut terms,
        )?;
    }

    if terms.is_empty() {
        return Ok(String::new());
    }
    Ok(format!(" ORDER BY {}", terms.join(", ")))
}

/// Collect the `ORDER BY` terms one entry contributes.
///
/// A key whose value is a direction is a column of this table. A key whose
/// value is an object is something the row points at, and ordering by it is a
/// correlated subselect: one related row contributes its column, and many
/// contribute an aggregate. That is the whole difference between the two
/// sides, and PostgreSQL will take a scalar subquery anywhere a column goes.
#[allow(clippy::too_many_arguments)]
fn order_terms_into(
    entry: &serde_json::Value,
    cache: &SchemaCache,
    relationships: &HashMap<String, Vec<RelationshipField>>,
    type_name: &str,
    table: &postrust_core::schema_cache::Table,
    reference: &str,
    alias_counter: &mut usize,
    terms: &mut Vec<String>,
) -> Result<(), async_graphql::Error> {
    let serde_json::Value::Object(map) = entry else {
        return Err(async_graphql::Error::new(
            "each order_by entry is an object mapping a column to a direction",
        ));
    };
    let available: &[RelationshipField] = relationships
        .get(type_name)
        .map(|r| r.as_slice())
        .unwrap_or(&[]);

    for (key, value) in map {
        // A direction: this table's own column.
        if let Some(name) = value.as_str() {
            if table.get_column(key).is_none() {
                return Err(async_graphql::Error::new(format!(
                    "cannot order by unknown column \"{}\" on \"{}\"",
                    key, table.name
                )));
            }
            let sql = crate::input::order_by::direction_sql(name).ok_or_else(|| {
                async_graphql::Error::new(format!(
                    "\"{}\" is not a sort direction; expected one of asc, desc, \
                     asc_nulls_first, asc_nulls_last, desc_nulls_first, desc_nulls_last",
                    name
                ))
            })?;
            terms.push(format!("{}.{} {}", reference, postrust_sql::escape_ident(key), sql));
            continue;
        }

        // An aggregate of the rows that point here.
        if let Some(rel) = key
            .strip_suffix("_aggregate")
            .and_then(|name| available.iter().find(|r| r.name == name && r.is_list))
        {
            aggregate_order_terms(value, rel, cache, reference, alias_counter, terms)?;
            continue;
        }

        // A column of the row this one points at.
        if let Some(rel) = available.iter().find(|r| r.name == *key && !r.is_list) {
            let plan = postrust_core::embed::EmbedPlan::resolve(&rel.relationship, cache)
                .map_err(|e| async_graphql::Error::new(e.to_string()))?;
            if plan.columns.is_empty() {
                return Err(async_graphql::Error::new(format!(
                    "cannot order by \"{}\": it is not reached by a key",
                    key
                )));
            }
            *alias_counter += 1;
            let alias = format!("pgrst_ord_{}", alias_counter);
            let quoted_alias = postrust_sql::escape_ident(&alias);
            let correlation = plan
                .columns
                .iter()
                .map(|(local, foreign)| {
                    format!(
                        "{} = {}.{}",
                        format_args!("{}.{}", reference, postrust_sql::escape_ident(local)),
                        quoted_alias,
                        postrust_sql::escape_ident(foreign)
                    )
                })
                .collect::<Vec<_>>()
                .join(" AND ");

            let target_qi = postrust_core::api_request::QualifiedIdentifier::new(
                &plan.foreign_schema,
                &plan.foreign_table,
            );
            let Some(target) = cache.get_table(&target_qi) else {
                return Err(async_graphql::Error::new(format!(
                    "unknown table \"{}\"",
                    plan.foreign_table
                )));
            };

            // The related row's own terms, then wrapped one at a time: a
            // subquery yields one value, so each term is its own subselect.
            let mut nested = Vec::new();
            order_terms_into(
                value,
                cache,
                relationships,
                &rel.target_type,
                target,
                &quoted_alias,
                alias_counter,
                &mut nested,
            )?;
            for term in nested {
                let (expression, direction) = term
                    .rsplit_once(' ')
                    .map(|(e, d)| (e.to_string(), d.to_string()))
                    .unwrap_or((term.clone(), String::new()));
                terms.push(format!(
                    "(SELECT {} FROM {}.{} AS {} WHERE {}) {}",
                    expression,
                    postrust_sql::escape_ident(&plan.foreign_schema),
                    postrust_sql::escape_ident(&plan.foreign_table),
                    quoted_alias,
                    correlation,
                    direction
                ));
            }
            continue;
        }

        return Err(async_graphql::Error::new(format!(
            "cannot order by \"{}\" on \"{}\"",
            key, table.name
        )));
    }
    Ok(())
}

/// The `ORDER BY` terms for an aggregate of a row's children.
fn aggregate_order_terms(
    value: &serde_json::Value,
    rel: &RelationshipField,
    cache: &SchemaCache,
    reference: &str,
    alias_counter: &mut usize,
    terms: &mut Vec<String>,
) -> Result<(), async_graphql::Error> {
    let serde_json::Value::Object(spec) = value else {
        return Err(async_graphql::Error::new(
            "ordering by an aggregate takes an object, such as {count: desc}",
        ));
    };
    let plan = postrust_core::embed::EmbedPlan::resolve(&rel.relationship, cache)
        .map_err(|e| async_graphql::Error::new(e.to_string()))?;
    if plan.columns.is_empty() {
        return Err(async_graphql::Error::new(format!(
            "cannot order by an aggregate of \"{}\": it is not reached by a key",
            rel.name
        )));
    }

    *alias_counter += 1;
    let alias = postrust_sql::escape_ident(&format!("pgrst_ord_{}", alias_counter));
    let correlation = plan
        .columns
        .iter()
        .map(|(local, foreign)| {
            format!(
                "{}.{} = {}.{}",
                reference,
                postrust_sql::escape_ident(local),
                alias,
                postrust_sql::escape_ident(foreign)
            )
        })
        .collect::<Vec<_>>()
        .join(" AND ");

    // `{count: desc}` is one term; `{max: {id: desc}}` is one per column.
    let mut wanted: Vec<(String, String)> = Vec::new();
    for (function, argument) in spec {
        if function == "count" {
            let name = argument.as_str().unwrap_or_default();
            let Some(direction) = crate::input::order_by::direction_sql(name) else {
                return Err(async_graphql::Error::new(format!(
                    "\"{}\" is not a sort direction",
                    name
                )));
            };
            wanted.push(("count(*)".to_string(), direction.to_string()));
            continue;
        }
        let serde_json::Value::Object(columns) = argument else {
            continue;
        };
        for (column, direction) in columns {
            let name = direction.as_str().unwrap_or_default();
            let Some(direction) = crate::input::order_by::direction_sql(name) else {
                return Err(async_graphql::Error::new(format!(
                    "\"{}\" is not a sort direction",
                    name
                )));
            };
            // The function comes from the generated input, not from the
            // request: a client can only name one that was offered.
            wanted.push((
                format!(
                    "{}({}.{})",
                    function,
                    alias,
                    postrust_sql::escape_ident(column)
                ),
                direction.to_string(),
            ));
        }
    }

    for (expression, direction) in wanted {
        terms.push(format!(
            "(SELECT {} FROM {}.{} AS {} WHERE {}) {}",
            expression,
            postrust_sql::escape_ident(&plan.foreign_schema),
            postrust_sql::escape_ident(&plan.foreign_table),
            alias,
            correlation,
            direction
        ));
    }
    Ok(())
}

/// Build the `DISTINCT ON (...)` prefix from the `distinct_on` argument.
///
/// PostgreSQL requires the leftmost `ORDER BY` terms to match the distinct
/// columns, and picks an arbitrary surviving row otherwise. The caller
/// prepends these terms to whatever ordering was asked for rather than
/// replacing it, so `distinct_on: [name]` with `order_by: [{id: desc}]` keeps
/// the highest id per name instead of an unpredictable one.
async fn build_distinct_on(
    ctx: &ResolverContext<'_>,
    schema_cache: &postrust_core::schema_cache::SchemaCacheRef,
    schema_name: &str,
    table_name: &str,
) -> Result<Vec<String>, async_graphql::Error> {
    let Ok(arg) = ctx.args.try_get("distinct_on") else {
        return Ok(Vec::new());
    };
    let value = accessor_to_json(&arg);
    let names: Vec<String> = match &value {
        serde_json::Value::Array(items) => items
            .iter()
            .filter_map(|i| i.as_str().map(|s| s.to_string()))
            .collect(),
        serde_json::Value::String(one) => vec![one.clone()],
        _ => Vec::new(),
    };
    if names.is_empty() {
        return Ok(Vec::new());
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

    let mut quoted = Vec::with_capacity(names.len());
    for name in names {
        if table.get_column(&name).is_none() {
            return Err(async_graphql::Error::new(format!(
                "cannot take distinct on unknown column \"{}\" of \"{}\"",
                name, table_name
            )));
        }
        quoted.push(postrust_sql::escape_ident(&name));
    }
    Ok(quoted)
}

/// Build the SELECT-list expressions that embed relationships in one query.
///
/// The GraphQL mirror of the REST builder: each requested relationship becomes a
/// correlated subselect yielding JSON, so the whole selection comes back from
/// the parent query instead of one query per relationship per level.
/// The SELECT-list entries for any computed columns the selection asks for.
///
/// A computed column is a function of one argument of the table's own row
/// type, so it is not in `*` and has to be named. PostgreSQL's functional
/// notation reads `upper_name(author.*)` and `author.upper_name` as the same
/// call; the explicit form is written here because it says which function is
/// being called.
fn computed_projections(
    table: &postrust_core::schema_cache::Table,
    selection: async_graphql::SelectionField<'_>,
    row_reference: &str,
    names: &crate::names::NameOverrides,
) -> Vec<String> {
    let mut projections = Vec::new();
    for field in selection.selection_set() {
        let name = field.name();
        // A real column wins, and is already in `*`.
        if table.get_column(name).is_some() {
            continue;
        }
        // The field may be exposed under a name that was given rather than
        // the function's own, so the call is looked up from either side.
        let function = names
            .computed_source(&table.schema, &table.name, name)
            .unwrap_or(name);
        let Some(computed) = table.get_computed_column(function) else {
            continue;
        };
        projections.push(format!(
            "{}.{}({}) AS {}",
            postrust_sql::escape_ident(&computed.function.schema),
            postrust_sql::escape_ident(&computed.function.name),
            row_reference,
            postrust_sql::escape_ident(name)
        ));
    }
    projections
}

/// The SELECT list for a nested aggregate.
///
/// `articles_aggregate { aggregate { count } nodes { title } }` becomes one
/// row per parent, correlated the way any embed is. Both halves are aggregates
/// over the same correlated set, so they go in one select list rather than two
/// queries -- `count(*)` and `json_agg(...)` read the same rows.
fn nested_aggregate_select(
    selection: async_graphql::SelectionField<'_>,
    child_alias: &str,
) -> String {
    use crate::schema::aggregate as agg;

    let mut parts: Vec<String> = Vec::new();

    for child in selection.selection_set() {
        match child.name() {
            "aggregate" => {
                let mut fields = vec!["'count', count(*)".to_string()];
                for function in child.selection_set() {
                    if function.name() == "count" {
                        continue;
                    }
                    let sql_function = agg::NUMERIC_AGGREGATES
                        .iter()
                        .chain(agg::ORDERED_AGGREGATES.iter())
                        .find(|(name, _)| *name == function.name())
                        .map(|(name, _)| *name);
                    let Some(sql_function) = sql_function else {
                        continue;
                    };
                    let columns: Vec<String> = function
                        .selection_set()
                        .map(|column| {
                            format!(
                                "'{}', {}({}.{})",
                                column.name().replace('\'', "''"),
                                sql_function,
                                postrust_sql::escape_ident(child_alias),
                                postrust_sql::escape_ident(column.name())
                            )
                        })
                        .collect();
                    if !columns.is_empty() {
                        fields.push(format!(
                            "'{}', json_build_object({})",
                            sql_function,
                            columns.join(", ")
                        ));
                    }
                }
                parts.push(format!(
                    "json_build_object({}) AS {}",
                    fields.join(", "),
                    postrust_sql::escape_ident("aggregate")
                ));
            }
            "nodes" => {
                let columns: Vec<String> = child
                    .selection_set()
                    .map(|column| {
                        format!(
                            "'{}', {}.{}",
                            column.name().replace('\'', "''"),
                            postrust_sql::escape_ident(child_alias),
                            postrust_sql::escape_ident(column.name())
                        )
                    })
                    .collect();
                let row = if columns.is_empty() {
                    format!("row_to_json({})", postrust_sql::escape_ident(child_alias))
                } else {
                    format!("json_build_object({})", columns.join(", "))
                };
                parts.push(format!(
                    "COALESCE(json_agg({}), '[]'::json) AS {}",
                    row,
                    postrust_sql::escape_ident("nodes")
                ));
            }
            _ => {}
        }
    }

    if parts.is_empty() {
        // A selection of nothing but `__typename`. One row, no columns, is not
        // valid SQL; a count nobody reads is.
        parts.push("count(*) AS pgrst_empty".to_string());
    }
    parts.join(", ")
}

/// Render `order_by` terms against a table, qualified with an alias.
///
/// The root field's ordering reads its argument from the resolver context and
/// needs the schema cache asynchronously; an embed already holds both, so this
/// takes the value directly. Columns are checked against the table before they
/// are quoted, so an unknown or crafted name is refused rather than
/// interpolated.
fn order_terms(
    order: &serde_json::Value,
    schema_cache: &SchemaCache,
    schema_name: &str,
    table_name: &str,
    alias: &str,
) -> Result<Option<String>, async_graphql::Error> {
    let entries: Vec<&serde_json::Value> = match order {
        serde_json::Value::Array(items) => items.iter().collect(),
        serde_json::Value::Object(_) => vec![order],
        _ => return Ok(None),
    };

    let qi = postrust_core::api_request::QualifiedIdentifier::new(schema_name, table_name);
    let table = schema_cache
        .get_table(&qi)
        .ok_or_else(|| async_graphql::Error::new(format!("unknown table \"{}\"", table_name)))?;

    let mut terms = Vec::new();
    for entry in entries {
        let serde_json::Value::Object(map) = entry else {
            continue;
        };
        for (column, direction) in map {
            if table.get_column(column).is_none() {
                return Err(async_graphql::Error::new(format!(
                    "cannot order by unknown column \"{}\" on \"{}\"",
                    column, table_name
                )));
            }
            let name = direction.as_str().unwrap_or_default();
            let sql = crate::input::order_by::direction_sql(name).ok_or_else(|| {
                async_graphql::Error::new(format!("\"{}\" is not a sort direction", name))
            })?;
            terms.push(format!(
                "{}.{} {}",
                postrust_sql::escape_ident(alias),
                postrust_sql::escape_ident(column),
                sql
            ));
        }
    }

    Ok(if terms.is_empty() {
        None
    } else {
        Some(terms.join(", "))
    })
}

fn build_embed_expressions(
    schema_cache: &SchemaCache,
    relationships: &HashMap<String, Vec<RelationshipField>>,
    type_name: &str,
    parent_alias: &str,
    selection: async_graphql::SelectionField<'_>,
    max_rows: Option<i64>,
    alias_counter: &mut usize,
    param_idx: &mut usize,
    values: &mut Vec<serde_json::Value>,
    names: &crate::names::NameOverrides,
) -> Result<Vec<(String, String)>, async_graphql::Error> {
    let Some(available) = relationships.get(type_name) else {
        return Ok(Vec::new());
    };

    let mut expressions = Vec::new();

    for field in selection.selection_set() {
        // `<relationship>_aggregate` is the same embed with an aggregate
        // select list, so it is resolved here rather than by a resolver of its
        // own -- the correlation is what makes it a per-parent answer.
        let aggregate_of = field
            .name()
            .strip_suffix("_aggregate")
            .and_then(|name| available.iter().find(|r| r.name == name && r.is_list));

        if let Some(rel) = aggregate_of {
            let plan = postrust_core::embed::EmbedPlan::resolve(&rel.relationship, schema_cache)
                .map_err(|e| async_graphql::Error::new(e.to_string()))?;
            *alias_counter += 1;
            let child_alias = format!("a{}", alias_counter);

            // One row per parent rather than a list of them: the aggregate is
            // the answer, not the rows it read.
            let mut one_row = plan.clone();
            one_row.is_list = false;

            let expression = one_row
                .embed_expression(
                    parent_alias,
                    &postrust_sql::escape_ident(parent_alias),
                    &child_alias,
                    &nested_aggregate_select(field, &child_alias),
                    None,
                    0,
                    None,
                    None,
                )
                .map_err(|e| async_graphql::Error::new(e.to_string()))?;
            expressions.push((field.name().to_string(), expression));
            continue;
        }

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
            param_idx,
            values,
            names,
        )?;

        // The arguments written on the embed itself.
        let arguments: HashMap<String, serde_json::Value> = field
            .arguments()
            .map(|args| {
                args.into_iter()
                    .map(|(name, value)| {
                        (
                            name.to_string(),
                            value.into_json().unwrap_or(serde_json::Value::Null),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();

        let child_where = match arguments.get("where") {
            Some(expression) if !expression.is_null() => {
                let child_scope = WhereScope::for_alias(
                    &child_alias,
                    &rel.target_type,
                    schema_cache,
                    relationships,
                );
                let mut nested_alias = 0usize;
                build_condition(expression, &child_scope, param_idx, values, &mut nested_alias)?
            }
            _ => None,
        };

        let child_order = match arguments.get("order_by") {
            Some(order) if !order.is_null() => order_terms(
                order,
                schema_cache,
                &plan.foreign_schema,
                &plan.foreign_table,
                &child_alias,
            )?,
            _ => None,
        };

        // A limit written on the embed is what the client asked for; the
        // configured ceiling still applies as an upper bound, the same way it
        // does at the top level.
        let child_limit = match arguments.get("limit").and_then(|v| v.as_i64()) {
            Some(requested) => match max_rows {
                Some(ceiling) => Some(requested.min(ceiling)),
                None => Some(requested),
            },
            None => max_rows,
        };
        let child_offset = arguments.get("offset").and_then(|v| v.as_i64()).unwrap_or(0);

        // Leaf fields are columns; anything that resolved to a relationship is
        // an expression instead.
        let child_relationships = relationships.get(&rel.target_type);
        let child_qi = postrust_core::api_request::QualifiedIdentifier::new(
            &plan.foreign_schema,
            &plan.foreign_table,
        );
        let child_table = schema_cache.get_table(&child_qi);
        let mut parts: Vec<String> = Vec::new();
        for sub in field.selection_set() {
            let name = sub.name();
            let is_relationship = child_relationships
                .map(|rels| {
                    rels.iter().any(|r| {
                        r.name == name || (r.is_list && format!("{}_aggregate", r.name) == name)
                    })
                })
                .unwrap_or(false);
            if is_relationship {
                continue;
            }
            // A computed column inside an embed is the same call, against the
            // child's alias rather than the table.
            let computed = child_table
                .filter(|t| t.get_column(name).is_none())
                .and_then(|t| {
                    let function = names
                        .computed_source(&t.schema, &t.name, name)
                        .unwrap_or(name);
                    t.get_computed_column(function)
                });
            match computed {
                Some(definition) => parts.push(format!(
                    "{}.{}({}.*) AS {}",
                    postrust_sql::escape_ident(&definition.function.schema),
                    postrust_sql::escape_ident(&definition.function.name),
                    postrust_sql::escape_ident(&child_alias),
                    postrust_sql::escape_ident(name)
                )),
                None => parts.push(postrust_sql::escape_ident(name)),
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
                // A computed relationship is correlated by argument rather
                // than by a key: the function takes the parent row, and an
                // alias names that row.
                &postrust_sql::escape_ident(parent_alias),
                &child_alias,
                &parts.join(", "),
                child_limit,
                child_offset,
                child_where.as_deref(),
                child_order.as_deref(),
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

    // `update_x_by_pk` takes its key as one `pk_columns` object; the others
    // take the key columns as arguments of their own. Both spellings are what
    // a client was generated against, so both are read here.
    let from_object = ctx
        .args
        .try_get("pk_columns")
        .ok()
        .map(|v| accessor_to_json(&v));

    let mut conditions = serde_json::Map::new();
    for (col_name, _) in pk_columns {
        let value = match &from_object {
            Some(serde_json::Value::Object(map)) => map.get(col_name).cloned().ok_or_else(|| {
                async_graphql::Error::new(format!(
                    "pk_columns is missing the key column \"{}\"",
                    col_name
                ))
            })?,
            _ => {
                let arg = ctx.args.try_get(col_name).map_err(|_| {
                    async_graphql::Error::new(format!(
                        "missing required primary key argument \"{}\"",
                        col_name
                    ))
                })?;
                accessor_to_json(&arg)
            }
        };
        conditions.insert(col_name.clone(), serde_json::json!({ "_eq": value }));
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
    } else if let Ok(name) = accessor.enum_name() {
        // An enum value is not a string to `string()`, and falling through to
        // null made every `order_by: {name: asc}` read as an empty direction.
        serde_json::Value::String(name.to_string())
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

/// Register filter input types.
///
/// These are currently unreachable: the `filter` and `where` arguments are
/// declared as the `JSON` scalar, so no field references these input objects
/// and async-graphql prunes them from the published schema (introspecting
/// `IntFilterInput` returns null). They are kept as the shape to move to if
/// filters become typed per column; until then the operators a filter actually
/// supports are the ones `build_where_clause` implements, and it rejects
/// anything else rather than ignoring it.
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
                domain_type: None,
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
                domain_type: None,
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
            unique_constraints: Vec::new(),
            columns,
            computed_columns: Default::default(),
            is_partitioned: false,
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
            representations: Default::default(),
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

        let result = build_dynamic_schema(&generated, &cache, None, None, Arc::new(Default::default()));
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

        let _query = create_query_type(&generated, None, Arc::new(HashMap::new()), Arc::new(Default::default()));
    }

    #[test]
    fn test_create_mutation_type() {
        let cache = create_test_schema_cache();
        let config = SchemaConfig::default();
        let generated = build_schema(&cache, &config);

        let _mutation = create_mutation_type(&generated, Arc::new(HashMap::new()), Arc::new(HashMap::new()), Arc::new(Default::default()), None);
    }

    // ============================================================================
    // Scalar Tests
    // ============================================================================

    #[test]
    fn a_scalar_is_named_the_way_a_client_declares_it() {
        use crate::types::{pg_type_to_graphql, GraphQLType};

        // These names appear in the client's own queries -- `query ($x:
        // jsonb!)` names a type that has to exist under exactly that
        // spelling -- so they are asserted rather than left to the Display
        // impl.
        assert_eq!(pg_type_to_graphql("jsonb").to_string(), "jsonb");
        assert_eq!(pg_type_to_graphql("json").to_string(), "json");
        assert_eq!(pg_type_to_graphql("int8").to_string(), "bigint");
        assert_eq!(pg_type_to_graphql("numeric").to_string(), "numeric");
        assert_eq!(pg_type_to_graphql("timestamptz").to_string(), "timestamptz");
        assert_eq!(pg_type_to_graphql("timestamp").to_string(), "timestamp");
        assert_eq!(pg_type_to_graphql("uuid").to_string(), "uuid");

        // A type this server knows nothing about keeps its own name, which is
        // how a PostGIS column and a database enum both become usable.
        assert_eq!(pg_type_to_graphql("geometry").to_string(), "geometry");
        assert_eq!(
            pg_type_to_graphql("colors_enum"),
            GraphQLType::Custom("colors_enum".to_string())
        );

        // And the leaf of an array is what gets registered, not the array.
        assert_eq!(
            leaf_scalar_name(&pg_type_to_graphql("_text")),
            "String".to_string()
        );
    }

    // ============================================================================
    // Filter Input Type Tests
    // ============================================================================

    #[test]
    fn every_table_gets_a_boolean_expression() {
        let cache = create_test_schema_cache();
        let config = SchemaConfig::default();
        let generated = build_schema(&cache, &config);

        let (inputs, _) = crate::input::bool_exp::build_inputs(
            &generated.object_types,
            &generated.relationship_fields,
        );
        let names: HashSet<String> = inputs.iter().map(|i| i.type_name().to_string()).collect();

        for table in generated.object_types.keys() {
            let expected = format!("{}_bool_exp", table);
            assert!(
                names.contains(&expected),
                "no {} among {:?}",
                expected,
                names
            );
        }
        // The scalars the fixture's columns use.
        assert!(names.contains("String_comparison_exp"));
        assert!(names.contains("Int_comparison_exp"));
    }

    #[test]
    fn a_schema_carrying_the_boolean_expressions_still_builds() {
        let cache = create_test_schema_cache();
        let config = SchemaConfig::default();
        let generated = build_schema(&cache, &config);

        let schema = build_dynamic_schema(&generated, &cache, None, None, Arc::new(Default::default()));
        assert!(schema.is_ok(), "{:?}", schema.err());

        let sdl = schema.unwrap().sdl();
        assert!(sdl.contains("_bool_exp"), "no boolean expressions in:\n{}", sdl);
        assert!(sdl.contains("_and"), "no _and in the generated expressions");
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
        let result = build_dynamic_schema(&generated, &cache, Some(&sub_fields), None, Arc::new(Default::default()));
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
