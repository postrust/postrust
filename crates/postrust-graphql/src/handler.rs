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
    generate_subscription_fields, NotifyBroker, SubscriptionField as SubField,
};
use async_graphql::dynamic::*;
use async_graphql::Value;
use async_graphql_axum::{GraphQLRequest, GraphQLResponse};
use axum::extract::State;
use axum::response::IntoResponse;
use postrust_core::schema_cache::SchemaCache;
use sqlx::PgPool;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
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
    ///
    /// The unrestricted one: what an administrator is answered from, and what
    /// every caller is answered from on a server with no permission document.
    pub schema: Schema,
    /// One schema per role the permission document names.
    ///
    /// Hasura builds a schema per role because a permission is a statement
    /// about what exists, not about what is allowed: a role with no `select`
    /// on a table has no field to name, and naming one is a validation failure
    /// rather than a denial. Each is built from a schema cache already reduced
    /// to what that role may see.
    ///
    /// Built at startup rather than on first use, so a permission document
    /// that cannot be built fails while someone is watching rather than on the
    /// first request from whichever role it broke.
    ///
    /// Keyed by the role and by whether the caller asked for what only a
    /// backend may name -- which is a second schema rather than a check,
    /// because `backend_only` decides whether a field exists.
    pub role_schemas: HashMap<(String, bool), Schema>,
    /// Schema configuration
    pub config: SchemaConfig,
    /// Subscription fields
    pub subscription_fields: Vec<SubField>,
    /// Notification broker for subscriptions
    pub broker: Arc<RwLock<Option<NotifyBroker>>>,
}

/// Build one schema for each role the permission document names.
///
/// Each is built from a cache reduced to what that role may see, so nothing in
/// the builders needs to know a permission exists -- a table this role cannot
/// read is one the code generating root fields cannot see either. See
/// [`crate::role`] for why the filtering is done there rather than threaded
/// through.
///
/// Empty when the document carries no permissions, which is what leaves an
/// unconfigured server building exactly the one schema it always did.
fn build_role_schemas(
    schema_cache: &Arc<SchemaCache>,
    config: &SchemaConfig,
) -> Result<HashMap<(String, bool), Schema>, GraphQLError> {
    if !config.names.has_permissions() {
        return Ok(HashMap::new());
    }

    let mut schemas = HashMap::new();
    for role in config.names.roles() {
        // An administrator is not a role with permissions; it is the caller
        // the permissions do not apply to. Building it a restricted schema
        // would take the bypass away.
        if role == postrust_auth::hasura::ADMIN_ROLE {
            continue;
        }

        // A second schema only where the role has something a backend alone
        // may name. Otherwise the one schema answers both, since there is
        // nothing for the flag to hide.
        let variants: &[bool] = match crate::role::has_backend_only(&config.names, role) {
            true => &[false, true],
            false => &[false],
        };

        for &backend in variants {
            let view = Arc::new(crate::role::cache_for_role(
                schema_cache,
                &config.names,
                role,
                backend,
            ));
            let mut role_config = config.clone();
            role_config.role = Some(role.to_string());

            let generated = build_schema(&view, &role_config);
            // Subscriptions are left to the unrestricted schema for now: a live
            // query is one this server answers from notifications rather than from
            // the query root, so restricting it is a different piece of work from
            // restricting a read.
            let built = build_dynamic_schema(
                &generated,
                &view,
                None,
                role_config.max_rows,
                Arc::new(role_config.names.clone()),
                role_config.subscription_refresh(),
                Some(role),
            );

            match built {
                Ok(schema) => {
                    schemas.insert((role.to_string(), backend), schema);
                }
                // One role's permissions being unbuildable is not a reason for
                // every other role to lose its API, which is what returning here
                // would mean -- the caller serves no GraphQL at all on an error.
                // The role is left without a schema instead, which the request
                // path already reads as a refusal: it fails closed, loudly, and
                // alone.
                Err(e) => tracing::error!(
                    "GraphQL schema for role \"{}\" could not be built, \
                 so that role is refused: {}",
                    role,
                    e
                ),
            }
        }
    }

    tracing::info!("GraphQL schemas built for {} roles", schemas.len());
    Ok(schemas)
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
            config.subscription_refresh(),
            None,
        )?;

        let role_schemas = build_role_schemas(&schema_cache, &config)?;

        Ok(Self {
            pool: pool.clone(),
            schema_cache,
            generated_schema,
            schema,
            role_schemas,
            config,
            subscription_fields,
            broker: Arc::new(RwLock::new(None)),
        })
    }

    /// Which schema answers for a caller speaking as this role.
    ///
    /// `None` for a role the document does not name, which the caller turns
    /// into a refusal. Three things are deliberately not that: a server with no
    /// permission document answers everyone from the unrestricted schema, an
    /// administrator is answered from it too -- permissions are rules for
    /// everyone else -- and so is a request on a server where the layer is off.
    pub fn schema_for(&self, role: Option<&str>, backend: bool) -> Option<&Schema> {
        if !self.config.names.has_permissions() {
            return Some(&self.schema);
        }
        match role {
            None | Some(postrust_auth::hasura::ADMIN_ROLE) => Some(&self.schema),
            Some(role) => self
                .role_schemas
                .get(&(role.to_string(), backend))
                // A role with nothing backend-only has one schema, under
                // `false`, and it answers a backend caller too.
                .or_else(|| self.role_schemas.get(&(role.to_string(), false))),
        }
    }

    /// Rebuild the schema (e.g., after schema cache refresh).
    #[allow(clippy::needless_update)]
    pub fn rebuild(&mut self) -> Result<(), GraphQLError> {
        self.role_schemas = build_role_schemas(&self.schema_cache, &self.config)?;
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
            self.config.subscription_refresh(),
            None,
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

/// The column a field name refers to.
///
/// The name itself, unless the table exposes that column under another one.
/// Every path from a request to SQL goes through here: a renamed column is a
/// name in the schema and in nothing else, so the translation has to happen at
/// each boundary rather than once.
fn column_for<'a>(
    names: &'a crate::names::NameOverrides,
    schema: &str,
    table: &str,
    field: &'a str,
) -> &'a str {
    names.column_source(schema, table, field).unwrap_or(field)
}

/// The same, given the table itself.
fn table_column_for<'a>(
    names: &'a crate::names::NameOverrides,
    table: &postrust_core::schema_cache::Table,
    field: &'a str,
) -> &'a str {
    column_for(names, &table.schema, &table.name, field)
}

/// The select list that renames a table's columns to the fields they are
/// exposed as, or `None` where nothing is renamed.
///
/// Applied to a projection over the table rather than to the table itself, so
/// the row a computed field or a computed relationship is passed stays the
/// table's own composite -- `cannot cast type record to author` is what
/// happens when it does not.
fn rename_projection(
    table: &postrust_core::schema_cache::Table,
    alias: &str,
    names: &crate::names::NameOverrides,
) -> Option<String> {
    if !names.renames_columns(&table.schema, &table.name) {
        return None;
    }
    let parts: Vec<String> = table
        .columns
        .values()
        .map(|column| {
            let exposed = names
                .column(&table.schema, &table.name, &column.name)
                .unwrap_or(&column.name);
            format!(
                "{}.{} AS {}",
                postrust_sql::escape_ident(alias),
                postrust_sql::escape_ident(&column.name),
                postrust_sql::escape_ident(exposed)
            )
        })
        .collect();
    match parts.is_empty() {
        true => None,
        false => Some(parts.join(", ")),
    }
}

/// A table's column types, keyed by the names those columns are exposed under.
///
/// [`row_json`] names the shape columns it has to rewrite as GeoJSON, and it
/// names them in whatever the projection called them. Over a renamed
/// projection that is the field name, not the column's.
fn exposed_column_types(
    table: &postrust_core::schema_cache::Table,
    names: &crate::names::NameOverrides,
) -> HashMap<String, String> {
    table
        .columns
        .values()
        .map(|column| {
            let exposed = names
                .column(&table.schema, &table.name, &column.name)
                .unwrap_or(&column.name);
            (exposed.to_string(), column.nominal_type.clone())
        })
        .collect()
}

/// The type a value is written as, which keeps the list an array column is.
///
/// Not the leaf scalar: `c34_text_array` is a `text[]`, and a client sending
/// `$textArray: [String]` into it was refused for offering a list where the
/// input said `String`. Every write is optional, so nothing here is non-null.
fn write_type_ref(graphql_type: &crate::types::GraphQLType) -> TypeRef {
    match graphql_type {
        crate::types::GraphQLType::List(inner) => TypeRef::named_list(leaf_scalar_name(inner)),
        other => TypeRef::named(leaf_scalar_name(other)),
    }
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
    subscription_refresh: std::time::Duration,
    // Whose schema this is, where it is one role's. What it decides here is
    // which columns a write may name, which is not the same question as which
    // columns may be read -- Hasura grants the two separately, and a role that
    // may set a column it cannot read is ordinary rather than exotic.
    role: Option<&str>,
) -> Result<Schema, GraphQLError> {
    // Create object types for each table
    let mut object_types: HashMap<String, Object> = HashMap::new();

    // A table with no readable field gets no type at all. That is not an
    // empty table -- it is one this role may write and not read, and a GraphQL
    // object type with no fields is not a legal type. Everything that would
    // have returned it is left out with it.
    for (type_name, obj) in &generated.object_types {
        if obj.fields.is_empty() {
            continue;
        }
        let relationships = generated
            .relationship_fields
            .get(type_name)
            .map(|r| r.as_slice())
            .unwrap_or(&[]);
        let table_obj = create_object_type(obj, relationships);
        object_types.insert(type_name.clone(), table_obj);
    }

    // Aggregate types: every readable table gets them, because every table has
    // a count even when it has nothing to sum -- and so does a table a role may
    // count and not read, which is the whole of what such a role was granted.
    // Read from the roots that were generated rather than worked out again,
    // for the reason the write inputs are: two answers to one question drift.
    let countable: HashSet<&str> = generated
        .query_fields
        .iter()
        .filter(|field| field.aggregates && !field.is_by_pk)
        .map(|field| field.type_name.as_str())
        .collect();
    for (type_name, obj) in &generated.object_types {
        if obj.fields.is_empty() && !countable.contains(type_name.as_str()) {
            continue;
        }
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
            // The table's own name. A bulk write already answers with the
            // response type, and trimming the suffix here is what keeps
            // `<t>_mutation_response_mutation_response` from being built: the
            // set used to be carried by `insert_one` and `update_by_pk`
            // naming the bare type, so a table with only bulk writes -- which
            // is what a role that cannot read one has -- registered the wrong
            // name and left the right one missing.
            f.return_type
                .trim_matches(|c| c == '[' || c == ']' || c == '!')
                .trim_end_matches("_mutation_response")
        })
        .collect();
    for base_name in mutable {
        // Whether it has rows to give back. A write to a table this role
        // cannot read answers with `affected_rows` and nothing else -- there
        // is no row type for `returning` to be a list of.
        let returning = generated
            .object_types
            .get(base_name)
            .is_some_and(|object| !object.fields.is_empty());
        object_types.insert(
            mutation_response_type_name(base_name),
            create_mutation_response_type(base_name, returning),
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
    let query = add_function_fields(
        create_query_type(
            generated,
            max_rows,
            Arc::clone(&relationships),
            Arc::clone(&names),
        ),
        generated,
        false,
        max_rows,
        Arc::clone(&relationships),
        Arc::clone(&names),
    );

    // Create mutation type
    let mutation = if !generated.mutation_fields.is_empty()
        || generated.function_fields.iter().any(|f| f.volatile)
    {
        Some(add_function_fields(
            create_mutation_type(
                generated,
                Arc::clone(&relationships),
                Arc::clone(&type_names),
                Arc::clone(&names),
                max_rows,
                role,
            ),
            generated,
            true,
            max_rows,
            Arc::clone(&relationships),
            Arc::clone(&names),
        ))
    } else {
        None
    };

    // Create subscription type if enabled
    let subscription = subscription_fields.map(|_| {
        create_subscription_type(
            generated,
            max_rows,
            Arc::clone(&relationships),
            Arc::clone(&names),
            subscription_refresh,
        )
    });

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
        // Both halves. A column a role may set without seeing is named by the
        // insert input and by nothing a read produces, so collecting only the
        // readable fields leaves its scalar unregistered -- and a scalar the
        // schema mentions but never registers makes the whole schema
        // unbuildable, which costs the role every field it has.
        for field in object.fields.iter().chain(&object.writable_fields) {
            scalar_names.insert(leaf_scalar_name(&field.graphql_type));
        }
    }
    // Used as an argument type in its own right, whether or not any column is
    // one: `objects`, `_set` and the mutation inputs are still JSON.
    scalar_names.insert("JSON".to_string());
    // A function's arguments name scalars of their own: a table may have no
    // geometry column while a function takes one.
    for function in &generated.function_fields {
        for (_, pg_type, _) in &function.arguments {
            scalar_names.insert(leaf_scalar_name(&crate::types::pg_type_to_graphql(pg_type)));
        }
    }
    // The same for the functions behind computed fields, whose arguments are
    // written where the field is asked for rather than at a root.
    for object in generated.object_types.values() {
        for field in &object.fields {
            for (_, pg_type, _) in &field.arguments {
                scalar_names.insert(leaf_scalar_name(&crate::types::pg_type_to_graphql(pg_type)));
            }
        }
    }
    for relationships in generated.relationship_fields.values() {
        for relationship in relationships {
            for (_, pg_type, _) in &relationship.arguments {
                scalar_names.insert(leaf_scalar_name(&crate::types::pg_type_to_graphql(pg_type)));
            }
        }
    }

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
        builder = builder
            .register(Scalar::new(&name).description(format!("The PostgreSQL `{}` type.", name)));
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
                names
                    .column(&field.schema_name, &field.table_name, column)
                    .unwrap_or(column),
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
    // Which types have an `_insert_input` at all, which is what a nested write
    // names. Collected first because a relationship may point at a table this
    // loop has not reached yet -- and because the answer is now a permission
    // question: a role granted `insert` on `article` and only `select` on
    // `author` gets an `article_insert_input`, and writing `author` inside it
    // would name a type nobody registered. async-graphql refuses to build a
    // schema with a dangling reference, so the whole role would have no schema
    // over one field it was never allowed to use.
    // Which types actually got each write input, recorded as they are built
    // rather than predicted from the object type's fields. The two answers
    // used to be the same and are not any more: an input is built from the
    // write permission's columns, so a table whose readable columns include a
    // number may still have no `_inc_input` -- and an argument naming a type
    // nobody registered is a schema async-graphql refuses to build, which
    // costs the role every field it has rather than the one it cannot use.
    let insertable_types: HashSet<&str> = generated
        .object_types
        .iter()
        .filter(|(_, object)| crate::input::mutation::is_insertable(&object.table))
        .map(|(type_name, _)| type_name.as_str())
        .collect();

    for (type_name, object) in &generated.object_types {
        let table = &object.table;
        // Which columns a write may name, which is the write permission's
        // answer and not the read permission's. A role granted `columns:
        // ["age"]` on insert may not name `is_user` even where it can read it,
        // and the corpus tests exactly that: `field 'is_user' not found in
        // type: 'resident_insert_input'`.
        // Only real columns are written. A computed field is a function of the
        // row and a relationship is handled separately below.
        let writable_named = |allowed: Option<&crate::names::ColumnSet>| {
            object
                .writable_fields
                .iter()
                .filter(|field| {
                    let column = table_column_for(&names, table, &field.name);
                    table.get_column(column).is_some()
                        && allowed.is_none_or(|set| set.allows(column))
                })
                .collect::<Vec<&crate::schema::object::GraphQLField>>()
        };
        let writable = writable_named(None);
        let insert_fields = write_fields(object, &names, role, crate::role::Verb::Insert);
        let update_fields = write_fields(object, &names, role, crate::role::Verb::Update);
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
            for field in &insert_fields {
                // Every column optional: which ones the database insists on is
                // the database's answer, and a column that is NOT NULL with a
                // default does not have to be given.
                taken.insert(field.name.as_str());
                insert = insert.field(InputValue::new(
                    &field.name,
                    write_type_ref(&field.graphql_type),
                ));
            }
            // A nested write: the rows to insert beside this one. Only where
            // the far side is something this schema can write -- see
            // `insertable_types`.
            for relationship in relationships {
                if !insertable_types.contains(relationship.target_type.as_str()) {
                    continue;
                }
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
            let mut any_jsonb = false;
            for field in &update_fields {
                if matches!(&field.graphql_type, crate::types::GraphQLType::Json) {
                    any_jsonb = true;
                }
                set = set.field(InputValue::new(
                    &field.name,
                    write_type_ref(&field.graphql_type),
                ));
                // Only a number can be added to, and a list of them is not a
                // number: `_inc` takes the scalar or nothing.
                if crate::schema::aggregate::is_incrementable(&field.graphql_type) {
                    any_numeric = true;
                    numeric = numeric.field(InputValue::new(
                        &field.name,
                        TypeRef::named(leaf_scalar_name(&field.graphql_type)),
                    ));
                }
            }
            builder = builder.register(set);
            // A table with nothing to add to gets no type for adding to it.
            if any_numeric {
                builder = builder.register(numeric);
            }

            // One entry of `update_x_many`: a filter and the values to write
            // where it matches. The whole list runs in one transaction, in the
            // order it was given, which is what makes it different from
            // sending the updates one at a time.
            let mut updates = InputObject::new(format!("{}_updates", type_name))
                .description(format!(
                    "One update to {}: which rows, and what to write.",
                    type_name
                ))
                .field(InputValue::new(
                    "where",
                    TypeRef::named_nn(crate::input::bool_exp::bool_exp_type_name(type_name)),
                ))
                .field(InputValue::new(
                    "_set",
                    TypeRef::named(format!("{}_set_input", type_name)),
                ));
            if any_numeric {
                updates = updates.field(InputValue::new(
                    "_inc",
                    TypeRef::named(format!("{}_inc_input", type_name)),
                ));
            }
            // What a document column may be told to do, one input per
            // operator: each takes a value of a different shape -- a
            // document, a key, an index, a path -- and only the columns that
            // hold documents may be told any of it. A table with none is not
            // given the operators at all, which is what says in the schema
            // that `_append` is not a thing to write there.
            for (operator, _) in JSONB_OPERATORS {
                if !any_jsonb {
                    break;
                }
                updates = updates.field(InputValue::new(
                    *operator,
                    TypeRef::named(jsonb_operator_input(type_name, operator)),
                ));
            }
            builder = builder.register(updates);
            if any_jsonb {
                for (operator, item) in JSONB_OPERATORS {
                    let mut input =
                        InputObject::new(jsonb_operator_input(type_name, operator)).description(
                            format!("Columns of {} to apply `{}` to.", type_name, operator),
                        );
                    for field in &update_fields {
                        if !matches!(&field.graphql_type, crate::types::GraphQLType::Json) {
                            continue;
                        }
                        input = input.field(InputValue::new(&field.name, item()));
                    }
                    builder = builder.register(input);
                }
            }
        }
    }

    // A function's own arguments, under a name of their own so they cannot
    // collide with `where` or `limit`.
    for function in &generated.function_fields {
        if function.arguments.is_empty() {
            continue;
        }
        let mut args = InputObject::new(format!("{}_args", function.name))
            .description(format!("Arguments to {}.", function.name));
        for (name, pg_type, _) in &function.arguments {
            let scalar = crate::types::pg_type_to_graphql(pg_type).to_string();
            // Nullable, whether or not the parameter has a default. Every
            // PostgreSQL argument accepts a null, and a client passing one
            // through a variable declares it as the nullable type -- `query
            // ($point: json)` used where `json!` was expected is a query the
            // spec's variable rule refuses. Whether a defaulted argument may
            // be left out entirely is enforced where the call is written,
            // which is the only place that can tell.
            args = args.field(InputValue::new(name, TypeRef::named(scalar)));
        }
        builder = builder.register(args);
    }

    // The same for a computed relationship's function, which takes its
    // arguments where it is embedded rather than at the root. Named after the
    // type and the field, since two tables may reach the same function and one
    // table may reach two.
    for (type_name, relationships) in &generated.relationship_fields {
        for relationship in relationships {
            if relationship.arguments.is_empty() {
                continue;
            }
            let mut args = InputObject::new(computed_args_type_name(type_name, &relationship.name))
                .description(format!(
                    "Arguments to {}, beside the row it is asked of.",
                    relationship.name
                ));
            for (name, pg_type, _) in &relationship.arguments {
                let scalar = crate::types::pg_type_to_graphql(pg_type).to_string();
                // Nullable, for the reason the function arguments above are.
                args = args.field(InputValue::new(name, TypeRef::named(scalar)));
            }
            builder = builder.register(args);
        }
    }

    // And for a computed *column*'s function, which likewise takes its
    // arguments where the field is asked for. `locations { distance(args: {
    // from: ... }) }` -- the field is a function of the row and of what the
    // caller wants measured against it.
    for (type_name, object) in &generated.object_types {
        for field in &object.fields {
            if field.arguments.is_empty() {
                continue;
            }
            let mut args = InputObject::new(computed_args_type_name(type_name, &field.name))
                .description(format!(
                    "Arguments to {}, beside the row it is asked of.",
                    field.name
                ));
            for (name, pg_type, _) in &field.arguments {
                let scalar = crate::types::pg_type_to_graphql(pg_type).to_string();
                // Nullable, for the reason the function arguments above are.
                args = args.field(InputValue::new(name, TypeRef::named(scalar)));
            }
            builder = builder.register(args);
        }
    }

    // What a predicate over a related set may ask. Registered only for the
    // tables something points at with a to-many relationship, which is the
    // only place the field naming them exists.
    {
        use crate::input::bool_exp as be;
        let mut targets: BTreeSet<&str> = BTreeSet::new();
        for relationships in generated.relationship_fields.values() {
            for relationship in relationships {
                if relationship.is_list {
                    targets.insert(relationship.target_type.as_str());
                }
            }
        }
        for target in targets {
            let Some(object) = generated.object_types.get(target) else {
                continue;
            };
            // A boolean column is what `bool_and` and `bool_or` fold, and a
            // GraphQL enum may not be empty -- so a table with none gets
            // neither the aggregates nor the enum naming their columns.
            let booleans: Vec<&str> = object
                .fields
                .iter()
                .filter(|field| {
                    object.table.get_column(&field.name).is_some()
                        && matches!(field.graphql_type, crate::types::GraphQLType::Boolean)
                })
                .map(|field| field.name.as_str())
                .collect();

            let mut over = InputObject::new(be::aggregate_bool_exp_type_name(target)).description(
                format!("A question about the whole set of related {} rows.", target),
            );
            for function in be::AGGREGATE_PREDICATES {
                if function != "count" && booleans.is_empty() {
                    continue;
                }
                over = over.field(InputValue::new(
                    function,
                    TypeRef::named(be::aggregate_bool_exp_function(target, function)),
                ));

                // `count` may be told which columns to count, as the field
                // itself may; `bool_and` folds exactly one.
                let (arguments, predicate) = match function {
                    "count" => (
                        TypeRef::named_nn_list(crate::input::order_by::select_column_type_name(
                            target,
                        )),
                        be::comparison_type_name("Int"),
                    ),
                    _ => (
                        TypeRef::named_nn(be::aggregate_bool_exp_columns(target, function)),
                        be::comparison_type_name("Boolean"),
                    ),
                };
                let mut input = InputObject::new(be::aggregate_bool_exp_function(target, function))
                    .description(format!(
                        "`{}` over the related {} rows, and what it has to be.",
                        function, target
                    ))
                    .field(InputValue::new("arguments", arguments))
                    .field(InputValue::new("distinct", TypeRef::named("Boolean")))
                    .field(InputValue::new(
                        "filter",
                        TypeRef::named(be::bool_exp_type_name(target)),
                    ))
                    .field(InputValue::new("predicate", TypeRef::named_nn(predicate)));
                if function == "count" {
                    input = input.description(format!(
                        "How many related {} rows there have to be.",
                        target
                    ));
                }
                builder = builder.register(input);

                if function != "count" {
                    let mut columns = Enum::new(be::aggregate_bool_exp_columns(target, function))
                        .description(format!("A boolean column of {}.", target));
                    for name in &booleans {
                        columns = columns.item(EnumItem::new(*name));
                    }
                    builder = builder.register(columns);
                }
            }
            builder = builder.register(over);
        }
    }

    // Upserts. A table with no unique constraint has no conflict to resolve,
    // and a GraphQL enum may not be empty, so it gets none of these types
    // rather than an unusable set of them.
    for (type_name, object) in &generated.object_types {
        // What an upsert may write, which is the update permission's answer
        // and not the read permission's: `update_columns` names columns to
        // overwrite, so a role that may set a column without seeing it may
        // name it here. Where no update permission narrows them these are the
        // table's own columns, which is what the enum listed before.
        let updatable_columns = write_fields(object, &names, role, crate::role::Verb::Update);
        if object.table.unique_constraints.is_empty() {
            continue;
        }

        let mut constraints = Enum::new(format!("{}_constraint", type_name)).description(format!(
            "A uniqueness of {} that an insert may conflict with.",
            type_name
        ));
        for (name, columns) in &object.table.unique_constraints {
            constraints = constraints
                .item(EnumItem::new(name).description(format!("unique ({})", columns.join(", "))));
        }

        let mut updatable = Enum::new(format!("{}_update_column", type_name))
            .description(format!("A column of {} an upsert may write.", type_name));
        for field in &updatable_columns {
            updatable = updatable.item(EnumItem::new(&field.name));
        }
        // An update permission granting no column leaves the enum with no
        // members, which is not a legal type -- and dropping the enum would
        // drop `on_conflict` with it, so an insert that names one would fail
        // to build rather than be refused. Hasura's answer is a member that
        // names no column: the upsert is still expressible, `update_columns:
        // []` still means `DO NOTHING`, and naming the placeholder is refused
        // where it is used. `article_on_conflict_restricted_role.yaml` tests
        // both halves.
        if updatable_columns.is_empty() {
            updatable = updatable.item(
                EnumItem::new(PLACEHOLDER_COLUMN)
                    .description("No column may be written by this upsert."),
            );
        }

        let on_conflict = InputObject::new(format!("{}_on_conflict", type_name))
            .description(format!(
                "What to do when an insert into {} conflicts.",
                type_name
            ))
            .field(InputValue::new(
                "constraint",
                TypeRef::named_nn(format!("{}_constraint", type_name)),
            ))
            // An empty list is `DO NOTHING`, which is how Hasura spells "leave
            // the row that is already there alone" -- and is the default, so
            // `on_conflict: {constraint: article_pkey}` means exactly that
            // rather than being refused for saying nothing about columns.
            .field(
                InputValue::new(
                    "update_columns",
                    TypeRef::named_nn_list_nn(format!("{}_update_column", type_name)),
                )
                .default_value(Value::List(Vec::new())),
            )
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
        let mut generated_enum =
            Enum::new(type_name).description(format!("The values {} allows.", type_name));
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
    let (order_inputs, order_enums) = crate::input::order_by::build_inputs(
        &generated.object_types,
        &generated.relationship_fields,
    );
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
/// The enum member that stands in for an update permission granting no column.
///
/// Hasura's spelling, and it is load-bearing: `article_update_column` must be
/// a legal enum for `article_on_conflict` to exist, and `on_conflict` must
/// exist for an insert that carries one to be refused rather than unbuildable.
const PLACEHOLDER_COLUMN: &str = "_PLACEHOLDER";

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
fn create_mutation_response_type(base_name: &str, returning: bool) -> Object {
    let response_name = mutation_response_type_name(base_name);
    let row_type = base_name.to_string();

    let response = Object::new(&response_name)
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
        ));
    // A table this role may write and not read has no row type, so there is
    // nothing for `returning` to be a list of. Hasura answers such a write
    // with the count alone, and the corpus asks for nothing else.
    if !returning {
        return response;
    }
    response.field(Field::new(
        "returning",
        TypeRef::named_nn_list_nn(row_type),
        |ctx| {
            FieldFuture::new(async move {
                let rows = match ctx.parent_value.as_value() {
                    Some(Value::Object(map)) => {
                        match map.get(&async_graphql::Name::new("returning")) {
                            Some(Value::List(items)) => items.clone(),
                            _ => Vec::new(),
                        }
                    }
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
    // Whether there are rows to hand back beside the numbers. A role granted
    // "how many" and not "which" has no row type, so `nodes` has nothing to be
    // a list of.
    let has_rows = !object.fields.is_empty();
    let mut over = Object::new(agg::aggregate_type_name(base_name))
        .description(match has_rows {
            true => format!("Aggregates over {}, with the rows themselves.", base_name),
            false => format!("Aggregates over {}.", base_name),
        })
        .field(Field::new(
            "aggregate",
            TypeRef::named(&fields_type),
            |ctx| FieldFuture::new(async move { Ok(child_of(&ctx, "aggregate")) }),
        ));
    if has_rows {
        over = over.field(Field::new(
            "nodes",
            TypeRef::named_nn_list_nn(base_name.to_string()),
            |ctx| {
                FieldFuture::new(async move {
                    let rows = match child_value(&ctx, "nodes") {
                        Some(Value::List(items)) => items,
                        _ => Vec::new(),
                    };
                    Ok(Some(FieldValue::list(
                        rows.into_iter().map(FieldValue::value),
                    )))
                })
            },
        ));
    }
    types.push(over);

    // `<t>_aggregate_fields`: count, and one field per function.
    let mut count = Field::new("count", TypeRef::named_nn(TypeRef::INT), |ctx| {
        // Read under the name it was asked for, not under `count`.
        // Two counts of different things sit in one selection --
        // `count` beside `distinct_authors: count(columns: [author_id],
        // distinct: true)` -- and they are different numbers.
        let key = ctx.ctx.field().alias().unwrap_or("count").to_string();
        FieldFuture::new(async move {
            Ok(Some(FieldValue::value(
                child_value(&ctx, &key).unwrap_or(Value::from(0)),
            )))
        })
    });
    // `count(columns:)` counts the rows where those columns are not null, and
    // `distinct` counts distinct values among them. Neither is offered where
    // the role may name no column: `count` is then a count of rows and nothing
    // else, and the corpus tests exactly that -- `'count' has no argument
    // named 'columns'`.
    if has_rows {
        count = count
            .argument(InputValue::new(
                "columns",
                TypeRef::named_nn_list(agg_select_column(base_name)),
            ))
            .argument(InputValue::new("distinct", TypeRef::named("Boolean")));
    }
    let mut aggregate_fields = Object::new(&fields_type)
        .description(format!("Aggregate functions over {}.", base_name))
        .field(count);

    for (function, returns, columns) in agg::functions_for(object) {
        let function_type = agg::function_fields_type_name(base_name, function);
        let mut per_column = Object::new(&function_type).description(format!(
            "`{}` of each {} column it applies to.",
            function, base_name
        ));
        for column in &columns {
            let column_name = column.clone();
            let type_name = agg::field_type_for(object, column, returns);
            per_column =
                per_column.field(Field::new(column, TypeRef::named(type_name), move |ctx| {
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

/// An error carrying the code Hasura would give it.
///
/// The default coding reads the message, and a refusal written here has no
/// word in it that a message from PostgreSQL would not also have -- so it says
/// so itself rather than being guessed at.
fn coded_error(code: &'static str, message: impl Into<String>) -> async_graphql::Error {
    let mut error = async_graphql::Error::new(message);
    let mut extensions = async_graphql::ErrorExtensionValues::default();
    extensions.set("code", code);
    error.extensions = Some(extensions);
    error
}

/// A refusal the client could have avoided by writing the request differently.
fn validation_error(message: impl Into<String>) -> async_graphql::Error {
    coded_error("validation-failed", message)
}

/// A database error, classified the way Hasura classifies it.
///
/// PostgreSQL's own message is the whole of what either server says about
/// what went wrong; what Hasura adds is a word for *which kind* of wrong,
/// taken from the SQLSTATE rather than from the text: `Uniqueness violation.
/// duplicate key value violates unique constraint "author_name_key"`. A client
/// reporting that to a user is reporting text it already ships.
///
/// Read from the code, never from the message. The message is localised, the
/// code is not, and every attempt to tell these apart by reading English is
/// wrong the first time a server runs in another language.
fn database_error(error: sqlx::Error) -> async_graphql::Error {
    let sqlx::Error::Database(db) = &error else {
        return async_graphql::Error::new(error.to_string());
    };
    let described = match db.code().as_deref() {
        // Class 23, integrity constraint violation: the four PostgreSQL
        // raises and Hasura names.
        Some("23505") => "Uniqueness violation. ",
        Some("23502") => "Not-NULL violation. ",
        Some("23503") => "Foreign key violation. ",
        Some("23514") => "Check constraint violation. ",
        // Everything else is answered with PostgreSQL's own words and no
        // heading. Inventing one for a class Hasura says nothing about would
        // be a difference rather than a match.
        _ => "",
    };
    let error = async_graphql::Error::new(format!("{}{}", described, db.message()));
    // And the code, where the SQLSTATE says which one. Only the codes Hasura's
    // corpus pins are set: everything else is left for `code_for` to classify,
    // which is a guess from the message text and says so. Replacing that guess
    // wholesale would be the same guess with fewer places to notice it.
    let coded = match db.code().as_deref() {
        Some(code) if code.starts_with("23") => Some("constraint-violation"),
        // A `LIKE` pattern that ends mid-escape. Not a data exception to
        // Hasura -- the pattern came from the request, so the request is what
        // was wrong.
        Some("22025") => Some("bad-request"),
        // A negative `OFFSET`, and the rest of class 22 with it.
        Some(code) if code.starts_with("22") => Some("data-exception"),
        _ => None,
    };
    match coded {
        Some(code) => at_code(error, code),
        None => error,
    }
}

/// The same error, coded as Hasura codes it.
fn at_code(mut error: async_graphql::Error, code: &'static str) -> async_graphql::Error {
    let mut extensions = error.extensions.take().unwrap_or_default();
    extensions.set("code", code);
    error.extensions = Some(extensions);
    error
}

/// The same error, told where in the request it happened.
///
/// Hasura names the place a write went wrong inside the *arguments* --
/// `$.selectionSet.insert_author.args.objects[0].bio` -- and a response path
/// cannot say that: it names fields of the answer, and the answer has no
/// `objects`. So the path is written where the error is raised, by whoever
/// knows which row and which column it is about, and
/// [`crate::hasura::path_for`] prefers it over the response path.
fn at_path(mut error: async_graphql::Error, path: &str) -> async_graphql::Error {
    let mut extensions = error.extensions.take().unwrap_or_default();
    extensions.set("path", path);
    error.extensions = Some(extensions);
    error
}

fn create_object_type(obj: &TableObjectType, relationships: &[RelationshipField]) -> Object {
    let mut object = Object::new(&obj.name);

    // A table with no comment still gets a description, because Hasura gives
    // one and a client generating documentation from the schema would
    // otherwise show a blank where it used to show this.
    object = match obj.description() {
        // An empty description is one that was given and is empty: metadata
        // said the type has none, which is not the same as saying nothing.
        Some("") => object,
        Some(desc) => object.description(desc),
        None => object.description(format!(
            "columns and relationships of \"{}\"",
            obj.table_name()
        )),
    };

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

    // Collected rather than added as they are built, so they can go on in name
    // order -- which is the order Hasura answers introspection in, and does not
    // depend on which column the catalogue happened to list first.
    let mut fields: Vec<(String, Field)> = Vec::new();

    for field in &obj.fields {
        let field_name = field.name.clone();
        let field_type = graphql_type_ref(&field.type_string());
        // A document-valued column can be asked for one part of itself, the
        // same way `#>` reads one. It is answered here rather than in SQL
        // because the same column may be asked for under several aliases --
        // `c32_json(path: "a")` beside `c32_json(path: "arr[0]")` -- and
        // one projection cannot carry both.
        let takes_path = matches!(&field.graphql_type, crate::types::GraphQLType::Json)
            || matches!(
                &field.graphql_type,
                crate::types::GraphQLType::Custom(name) if name == "json"
            );

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
                    let path = match takes_path {
                        true => ctx.args.get("path").and_then(|v| v.string().ok()),
                        false => None,
                    };
                    match map.get(&key) {
                        // A null is the answer, not a value to resolve. An
                        // enum-typed column said so with `internal: invalid
                        // item for enum` rather than answering null, because a
                        // null is not one of its members.
                        Some(Value::Null) => return Ok(None),
                        Some(val) => {
                            let val = match path {
                                Some(path) => walk_json_path(val, path)?,
                                None => val.clone(),
                            };
                            return match val {
                                Value::Null => Ok(None),
                                val => Ok(Some(FieldValue::value(val))),
                            };
                        }
                        None => {}
                    }
                }

                // Field not found or parent not a Value::Object
                Ok(None)
            })
        });

        let gql_field = match takes_path {
            true => gql_field.argument(
                InputValue::new("path", TypeRef::named("String")).description("JSON select path"),
            ),
            false => gql_field,
        };

        // A computed field whose function takes more than the row: the
        // caller writes those under `args`, for the reason a computed
        // relationship's are written there.
        let gql_field = match field.arguments.is_empty() {
            true => gql_field,
            false => gql_field.argument(InputValue::new(
                "args",
                TypeRef::named_nn(computed_args_type_name(&obj.name, &field.name)),
            )),
        };

        let gql_field = if let Some(desc) = &field.description {
            gql_field.description(desc)
        } else {
            gql_field
        };

        fields.push((field.name.clone(), gql_field));
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
        } else if always_present(rel, &obj.table) {
            TypeRef::named_nn(&rel.target_type)
        } else {
            TypeRef::named(&rel.target_type)
        };

        let is_list = rel.is_list;
        let mut gql_field = Field::new(&rel.name, field_type, move |ctx| {
            let field_name = field_name.clone();
            FieldFuture::new(async move {
                if let Some(Value::Object(map)) = ctx.parent_value.as_value() {
                    let key = async_graphql::Name::new(&field_name);
                    match map.get(&key) {
                        // A row that points at nothing has no related row, and
                        // the answer to that is null -- not an object whose
                        // every field is null, which is what handing a null
                        // parent value to the field resolvers produced.
                        Some(Value::Null) if !is_list => return Ok(None),
                        Some(val) => return Ok(Some(FieldValue::value(val.clone()))),
                        None => {}
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
        // A computed relationship's function may take more than the row --
        // "the articles of this author matching a search" -- and `args` is
        // where the caller writes them, so a term called `limit` cannot shadow
        // the one that pages the result.
        if !rel.arguments.is_empty() {
            gql_field = gql_field.argument(InputValue::new(
                "args",
                TypeRef::named_nn(computed_args_type_name(&obj.name, &rel.name)),
            ));
        }
        if rel.is_list {
            gql_field = gql_field
                .argument(InputValue::new(
                    "distinct_on",
                    TypeRef::named_nn_list(crate::input::order_by::select_column_type_name(
                        &rel.target_type,
                    )),
                ))
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

        fields.push((rel.name.clone(), gql_field));

        // `author { articles_aggregate { aggregate { count } } }` -- the count
        // of a row's children without fetching them, which is the query behind
        // every "12 comments" beside a post. Only for a relationship to many:
        // counting one row is not a question anyone asks.
        if rel.is_list {
            let aggregate_field = format!("{}_aggregate", rel.name);
            if taken.insert(aggregate_field.clone()) {
                let key = aggregate_field.clone();
                fields.push((
                    aggregate_field.clone(),
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
                    // The same arguments the list itself takes. "How many of
                    // this author's articles were published this year" is the
                    // question a count is usually asked as, and without a
                    // `where` there is no way to write it.
                    .argument(InputValue::new(
                        "distinct_on",
                        TypeRef::named_nn_list(crate::input::order_by::select_column_type_name(
                            &rel.target_type,
                        )),
                    ))
                    .argument(InputValue::new(
                        "where",
                        TypeRef::named(crate::input::bool_exp::bool_exp_type_name(
                            &rel.target_type,
                        )),
                    ))
                    .argument(InputValue::new(
                        "order_by",
                        TypeRef::named_nn_list(crate::input::order_by::order_by_type_name(
                            &rel.target_type,
                        )),
                    ))
                    .argument(InputValue::new("limit", TypeRef::named("Int")))
                    .argument(InputValue::new("offset", TypeRef::named("Int")))
                    .description(format!("Aggregates over {}.", rel.name)),
                ));
                // Counting the rows a function answers with means calling it,
                // so the aggregate takes whatever the function takes.
                if !rel.arguments.is_empty() {
                    let last = fields.len() - 1;
                    let (name, field) = fields.remove(last);
                    fields.push((
                        name,
                        field.argument(InputValue::new(
                            "args",
                            TypeRef::named_nn(computed_args_type_name(&obj.name, &rel.name)),
                        )),
                    ));
                }
            }
        }
    }

    fields.sort_by(|(a, _), (b, _)| a.cmp(b));
    for (_, field) in fields {
        object = object.field(field);
    }

    object
}

/// A root field, whichever root it is on.
///
/// A query field and a subscription field take the same arguments and mean the
/// same by them -- the difference is only whether the answer is sent once or
/// whenever it changes -- and the two builders share no trait of their own, so
/// this is what lets the argument list be written once.
trait RootField: Sized {
    fn with_argument(self, value: InputValue) -> Self;
}

impl RootField for Field {
    fn with_argument(self, value: InputValue) -> Self {
        self.argument(value)
    }
}

impl RootField for SubscriptionField {
    fn with_argument(self, value: InputValue) -> Self {
        self.argument(value)
    }
}

/// The five arguments that narrow a set of rows, in the order Hasura lists
/// them.
fn with_row_arguments<F: RootField>(field: F, type_name: &str) -> F {
    with_row_arguments_named(field, type_name, true)
}

/// The same, where `columns` says whether the table has any to name.
///
/// A table a role may count and not read has none: there is no column enum to
/// order by and none to be distinct on, so those two arguments are not there
/// -- and could not be, since the types they name are not built either. What
/// is left still narrows the set being counted, which is the whole of what
/// such a root is for.
fn with_row_arguments_named<F: RootField>(field: F, type_name: &str, columns: bool) -> F {
    let mut field = field;
    if columns {
        field = field.with_argument(InputValue::new(
            "distinct_on",
            TypeRef::named_nn_list(crate::input::order_by::select_column_type_name(type_name)),
        ));
    }
    field = field
        .with_argument(InputValue::new("limit", TypeRef::named("Int")))
        .with_argument(InputValue::new("offset", TypeRef::named("Int")));
    if columns {
        field = field.with_argument(InputValue::new(
            "order_by",
            TypeRef::named_nn_list(crate::input::order_by::order_by_type_name(type_name)),
        ));
    }
    field.with_argument(InputValue::new(
        "where",
        TypeRef::named(crate::input::bool_exp::bool_exp_type_name(type_name)),
    ))
}

/// One argument per primary key column, named and typed after the column
/// itself rather than assuming an integer `id`.
fn with_key_arguments<F: RootField>(
    field: F,
    schema_name: &str,
    table_name: &str,
    pk_columns: &[(String, String)],
    names: &crate::names::NameOverrides,
) -> F {
    let mut field = field;
    for (column, pg_type) in pk_columns {
        field = field.with_argument(InputValue::new(
            names
                .column(schema_name, table_name, column)
                .unwrap_or(column),
            TypeRef::named_nn(pk_argument_type(pg_type)),
        ));
    }
    field
}

/// Create the Query type with all table query fields.
fn create_query_type(
    generated: &GeneratedSchema,
    max_rows: Option<i64>,
    relationships: Arc<HashMap<String, Vec<RelationshipField>>>,
    names: Arc<crate::names::NameOverrides>,
) -> Object {
    // Collected rather than added as they are built, so they can go on in name
    // order: that is the order Hasura answers introspection in, and it is the
    // order a client diffing two schemas, or a generator writing its types
    // out, gets a stable answer from.
    let mut roots: Vec<(String, Field)> = Vec::new();

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
            call: None,
        });

        let mut gql_field = Field::new(&field.name, return_type, move |ctx| {
            let spec = Arc::clone(&spec);
            FieldFuture::new(async move { resolve_query(&ctx, &spec).await })
        });

        // Add standard query arguments
        gql_field = match is_by_pk {
            false => with_row_arguments(gql_field, &spec_type_name),
            true => with_key_arguments(
                gql_field,
                &field.schema_name,
                &field.table_name,
                &pk_columns,
                names.as_ref(),
            ),
        };

        if let Some(desc) = &field.description {
            gql_field = gql_field.description(desc);
        }

        // A table this role may count and not read has no row type, so the
        // list root that would answer with one is not built. The entry is
        // still here for the aggregate root below, which is what such a role
        // was granted.
        if field.rows {
            roots.push((field.name.clone(), gql_field));
        }

        // The same rows, with numbers about them. Same arguments as the list
        // field, because `author_aggregate(where: ...)` counts the set the
        // filter describes, not the whole table.
        //
        // Not built at all for a role whose permission withholds it: counting
        // rows is a way of learning about them, so it is granted separately
        // from reading them and refused by absence rather than at execution.
        if !is_by_pk && field.aggregates {
            let agg_spec = Arc::new(AggregateSpec {
                schema_name: field.schema_name.clone(),
                table_name: field.table_name.clone(),
                type_name: field.type_name.clone(),
                max_rows,
                relationships: Arc::clone(&relationships),
                names: Arc::clone(&names),
                call: None,
            });
            let aggregate_field_name = field
                .aggregate_name
                .clone()
                .unwrap_or_else(|| crate::schema::aggregate::aggregate_type_name(&field.type_name));
            let aggregate_field_name_for_sorting = aggregate_field_name.clone();
            let mut agg_field = Field::new(
                aggregate_field_name,
                TypeRef::named_nn(crate::schema::aggregate::aggregate_type_name(
                    &field.type_name,
                )),
                move |ctx| {
                    let agg_spec = Arc::clone(&agg_spec);
                    FieldFuture::new(async move { resolve_aggregate(&ctx, &agg_spec).await })
                },
            );
            agg_field = with_row_arguments_named(agg_field, &spec_type_name, field.rows);
            agg_field = match field.aggregate_description.as_deref() {
                Some("") => agg_field,
                Some(given) => agg_field.description(given),
                None => agg_field.description(format!(
                    "fetch aggregated fields from the table: \"{}\"",
                    field.table_name
                )),
            };
            roots.push((aggregate_field_name_for_sorting, agg_field));
        }
    }

    roots.sort_by(|(a, _), (b, _)| a.cmp(b));
    let mut query = Object::new("query_root");
    // A GraphQL object may not have no fields, so a schema that exposes no
    // table needs something on its query root. Hasura puts a placeholder there
    // and calls it this; a client that reaches it has nothing to read.
    if roots.is_empty() {
        return query.field(
            Field::new("no_queries_available", TypeRef::named("String"), |_| {
                FieldFuture::new(async move { Ok(None::<Value>) })
            })
            .description("There are no queries available to the current role."),
        );
    }
    for (_, field) in roots {
        query = query.field(field);
    }

    query
}

/// One document operator: its name, and the type its columns are given.
type JsonbOperator = (&'static str, fn() -> TypeRef);

/// What a document column may be told to do, and what each is told with.
///
/// `_append` and `_prepend` take another document; `_delete_key` takes a key;
/// `_delete_elem` takes an index into an array; `_delete_at_path` takes the
/// path to what is being removed. The shapes are per operator rather than per
/// column, which is why one input type per operator says all of it.
const JSONB_OPERATORS: &[JsonbOperator] = &[
    ("_append", || TypeRef::named("jsonb")),
    ("_delete_at_path", || TypeRef::named_nn_list("String")),
    ("_delete_elem", || TypeRef::named("Int")),
    ("_delete_key", || TypeRef::named("String")),
    ("_prepend", || TypeRef::named("jsonb")),
];

/// The name of the input holding one document operator's columns.
fn jsonb_operator_input(type_name: &str, operator: &str) -> String {
    format!("{}{}_input", type_name, operator)
}

/// Add the operators an update may be written with.
///
/// All optional, and at least one required -- which GraphQL cannot express, so
/// the resolver says so instead of the schema. Making `_set` non-null would
/// have been expressible and wrong: an update that only increments a counter
/// never sends one.
fn with_update_operators(
    field: Field,
    base_name: &str,
    has_numeric: bool,
    has_jsonb: bool,
) -> Field {
    let mut field = field;
    // A table with no document column is not offered the document operators:
    // there is nothing there to append to.
    if has_jsonb {
        for (operator, _) in JSONB_OPERATORS {
            field = field.argument(InputValue::new(
                *operator,
                TypeRef::named(jsonb_operator_input(base_name, operator)),
            ));
        }
    }
    if has_numeric {
        field = field.argument(InputValue::new(
            "_inc",
            TypeRef::named(format!("{}_inc_input", base_name)),
        ));
    }
    field.argument(InputValue::new(
        "_set",
        TypeRef::named(format!("{}_set_input", base_name)),
    ))
}

/// Create the Mutation type with all mutation fields.
/// Which columns a write may name, and so which of its inputs exist.
///
/// One definition, called both where the inputs are registered and where the
/// arguments naming them are declared. They used to agree by coincidence --
/// both read the object type's fields -- and stopped the moment a write input
/// began following its own permission. An argument naming a type nobody
/// registered is a schema async-graphql refuses to build, which costs a role
/// every field it has over the one it cannot use.
fn write_fields<'a>(
    object: &'a crate::schema::object::TableObjectType,
    names: &crate::names::NameOverrides,
    role: Option<&str>,
    verb: crate::role::Verb,
) -> Vec<&'a crate::schema::object::GraphQLField> {
    let allowed = role.and_then(|role| {
        names
            .permissions(&object.table.schema, &object.table.name, role)
            .and_then(|granted| granted.write_columns(verb))
    });
    object
        .writable_fields
        .iter()
        .filter(|field| {
            let column = table_column_for(names, &object.table, &field.name);
            // A column PostgreSQL generates always is not one a write may
            // name. Leaving it in the input type moved the refusal to the
            // database, which answers `cannot insert a non-DEFAULT value into
            // column "id"` and has by then forgotten which argument the value
            // came from. Out of the type, the walk over the request says
            // `field 'id' not found in type: 'author_insert_input'` and can
            // still point at it.
            object
                .table
                .get_column(column)
                .is_some_and(|c| !c.always_generated)
                && allowed.is_none_or(|set| set.allows(column))
        })
        .collect()
}

/// Whether a type has an `_inc_input` and a set of document operator inputs.
fn update_inputs(
    object: &crate::schema::object::TableObjectType,
    names: &crate::names::NameOverrides,
    role: Option<&str>,
) -> (bool, bool) {
    let fields = write_fields(object, names, role, crate::role::Verb::Update);
    (
        fields
            .iter()
            .any(|f| crate::schema::aggregate::is_incrementable(&f.graphql_type)),
        fields
            .iter()
            .any(|f| matches!(&f.graphql_type, crate::types::GraphQLType::Json)),
    )
}

fn create_mutation_type(
    generated: &GeneratedSchema,
    relationships: Arc<HashMap<String, Vec<RelationshipField>>>,
    type_names: Arc<HashMap<(String, String), String>>,
    names: Arc<crate::names::NameOverrides>,
    max_rows: Option<i64>,
    role: Option<&str>,
) -> Object {
    let mut mutation = Object::new("mutation_root");
    // In name order, for the reason the query root is: see there.
    let mut roots: Vec<(String, Field)> = Vec::new();

    // Only a table with a unique constraint has a conflict to name, and only
    // those got the types for it.
    // Which tables have something to add to, and which have a document to
    // operate on. Read from what was registered above rather than worked out
    // again from the object types: the second answer is the one that can
    // drift, and drifting means an argument naming a type that is not there.
    let mut has_numeric_column: HashSet<String> = HashSet::new();
    let mut has_jsonb_column: HashSet<String> = HashSet::new();
    for (type_name, object) in &generated.object_types {
        let (numeric, jsonb) = update_inputs(object, &names, role);
        if numeric {
            has_numeric_column.insert(type_name.clone());
        }
        if jsonb {
            has_jsonb_column.insert(type_name.clone());
        }
    }

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
                    gql_field.argument(InputValue::new("object", TypeRef::named_nn(&insert_input)))
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
                    has_jsonb_column.contains(&where_type),
                );
            }
            MutationType::Update => {
                gql_field = gql_field.argument(InputValue::new(
                    "where",
                    TypeRef::named_nn(crate::input::bool_exp::bool_exp_type_name(&where_type)),
                ));
                gql_field = with_update_operators(
                    gql_field,
                    &where_type,
                    has_numeric_column.contains(&where_type),
                    has_jsonb_column.contains(&where_type),
                );
            }
            MutationType::UpdateMany => {
                gql_field = gql_field.argument(InputValue::new(
                    "updates",
                    TypeRef::named_nn_list_nn(format!("{}_updates", where_type)),
                ));
            }
            MutationType::DeleteByPk => {
                for (col_name, pg_type) in &pk_columns {
                    gql_field = gql_field.argument(InputValue::new(
                        names
                            .column(&field.schema_name, &field.table_name, col_name)
                            .unwrap_or(col_name),
                        TypeRef::named_nn(pk_argument_type(pg_type)),
                    ));
                }
            }
            MutationType::Delete => {
                // Required, as it is on an update: a delete with no predicate
                // is a delete of the whole table, and this refused one at
                // execution while the schema said it was a query worth
                // writing. Saying so in the type is what Hasura does, and it
                // is what a client's own tooling can catch before the request
                // is sent.
                gql_field = gql_field.argument(InputValue::new(
                    "where",
                    TypeRef::named_nn(crate::input::bool_exp::bool_exp_type_name(&where_type)),
                ));
            }
        }

        if let Some(desc) = &field.description {
            gql_field = gql_field.description(desc);
        }

        roots.push((field.name.clone(), gql_field));
    }

    roots.sort_by(|(a, _), (b, _)| a.cmp(b));
    for (_, field) in roots {
        mutation = mutation.field(field);
    }

    mutation
}

/// Create the Subscription type: the query root, answered again on change.
///
/// A subscription here is a **live query**. It carries the same fields the
/// query root does, takes the same arguments, and answers with the same rows
/// -- the difference is that the answer is sent again whenever it stops being
/// true. That is the contract a client generated against Hasura expects, and
/// the reason its `subscription_root` is a mirror of its `query_root`.
///
/// What wakes it is this server's own: a trigger publishes on the changed
/// table and the query is read again, which costs nothing while nothing is
/// written and answers in milliseconds when something is. A trigger cannot
/// see everything a query can -- a view has none, an embedded row may live in
/// a table that carries none, and `where: {expires_at: {_lt: "now()"}}`
/// changes with no write at all -- so a slow refresh runs beside it and
/// closes exactly those gaps. Polling alone is what Hasura does, and it pays
/// for every subscriber every tick whether or not anything happened.
fn create_subscription_type(
    generated: &GeneratedSchema,
    max_rows: Option<i64>,
    relationships: Arc<HashMap<String, Vec<RelationshipField>>>,
    names: Arc<crate::names::NameOverrides>,
    refresh: std::time::Duration,
) -> Subscription {
    let mut roots: Vec<(String, SubscriptionField)> = Vec::new();

    for field in &generated.query_fields {
        let spec_type_name = field.type_name.clone();
        let pk_columns = field.pk_columns.clone();
        let is_by_pk = field.is_by_pk;
        let return_type = graphql_type_ref(&field.return_type);
        let watched = vec![crate::subscription::table_channel_name(
            &field.schema_name,
            &field.table_name,
        )];

        let spec = Arc::new(QueryFieldSpec {
            schema_name: field.schema_name.clone(),
            table_name: field.table_name.clone(),
            type_name: field.type_name.clone(),
            is_by_pk,
            pk_columns: pk_columns.clone(),
            max_rows,
            relationships: Arc::clone(&relationships),
            names: Arc::clone(&names),
            call: None,
        });

        let mut gql_field = SubscriptionField::new(&field.name, return_type, move |ctx| {
            let spec = Arc::clone(&spec);
            let watched = watched.clone();
            SubscriptionFieldFuture::new(async move {
                let wake = wake_on(&ctx, &watched, refresh).await;
                Ok(live(ctx, LiveSource::Rows(spec), wake))
            })
        });

        gql_field = match is_by_pk {
            false => with_row_arguments(gql_field, &spec_type_name),
            true => with_key_arguments(
                gql_field,
                &field.schema_name,
                &field.table_name,
                &pk_columns,
                names.as_ref(),
            ),
        };
        if let Some(desc) = &field.description {
            gql_field = gql_field.description(desc);
        }
        // No row type, no root that answers with one -- see the query root.
        if field.rows {
            roots.push((field.name.clone(), gql_field));
        }

        // A live query mirrors the query root, so an aggregate the role may not
        // ask for once is not one it may ask for continuously either.
        if is_by_pk || !field.aggregates {
            continue;
        }

        // The same rows, with numbers about them, watched the same way.
        let agg_spec = Arc::new(AggregateSpec {
            schema_name: field.schema_name.clone(),
            table_name: field.table_name.clone(),
            type_name: field.type_name.clone(),
            max_rows,
            relationships: Arc::clone(&relationships),
            names: Arc::clone(&names),
            call: None,
        });
        let agg_watched = vec![crate::subscription::table_channel_name(
            &field.schema_name,
            &field.table_name,
        )];
        let agg_name = field
            .aggregate_name
            .clone()
            .unwrap_or_else(|| crate::schema::aggregate::aggregate_type_name(&field.type_name));
        let mut agg_field = SubscriptionField::new(
            agg_name.clone(),
            TypeRef::named_nn(crate::schema::aggregate::aggregate_type_name(
                &field.type_name,
            )),
            move |ctx| {
                let agg_spec = Arc::clone(&agg_spec);
                let watched = agg_watched.clone();
                SubscriptionFieldFuture::new(async move {
                    let wake = wake_on(&ctx, &watched, refresh).await;
                    Ok(live(ctx, LiveSource::Aggregate(agg_spec), wake))
                })
            },
        );
        agg_field = with_row_arguments_named(agg_field, &spec_type_name, field.rows);
        agg_field = match field.aggregate_description.as_deref() {
            Some("") => agg_field,
            Some(given) => agg_field.description(given),
            None => agg_field.description(format!(
                "fetch aggregated fields from the table: \"{}\"",
                field.table_name
            )),
        };
        roots.push((agg_name, agg_field));
    }

    roots.sort_by(|(a, _), (b, _)| a.cmp(b));
    let mut subscription = Subscription::new("subscription_root");
    // A GraphQL object may not have no fields, and a schema that exposes no
    // table still has a subscription root if subscriptions are on.
    if roots.is_empty() {
        return subscription.field(SubscriptionField::new(
            "no_subscriptions_available",
            TypeRef::named("String"),
            |_| {
                SubscriptionFieldFuture::new(async move {
                    Ok(futures::stream::empty::<
                        Result<FieldValue, async_graphql::Error>,
                    >())
                })
            },
        ));
    }
    for (_, field) in roots {
        subscription = subscription.field(field);
    }
    subscription
}

/// What tells a live query its answer may have changed.
///
/// Every notification on the tables it reads, and a slow tick beside them.
/// Yields once per wake and never ends -- a subscription lasts as long as the
/// client holds it open.
async fn wake_on(
    ctx: &ResolverContext<'_>,
    channels: &[String],
    refresh: std::time::Duration,
) -> futures::stream::BoxStream<'static, ()> {
    use futures::StreamExt;

    let mut streams: Vec<futures::stream::BoxStream<'static, ()>> =
        Vec::with_capacity(channels.len() + 1);
    // A tick even where nothing is listening: a table with no trigger, a view,
    // and a predicate that changes with the clock all depend on it.
    streams.push(
        tokio_stream::wrappers::IntervalStream::new({
            let mut interval = tokio::time::interval(refresh);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            interval
        })
        .map(|_| ())
        .boxed(),
    );

    if let Ok(broker_arc) = ctx.data::<Arc<RwLock<Option<NotifyBroker>>>>() {
        let guard = broker_arc.read().await;
        if let Some(broker) = guard.as_ref() {
            for channel in channels {
                // `subscribe_or_create` rather than `subscribe`: a table whose
                // trigger was never installed has no channel, and a live query
                // on it is answered by the refresh rather than refused.
                streams.push(
                    broker
                        .subscribe_or_create(channel)
                        .await
                        .map(|_| ())
                        .boxed(),
                );
            }
        }
    }

    futures::stream::select_all(streams).boxed()
}

/// What a live query reads each time it is woken.
///
/// An enum rather than a closure because the answer borrows the context it is
/// read from, and a closure returning a future that borrows its argument is
/// not something a bound can say.
enum LiveSource {
    Rows(Arc<QueryFieldSpec>),
    Aggregate(Arc<AggregateSpec>),
}

/// Read one answer, and the value to compare it against the last one by.
async fn read_live<'a>(
    ctx: &ResolverContext<'a>,
    source: &LiveSource,
) -> Result<(serde_json::Value, Option<FieldValue<'a>>), async_graphql::Error> {
    match source {
        LiveSource::Rows(spec) => {
            let rows = query_rows(ctx, spec).await?;
            Ok((
                serde_json::Value::Array(rows.clone()),
                rows_as_field_value(rows, spec.is_by_pk),
            ))
        }
        LiveSource::Aggregate(spec) => {
            let value = aggregate_value(ctx, spec).await?;
            let seen = serde_json::to_value(&value).unwrap_or_default();
            Ok((seen, Some(FieldValue::value(value))))
        }
    }
}

/// A live query: the answer now, and the answer again each time it changes.
///
/// The first item is sent as soon as the client subscribes, which is what
/// makes this a query rather than a feed -- there is no window in which the
/// client knows nothing. After that, `answer` is read again on every wake and
/// sent only when what came back is not what was sent last: a write that
/// changes a row the subscription does not select is a wake and not a message.
fn live<'a>(
    ctx: ResolverContext<'a>,
    source: LiveSource,
    wake: futures::stream::BoxStream<'static, ()>,
) -> impl futures::Stream<Item = Result<FieldValue<'a>, async_graphql::Error>> + Send + 'a {
    use futures::StreamExt;

    struct Live<'a> {
        ctx: ResolverContext<'a>,
        source: LiveSource,
        wake: futures::stream::BoxStream<'static, ()>,
        sent: Option<serde_json::Value>,
        started: bool,
    }

    futures::stream::unfold(
        Live {
            ctx,
            source,
            wake,
            sent: None,
            started: false,
        },
        |mut live: Live<'a>| async move {
            loop {
                if live.started {
                    live.wake.next().await?;
                } else {
                    live.started = true;
                }
                let read = read_live(&live.ctx, &live.source).await;
                match read {
                    // An error ends the subscription: the client asked
                    // something that cannot be answered, and answering it
                    // again on the next tick would only repeat that.
                    Err(e) => return Some((Err(e), live)),
                    Ok((seen, value)) => {
                        if live.sent.as_ref() == Some(&seen) {
                            continue;
                        }
                        live.sent = Some(seen);
                        let value = value.unwrap_or_else(|| FieldValue::value(Value::Null));
                        return Some((Ok(value), live));
                    }
                }
            }
        },
    )
    .take_while(|item| futures::future::ready(item.is_ok()))
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
    /// Set where the rows come from a function rather than from the table
    /// itself. Everything else about reading them is the same.
    call: Option<Arc<FunctionCall>>,
}

/// Add the root fields for functions that answer with rows of a table.
///
/// The same arguments a table's own root field takes, because the rows are the
/// same rows: what the function adds is `args`, which is where its own
/// arguments go so that a parameter called `limit` cannot shadow the one that
/// pages the result.
fn add_function_fields(
    mut root: Object,
    generated: &GeneratedSchema,
    volatile: bool,
    max_rows: Option<i64>,
    relationships: Arc<HashMap<String, Vec<RelationshipField>>>,
    names: Arc<crate::names::NameOverrides>,
) -> Object {
    for function in &generated.function_fields {
        if function.volatile != volatile {
            continue;
        }
        let spec = Arc::new(QueryFieldSpec {
            schema_name: function.returns_table.0.clone(),
            table_name: function.returns_table.1.clone(),
            type_name: function.returns.clone(),
            is_by_pk: false,
            pk_columns: Vec::new(),
            max_rows,
            relationships: Arc::clone(&relationships),
            names: Arc::clone(&names),
            call: Some(Arc::new(FunctionCall {
                schema: function.schema_name.clone(),
                name: function.function_name.clone(),
                arguments: function.arguments.clone(),
                session_argument: function.session_argument.clone(),
            })),
        });

        let mut field = Field::new(
            &function.name,
            TypeRef::named_nn_list_nn(&function.returns),
            move |ctx| {
                let spec = Arc::clone(&spec);
                FieldFuture::new(async move { resolve_query(&ctx, &spec).await })
            },
        );
        if !function.arguments.is_empty() {
            field = field.argument(InputValue::new(
                "args",
                TypeRef::named_nn(format!("{}_args", function.name)),
            ));
        }
        field = field
            .argument(InputValue::new(
                "where",
                TypeRef::named(crate::input::bool_exp::bool_exp_type_name(
                    &function.returns,
                )),
            ))
            .argument(InputValue::new(
                "order_by",
                TypeRef::named_nn_list(crate::input::order_by::order_by_type_name(
                    &function.returns,
                )),
            ))
            .argument(InputValue::new(
                "distinct_on",
                TypeRef::named_nn_list(crate::input::order_by::select_column_type_name(
                    &function.returns,
                )),
            ))
            .argument(InputValue::new("limit", TypeRef::named("Int")))
            .argument(InputValue::new("offset", TypeRef::named("Int")));
        if let Some(description) = &function.description {
            field = field.description(description);
        }
        root = root.field(field);

        // The same rows, with numbers about them. Every table root has one and
        // so does every function root: `search_tracks_aggregate(args: {…})`
        // counts what `search_tracks(args: {…})` would have answered with,
        // which means calling the function and taking the same arguments.
        let agg_spec = Arc::new(AggregateSpec {
            schema_name: function.returns_table.0.clone(),
            table_name: function.returns_table.1.clone(),
            type_name: function.returns.clone(),
            max_rows,
            relationships: Arc::clone(&relationships),
            names: Arc::clone(&names),
            call: Some(Arc::new(FunctionCall {
                schema: function.schema_name.clone(),
                name: function.function_name.clone(),
                arguments: function.arguments.clone(),
                session_argument: function.session_argument.clone(),
            })),
        });
        let mut aggregate = Field::new(
            format!("{}_aggregate", function.name),
            TypeRef::named_nn(crate::schema::aggregate::aggregate_type_name(
                &function.returns,
            )),
            move |ctx| {
                let agg_spec = Arc::clone(&agg_spec);
                FieldFuture::new(async move { resolve_aggregate(&ctx, &agg_spec).await })
            },
        );
        if !function.arguments.is_empty() {
            aggregate = aggregate.argument(InputValue::new(
                "args",
                TypeRef::named_nn(format!("{}_args", function.name)),
            ));
        }
        aggregate = with_row_arguments(aggregate, &function.returns).description(format!(
            "fetch aggregated fields from the table: \"{}\"",
            function.returns_table.1
        ));
        root = root.field(aggregate);
    }
    root
}

/// The FROM entry for a root field that calls a function.
///
/// Aliased as the table the function answers with rows of, so a filter or an
/// ordering written against that table's columns finds them: without it the
/// FROM clause names the function and `article.score` is a table nobody
/// mentioned. Whatever the caller put under `args` is bound here, which is why
/// it takes the parameter list.
fn function_source(
    ctx: &ResolverContext<'_>,
    call: &FunctionCall,
    alias: &str,
    bound_values: &mut Vec<serde_json::Value>,
) -> Result<String, async_graphql::Error> {
    let given = ctx
        .args
        .try_get("args")
        .ok()
        .map(|v| accessor_to_json(&v))
        .unwrap_or_else(|| serde_json::json!({}));
    let mut passed = Vec::new();
    for (name, _, required) in &call.arguments {
        match given.get(name) {
            None if *required => {
                return Err(async_graphql::Error::new(format!(
                    "{} needs the argument \"{}\"",
                    call.name, name
                )))
            }
            // An argument left out is left out, so the function's own default
            // applies. Passing null instead would override it.
            None => continue,
            Some(value) => {
                // Named notation, so the arguments a client did give land on
                // the parameters it meant regardless of order. A GraphQL Int
                // binds as a bigint, and `f(article_id => bigint)` is not
                // `f(integer)` -- the function is looked up by the types of
                // what it was handed, so what it was handed has to be the
                // types it declares.
                let (_, pg_type, _) = call
                    .arguments
                    .iter()
                    .find(|(argument, _, _)| argument == name)
                    .expect("iterating the arguments");
                passed.push(format!(
                    "{} => ${}::{}",
                    postrust_sql::escape_ident(name),
                    bound_values.len() + 1,
                    pg_type
                ));
                bound_values.push(value.clone());
            }
        }
    }
    // The session argument is not the client's to send: it says who is
    // asking, and a caller that could write it could name any identity it
    // liked. It is filled here, from the verified token.
    if let Some(session) = &call.session_argument {
        passed.push(format!(
            "{} => {}",
            postrust_sql::escape_ident(session),
            SESSION_ARGUMENT
        ));
    }
    Ok(format!(
        "{}.{}({}) AS {}",
        postrust_sql::escape_ident(&call.schema),
        postrust_sql::escape_ident(&call.name),
        passed.join(", "),
        postrust_sql::escape_ident(alias)
    ))
}

/// A function a root field calls instead of reading a table.
struct FunctionCall {
    schema: String,
    name: String,
    arguments: Vec<(String, String, bool)>,
    /// The parameter filled from the caller's session rather than from the
    /// request. See [`crate::schema::FunctionField::session_argument`].
    session_argument: Option<String>,
}

/// Everything an aggregate field's resolver needs.
struct AggregateSpec {
    schema_name: String,
    table_name: String,
    type_name: String,
    max_rows: Option<i64>,
    relationships: Arc<HashMap<String, Vec<RelationshipField>>>,
    names: Arc<crate::names::NameOverrides>,
    /// Set where the rows come from a function rather than from the table
    /// itself, exactly as it is on [`QueryFieldSpec`]. Counting the rows a
    /// function answers with means calling it.
    call: Option<Arc<FunctionCall>>,
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
    Ok(Some(FieldValue::value(aggregate_value(ctx, spec).await?)))
}

/// The numbers an aggregate field answers with.
///
/// Split from the resolver for the reason [`query_rows`] is: a subscription
/// reads them over and over and has to tell one answer from the last.
async fn aggregate_value(
    ctx: &ResolverContext<'_>,
    spec: &AggregateSpec,
) -> Result<Value, async_graphql::Error> {
    use crate::schema::aggregate as agg;

    let pool = ctx.data::<PgPool>()?;
    let gql_ctx = ctx.data::<GraphQLContext>()?;

    let mut bound_values: Vec<serde_json::Value> = Vec::new();
    // A function's own arguments are bound before anything else, so a
    // predicate written beside them continues the numbering rather than
    // colliding with it.
    let source = match &spec.call {
        None => format!(
            "{}.{}",
            postrust_sql::escape_ident(&spec.schema_name),
            postrust_sql::escape_ident(&spec.table_name)
        ),
        Some(call) => function_source(ctx, call, &spec.table_name, &mut bound_values)?,
    };

    let mut where_sql = String::new();
    {
        // Counting rows is reading them, so the permission applies here in the
        // same breath as the request's own filter -- otherwise `count` would
        // report a total the same role cannot list.
        let requested = ctx.args.try_get("where").ok().map(|v| accessor_to_json(&v));
        let permission = permission_predicate(
            &gql_ctx.caller(),
            spec.names.as_ref(),
            &spec.schema_name,
            &spec.table_name,
        )?;
        if let Some(predicate) = and_predicates(requested, permission) {
            let guard = gql_ctx
                .schema_cache
                .get()
                .await
                .map_err(|e| async_graphql::Error::new(e.to_string()))?;
            let cache = guard
                .as_ref()
                .ok_or_else(|| async_graphql::Error::new("schema cache is not loaded"))?;
            let scope = WhereScope::table(
                &spec.schema_name,
                &spec.table_name,
                &spec.type_name,
                spec.names.as_ref(),
            )
            .with_resolution(cache, spec.relationships.as_ref())
            .for_caller(gql_ctx.caller());
            let (sql, values) =
                build_where_clause(Some(&predicate), bound_values.len() + 1, &scope)?;
            if !sql.is_empty() {
                where_sql = format!(" {}", sql);
                bound_values.extend(values);
            }
        }
    }

    let order_sql = build_order_by_clause(
        ctx,
        &gql_ctx.schema_cache,
        &spec.schema_name,
        &spec.table_name,
        &spec.type_name,
        spec.relationships.as_ref(),
        &format!(
            "{}.{}",
            postrust_sql::escape_ident(&spec.schema_name),
            postrust_sql::escape_ident(&spec.table_name)
        ),
        spec.names.as_ref(),
    )
    .await?;

    let requested_limit = ctx.args.try_get("limit").ok().and_then(|v| v.i64().ok());
    let offset = ctx.args.try_get("offset").ok().and_then(|v| v.i64().ok());
    let ceiling = match (
        spec.max_rows,
        permission_limit(
            &gql_ctx.caller(),
            spec.names.as_ref(),
            &spec.schema_name,
            &spec.table_name,
        ),
    ) {
        (Some(configured), Some(granted)) => Some(configured.min(granted)),
        (only, None) | (None, only) => only,
    };
    let limit = match (requested_limit, ceiling) {
        (Some(requested), Some(ceiling)) => Some(requested.min(ceiling)),
        (Some(requested), None) => Some(requested),
        (None, ceiling) => ceiling,
    };

    let rows = format!("SELECT * FROM {}{}{}", source, where_sql, order_sql);

    // What `nodes` reads: the page, bounded by whichever ceiling is lowest.
    let mut inner = rows.clone();
    if let Some(limit) = limit {
        inner.push_str(&format!(" LIMIT {}", limit));
    }
    if let Some(offset) = offset {
        inner.push_str(&format!(" OFFSET {}", offset));
    }

    // What `aggregate` reads, which is not the same rows. A count is not a
    // page: a ceiling exists to bound how many rows travel, and answering
    // "how many are there" with "as many as I would have sent you" is not an
    // answer to the question. Hasura says so too -- a role limited to one row
    // of `article` still counts three of them and still gets `max(id)` over
    // all three, which is what `agg_perm` tests.
    //
    // The request's own `limit` and `offset` do still apply: those were asked
    // for, and `article_aggregate(limit: 2)` is a question about two rows. The
    // corpus proves this only for the permission's ceiling; the configured one
    // is treated the same way because the reason is the same, and because a
    // server whose `PGRST_MAX_ROWS` silently became the answer to `count`
    // would be wrong in the way that is hardest to notice.
    let mut counted = rows;
    if let Some(limit) = requested_limit {
        counted.push_str(&format!(" LIMIT {}", limit));
    }
    if let Some(offset) = offset {
        counted.push_str(&format!(" OFFSET {}", offset));
    }

    // What the client actually asked for. Two selections of one aggregate are
    // one entry: `sum { id }` beside `totals: sum { views }` reads the same
    // object under two names, so the columns are the union of what both named
    // and the object is built once.
    let mut wants_nodes = false;
    let mut wanted: BTreeMap<String, Vec<String>> = BTreeMap::new();
    // `count` is different: each occurrence may count something else, so each
    // is answered under the name it was asked for.
    let mut counts: Vec<(String, String)> = Vec::new();
    for selection in ctx.field().selection_set() {
        match selection.name() {
            "nodes" => wants_nodes = true,
            "aggregate" => {
                for function in selection.selection_set() {
                    if function.name() == "count" {
                        counts.push((
                            function.alias().unwrap_or("count").to_string(),
                            count_expression(
                                function,
                                (&spec.schema_name, &spec.table_name),
                                None,
                                spec.names.as_ref(),
                            ),
                        ));
                        continue;
                    }
                    let columns = wanted.entry(function.name().to_string()).or_default();
                    for column in function.selection_set() {
                        let name = column.name().to_string();
                        if !columns.contains(&name) {
                            columns.push(name);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    wanted.retain(|_, columns| !columns.is_empty());

    let mut result = async_graphql::indexmap::IndexMap::new();

    if !counts.is_empty() || !wanted.is_empty() {
        // `count` is answered whether or not it was asked for: the aggregate
        // type's own resolver reads a missing key as zero, and a client that
        // asked for nothing else still gets a number rather than a null.
        let mut parts = match counts.iter().any(|(key, _)| key == "count") {
            true => Vec::new(),
            false => vec!["'count', count(*)".to_string()],
        };
        for (key, expression) in &counts {
            parts.push(format!("'{}', {}", key.replace('\'', "''"), expression));
        }
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
                .map(|field| {
                    let source = spec
                        .names
                        .column_source(&spec.schema_name, &spec.table_name, field)
                        .unwrap_or(field);
                    format!(
                        "'{}', {}({})",
                        field.replace('\'', "''"),
                        sql_function,
                        postrust_sql::escape_ident(source)
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
            counted
        );
        let mut conn =
            begin_with_session(pool, gql_ctx.role(), &gql_ctx.session_settings()).await?;
        let rows = execute_query_on(&mut conn, &sql, &bound_values).await?;
        conn.commit().await?;
        if let Some(first) = rows.into_iter().next() {
            if let Value::Object(map) = json_to_value(first) {
                result.insert(async_graphql::Name::new("aggregate"), Value::Object(map));
            }
        }
    }

    if wants_nodes {
        // `nodes` is the same rows selection the plain root field answers, so
        // it gets the same projection: computed fields, which are functions of
        // the row rather than part of it, and relationships, which are
        // correlated subselects. Reading it as columns alone was why a
        // relationship asked for beside a count came back null.
        let nodes = ctx
            .field()
            .selection_set()
            .find(|selection| selection.name() == "nodes")
            .ok_or_else(|| async_graphql::Error::new("nodes was asked for and then was not"))?;

        // The aggregate query has already been sent, so its parameters are
        // fixed; anything an embed binds continues that numbering in a vector
        // of its own.
        let mut node_values = bound_values.clone();
        let (projection, row_column_types) = {
            let guard = gql_ctx
                .schema_cache
                .get()
                .await
                .map_err(|e| async_graphql::Error::new(e.to_string()))?;
            let cache = guard
                .as_ref()
                .ok_or_else(|| async_graphql::Error::new("schema cache is not loaded"))?;
            let qi = postrust_core::api_request::QualifiedIdentifier::new(
                &spec.schema_name,
                &spec.table_name,
            );
            let table = cache.get_table(&qi);
            let mut param_idx = node_values.len() + 1;
            let computed = match table {
                Some(table) => computed_projections(
                    table,
                    nodes,
                    "src",
                    spec.names.as_ref(),
                    cache,
                    &mut param_idx,
                    &mut node_values,
                )?,
                None => Vec::new(),
            };
            let embeds = build_embed_expressions(
                &gql_ctx.caller(),
                cache,
                spec.relationships.as_ref(),
                &spec.type_name,
                "src",
                nodes,
                spec.max_rows,
                &mut 0,
                &mut param_idx,
                &mut node_values,
                spec.names.as_ref(),
            )?;
            // A renamed column is renamed here, in the projection over the
            // subquery -- not inside it, where `src` has to stay the table's
            // own composite for a computed field to be passed the row.
            let mut projection = table
                .and_then(|table| rename_projection(table, "src", spec.names.as_ref()))
                .unwrap_or_else(|| "src.*".to_string());
            for expression in &computed {
                projection.push_str(", ");
                projection.push_str(expression);
            }
            for (field_name, expression) in &embeds {
                projection.push_str(", ");
                projection.push_str(expression);
                projection.push_str(" AS ");
                projection.push_str(&postrust_sql::escape_ident(field_name));
            }
            (
                projection,
                table
                    .map(|table| exposed_column_types(table, spec.names.as_ref()))
                    .unwrap_or_default(),
            )
        };

        let sql = format!(
            "SELECT {} FROM (SELECT {} FROM ({}) AS src) AS pgrst_nodes",
            row_json("pgrst_nodes", &row_column_types),
            projection,
            inner
        );
        let mut conn =
            begin_with_session(pool, gql_ctx.role(), &gql_ctx.session_settings()).await?;
        let rows = execute_query_on(&mut conn, &sql, &node_values).await?;
        conn.commit().await?;
        result.insert(
            async_graphql::Name::new("nodes"),
            Value::List(rows.into_iter().map(json_to_value).collect()),
        );
    }

    let _ = &spec.type_name;
    Ok(Value::Object(result))
}

/// Resolve a query field.
async fn resolve_query<'a>(
    ctx: &ResolverContext<'a>,
    spec: &QueryFieldSpec,
) -> Result<Option<FieldValue<'a>>, async_graphql::Error> {
    Ok(rows_as_field_value(
        query_rows(ctx, spec).await?,
        spec.is_by_pk,
    ))
}

/// The rows a query field answers with, as JSON.
///
/// Split from the field's own resolver because a subscription reads the same
/// rows over and over and has to tell one answer from the last: a
/// `FieldValue` is neither comparable nor readable, and this is.
async fn query_rows(
    ctx: &ResolverContext<'_>,
    spec: &QueryFieldSpec,
) -> Result<Vec<serde_json::Value>, async_graphql::Error> {
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
        // The permission's ceiling is one more of the same kind, so it folds
        // in the same way: whichever of the three is smallest wins, and a
        // request naming none takes the lowest that was named.
        let ceiling = match (
            max_rows,
            permission_limit(
                &gql_ctx.caller(),
                spec.names.as_ref(),
                schema_name,
                table_name,
            ),
        ) {
            (Some(configured), Some(granted)) => Some(configured.min(granted)),
            (only, None) | (None, only) => only,
        };
        match (requested_limit, ceiling) {
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

    // Where the rows come from. A function's own arguments are bound before
    // anything else, because they sit in the FROM clause and every other
    // parameter is numbered after them.
    let source = match &spec.call {
        // Aliased rather than named. A whole-row reference -- what a computed
        // field is passed -- can only be written as a bare name, and
        // `"public"."author"` is not one: PostgreSQL reads it as a column of a
        // table called `public`. An alias no column can share is a name that
        // works in both positions.
        None => format!(
            "{}.{} AS {}",
            postrust_sql::escape_ident(schema_name),
            postrust_sql::escape_ident(table_name),
            postrust_sql::escape_ident(READ_ROW)
        ),
        Some(call) => function_source(ctx, call, table_name, &mut bound_values)?,
    };

    // How a column of the source is referred to: a table by its qualified
    // name, a function's rows by the alias above.
    let source_ref = match &spec.call {
        None => postrust_sql::escape_ident(READ_ROW),
        Some(_) => postrust_sql::escape_ident(table_name),
    };

    if is_by_pk {
        if pk_columns.is_empty() {
            return Err(async_graphql::Error::new(format!(
                "\"{}\" has no primary key, so it cannot be queried by key",
                table_name
            )));
        }

        let mut conditions = Vec::with_capacity(pk_columns.len());
        for (idx, (col_name, pg_type)) in pk_columns.iter().enumerate() {
            let field = spec
                .names
                .column(schema_name, table_name, col_name)
                .unwrap_or(col_name);
            let value = ctx.args.try_get(field).map_err(|_| {
                async_graphql::Error::new(format!(
                    "missing required primary key argument \"{}\"",
                    field
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
    }

    // What this role may read, beside what the request asked for. Applied to
    // both shapes and to neither: a query naming no `where` at all is still a
    // read, and a by-key query that skipped the permission would be the one
    // way to fetch a row the filter withholds.
    {
        let requested = match is_by_pk {
            true => None,
            false => ctx.args.try_get("where").ok().map(|v| accessor_to_json(&v)),
        };
        let permission = permission_predicate(
            &gql_ctx.caller(),
            spec.names.as_ref(),
            schema_name,
            table_name,
        )?;

        if let Some(predicate) = and_predicates(requested, permission) {
            let guard = gql_ctx
                .schema_cache
                .get()
                .await
                .map_err(|e| async_graphql::Error::new(e.to_string()))?;
            let cache = guard
                .as_ref()
                .ok_or_else(|| async_graphql::Error::new("schema cache is not loaded"))?;
            let scope = match &spec.call {
                None => WhereScope::table(schema_name, table_name, type_name, spec.names.as_ref())
                    .under_alias(READ_ROW)
                    .with_resolution(cache, relationships)
                    .for_caller(gql_ctx.caller()),
                // A function's rows are the table's rows, so they are renamed
                // the same way; only how they are reached differs.
                Some(_) => WhereScope::for_alias(
                    schema_name,
                    table_name,
                    table_name,
                    type_name,
                    cache,
                    relationships,
                    spec.names.as_ref(),
                )
                .for_caller(gql_ctx.caller()),
            };
            let (filter_sql, filter_values) =
                build_where_clause(Some(&predicate), bound_values.len() + 1, &scope)?;
            if !filter_sql.is_empty() {
                // A by-key query already has a `WHERE`, so this joins it
                // rather than replacing it.
                where_sql = match where_sql.is_empty() {
                    true => format!(" {}", filter_sql),
                    false => format!(
                        "{} AND ({})",
                        where_sql,
                        filter_sql.trim_start_matches("WHERE ")
                    ),
                };
                bound_values.extend(filter_values);
            }
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
                &source_ref,
                spec.names.as_ref(),
            )
            .await?,
            build_distinct_on(
                ctx,
                &gql_ctx.schema_cache,
                schema_name,
                table_name,
                spec.names.as_ref(),
            )
            .await?,
        )
    };

    // PostgreSQL keeps the first row of each DISTINCT ON group in the query's
    // own order, and picks arbitrarily where the ordering does not begin with
    // the distinct columns -- so which row survives would depend on the plan.
    // Hasura refuses that query rather than answering it, and so does this:
    // prepending the distinct columns instead produced an answer, and a wrong
    // one, since `ORDER BY "department", "department" DESC` is decided by its
    // first term and sorts ascending.
    let (distinct_sql, order_sql) = {
        let written = order_sql.strip_prefix(" ORDER BY ");
        // Reported against the field's arguments as a whole: the rule is about
        // two of them disagreeing, so neither one is where it went wrong.
        let (prefix, order) = distinct_on_clause(&distinct_on, written).map_err(|error| {
            at_path(
                error,
                &format!(
                    "$.selectionSet.{}.args",
                    ctx.ctx.field().alias().unwrap_or(ctx.ctx.field().name())
                ),
            )
        })?;
        (
            prefix,
            match order {
                Some(order) => format!(" ORDER BY {}", order),
                None => String::new(),
            },
        )
    };

    // A computed column is a function of the row rather than part of it, so
    // it is not in `*` and is named only when it was asked for.
    let (computed, row_column_types) = {
        let guard = gql_ctx
            .schema_cache
            .get()
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;
        match guard.as_ref() {
            Some(cache) => {
                let qi =
                    postrust_core::api_request::QualifiedIdentifier::new(schema_name, table_name);
                match cache.get_table(&qi) {
                    Some(table) => {
                        let mut param_idx = bound_values.len() + 1;
                        (
                            computed_projections(
                                table,
                                ctx.field(),
                                "src",
                                spec.names.as_ref(),
                                cache,
                                &mut param_idx,
                                &mut bound_values,
                            )?,
                            exposed_column_types(table, spec.names.as_ref()),
                        )
                    }
                    None => (Vec::new(), HashMap::new()),
                }
            }
            None => (Vec::new(), HashMap::new()),
        }
    };
    // ORDER BY, LIMIT and OFFSET belong inside the subquery: applying them to
    // the outer `row_to_json` projection would leave the ordering of the rows
    // that survive the limit unspecified.
    let mut inner = format!(
        "SELECT {}* FROM {}{}{}",
        distinct_sql, source, where_sql, order_sql
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
                    &gql_ctx.caller(),
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
    let renamed = {
        let guard = gql_ctx
            .schema_cache
            .get()
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;
        guard
            .as_ref()
            .and_then(|cache| {
                cache.get_table(&postrust_core::api_request::QualifiedIdentifier::new(
                    schema_name,
                    table_name,
                ))
            })
            .and_then(|table| rename_projection(table, "src", spec.names.as_ref()))
    };
    let inner = if embed_expressions.is_empty() && computed.is_empty() && renamed.is_none() {
        inner
    } else {
        let mut projection = renamed.unwrap_or_else(|| "src.*".to_string());
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

    let sql = format!(
        "SELECT {} FROM ({}) t",
        row_json("t", &row_column_types),
        inner
    );

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

    Ok(result)
}

/// The rows a field answers with, in the shape its type says.
///
/// A by-key field is one row or none; every other is a list, empty where
/// nothing matched rather than null.
fn rows_as_field_value<'a>(rows: Vec<serde_json::Value>, is_by_pk: bool) -> Option<FieldValue<'a>> {
    match is_by_pk {
        true => rows
            .into_iter()
            .next()
            .map(|v| FieldValue::value(json_to_value(v))),
        false => Some(FieldValue::list(
            rows.into_iter()
                .map(|v| FieldValue::value(json_to_value(v))),
        )),
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

    // A relationship or a computed field asked for beside the written columns
    // is not in `RETURNING`, so the rows are read again through the projection
    // an ordinary query uses -- and only when the selection asks for something
    // that needs it. A delete cannot do that afterwards and builds it into the
    // statement instead, which is why this is settled before the statement is.
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

    // Several updates, each with its own filter, applied in the order given.
    // They share the operation's transaction like any other write, which is
    // the whole difference from sending them one at a time: either all of them
    // happened or none did.
    if mutation_type == MutationType::UpdateMany {
        let updates = ctx
            .args
            .try_get("updates")
            .ok()
            .map(|v| accessor_to_json(&v))
            .unwrap_or(serde_json::Value::Null);
        let serde_json::Value::Array(entries) = updates else {
            return Err(async_graphql::Error::new(
                "\"updates\" takes a list of {where, _set, …} objects",
            ));
        };
        let column_types = column_types_of(&gql_ctx.schema_cache, schema_name, table_name).await;
        let mut answers: Vec<FieldValue<'_>> = Vec::with_capacity(entries.len());
        for entry in entries {
            let serde_json::Value::Object(entry) = entry else {
                return Err(async_graphql::Error::new(
                    "each update is an object of {where, _set, …}",
                ));
            };
            let operators: Vec<(&'static str, serde_json::Value)> = UPDATE_OPERATORS
                .iter()
                .filter_map(|name| entry.get(*name).map(|value| (*name, value.clone())))
                .filter(|(_, value)| !value.is_null())
                .collect();
            let rows = execute_update(
                pool,
                gql_ctx,
                schema_name,
                table_name,
                &type_name,
                operators,
                column_types.clone(),
                entry.get("where").cloned(),
                relationships.as_ref(),
                names.as_ref(),
            )
            .await?;
            let affected = rows.len();
            let rows = match returning {
                Some(returning) if !rows.is_empty() => {
                    reread_returning(
                        pool,
                        gql_ctx,
                        schema_name,
                        table_name,
                        &type_name,
                        rows,
                        returning,
                        relationships.as_ref(),
                        names.as_ref(),
                        max_rows,
                    )
                    .await?
                }
                _ => rows,
            };
            answers.extend(mutation_result(rows, affected, false));
        }
        return Ok(Some(FieldValue::list(answers)));
    }

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
            // Where this write is, in the document the client sent. The
            // field as it was written -- an alias if there was one, since that
            // is what names this selection.
            let here = format!(
                "$.selectionSet.{}",
                ctx.ctx.field().alias().unwrap_or(ctx.ctx.field().name())
            );
            // `insert_x(objects: [...])` against `insert_x_one(object: {...})`:
            // the same write under two spellings, and the path says which.
            let named = match mutation_type {
                MutationType::InsertOne => "object",
                _ => "objects",
            };
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
                names: names.as_ref(),
                caller: gql_ctx.caller(),
                // Filled in per row by `execute_insert`, which is where the
                // index is known.
                row: String::new(),
                objects: format!("{}.args.{}", here, named),
                conflict: format!("{}.args.on_conflict", here),
            };

            execute_insert(
                pool,
                gql_ctx,
                schema_name,
                table_name,
                objects,
                &context,
                mutation_type == MutationType::InsertOne,
            )
            .await?
        }
        // `UpdateMany` answered above: it is a list of responses rather than
        // one, so it does not share this shape.
        MutationType::UpdateMany | MutationType::Update | MutationType::UpdateByPk => {
            // `_set` replaces, the others read the column they write. A client
            // may send more than one, so all of them are collected rather than
            // the first that happens to be present.
            let operators: Vec<(&'static str, serde_json::Value)> = UPDATE_OPERATORS
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
                Some(pk_where_from_args(
                    ctx,
                    schema_name,
                    table_name,
                    pk_columns,
                    names.as_ref(),
                )?)
            } else {
                ctx.args.try_get("where").ok().map(|v| accessor_to_json(&v))
            };
            // Which rows this role may change, beside which rows the request
            // asked to change. A by-key update is narrowed too: naming the key
            // of a row the permission withholds must not be a way to reach it.
            let where_clause = and_predicates(
                where_clause,
                permission_filter(
                    &gql_ctx.caller(),
                    names.as_ref(),
                    schema_name,
                    table_name,
                    crate::role::Verb::Update,
                )?,
            );

            execute_update(
                pool,
                gql_ctx,
                schema_name,
                table_name,
                &type_name,
                operators,
                column_types_of(&gql_ctx.schema_cache, schema_name, table_name).await,
                where_clause,
                relationships.as_ref(),
                names.as_ref(),
            )
            .await
            .map(|rows| {
                let count = rows.len();
                (rows, count)
            })?
        }
        MutationType::Delete | MutationType::DeleteByPk => {
            let where_clause = if mutation_type == MutationType::DeleteByPk {
                Some(pk_where_from_args(
                    ctx,
                    schema_name,
                    table_name,
                    pk_columns,
                    names.as_ref(),
                )?)
            } else {
                ctx.args.try_get("where").ok().map(|v| accessor_to_json(&v))
            };
            let where_clause = and_predicates(
                where_clause,
                permission_filter(
                    &gql_ctx.caller(),
                    names.as_ref(),
                    schema_name,
                    table_name,
                    crate::role::Verb::Delete,
                )?,
            );

            execute_delete(
                pool,
                gql_ctx,
                schema_name,
                table_name,
                &type_name,
                where_clause,
                returning,
                relationships.as_ref(),
                names.as_ref(),
                max_rows,
            )
            .await
            .map(|rows| {
                let count = rows.len();
                (rows, count)
            })?
        }
    };

    let is_delete = matches!(
        mutation_type,
        MutationType::Delete | MutationType::DeleteByPk
    );
    let result = match returning {
        Some(returning) if !result.is_empty() && !is_delete => {
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

/// The transaction this operation's writes share, opening it if this is the
/// first one.
///
/// Held for the length of the write, and settled by whoever answers the
/// request -- see [`crate::context::SharedWrite`]. Mutation root fields are
/// resolved one after another, so the lock is never contended; it is what
/// makes the transaction reachable from a resolver that owns nothing.
async fn write_tx<'a>(
    gql_ctx: &'a GraphQLContext,
    pool: &PgPool,
) -> Result<
    tokio::sync::MutexGuard<'a, Option<sqlx::Transaction<'static, sqlx::Postgres>>>,
    async_graphql::Error,
> {
    let mut guard = gql_ctx.write.lock().await;
    if guard.is_none() {
        *guard = Some(begin_with_session(pool, gql_ctx.role(), &gql_ctx.session_settings()).await?);
    }
    Ok(guard)
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

/// Render a row as JSON, with any shape column rendered as GeoJSON.
///
/// PostgreSQL's text form for a geometry is WKB hex --
/// `0103000020E6100000…` -- which is what `row_to_json` puts in the response
/// and is not what any client can read. Hasura answers GeoJSON, and a client
/// migrating from it parses GeoJSON.
///
/// The shape columns are merged over the row rather than replacing the
/// projection, so a table with no shape column produces exactly the SQL it did
/// before and one with three needs no list of the others.
fn row_json(expr: &str, column_types: &HashMap<String, String>) -> String {
    let mut shapes: Vec<(&String, &String)> = column_types
        .iter()
        .filter(|(_, pg_type)| matches!(pg_type.as_str(), "geometry" | "geography"))
        .collect();
    if shapes.is_empty() {
        return format!("row_to_json({})", expr);
    }
    shapes.sort();

    let overrides: Vec<String> = shapes
        .iter()
        .map(|(name, _)| {
            format!(
                // Option 4 asks PostGIS for the long-form CRS member --
                // `urn:ogc:def:crs:EPSG::4326` -- which Hasura includes and
                // which a client round-tripping a shape needs in order to
                // write it back where it came from. PostGIS emits it only
                // where the geometry has an SRID to report.
                "'{}', ST_AsGeoJSON({}.{}, 9, 4)::jsonb",
                name.replace('\'', "''"),
                expr,
                postrust_sql::escape_ident(name)
            )
        })
        .collect();
    format!(
        "(to_jsonb({}) || jsonb_build_object({}))",
        expr,
        overrides.join(", ")
    )
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
    let rows = query.fetch_all(&mut *conn).await.map_err(database_error)?;

    // Return raw JSON values - don't convert to async_graphql::Value
    // This allows field resolvers to use try_downcast_ref::<serde_json::Value>()
    let results: Vec<serde_json::Value> = rows
        .into_iter()
        .filter_map(|row| row.try_get::<serde_json::Value, _>(0).ok())
        .collect();

    Ok(results)
}

/// Refuse a GeoJSON document PostGIS would accept and should not.
///
/// `ST_GeomFromGeoJSON` will build a "Polygon" from three points that do not
/// meet, and a "LineString" from one point -- neither of which is a shape, and
/// both of which are then stored and read back as though they were. Hasura
/// checks the document before it reaches the database, refuses with
/// `parse-failed`, and says which rule was broken; the messages here are its
/// messages, because a client that reports them to a user is reporting text it
/// already ships.
///
/// A value that is not an object is left alone: a string in a geometry column
/// is WKT or WKB hex, which is a different thing entirely.
fn check_geojson(value: &serde_json::Value) -> Result<(), async_graphql::Error> {
    fn refuse<T>(message: &str) -> Result<T, async_graphql::Error> {
        Err(coded_error("parse-failed", message))
    }

    fn position(
        value: &serde_json::Value,
    ) -> Result<&Vec<serde_json::Value>, async_graphql::Error> {
        match value.as_array() {
            Some(items) if items.len() >= 2 && items.iter().all(|i| i.is_number()) => Ok(items),
            _ => refuse("A Position needs at least 2 elements"),
        }
    }

    fn positions(
        value: Option<&serde_json::Value>,
        least: usize,
        what: &str,
    ) -> Result<Vec<Vec<serde_json::Value>>, async_graphql::Error> {
        let items = match value.and_then(|v| v.as_array()) {
            Some(items) => items,
            None => return refuse(&format!("A {} needs at least {} Positions", what, least)),
        };
        if items.len() < least {
            return refuse(&format!("A {} needs at least {} Positions", what, least));
        }
        items.iter().map(|item| position(item).cloned()).collect()
    }

    fn ring(value: &serde_json::Value) -> Result<(), async_graphql::Error> {
        let points = positions(Some(value), 4, "LinearRing")?;
        match (points.first(), points.last()) {
            (Some(first), Some(last)) if first == last => Ok(()),
            _ => refuse("the first and last locations have to be equal for a LinearRing"),
        }
    }

    fn polygon(value: &serde_json::Value) -> Result<(), async_graphql::Error> {
        let Some(rings) = value.as_array() else {
            return refuse("A LinearRing needs at least 4 Positions");
        };
        rings.iter().try_for_each(ring)
    }

    let serde_json::Value::Object(map) = value else {
        return Ok(());
    };
    let Some(kind) = map.get("type").and_then(|v| v.as_str()) else {
        return Ok(());
    };
    let coordinates = map.get("coordinates");
    match kind {
        "Point" => {
            position(coordinates.unwrap_or(&serde_json::Value::Null))?;
        }
        "MultiPoint" => {
            positions(coordinates, 0, "MultiPoint")?;
        }
        "LineString" => {
            positions(coordinates, 2, "LineString")?;
        }
        "MultiLineString" => {
            for line in coordinates.and_then(|v| v.as_array()).into_iter().flatten() {
                positions(Some(line), 2, "LineString")?;
            }
        }
        "Polygon" => polygon(coordinates.unwrap_or(&serde_json::Value::Null))?,
        "MultiPolygon" => {
            for one in coordinates.and_then(|v| v.as_array()).into_iter().flatten() {
                polygon(one)?;
            }
        }
        "GeometryCollection" => {
            for geometry in map
                .get("geometries")
                .and_then(|v| v.as_array())
                .into_iter()
                .flatten()
            {
                check_geojson(geometry)?;
            }
        }
        other => return refuse(&format!("unexpected geometry type: {}", other)),
    }
    Ok(())
}

/// The call that produces a computed field's value.
///
/// Two spellings of the same row, because the two calls need different ones.
/// A positional call takes `alias.*`, which is what a qualified table can be
/// written as; a named call takes the row as one value, which only a bare
/// alias is. Named notation is needed exactly where a session argument is,
/// since the row is then not the only parameter and may not be the first.
fn computed_call(
    definition: &postrust_core::schema_cache::ComputedColumn,
    row: &str,
    row_by_name: &str,
) -> String {
    let function = format!(
        "{}.{}",
        postrust_sql::escape_ident(&definition.function.schema),
        postrust_sql::escape_ident(&definition.function.name)
    );
    match (&definition.session_argument, &definition.row_argument) {
        (Some(session), Some(row_argument)) => format!(
            "{}({} => {}, {} => {})",
            function,
            postrust_sql::escape_ident(row_argument),
            row_by_name,
            postrust_sql::escape_ident(session),
            SESSION_ARGUMENT
        ),
        _ => format!("{}({})", function, row),
    }
}

/// What a `hasura_session` argument is given.
///
/// The session document, read from the setting `begin_with_session` writes.
/// `current_setting`'s second argument makes a missing setting null rather than
/// an error, which is the case of a request with no session at all.
const SESSION_ARGUMENT: &str = "coalesce(current_setting('hasura.session', true), '{}')::json";

/// Everything an update may be told to do, in the order the schema declares
/// them. Read by both spellings: one update, and one of many.
const UPDATE_OPERATORS: [&str; 7] = [
    "_set",
    "_inc",
    "_append",
    "_prepend",
    "_delete_key",
    "_delete_elem",
    "_delete_at_path",
];

/// What a read table is called inside its own query.
///
/// See [`WRITTEN_ROW`]: the row a query passes to a computed field has to be
/// named, and a table's qualified name is not a name a row can be written
/// under.
const READ_ROW: &str = "pgrst_src";

/// What a written table is called inside its own statement.
///
/// See [`WhereScope::under_alias`]: the row a statement returns has to be
/// named, and a table's own name is read as a column first.
const WRITTEN_ROW: &str = "pgrst_row";

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

/// The value one column is written with, as it goes over the wire.
///
/// A `json` column holds a JSON value, and `"[]"` is one -- a string whose
/// characters happen to be brackets. Binding the bare text and casting stored
/// an empty array instead, so a client that wrote a string read back a list.
/// The document a column is given is the document it keeps.
fn write_operand(
    column_types: &HashMap<String, String>,
    column: &str,
    value: &serde_json::Value,
) -> serde_json::Value {
    match column_types.get(column).map(String::as_str) {
        Some("json") | Some("jsonb") => match value {
            serde_json::Value::String(_)
            | serde_json::Value::Number(_)
            | serde_json::Value::Bool(_) => serde_json::Value::String(value.to_string()),
            other => other.clone(),
        },
        // An array column takes an array literal. `["a","b"]` is JSON, and
        // PostgreSQL reads a text parameter destined for a `text[]` as
        // `{a,b}` -- `malformed array literal` is what it says about the
        // other spelling.
        Some(pg_type) if is_array_type(pg_type) => match value {
            serde_json::Value::Array(_) => serde_json::Value::String(array_literal(value)),
            other => other.clone(),
        },
        _ => value.clone(),
    }
}

/// Whether a PostgreSQL type name is an array.
///
/// Both spellings appear in the catalogue depending on how it was asked:
/// `text[]` from `format_type`, `_text` from `typname`.
fn is_array_type(pg_type: &str) -> bool {
    pg_type.ends_with("[]") || pg_type.starts_with('_')
}

/// A JSON array as PostgreSQL writes one.
///
/// `["a","b"]` becomes `{"a","b"}`. Every element is quoted, so a value
/// containing a comma, a brace or a backslash survives; a null is the unquoted
/// `NULL`, which is the only way to write one.
fn array_literal(value: &serde_json::Value) -> String {
    fn element(value: &serde_json::Value) -> String {
        match value {
            serde_json::Value::Null => "NULL".to_string(),
            serde_json::Value::Array(_) => array_literal(value),
            serde_json::Value::String(text) => {
                format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
            }
            other => format!("\"{}\"", other.to_string().replace('"', "\\\"")),
        }
    }
    let items = match value {
        serde_json::Value::Array(items) => items,
        other => return element(other),
    };
    format!(
        "{{{}}}",
        items.iter().map(element).collect::<Vec<_>>().join(",")
    )
}

/// The expression that writes one bound value into one column.
///
/// A cast is enough for almost everything. A shape is the exception: a client
/// sends GeoJSON, which is what Hasura accepts, and `'{"type":"Point",…}'` is
/// not something PostgreSQL will cast to a geometry -- it has a function for
/// reading that, and the function is the only way in.
///
/// Only when the value actually is an object. A string in a geometry column is
/// WKT or WKB hex, which the cast does read, and passing that to
/// `ST_GeomFromGeoJSON` would refuse a perfectly good value.
fn write_expression(
    column_types: &HashMap<String, String>,
    column: &str,
    value: &serde_json::Value,
    placeholder: &str,
) -> String {
    let pg_type = column_types.get(column).map(String::as_str);
    if value.is_object() && matches!(pg_type, Some("geometry") | Some("geography")) {
        return match pg_type {
            Some("geography") => format!("ST_GeomFromGeoJSON({})::geography", placeholder),
            _ => format!("ST_GeomFromGeoJSON({})", placeholder),
        };
    }
    // An amount written as a number is reached through one: PostgreSQL has no
    // cast from `double precision` to `money`, which is what a bound JSON
    // number arrives as, and `numeric` is the type it does take one from. One
    // written as text -- `"$12,344.57"` -- is read by `money` directly, and
    // going through `numeric` would refuse the currency symbol.
    if pg_type == Some("money") && value.is_number() {
        return format!("{}::numeric::money", placeholder);
    }
    format!("{}{}", placeholder, write_cast(column_types, column))
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
    Box<
        dyn std::future::Future<Output = Result<serde_json::Value, async_graphql::Error>>
            + Send
            + 'life,
    >,
> {
    Box::pin(async move {
        use sqlx::Row;

        let qi = postrust_core::api_request::QualifiedIdentifier::new(schema_name, table_name);
        let table = context.cache.get_table(&qi).ok_or_else(|| {
            async_graphql::Error::new(format!("unknown table \"{}\"", table_name))
        })?;
        let column_types: HashMap<String, String> = table
            .columns
            .values()
            .map(|c| (c.name.clone(), c.nominal_type.clone()))
            .collect();

        // A key that is a relationship rather than a column is a nested write.
        let type_name = context
            .type_names
            .get(&(schema_name.to_string(), table_name.to_string()));
        let relationships: &[RelationshipField] = type_name
            .and_then(|name| context.relationships.get(name))
            .map(|r| r.as_slice())
            .unwrap_or(&[]);

        let mut columns = serde_json::Map::new();
        type Nested<'r> = (
            &'r RelationshipField,
            serde_json::Value,
            Option<serde_json::Value>,
        );
        let mut to_one: Vec<Nested> = Vec::new();
        let mut to_many: Vec<Nested> = Vec::new();

        for (key, value) in object {
            match relationships.iter().find(|r| r.name == key) {
                None => {
                    // A written value is keyed by the field it was sent under;
                    // the statement is written in the table's own columns.
                    columns.insert(
                        table_column_for(context.names, table, &key).to_string(),
                        value,
                    );
                }
                Some(_) if value.is_null() => {
                    // `author: null` is a row with no author, not a row whose
                    // author is a null object. A client writing one insert for
                    // both cases sends the relationship either way.
                    continue;
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
                    // Which row goes first follows from which side holds the
                    // key, not from how many rows there are. A one-to-one
                    // whose child key *is* the parent's -- `author_detail.id`
                    // referencing `author.id` -- is one row either way, and
                    // the parent still has to be written before there is a key
                    // to give it.
                    if child_holds_the_key(&relationship.relationship) {
                        to_many.push((relationship, data, conflict));
                    } else {
                        to_one.push((relationship, data, conflict));
                    }
                }
            }
        }

        // A shape is checked before it is written, since PostGIS would build
        // one from a document that is not a shape.
        for (column, value) in &columns {
            if matches!(
                column_types.get(column).map(String::as_str),
                Some("geometry") | Some("geography")
            ) {
                // Under the name the client wrote it under, which is the
                // exposed one rather than the column's where they differ.
                check_geojson(value).map_err(|error| {
                    let field = context
                        .names
                        .column(schema_name, table_name, column)
                        .unwrap_or(column);
                    at_path(error, &format!("{}.{}", context.row, field))
                })?;
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
            // Writing the related row is what fills this row's key column in,
            // so a value already written there is a second answer to the same
            // question and one of them would be silently discarded.
            for (local, _) in &plan.columns {
                if columns.get(local).is_some_and(|v| !v.is_null()) {
                    return Err(at_path(
                        validation_error(format!(
                            "cannot insert object relationship \"{}\" as \"{}\" column \
                             values are already determined",
                            relationship.name, local
                        )),
                        &format!("{}.{}", context.row, relationship.name),
                    ));
                }
            }
            // A to-one row is written under the relationship's own name:
            // `...objects[0].author.data`, and its `on_conflict` beside it.
            let under = format!("{}.{}", context.row, relationship.name);
            let nested = InsertContext {
                on_conflict: conflict,
                cache: context.cache,
                relationships: context.relationships,
                type_names: context.type_names,
                names: context.names,
                caller: context.caller,
                row: format!("{}.data", under),
                objects: format!("{}.data", under),
                conflict: format!("{}.on_conflict", under),
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

        // What the server fills in regardless of what was sent. Applied after
        // the nested rows have contributed their keys and before the columns
        // are counted, because a preset is a column this statement writes --
        // and applied *over* the request, since a preset the client could
        // override is not a preset.
        for (column, value) in crate::role::presets(
            &context.caller,
            context.names,
            schema_name,
            table_name,
            crate::role::Verb::Insert,
        )
        .map_err(|fault| coded_error(fault.code(), fault.to_string()))?
        {
            columns.insert(column, value);
        }

        // `ON CONFLICT` says which uniqueness is being resolved against and
        // what to do about it. An empty `update_columns` is `DO NOTHING`,
        // which is how a client says "leave the row that is already there".
        // The columns this statement writes, settled before the conflict
        // clause so a predicate on it can number its parameters after theirs.
        let column_names: Vec<&str> = columns.keys().map(|k| k.as_str()).collect();
        let mut conflict_values: Vec<serde_json::Value> = Vec::new();
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
                // The member that stands for the columns this role has none
                // of. It is in the enum so that `on_conflict` exists at all;
                // naming it is the one thing it cannot be used for.
                if updates.contains(&PLACEHOLDER_COLUMN) {
                    return Err(at_path(
                        coded_error("validation-failed", "erroneous column name"),
                        &context.conflict,
                    ));
                }

                if updates.is_empty() {
                    format!(
                        " ON CONFLICT ON CONSTRAINT {} DO NOTHING",
                        postrust_sql::escape_ident(constraint)
                    )
                } else {
                    // An `ON CONFLICT DO UPDATE` is an update, so the update
                    // permission's presets are written here too -- and a
                    // column a preset writes does not also take its value from
                    // `EXCLUDED`, which is what "a preset overrides the
                    // request" means on this side.
                    let preset = crate::role::presets(
                        &context.caller,
                        context.names,
                        schema_name,
                        table_name,
                        crate::role::Verb::Update,
                    )
                    .map_err(|fault| coded_error(fault.code(), fault.to_string()))?;

                    // Numbered after the row's own values and before the
                    // predicate below, because `SET` is written before `WHERE`
                    // and the parameters are bound in the order they appear.
                    let mut param_idx = column_names.len() + 1;
                    let mut assignments: Vec<String> =
                        Vec::with_capacity(preset.len() + updates.len());
                    for (column, value) in &preset {
                        assignments.push(format!(
                            "{} = {}",
                            postrust_sql::escape_ident(column),
                            write_expression(
                                &column_types,
                                column,
                                value,
                                &format!("${}", param_idx)
                            )
                        ));
                        conflict_values.push(write_operand(&column_types, column, value));
                        param_idx += 1;
                    }
                    for field in &updates {
                        let column = table_column_for(context.names, table, field);
                        if preset.iter().any(|(written, _)| written == column) {
                            continue;
                        }
                        assignments.push(format!(
                            "{} = EXCLUDED.{}",
                            postrust_sql::escape_ident(column),
                            postrust_sql::escape_ident(column)
                        ));
                    }
                    // `where` on an upsert decides whether the row that is
                    // already there is overwritten -- "only if what I am
                    // writing is newer". It reads the existing row, which is
                    // what the statement's alias names.
                    //
                    // The update permission's own filter is ANDed into it: an
                    // upsert that overwrites a row is an update, and a role
                    // that may not update a row may not reach it by inserting
                    // over it. Compiling that filter is also where a session
                    // variable it names and the caller does not carry is
                    // noticed -- which is the answer Hasura gives before it
                    // writes anything.
                    let granted = permission_filter(
                        &context.caller,
                        context.names,
                        schema_name,
                        table_name,
                        crate::role::Verb::Update,
                    )?;
                    let requested = spec.get("where").filter(|f| !f.is_null()).cloned();
                    let condition = match and_predicates(requested, granted) {
                        Some(filter) => {
                            let filter = &filter;
                            let type_name = context
                                .type_names
                                .get(&(schema_name.to_string(), table_name.to_string()))
                                .map(String::as_str)
                                .unwrap_or(table_name);
                            let scope = WhereScope::table(
                                schema_name,
                                table_name,
                                type_name,
                                context.names,
                            )
                            .under_alias(WRITTEN_ROW)
                            .with_resolution(context.cache, context.relationships);
                            let mut alias_counter = 0usize;
                            build_condition(
                                filter,
                                &scope,
                                &mut param_idx,
                                &mut conflict_values,
                                &mut alias_counter,
                            )?
                            .map(|sql| format!(" WHERE {}", sql))
                            .unwrap_or_default()
                        }
                        None => String::new(),
                    };
                    format!(
                        " ON CONFLICT ON CONSTRAINT {} DO UPDATE SET {}{}",
                        postrust_sql::escape_ident(constraint),
                        assignments.join(", "),
                        condition
                    )
                }
            }
            _ => String::new(),
        };

        // The row this statement writes, as `RETURNING` names it. Not the
        // table's own name: qualifying it reads as a column of a table called
        // `public`, which is what `missing FROM-clause entry for table
        // "public"` means, and leaving it bare reads as a column of the table
        // where one shares the name.
        let qualified_table = postrust_sql::escape_ident(WRITTEN_ROW);

        // What the written row has to satisfy for the write to stand,
        // evaluated in `RETURNING` against the row as written. Its parameters
        // are numbered after the values and the conflict clause, since
        // `RETURNING` is last.
        let check_scope = WhereScope::for_alias(
            schema_name,
            table_name,
            WRITTEN_ROW,
            context
                .type_names
                .get(&(schema_name.to_string(), table_name.to_string()))
                .map(String::as_str)
                .unwrap_or(table_name),
            context.cache,
            context.relationships,
            context.names,
        );
        let (check_sql, check_values) = {
            let check = crate::role::write_check(
                &context.caller,
                context.names,
                schema_name,
                table_name,
                crate::role::Verb::Insert,
            )
            .map_err(|fault| coded_error(fault.code(), fault.to_string()))?;
            match check {
                None => (String::new(), Vec::new()),
                Some(check) => {
                    let mut values = Vec::new();
                    let mut param_idx = column_names.len() + conflict_values.len() + 1;
                    let mut alias_counter = 0usize;
                    match build_condition(
                        &check,
                        &check_scope,
                        &mut param_idx,
                        &mut values,
                        &mut alias_counter,
                    )? {
                        None => (String::new(), Vec::new()),
                        Some(sql) => (
                            format!(
                                ", ({}) AS {}",
                                sql,
                                postrust_sql::escape_ident(CHECK_COLUMN)
                            ),
                            values,
                        ),
                    }
                }
            }
        };

        let names = column_names;
        let written = if names.is_empty() {
            // Every column defaulted. `DEFAULT VALUES` is how SQL says that;
            // an empty column list is a syntax error.
            let sql = format!(
                "INSERT INTO {}.{} AS {} DEFAULT VALUES{} RETURNING {}{}",
                postrust_sql::escape_ident(schema_name),
                postrust_sql::escape_ident(table_name),
                qualified_table,
                conflict_sql,
                row_json(&qualified_table, &column_types),
                check_sql
            );
            let mut query = sqlx::query(&sql);
            for value in &conflict_values {
                query = bind_json_value(query, value);
            }
            for value in &check_values {
                query = bind_json_value(query, value);
            }
            query
                .fetch_optional(&mut *conn)
                .await
                // A constraint the write violated is reported against the rows
                // the statement was given, which is where Hasura reports it.
                .map_err(|error| at_path(database_error(error), &context.objects))?
        } else {
            let placeholders: Vec<String> = names
                .iter()
                .enumerate()
                .map(|(i, column)| {
                    let placeholder = format!("${}", i + 1);
                    match columns.get(*column) {
                        Some(value) => write_expression(&column_types, column, value, &placeholder),
                        None => placeholder,
                    }
                })
                .collect();
            let sql = format!(
                "INSERT INTO {}.{} AS {} ({}) VALUES ({}){} RETURNING {}{}",
                postrust_sql::escape_ident(schema_name),
                postrust_sql::escape_ident(table_name),
                qualified_table,
                names
                    .iter()
                    .map(|c| postrust_sql::escape_ident(c))
                    .collect::<Vec<_>>()
                    .join(", "),
                placeholders.join(", "),
                conflict_sql,
                row_json(&qualified_table, &column_types),
                check_sql
            );
            trace!("Executing INSERT SQL: {}", sql);
            let mut query = sqlx::query(&sql);
            for column in &names {
                if let Some(value) = columns.get(*column) {
                    query = bind_json_value(query, &write_operand(&column_types, column, value));
                }
            }
            for value in &conflict_values {
                query = bind_json_value(query, value);
            }
            for value in &check_values {
                query = bind_json_value(query, value);
            }
            query
                .fetch_optional(&mut *conn)
                .await
                // A constraint the write violated is reported against the rows
                // the statement was given, which is where Hasura reports it.
                .map_err(|error| at_path(database_error(error), &context.objects))?
        };

        // `DO NOTHING` writes nothing and returns nothing, which is the answer
        // rather than an error: the row that was already there stays.
        let Some(written) = written else {
            return Ok(serde_json::Value::Null);
        };
        refuse_unchecked_rows(std::slice::from_ref(&written), !check_sql.is_empty())
            .map_err(|error| at_path(error, &context.objects))?;
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
            for (child_index, child) in children.into_iter().enumerate() {
                let serde_json::Value::Object(mut child) = child else {
                    continue;
                };
                for (local, foreign) in &plan.columns {
                    // The parent's key is what makes the child a child, so a
                    // child that writes it too is asking for a different
                    // parent than the one it is nested inside.
                    if child.get(foreign).is_some_and(|v| !v.is_null()) {
                        return Err(at_path(
                            validation_error(format!(
                                "cannot insert \"{}\" columns as their values are already \
                                 being determined by parent insert",
                                foreign
                            )),
                            &format!(
                                "{}.{}.data[{}]",
                                context.row, relationship.name, child_index
                            ),
                        ));
                    }
                    if let Some(value) = row.get(local) {
                        child.insert(foreign.clone(), value.clone());
                    }
                }
                // `...objects[0].articles.data[0]`, which is the path
                // Hasura answers for a child that writes its parent's key.
                let under = format!("{}.{}", context.row, relationship.name);
                let nested = InsertContext {
                    on_conflict: conflict.clone(),
                    cache: context.cache,
                    relationships: context.relationships,
                    type_names: context.type_names,
                    names: context.names,
                    caller: context.caller,
                    row: format!("{}.data[{}]", under, child_index),
                    objects: format!("{}.data", under),
                    conflict: format!("{}.on_conflict", under),
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
///
/// Cloned per row and per nested row, which costs three strings: the paths are
/// the only part that differs between them, and the rest is references and a
/// `Caller`.
#[derive(Clone)]
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
    /// The names columns are exposed under, so a written value keyed by a
    /// field reaches the column it names.
    names: &'a crate::names::NameOverrides,
    /// Who is writing, so that a permission's presets and its check reach
    /// every row this insert writes -- including the nested ones, which are
    /// rows of other tables under other rules.
    caller: crate::role::Caller<'a>,
    /// Where this row is in the request, as Hasura writes such a place:
    /// `$.selectionSet.insert_author.args.objects[0]`, and one level down
    /// `$.selectionSet.insert_author.args.objects[0].articles.data[0]`. A
    /// column of it is that with the field's name appended.
    row: String,
    /// Where the rows this statement writes were named. What a refused `check`
    /// is reported against -- the argument, not the row inside it, which is
    /// what Hasura answers.
    objects: String,
    /// Where this statement's `on_conflict` was named.
    conflict: String,
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
        &gql_ctx.caller(),
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
    let computed = computed_projections(
        table,
        returning,
        "src",
        names,
        cache,
        &mut param_idx,
        &mut values,
    )?;
    // A rename is reason enough to read the rows again: `RETURNING` answers in
    // the table's own column names, which are not the names the client asked
    // under.
    let renames = names.renames_columns(&table.schema, &table.name);
    if embeds.is_empty() && computed.is_empty() && !renames {
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

    let mut projection =
        rename_projection(table, "src", names).unwrap_or_else(|| "src.*".to_string());
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
        "SELECT {} FROM (SELECT {} FROM (SELECT * FROM {}.{} WHERE {}) AS src) AS pgrst_r",
        row_json("pgrst_r", &exposed_column_types(table, names)),
        projection,
        postrust_sql::escape_ident(schema_name),
        postrust_sql::escape_ident(table_name),
        conditions.join(" OR ")
    );

    // The same transaction the write went into: the rows it is reading are the
    // rows that write produced, and nothing has committed yet.
    let mut guard = write_tx(gql_ctx, pool).await?;
    let conn = guard.as_mut().expect("write_tx opens one");
    let reread = execute_query_on(conn, &sql, &values).await?;
    drop(guard);

    if reread.is_empty() {
        return Ok(rows);
    }
    Ok(reread.into_iter().map(json_to_value).collect())
}

/// Execute an insert mutation.
async fn execute_insert(
    pool: &PgPool,
    gql_ctx: &GraphQLContext,
    schema_name: &str,
    table_name: &str,
    objects: serde_json::Value,
    context: &InsertContext<'_>,
    // Whether the client wrote one row rather than a list of them. Only the
    // error paths care: `insert_x_one(object: {...})` has no row to index, so
    // its rows are named `args.object` and not `args.object[0]`.
    single: bool,
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

    let mut guard = write_tx(gql_ctx, pool).await?;
    let conn = guard.as_mut().expect("write_tx opens one");

    let mut inserted: Vec<Value> = Vec::new();
    let mut written = 0usize;

    for (index, object) in objects_array.into_iter().enumerate() {
        let serde_json::Value::Object(map) = object else {
            return Err(async_graphql::Error::new(
                "each object to insert is an object",
            ));
        };
        // This row's own place in the request, which is what an error inside
        // it is reported against.
        let context = InsertContext {
            row: match single {
                true => context.objects.clone(),
                false => format!("{}[{}]", context.objects, index),
            },
            ..context.clone()
        };
        let row = insert_row(conn, schema_name, table_name, map, &context, &mut written).await?;
        // A row `DO NOTHING` left alone is not in `returning` and is not in
        // `affected_rows` either: nothing was written, and the row that was
        // already there is not this mutation's to report.
        if !row.is_null() {
            inserted.push(json_to_value(row));
        }
    }

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
#[allow(clippy::too_many_arguments)] // the predicate needs the schema it reads
async fn execute_update(
    pool: &PgPool,
    gql_ctx: &GraphQLContext,
    schema_name: &str,
    table_name: &str,
    type_name: &str,
    operators: Vec<(&'static str, serde_json::Value)>,
    column_types: HashMap<String, String>,
    where_clause: Option<serde_json::Value>,
    relationships: &HashMap<String, Vec<RelationshipField>>,
    names: &crate::names::NameOverrides,
) -> Result<Vec<Value>, async_graphql::Error> {
    use sqlx::Row;

    trace!("Update mutation for {}: {:?}", table_name, operators);

    // Build the SET clause. Each operator writes a column in terms of itself
    // except `_set`, which replaces it -- `_inc` adds, the jsonb operators
    // concatenate or remove, and a client may send several in one mutation as
    // long as they name different columns.
    let mut set_parts: Vec<String> = Vec::new();
    let mut set_values: Vec<serde_json::Value> = Vec::new();
    let mut param_idx = 1;
    let mut written: HashSet<String> = HashSet::new();

    // What the permission writes whatever the request said. A preset overrides
    // rather than collides -- one a client could override is not a preset --
    // so these go in first and the operators below step around the columns
    // they name.
    //
    // This is also what makes an update with no `_set` at all a real update:
    // `update_resident(where: {...})` under a permission whose `set` names
    // `city` changes `city`, and answers with the rows it changed rather than
    // with nothing.
    let preset = crate::role::presets(
        &gql_ctx.caller(),
        names,
        schema_name,
        table_name,
        crate::role::Verb::Update,
    )
    .map_err(|fault| coded_error(fault.code(), fault.to_string()))?;
    let preset_columns: HashSet<&str> = preset.iter().map(|(name, _)| name.as_str()).collect();
    for (column, value) in &preset {
        let placeholder =
            write_expression(&column_types, column, value, &format!("${}", param_idx));
        set_parts.push(format!(
            "{} = {}",
            postrust_sql::escape_ident(column),
            placeholder
        ));
        set_values.push(write_operand(&column_types, column, value));
        param_idx += 1;
    }

    for (operator, payload) in &operators {
        let serde_json::Value::Object(map) = payload else {
            return Err(async_graphql::Error::new(format!(
                "\"{}\" takes an object mapping columns to values",
                operator
            )));
        };
        for (field, value) in map {
            let column = &names
                .column_source(schema_name, table_name, field)
                .unwrap_or(field)
                .to_string();
            // A preset has this column. Not the duplicate-write error below:
            // the request is allowed to name it, and the permission wins.
            if preset_columns.contains(column.as_str()) {
                continue;
            }
            if !written.insert(column.clone()) {
                return Err(async_graphql::Error::new(format!(
                    "\"{}\" is written twice in one update; a column may be \
                     changed by one operator at a time",
                    column
                )));
            }
            let quoted = postrust_sql::escape_ident(column);
            // `_set` writes the column and `_inc` adds to it, so both take a
            // value of the column's type and are cast to it -- `money + $1`
            // is not an operator PostgreSQL has, and neither is `money +
            // double precision`, which is what a bound number arrives as. The
            // others already say what they need -- `::jsonb` for a
            // concatenation, `::text[]` for a path -- and casting twice would
            // be wrong for `_delete_elem`, whose operand is an integer rather
            // than a value of the column.
            let placeholder = match *operator {
                "_set" | "_inc" => {
                    write_expression(&column_types, column, value, &format!("${}", param_idx))
                }
                _ => format!("${}", param_idx),
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
                // A path is a list of keys, and one parameter carrying
                // `["name","last"]` is a JSON array -- which PostgreSQL reads
                // as an array literal and refuses, `malformed array literal`.
                // The elements are bound one at a time into an `ARRAY[...]`,
                // the same way the key-existence comparisons take theirs.
                "_delete_at_path" => {
                    let Some(steps) = value.as_array() else {
                        return Err(async_graphql::Error::new(format!(
                            "\"_delete_at_path\" on \"{}\" takes a list of keys",
                            column
                        )));
                    };
                    let path = sql_array(steps, "text[]", &mut param_idx, &mut set_values);
                    set_parts.push(format!("{} = {} #- {}", quoted, quoted, path));
                    continue;
                }
                other => {
                    return Err(async_graphql::Error::new(format!(
                        "unsupported update operator \"{}\"",
                        other
                    )))
                }
            };
            if *operator == "_set"
                && matches!(
                    column_types.get(column).map(String::as_str),
                    Some("geometry") | Some("geography")
                )
            {
                check_geojson(value)?;
            }
            set_parts.push(assignment);
            set_values.push(match *operator {
                "_set" => write_operand(&column_types, column, value),
                _ => value.clone(),
            });
            param_idx += 1;
        }
    }

    // An update that changes nothing changes no rows. Refusing it looks like
    // the stricter answer and is the wrong one: a client building `_set` from
    // a form the user submitted unchanged sends an empty object, and the
    // honest report of what happened is that nothing did.
    if set_parts.is_empty() {
        return Ok(Vec::new());
    }

    // Build WHERE clause. It can follow a relationship, the same way a query's
    // can: `update_article(where: {author: {name: {_eq: "x"}}})` names the
    // rows by something they point at rather than by a column of their own.
    let guard = gql_ctx
        .schema_cache
        .get()
        .await
        .map_err(|e| async_graphql::Error::new(e.to_string()))?;
    let cache = guard
        .as_ref()
        .ok_or_else(|| async_graphql::Error::new("schema cache is not loaded"))?;
    let scope = WhereScope::table(schema_name, table_name, type_name, names)
        .under_alias(WRITTEN_ROW)
        .with_resolution(cache, relationships);
    let (where_sql, where_values) = build_where_clause(where_clause.as_ref(), param_idx, &scope)?;

    // An absent or unrecognised `where` argument yields an empty clause, which
    // would update every row in the table. Refuse instead.
    if where_sql.is_empty() {
        return Err(async_graphql::Error::new(format!(
            "update on \"{}\" requires a `where` argument with at least one \
             recognised condition; refusing to update every row",
            table_name
        )));
    }

    // What the changed row has to satisfy for the change to stand. Evaluated
    // in the `RETURNING` clause, against the row as written -- which is the
    // only place it can honestly be evaluated, and costs neither a re-read nor
    // a way to identify the rows afterwards.
    let (check_sql, check_values) = permission_check_sql(
        gql_ctx,
        names,
        schema_name,
        table_name,
        crate::role::Verb::Update,
        &scope,
        param_idx + where_values.len(),
    )?;

    let sql = format!(
        "UPDATE {}.{} AS {} SET {} {} RETURNING {}{}",
        postrust_sql::escape_ident(schema_name),
        postrust_sql::escape_ident(table_name),
        postrust_sql::escape_ident(WRITTEN_ROW),
        set_parts.join(", "),
        where_sql,
        row_json(&postrust_sql::escape_ident(WRITTEN_ROW), &column_types),
        check_sql
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

    // And the check's, last, because `RETURNING` is.
    for val in &check_values {
        query = bind_json_value(query, val);
    }

    let mut guard = write_tx(gql_ctx, pool).await?;
    let conn = guard.as_mut().expect("write_tx opens one");
    let rows = query.fetch_all(&mut **conn).await.map_err(database_error)?;

    refuse_unchecked_rows(&rows, !check_sql.is_empty())?;

    let updated: Vec<Value> = rows
        .iter()
        .filter_map(|row| row.try_get::<serde_json::Value, _>(0).ok())
        .map(json_to_value)
        .collect();

    Ok(updated)
}

/// A permission's check, as a column of the write's `RETURNING`.
///
/// Empty where nothing has to be checked. Otherwise `, (<predicate>) AS ...`,
/// evaluated per written row against the row itself -- so a row the permission
/// refuses is known before the transaction is settled, and settling it is
/// already a rollback once an error is reported.
fn permission_check_sql(
    gql_ctx: &GraphQLContext,
    names: &crate::names::NameOverrides,
    schema_name: &str,
    table_name: &str,
    verb: crate::role::Verb,
    scope: &WhereScope<'_>,
    start_param_idx: usize,
) -> Result<(String, Vec<serde_json::Value>), async_graphql::Error> {
    let check = crate::role::write_check(&gql_ctx.caller(), names, schema_name, table_name, verb)
        .map_err(|fault| coded_error(fault.code(), fault.to_string()))?;
    let Some(check) = check else {
        return Ok((String::new(), Vec::new()));
    };

    let mut values = Vec::new();
    let mut param_idx = start_param_idx;
    let mut alias_counter = 0usize;
    let condition = build_condition(
        &check,
        scope,
        &mut param_idx,
        &mut values,
        &mut alias_counter,
    )?;
    Ok(match condition {
        // A check that compiles to nothing restricts nothing, which is what an
        // empty one says.
        None => (String::new(), Vec::new()),
        Some(sql) => (
            format!(
                ", ({}) AS {}",
                sql,
                postrust_sql::escape_ident(CHECK_COLUMN)
            ),
            values,
        ),
    })
}

/// The column a permission's check is returned in.
const CHECK_COLUMN: &str = "pgrst_permission_check";

/// Refuse the whole write if any row it produced fails the permission's check.
///
/// Every row, not the first: a mutation is one transaction and a single
/// refused row takes the rest with it, which is what makes a check a rule
/// about the write rather than about a row. A null counts as a failure -- an
/// unknown is not a satisfied condition.
fn refuse_unchecked_rows(
    rows: &[sqlx::postgres::PgRow],
    checked: bool,
) -> Result<(), async_graphql::Error> {
    if !checked {
        return Ok(());
    }
    use sqlx::Row;
    let all_passed = rows
        .iter()
        .all(|row| row.try_get::<bool, _>(CHECK_COLUMN).unwrap_or(false));
    match all_passed {
        true => Ok(()),
        false => Err(coded_error("permission-error", crate::role::CHECK_FAILED)),
    }
}

/// Execute a delete mutation.
#[allow(clippy::too_many_arguments)] // the projection needs the schema it reads
async fn execute_delete(
    pool: &PgPool,
    gql_ctx: &GraphQLContext,
    schema_name: &str,
    table_name: &str,
    type_name: &str,
    where_clause: Option<serde_json::Value>,
    returning: Option<async_graphql::SelectionField<'_>>,
    relationships: &HashMap<String, Vec<RelationshipField>>,
    names: &crate::names::NameOverrides,
    max_rows: Option<i64>,
) -> Result<Vec<Value>, async_graphql::Error> {
    use sqlx::Row;

    trace!("Delete mutation for {}", table_name);

    let guard = gql_ctx
        .schema_cache
        .get()
        .await
        .map_err(|e| async_graphql::Error::new(e.to_string()))?;
    let cache = guard
        .as_ref()
        .ok_or_else(|| async_graphql::Error::new("schema cache is not loaded"))?;
    let qi = postrust_core::api_request::QualifiedIdentifier::new(schema_name, table_name);
    let column_types = cache
        .get_table(&qi)
        .map(|table| exposed_column_types(table, names))
        .unwrap_or_default();

    // Build WHERE clause. It can follow a relationship -- `delete_article(where:
    // {author: {id: {_eq: 2}}})` is the article-by-author delete anyone writes
    // -- so the scope is given the schema to resolve one with.
    let scope = WhereScope::table(schema_name, table_name, type_name, names)
        .under_alias(WRITTEN_ROW)
        .with_resolution(cache, relationships);
    let (where_sql, mut values) = build_where_clause(where_clause.as_ref(), 1, &scope)?;

    // An absent or unrecognised `where` argument yields an empty clause, which
    // would delete every row in the table. Refuse instead.
    if where_sql.is_empty() {
        return Err(async_graphql::Error::new(format!(
            "delete on \"{}\" requires a `where` argument with at least one \
             recognised condition; refusing to delete every row",
            table_name
        )));
    }

    // A relationship or computed field asked for in `returning` cannot be read
    // afterwards -- the rows are gone by then, which is why this used to answer
    // the plain columns and then fail on a non-null list. Inside one statement
    // the deleted rows are still a table: the delete goes in a CTE and the
    // projection reads from it, while the rows it points at are still there.
    let mut projection = String::new();
    let renamed = cache
        .get_table(&qi)
        .and_then(|table| rename_projection(table, "src", names));
    if let Some(returning) = returning {
        let mut param_idx = values.len() + 1;
        let mut alias_counter = 0usize;
        let embeds = build_embed_expressions(
            &gql_ctx.caller(),
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
        let computed = match cache.get_table(&qi) {
            Some(table) => computed_projections(
                table,
                returning,
                "src",
                names,
                cache,
                &mut param_idx,
                &mut values,
            )?,
            None => Vec::new(),
        };
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
    }

    let written = postrust_sql::escape_ident(WRITTEN_ROW);
    let sql = if projection.is_empty() && renamed.is_none() {
        format!(
            "DELETE FROM {}.{} AS {} {} RETURNING {}",
            postrust_sql::escape_ident(schema_name),
            postrust_sql::escape_ident(table_name),
            written,
            where_sql,
            row_json(&written, &column_types)
        )
    } else {
        format!(
            "WITH pgrst_deleted AS (DELETE FROM {}.{} AS {} {} RETURNING {}.*) \
             SELECT {} FROM (SELECT {}{} FROM pgrst_deleted AS src) AS pgrst_r",
            postrust_sql::escape_ident(schema_name),
            postrust_sql::escape_ident(table_name),
            written,
            where_sql,
            written,
            row_json("pgrst_r", &column_types),
            renamed.unwrap_or_else(|| "src.*".to_string()),
            projection
        )
    };

    trace!("Executing DELETE SQL: {}", sql);

    let mut query = sqlx::query(&sql);
    for val in &values {
        query = bind_json_value(query, val);
    }

    let mut guard = write_tx(gql_ctx, pool).await?;
    let conn = guard.as_mut().expect("write_tx opens one");
    let rows = query.fetch_all(&mut **conn).await.map_err(database_error)?;

    let deleted: Vec<Value> = rows
        .iter()
        .filter_map(|row| row.try_get::<serde_json::Value, _>(0).ok())
        .map(json_to_value)
        .collect();

    Ok(deleted)
}

/// The predicate a role's permission adds to every read of one table.
///
/// `None` where nothing restricts the read: no permission document, an
/// administrator, or a permission whose filter is `{}`. The value is a
/// `<table>_bool_exp` like any other, because [`crate::role::permission_where`]
/// rewrites it into one -- which is what lets the same compiler serve both and
/// keeps a permission able to say everything a `where` can.
fn permission_predicate(
    caller: &crate::role::Caller<'_>,
    names: &crate::names::NameOverrides,
    schema: &str,
    table: &str,
) -> Result<Option<serde_json::Value>, async_graphql::Error> {
    permission_filter(caller, names, schema, table, crate::role::Verb::Select)
}

/// The same, for whichever grant the operation is asking under.
fn permission_filter(
    caller: &crate::role::Caller<'_>,
    names: &crate::names::NameOverrides,
    schema: &str,
    table: &str,
    verb: crate::role::Verb,
) -> Result<Option<serde_json::Value>, async_graphql::Error> {
    crate::role::row_filter(caller, names, schema, table, verb)
        .map_err(|fault| coded_error(fault.code(), fault.to_string()))
}

/// How many rows a permission lets one read return.
///
/// A ceiling rather than a default: a request asking for more gets this, one
/// asking for fewer gets what it asked for, and one asking for nothing gets
/// the ceiling. The same rule `PGRST_MAX_ROWS` follows, folded with it.
fn permission_limit(
    caller: &crate::role::Caller<'_>,
    names: &crate::names::NameOverrides,
    schema: &str,
    table: &str,
) -> Option<i64> {
    crate::role::read_limit(caller, names, schema, table)
}

/// Both predicates, as one.
///
/// Written as an `_and` rather than as two clauses so that everything
/// downstream -- parameter numbering, relationship resolution, the `EXISTS`
/// that a relationship predicate becomes -- happens once, in one place, for
/// both halves.
fn and_predicates(
    request: Option<serde_json::Value>,
    permission: Option<serde_json::Value>,
) -> Option<serde_json::Value> {
    match (request, permission) {
        (None, None) => None,
        (Some(only), None) | (None, Some(only)) => Some(only),
        (Some(request), Some(permission)) => {
            Some(serde_json::json!({ "_and": [request, permission] }))
        }
    }
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
        Some(value) => build_condition(
            value,
            scope,
            &mut param_idx,
            &mut values,
            &mut alias_counter,
        )?,
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
    /// The names this table's columns are exposed under, so a predicate
    /// written against a field can be written as SQL against a column.
    names: &'a crate::names::NameOverrides,
    resolution: Option<WhereResolution<'a>>,
    /// Who is asking, so that a predicate following a relationship narrows the
    /// far side to what that caller may read.
    ///
    /// Unrestricted unless a scope is told otherwise, which is what leaves
    /// every path that has no caller to hand -- the mutation ones -- behaving
    /// as it did.
    caller: Option<crate::role::Caller<'a>>,
}

/// What a scope needs to follow a relationship.
struct WhereResolution<'a> {
    cache: &'a SchemaCache,
    relationships: &'a HashMap<String, Vec<RelationshipField>>,
}

impl<'a> WhereScope<'a> {
    /// A scope over a table addressed by its qualified name.
    pub fn table(
        schema: &str,
        table: &str,
        type_name: &str,
        names: &'a crate::names::NameOverrides,
    ) -> Self {
        Self {
            qualified: postrust_core::api_request::QualifiedIdentifier::new(schema, table),
            sql_ref: format!(
                "{}.{}",
                postrust_sql::escape_ident(schema),
                postrust_sql::escape_ident(table)
            ),
            row_ref: postrust_sql::escape_ident(table),
            type_name: type_name.to_string(),
            names,
            resolution: None,
            caller: None,
        }
    }

    /// The same, referred to by an alias rather than by its name.
    ///
    /// A statement that writes a table has to name that table's row in
    /// `RETURNING`, and a bare table name there is read as a *column* first --
    /// so `area.area`, a geography column in a table of the same name, made
    /// `to_jsonb("area")` the shape rather than the row. The alias is a name
    /// no column has.
    fn under_alias(mut self, alias: &str) -> Self {
        self.sql_ref = postrust_sql::escape_ident(alias);
        self.row_ref = postrust_sql::escape_ident(alias);
        self
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
    #[allow(clippy::too_many_arguments)]
    fn for_alias(
        schema: &str,
        table: &str,
        alias: &str,
        type_name: &str,
        cache: &'a SchemaCache,
        relationships: &'a HashMap<String, Vec<RelationshipField>>,
        names: &'a crate::names::NameOverrides,
    ) -> Self {
        Self {
            qualified: postrust_core::api_request::QualifiedIdentifier::new(schema, table),
            sql_ref: postrust_sql::escape_ident(alias),
            row_ref: postrust_sql::escape_ident(alias),
            type_name: type_name.to_string(),
            names,
            resolution: Some(WhereResolution {
                cache,
                relationships,
            }),
            caller: None,
        }
    }

    /// A scope over an aliased table, for the inside of an `EXISTS`.
    /// A scope over the *other* table of a relationship, under an alias.
    ///
    /// The child's own qualified name, not the parent's: a comparison inside
    /// an `EXISTS` is against the child's columns, so it is the child's types
    /// and the child's renames that decide how to write it.
    fn aliased(
        schema: &str,
        table: &str,
        alias: &str,
        type_name: &str,
        from: &WhereScope<'a>,
    ) -> Self {
        Self {
            qualified: postrust_core::api_request::QualifiedIdentifier::new(schema, table),
            sql_ref: postrust_sql::escape_ident(alias),
            row_ref: postrust_sql::escape_ident(alias),
            type_name: type_name.to_string(),
            names: from.names,
            resolution: from.resolution.as_ref().map(|r| WhereResolution {
                cache: r.cache,
                relationships: r.relationships,
            }),
            // Carried across the relationship: the caller does not change on
            // the way into an EXISTS, and the far side has its own permission
            // to be narrowed by.
            caller: from.caller,
        }
    }

    /// Narrow what this scope's predicates may reach to one caller.
    pub fn for_caller(mut self, caller: crate::role::Caller<'a>) -> Self {
        self.caller = Some(caller);
        self
    }

    /// What a field name refers to, as SQL: a column of this table, or the
    /// call that produces a computed field.
    ///
    /// A computed field is in the boolean expression because it is on the type
    /// -- `where: {sum_float_offset: {_gt: 5}}` is a question a client can
    /// write -- and reading it as a column answered `column
    /// float_test.sum_float_offset does not exist`.
    fn column(&self, name: &str) -> String {
        let column = self.column_name(name);
        let plain = format!("{}.{}", self.sql_ref, postrust_sql::escape_ident(column));
        let Some(cache) = self.resolution.as_ref().map(|r| r.cache) else {
            return plain;
        };
        let Some(table) = cache.get_table(&self.qualified) else {
            return plain;
        };
        if table.get_column(column).is_some() {
            return plain;
        }
        let function = self
            .names
            .computed_source(&table.schema, &table.name, name)
            .unwrap_or(name);
        match table.get_computed_column(function) {
            Some(definition) => {
                computed_call(definition, &format!("{}.*", self.row_ref), &self.row_ref)
            }
            None => plain,
        }
    }

    /// The column behind a field name.
    fn column_name<'n>(&'n self, name: &'n str) -> &'n str {
        self.names
            .column_source(&self.qualified.schema, &self.qualified.name, name)
            .unwrap_or(name)
    }

    /// The PostgreSQL type of one of this table's columns, where it can be
    /// found. A spatial comparison needs it: the same function takes a
    /// geometry or a geography and the operand has to be cast to match.
    fn column_type(&self, name: &str) -> Option<String> {
        let cache = self.resolution.as_ref()?.cache;
        let table = cache.get_table(&self.qualified)?;
        table
            .get_column(self.column_name(name))
            .map(|c| c.nominal_type.clone())
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
            // A question about a table nothing relates this one to. Only a
            // permission writes it; no client can, because no boolean
            // expression has the field.
            "_exists" => {
                conditions.push(table_exists_sql(
                    val,
                    scope,
                    param_idx,
                    values,
                    alias_counter,
                )?);
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
            // A question about the whole related set rather than about any
            // one row of it. `where: {articles_aggregate: {count: {predicate:
            // {_gt: 2}}}}` keeps the authors with more than two articles,
            // which no `EXISTS` over one article can say.
            name if scope.column_type(name).is_none()
                && name
                    .strip_suffix("_aggregate")
                    .and_then(|rel| scope.relationship(rel))
                    .is_some_and(|rel| rel.is_list) =>
            {
                let relationship = name
                    .strip_suffix("_aggregate")
                    .and_then(|rel| scope.relationship(rel))
                    .expect("just checked");
                conditions.push(aggregate_predicate_sql(
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
    let cache = scope.resolution.as_ref().map(|r| r.cache).ok_or_else(|| {
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
    let child_scope = WhereScope::aliased(
        &plan.foreign_schema,
        &plan.foreign_table,
        &alias,
        &relationship.target_type,
        scope,
    );

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

    let child_condition = build_condition(
        child_expression,
        &child_scope,
        param_idx,
        values,
        alias_counter,
    )?;
    if let Some(sql) = child_condition {
        correlation.push(format!("({})", sql));
    }

    // The far side's own permission. `where: {author: {name: {_eq: "Ann"}}}`
    // asks whether a related row exists, and a row this caller may not read is
    // one it may not learn the existence of either -- so the child's filter
    // narrows the EXISTS exactly as it narrows a read of the same table.
    if let Some(caller) = &child_scope.caller {
        let permission = crate::role::read_predicate(
            caller,
            child_scope.names,
            &plan.foreign_schema,
            &plan.foreign_table,
        )
        .map_err(|fault| coded_error(fault.code(), fault.to_string()))?;
        if let Some(predicate) = permission {
            if let Some(sql) =
                build_condition(&predicate, &child_scope, param_idx, values, alias_counter)?
            {
                correlation.push(format!("({})", sql));
            }
        }
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

/// `_exists`, the predicate only a permission can write, as SQL.
///
/// ```json
/// {"_exists": {"_table": "user", "_where": {"id": "X-Hasura-User-Id",
///                                           "is_admin": true}}}
/// ```
///
/// It asks whether a row exists in *another* table -- one no foreign key
/// relates to this one. That is how Hasura writes "the caller is an
/// administrator" without asking the caller to say so: the row that decides it
/// lives in a table of its own, and whether the account this predicate guards
/// is readable does not depend on which account it is.
///
/// So the subselect is **uncorrelated**, which is the whole difference from
/// the `EXISTS` a relationship predicate builds. There are no key columns to
/// join on -- if there were, the permission would have been written as a
/// relationship -- and the `_where` is read against the named table's columns
/// alone.
///
/// The caller's own permissions are not applied inside it, and that is not an
/// oversight. The point of `_exists` is to consult a table the role has no
/// access to: in Hasura's corpus the role that reads `account` through this
/// predicate may not read `public.user` at all. Narrowing the subselect to
/// what the caller may read would make the predicate refuse itself.
fn table_exists_sql(
    spec: &serde_json::Value,
    scope: &WhereScope<'_>,
    param_idx: &mut usize,
    values: &mut Vec<serde_json::Value>,
    alias_counter: &mut usize,
) -> Result<String, async_graphql::Error> {
    let Some((schema, table)) = crate::role::exists_target(spec, &scope.qualified.schema) else {
        return Err(async_graphql::Error::new(
            "\"_exists\" needs a \"_table\" to look in",
        ));
    };

    // The name comes from a permission document rather than from a request,
    // but it still reaches SQL, and a table that is not there would reach it as
    // a syntax error at the far end rather than as an answer here.
    let qualified = postrust_core::api_request::QualifiedIdentifier::new(&schema, &table);
    if scope
        .resolution
        .as_ref()
        .is_some_and(|r| r.cache.get_table(&qualified).is_none())
    {
        return Err(async_graphql::Error::new(format!(
            "\"_exists\" names a table this server does not have: \"{}.{}\"",
            schema, table
        )));
    }

    *alias_counter += 1;
    let alias = format!("pgrst_exists_{}", alias_counter);
    // The type this table is exposed as, which is the key into the
    // relationship map -- so a `_where` may follow the named table's own
    // relationships, the way any other predicate over it could.
    let type_name = scope
        .names
        .base_name(&schema, &table)
        .unwrap_or(&table)
        .to_string();
    let mut inner = WhereScope::aliased(&schema, &table, &alias, &type_name, scope);
    // See above: the predicate is the permission, so it is not itself
    // subject to one.
    inner.caller = None;

    let condition = match spec.get("_where") {
        Some(predicate) => build_condition(predicate, &inner, param_idx, values, alias_counter)?,
        None => None,
    };

    Ok(format!(
        "EXISTS (SELECT 1 FROM {}.{} AS {} WHERE {})",
        postrust_sql::escape_ident(&schema),
        postrust_sql::escape_ident(&table),
        postrust_sql::escape_ident(&alias),
        condition.unwrap_or_else(|| "true".to_string())
    ))
}

/// A predicate over an aggregate of a related set, as SQL.
///
/// `{count: {predicate: {_gt: 2}, filter: {...}}}` becomes a scalar subselect
/// correlated back to the parent row, compared the way any column is:
///
/// ```text
/// (SELECT count(*) FROM "public"."article" AS pgrst_rel_1
///   WHERE "author"."id" = pgrst_rel_1."author_id" AND (...)) > $1
/// ```
///
/// A scalar subselect rather than an `EXISTS`, because the answer is a number
/// or a truth value and not whether a row is there -- and because over no rows
/// at all `count` is zero and `bool_and` is null, which is what SQL says and
/// what a client asking "no articles" is relying on.
fn aggregate_predicate_sql(
    relationship: &RelationshipField,
    expression: &serde_json::Value,
    scope: &WhereScope<'_>,
    param_idx: &mut usize,
    values: &mut Vec<serde_json::Value>,
    alias_counter: &mut usize,
) -> Result<String, async_graphql::Error> {
    let serde_json::Value::Object(over) = expression else {
        return Ok("true".to_string());
    };

    let cache = scope.resolution.as_ref().map(|r| r.cache).ok_or_else(|| {
        async_graphql::Error::new(format!(
            "filtering on the relationship \"{}\" is not available here",
            relationship.name
        ))
    })?;
    let plan = postrust_core::embed::EmbedPlan::resolve(&relationship.relationship, cache)
        .map_err(|e| async_graphql::Error::new(e.to_string()))?;
    if plan.junction.is_some() || (plan.function.is_none() && plan.columns.is_empty()) {
        return Err(async_graphql::Error::new(format!(
            "aggregating over \"{}\" is not supported: it is reached through a junction \
             rather than by a key",
            relationship.name
        )));
    }

    let mut conditions: Vec<String> = Vec::new();
    for (function, spec) in over {
        if spec.is_null() {
            continue;
        }
        let serde_json::Value::Object(spec) = spec else {
            continue;
        };
        let Some(predicate) = spec.get("predicate").filter(|p| !p.is_null()) else {
            return Err(validation_error(&format!(
                "\"{}\" over \"{}\" needs a predicate",
                function, relationship.name
            )));
        };

        *alias_counter += 1;
        let alias = format!("pgrst_agg_{}", alias_counter);
        let child_scope = WhereScope::aliased(
            &plan.foreign_schema,
            &plan.foreign_table,
            &alias,
            &relationship.target_type,
            scope,
        );

        // Correlated the way an `EXISTS` over the same relationship is: by
        // the key, or by handing the parent row to the function.
        let (source, mut correlation) = match &plan.function {
            Some(routine) => (
                format!(
                    "{}.{}({})",
                    postrust_sql::escape_ident(&routine.schema),
                    postrust_sql::escape_ident(&routine.name),
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
        // `filter` narrows what is aggregated, and is an ordinary boolean
        // expression over the child.
        if let Some(filter) = spec.get("filter").filter(|f| !f.is_null()) {
            if let Some(sql) =
                build_condition(filter, &child_scope, param_idx, values, alias_counter)?
            {
                correlation.push(format!("({})", sql));
            }
        }
        if correlation.is_empty() {
            correlation.push("true".to_string());
        }

        let arguments = spec.get("arguments").filter(|a| !a.is_null());
        let distinct = matches!(spec.get("distinct"), Some(serde_json::Value::Bool(true)));
        let (aggregate, operand_type) = match function.as_str() {
            "count" => {
                let columns: Vec<String> = match arguments {
                    Some(serde_json::Value::Array(items)) => items
                        .iter()
                        .filter_map(|item| item.as_str())
                        .map(|column| child_scope.column(column))
                        .collect(),
                    Some(serde_json::Value::String(one)) => vec![child_scope.column(one)],
                    _ => Vec::new(),
                };
                let counted = match columns.len() {
                    0 => "*".to_string(),
                    1 => columns.into_iter().next().unwrap_or_default(),
                    _ => format!("({})", columns.join(", ")),
                };
                let counted = match distinct && counted != "*" {
                    true => format!("DISTINCT {}", counted),
                    false => counted,
                };
                (format!("count({})", counted), "integer")
            }
            "bool_and" | "bool_or" => {
                let Some(column) = arguments.and_then(|a| a.as_str()) else {
                    return Err(validation_error(&format!(
                        "\"{}\" over \"{}\" needs the column to fold",
                        function, relationship.name
                    )));
                };
                (
                    format!("{}({})", function, child_scope.column(column)),
                    "boolean",
                )
            }
            other => {
                return Err(validation_error(&format!(
                    "\"{}\" is not an aggregate a predicate can be written over",
                    other
                )))
            }
        };

        let subselect = format!(
            "(SELECT {} FROM {} AS {} WHERE {})",
            aggregate,
            source,
            postrust_sql::escape_ident(&alias),
            correlation.join(" AND ")
        );
        let serde_json::Value::Object(operators) = predicate else {
            return Err(validation_error(&format!(
                "the predicate on \"{}\" is not a comparison",
                function
            )));
        };
        for (operator, operand) in operators {
            conditions.push(comparison_sql(
                &subselect,
                function,
                Some(operand_type),
                operator,
                operand,
                param_idx,
                values,
            )?);
        }
    }

    Ok(match conditions.is_empty() {
        true => "true".to_string(),
        false => format!("({})", conditions.join(" AND ")),
    })
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
            // On a sphere the answer depends on which sphere: `ST_DWithin`
            // over geography measures on the spheroid by default and on a
            // perfect sphere when told not to, and the two disagree by enough
            // to change which rows come back. Geometry has no such argument.
            let spheroid = match (is_geography, spec.get("use_spheroid")) {
                (true, Some(use_spheroid)) if !use_spheroid.is_null() => {
                    let placeholder = format!("${}", param_idx);
                    *param_idx += 1;
                    values.push(use_spheroid.clone());
                    format!(", {}::boolean", placeholder)
                }
                _ => String::new(),
            };
            Ok(format!(
                "{}({}, {}, {}::float8{})",
                function, quoted, from_sql, distance_sql, spheroid
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
            // The GraphQL name of what is being compared as, and the
            // PostgreSQL type behind it. `String` is the one that differs:
            // a document compared as text is compared as `text`.
            let pg_type = match target.as_str() {
                "geometry" | "geography" => target.as_str(),
                "String" => "text",
                _ => {
                    return Err(async_graphql::Error::new(format!(
                        "cannot compare \"{}\" as \"{}\"",
                        column, target
                    )))
                }
            };
            let serde_json::Value::Object(ops) = comparisons else {
                continue;
            };
            let cast = format!("{}::{}", quoted, pg_type);
            for (nested_op, nested_operand) in ops {
                conditions.push(comparison_sql(
                    &cast,
                    column,
                    Some(pg_type),
                    nested_op,
                    nested_operand,
                    param_idx,
                    values,
                )?);
            }
        }
        return Ok(match conditions.len() {
            0 => "true".to_string(),
            1 => conditions.pop().expect("just counted"),
            _ => format!("({})", conditions.join(" AND ")),
        });
    }

    // A spatial relation is a function of two shapes rather than an operator
    // between them, so it is written before the operator table is consulted.
    if let Some(function) = crate::input::bool_exp::postgis_function(op) {
        return postgis_sql(
            quoted,
            column_type,
            function,
            op,
            operand,
            param_idx,
            values,
        );
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

    // The path language is a query over the document rather than a comparison
    // against one, so it is a function of the column and the path: `_exists`
    // asks whether the path selects anything, `_match` whether the predicate
    // it ends in holds.
    if let Some(function) = match op {
        "_jsonb_path_exists" => Some("jsonb_path_exists"),
        "_jsonb_path_match" => Some("jsonb_path_match"),
        _ => None,
    } {
        let placeholder = format!("${}", param_idx);
        *param_idx += 1;
        values.push(operand.clone());
        return Ok(format!(
            "{}({}, {}::jsonpath)",
            function, quoted, placeholder
        ));
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
        // A containment operand is a whole document, and `"latest"` is one --
        // a JSON string, which is not the same text as `latest`. Binding the
        // bare string answered `invalid input syntax for type json`, so what
        // goes over the wire is the value as it is written in JSON.
        values.push(match op {
            "_contains" | "_contained_in" => match operand {
                serde_json::Value::String(_)
                | serde_json::Value::Number(_)
                | serde_json::Value::Bool(_) => serde_json::Value::String(operand.to_string()),
                other => other.clone(),
            },
            _ => operand.clone(),
        });
        // A bound parameter arrives as text and PostgreSQL infers a type for
        // it from the operator, which works only while the inference is
        // unambiguous. `jsonb @> text` is not an operator at all; and `id =
        // $1` against an integer column answers `operator does not exist:
        // integer = text` the moment the value is one PostgreSQL cannot read
        // as a number -- a null, most obviously, which is how a client asks a
        // question it expects an empty answer to.
        //
        // So the operand says what it is. The containment and key operators
        // name the type the operator needs; everything else names the
        // column's, which is the type it is being compared against.
        let cast = match op {
            "_contains" | "_contained_in" => "::jsonb".to_string(),
            "_has_key" => "::text".to_string(),
            // Only where inference would otherwise pick text and be wrong. A
            // cast tells PostgreSQL what the *result* is, and it infers the
            // parameter feeding it as text -- so `$1::int4` bound with a
            // number sends binary int8 bytes into a text parameter and
            // answers `invalid byte sequence for encoding "UTF8"`. A value
            // PostgreSQL can read as a number needs no help; a null is the one
            // that does, because `id = $1` with nothing to infer from is
            // `integer = text`.
            _ if operand.is_null() => match column_type {
                Some(pg_type) => format!("::{}", pg_type),
                None => String::new(),
            },
            // A parameter arrives as `text`, and PostgreSQL then resolves the
            // operator by taking the *column* down to text where it can --
            // which is the wrong operator, and sometimes no operator at all.
            // `citext = text` compares case-sensitively against a
            // case-insensitive column; `uuid = text` does not exist. So a
            // written value names the type it is being compared against.
            //
            // Only a string: a number bound with a cast is sent as binary into
            // a parameter PostgreSQL has inferred as text, which is the
            // `invalid byte sequence for encoding "UTF8"` this used to answer.
            _ if operand.is_string() => match column_type {
                Some(pg_type) => format!("::{}", pg_type),
                None => String::new(),
            },
            _ => String::new(),
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
                // Cast only a null, for the reason the binary comparisons
                // give: a cast makes PostgreSQL infer the parameter as text.
                let cast = match (item.is_null() || item.is_string(), column_type) {
                    (true, Some(pg_type)) => format!("::{}", pg_type),
                    _ => String::new(),
                };
                placeholders.push(format!("${}{}", param_idx, cast));
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
#[allow(clippy::too_many_arguments)] // one parameter per SQL clause
async fn build_order_by_clause(
    ctx: &ResolverContext<'_>,
    schema_cache: &postrust_core::schema_cache::SchemaCacheRef,
    schema_name: &str,
    table_name: &str,
    type_name: &str,
    relationships: &HashMap<String, Vec<RelationshipField>>,
    reference: &str,
    names: &crate::names::NameOverrides,
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
    for entry in entries {
        order_terms_into(
            entry,
            cache,
            relationships,
            type_name,
            table,
            reference,
            names,
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
    names: &crate::names::NameOverrides,
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
        // A direction given as null is no direction. `order_by: {id:
        // $direction}` with nothing bound for `$direction` is how a client
        // makes an ordering optional without writing the query twice, and the
        // same goes for `{author: $author_order_by}`, which is a whole
        // ordering left out.
        if value.is_null() {
            continue;
        }

        // A direction: this table's own column, or a function of its row.
        if let Some(name) = value.as_str() {
            let sql = crate::input::order_by::direction_sql(name).ok_or_else(|| {
                async_graphql::Error::new(format!(
                    "\"{}\" is not a sort direction; expected one of asc, desc, \
                     asc_nulls_first, asc_nulls_last, desc_nulls_first, desc_nulls_last",
                    name
                ))
            })?;
            let column = table_column_for(names, table, key);
            if table.get_column(column).is_some() {
                terms.push(format!(
                    "{}.{} {}",
                    reference,
                    postrust_sql::escape_ident(column),
                    sql
                ));
                continue;
            }
            // A computed field is a column as far as ordering is concerned:
            // the row is the argument, and the call is what is sorted on.
            let function = names
                .computed_source(&table.schema, &table.name, key)
                .unwrap_or(key);
            if let Some(definition) = table.get_computed_column(function) {
                terms.push(format!(
                    "{} {}",
                    computed_call(definition, &format!("{}.*", reference), reference),
                    sql
                ));
                continue;
            }
            return Err(async_graphql::Error::new(format!(
                "cannot order by unknown column \"{}\" on \"{}\"",
                key, table.name
            )));
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
                names,
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
    if plan.columns.is_empty() && plan.function.is_none() {
        return Err(async_graphql::Error::new(format!(
            "cannot order by an aggregate of \"{}\": it is not reached by a key",
            rel.name
        )));
    }
    // A computed relationship whose function takes more than the row cannot be
    // ordered by here: there is nowhere in `order_by` to write the arguments.
    if plan.function.is_some() && !rel.arguments.is_empty() {
        return Err(async_graphql::Error::new(format!(
            "cannot order by an aggregate of \"{}\": it takes arguments, and an \
             ordering has nowhere to write them",
            rel.name
        )));
    }

    *alias_counter += 1;
    let alias = postrust_sql::escape_ident(&format!("pgrst_ord_{}", alias_counter));
    // A computed relationship is correlated by argument -- the function takes
    // the parent row -- where a key relationship is correlated by columns.
    let (source, correlation) = match &plan.function {
        Some(function) => {
            let session = plan.row_argument.as_ref().and_then(|row_argument| {
                computed_session_argument(cache, function, row_argument)
                    .map(|session| (row_argument.clone(), session))
            });
            let call = match session {
                Some((row_argument, session)) => format!(
                    "{} => {}, {} => {}",
                    postrust_sql::escape_ident(&row_argument),
                    reference,
                    postrust_sql::escape_ident(&session),
                    SESSION_ARGUMENT
                ),
                None => reference.to_string(),
            };
            (
                format!(
                    "{}.{}({})",
                    postrust_sql::escape_ident(&function.schema),
                    postrust_sql::escape_ident(&function.name),
                    call
                ),
                "true".to_string(),
            )
        }
        None => (
            format!(
                "{}.{}",
                postrust_sql::escape_ident(&plan.foreign_schema),
                postrust_sql::escape_ident(&plan.foreign_table)
            ),
            plan.columns
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
                .join(" AND "),
        ),
    };

    // `{count: desc}` is one term; `{max: {id: desc}}` is one per column.
    let mut wanted: Vec<(String, String)> = Vec::new();
    for (function, argument) in spec {
        // A direction given as null is no direction, as everywhere else an
        // ordering is read.
        if argument.is_null() {
            continue;
        }
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
            if direction.is_null() {
                continue;
            }
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
            "(SELECT {} FROM {} AS {} WHERE {}) {}",
            expression, source, alias, correlation, direction
        ));
    }
    Ok(())
}

/// The `DISTINCT ON (...)` prefix, and the ordering that has to go with it.
///
/// PostgreSQL keeps the first row of each DISTINCT ON group in the query's own
/// order, and picks arbitrarily where the ordering does not begin with the
/// distinct columns -- so which row survives would depend on the plan. Hasura
/// refuses that query rather than answering it, and so does this: prepending
/// the distinct columns instead produced an answer, and a wrong one, since
/// `ORDER BY "department", "department" DESC` is decided by its first term and
/// sorts ascending. Where nothing was ordered at all there is nothing to
/// disagree with, so the distinct columns become the ordering.
///
/// `written` is the ordering as its comma-separated terms, without the
/// keyword; the answer is in the same spelling.
fn distinct_on_clause(
    distinct_on: &[String],
    written: Option<&str>,
) -> Result<(String, Option<String>), async_graphql::Error> {
    if distinct_on.is_empty() {
        return Ok((String::new(), written.map(str::to_string)));
    }
    let terms: Vec<&str> = written
        .map(|written| written.split(", ").collect())
        .unwrap_or_default();
    // Terms are qualified and the distinct columns may not be, so they are
    // compared by the name itself.
    let column_of = |term: &str| {
        let expression = term.split_whitespace().next().unwrap_or(term);
        expression
            .rsplit_once('.')
            .map(|(_, name)| name)
            .unwrap_or(expression)
            .to_string()
    };
    let leading: Vec<String> = terms
        .iter()
        .take(distinct_on.len())
        .map(|term| column_of(term))
        .collect();
    if !terms.is_empty()
        && (leading.len() != distinct_on.len()
            || distinct_on
                .iter()
                .any(|column| !leading.contains(&column_of(column))))
    {
        return Err(validation_error(
            "\"distinct_on\" columns must match initial \"order_by\" columns",
        ));
    }
    let order = match terms.is_empty() {
        true => Some(distinct_on.join(", ")),
        false => written.map(str::to_string),
    };
    Ok((format!("DISTINCT ON ({}) ", distinct_on.join(", ")), order))
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
    names: &crate::names::NameOverrides,
) -> Result<Vec<String>, async_graphql::Error> {
    let Ok(arg) = ctx.args.try_get("distinct_on") else {
        return Ok(Vec::new());
    };
    let value = accessor_to_json(&arg);
    let requested: Vec<String> = match &value {
        serde_json::Value::Array(items) => items
            .iter()
            .filter_map(|i| i.as_str().map(|s| s.to_string()))
            .collect(),
        serde_json::Value::String(one) => vec![one.clone()],
        _ => Vec::new(),
    };
    if requested.is_empty() {
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

    let mut quoted = Vec::with_capacity(requested.len());
    for field in requested {
        let name = table_column_for(names, table, &field).to_string();
        if table.get_column(&name).is_none() {
            return Err(async_graphql::Error::new(format!(
                "cannot take distinct on unknown column \"{}\" of \"{}\"",
                field, table_name
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
#[allow(clippy::too_many_arguments)] // the selection, its source, and the binding state
fn computed_projections(
    table: &postrust_core::schema_cache::Table,
    selection: async_graphql::SelectionField<'_>,
    row_reference: &str,
    names: &crate::names::NameOverrides,
    schema_cache: &SchemaCache,
    param_idx: &mut usize,
    values: &mut Vec<serde_json::Value>,
) -> Result<Vec<String>, async_graphql::Error> {
    let mut projections = Vec::new();
    for field in selection.selection_set() {
        let name = field.name();
        // A real column wins, and the projection already carries it -- under
        // this name, whether or not that is the column's own.
        if table
            .get_column(table_column_for(names, table, name))
            .is_some()
        {
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
        let arguments = crate::schema::computed_caller_arguments(computed, schema_cache);
        let given = match arguments.is_empty() {
            true => None,
            false => embed_arguments(field).remove("args"),
        };
        projections.push(format!(
            "{} AS {}",
            computed_column_call(
                computed,
                row_reference,
                name,
                &arguments,
                given.as_ref(),
                param_idx,
                values,
            )?,
            postrust_sql::escape_ident(name)
        ));
    }
    Ok(projections)
}

/// The call behind a computed field, including whatever the caller passed.
///
/// The ordinary case is a function of the row alone, which is written
/// positionally: that is the call REST writes, and the one a client that never
/// heard of `args` is asking for. Anything else has to be written by name --
/// the row is then not the only parameter and may not be the first -- so the
/// row, the session where the function asks for one, and each argument the
/// caller gave are all named.
fn computed_column_call(
    computed: &postrust_core::schema_cache::ComputedColumn,
    row: &str,
    field_name: &str,
    arguments: &[(String, String, bool)],
    given: Option<&serde_json::Value>,
    param_idx: &mut usize,
    values: &mut Vec<serde_json::Value>,
) -> Result<String, async_graphql::Error> {
    if arguments.is_empty() {
        return Ok(computed_call(computed, row, row));
    }
    let Some(row_argument) = &computed.row_argument else {
        return Ok(computed_call(computed, row, row));
    };
    let function = format!(
        "{}.{}",
        postrust_sql::escape_ident(&computed.function.schema),
        postrust_sql::escape_ident(&computed.function.name)
    );
    let mut passed = vec![format!(
        "{} => {}",
        postrust_sql::escape_ident(row_argument),
        row
    )];
    if let Some(session) = &computed.session_argument {
        passed.push(format!(
            "{} => {}",
            postrust_sql::escape_ident(session),
            SESSION_ARGUMENT
        ));
    }
    for (name, pg_type, required) in arguments {
        match given.and_then(|given| given.get(name)) {
            None if *required => {
                return Err(async_graphql::Error::new(format!(
                    "{} needs the argument \"{}\"",
                    field_name, name
                )))
            }
            // Left out is left out, so the function's own default applies.
            None => continue,
            Some(value) => {
                passed.push(format!(
                    "{} => ${}::{}",
                    postrust_sql::escape_ident(name),
                    param_idx,
                    pg_type
                ));
                *param_idx += 1;
                values.push(value.clone());
            }
        }
    }
    Ok(format!("{}({})", function, passed.join(", ")))
}

/// The `count(...)` a selection asked for.
///
/// `count` alone is `count(*)`, which counts rows. `count(columns: [a])` is
/// `count("a")`, which counts the rows where `a` is not null -- a different
/// number, and the one the client asked for. `distinct: true` counts the
/// distinct values among them, and several columns are counted as the row
/// they make, which is how PostgreSQL counts more than one thing at once.
///
/// The column names come from the generated enum, so a selection can only
/// name a column the table has; what it may not know is the column's own name
/// where the schema exposes it under another.
fn count_expression(
    field: async_graphql::SelectionField<'_>,
    table: (&str, &str),
    alias: Option<&str>,
    names: &crate::names::NameOverrides,
) -> String {
    let arguments = embed_arguments(field);
    let requested = match arguments.get("columns") {
        Some(serde_json::Value::Array(items)) => items
            .iter()
            .filter_map(|item| item.as_str())
            .collect::<Vec<_>>(),
        Some(serde_json::Value::String(one)) => vec![one.as_str()],
        _ => Vec::new(),
    };
    if requested.is_empty() {
        return "count(*)".to_string();
    }
    let columns: Vec<String> = requested
        .into_iter()
        .map(|column| {
            let (schema, name) = table;
            let source = column_for(names, schema, name, column);
            match alias {
                Some(alias) => format!(
                    "{}.{}",
                    postrust_sql::escape_ident(alias),
                    postrust_sql::escape_ident(source)
                ),
                None => postrust_sql::escape_ident(source),
            }
        })
        .collect();
    // One column counts itself; several count the row they make, since
    // `count(a, b)` is not a call PostgreSQL has.
    let counted = match columns.len() {
        1 => columns.into_iter().next().unwrap_or_default(),
        _ => format!("({})", columns.join(", ")),
    };
    let distinct = matches!(
        arguments.get("distinct"),
        Some(serde_json::Value::Bool(true))
    );
    match distinct {
        true => format!("count(DISTINCT {})", counted),
        false => format!("count({})", counted),
    }
}

/// The SELECT list for a nested aggregate.
///
/// `articles_aggregate { aggregate { count } nodes { title } }` becomes one
/// row per parent, correlated the way any embed is. Both halves are aggregates
/// over the same correlated set, so they go in one select list rather than two
/// queries -- `count(*)` and `json_agg(...)` read the same rows.
#[allow(clippy::too_many_arguments)] // the recursion carries its whole context
fn nested_aggregate_select(
    caller: &crate::role::Caller<'_>,
    selection: async_graphql::SelectionField<'_>,
    child_alias: &str,
    schema_cache: &SchemaCache,
    relationships: &HashMap<String, Vec<RelationshipField>>,
    target_type: &str,
    child_table: Option<&postrust_core::schema_cache::Table>,
    max_rows: Option<i64>,
    alias_counter: &mut usize,
    param_idx: &mut usize,
    values: &mut Vec<serde_json::Value>,
    names: &crate::names::NameOverrides,
) -> Result<String, async_graphql::Error> {
    use crate::schema::aggregate as agg;

    let mut parts: Vec<String> = Vec::new();

    for child in selection.selection_set() {
        match child.name() {
            "aggregate" => {
                let of_table = child_table
                    .map(|table| (table.schema.as_str(), table.name.as_str()))
                    .unwrap_or(("", ""));
                // Each `count` is answered under the name it was asked for,
                // since two of them may count different things. Every other
                // aggregate is one object read under however many names, so
                // its columns are the union of what all of them named.
                let mut counts: Vec<(String, String)> = Vec::new();
                let mut wanted: BTreeMap<&str, Vec<String>> = BTreeMap::new();
                for function in child.selection_set() {
                    if function.name() == "count" {
                        counts.push((
                            function.alias().unwrap_or("count").to_string(),
                            count_expression(function, of_table, Some(child_alias), names),
                        ));
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
                    let columns = wanted.entry(sql_function).or_default();
                    for column in function.selection_set() {
                        let name = column.name().to_string();
                        if !columns.contains(&name) {
                            columns.push(name);
                        }
                    }
                }
                // Answered whether or not it was asked for, for the reason
                // the root aggregate answers it.
                let mut fields = match counts.iter().any(|(key, _)| key == "count") {
                    true => Vec::new(),
                    false => vec!["'count', count(*)".to_string()],
                };
                for (key, expression) in &counts {
                    fields.push(format!("'{}', {}", key.replace('\'', "''"), expression));
                }
                for (sql_function, named) in &wanted {
                    let columns: Vec<String> = named
                        .iter()
                        .map(|column| {
                            let source = child_table
                                .map(|table| table_column_for(names, table, column))
                                .unwrap_or(column.as_str());
                            format!(
                                "'{}', {}({}.{})",
                                column.replace('\'', "''"),
                                sql_function,
                                postrust_sql::escape_ident(child_alias),
                                postrust_sql::escape_ident(source)
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
            // `nodes` under an aggregate is a rows selection like any other:
            // what it names may be a column, a computed field, or a
            // relationship of its own, and the last of those is a further
            // correlated subselect rather than a column of this one.
            "nodes" => {
                let embeds = build_embed_expressions(
                    caller,
                    schema_cache,
                    relationships,
                    target_type,
                    child_alias,
                    child,
                    max_rows,
                    alias_counter,
                    param_idx,
                    values,
                    names,
                )?;
                let mut columns: Vec<String> = Vec::new();
                for column in child.selection_set() {
                    let name = column.name();
                    if name == "__typename" || embeds.iter().any(|(field, _)| field == name) {
                        continue;
                    }
                    columns.push(format!(
                        "'{}', {}",
                        name.replace('\'', "''"),
                        column_expression(child_table, child_alias, name, names)
                    ));
                }
                for (field, expression) in &embeds {
                    columns.push(format!("'{}', {}", field.replace('\'', "''"), expression));
                }
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
    Ok(parts.join(", "))
}

/// How one named field of a row is read, given the alias the row goes by.
///
/// A column is itself; a computed field is the call that produces it. Which of
/// the two a name is depends on the table, so this is the one place that asks.
fn column_expression(
    table: Option<&postrust_core::schema_cache::Table>,
    alias: &str,
    name: &str,
    names: &crate::names::NameOverrides,
) -> String {
    let column = table
        .map(|t| table_column_for(names, t, name))
        .unwrap_or(name);
    let plain = format!(
        "{}.{}",
        postrust_sql::escape_ident(alias),
        postrust_sql::escape_ident(column)
    );
    let computed = table
        .filter(|t| t.get_column(column).is_none())
        .and_then(|t| {
            let function = names
                .computed_source(&t.schema, &t.name, name)
                .unwrap_or(name);
            t.get_computed_column(function)
        });
    match computed {
        Some(definition) => computed_call(
            definition,
            &format!("{}.*", postrust_sql::escape_ident(alias)),
            &postrust_sql::escape_ident(alias),
        ),
        None => plain,
    }
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
    names: &crate::names::NameOverrides,
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
        for (field, direction) in map {
            // A direction given as null is no direction, as at the root.
            if direction.is_null() {
                continue;
            }
            let column = table_column_for(names, table, field);
            if table.get_column(column).is_none() {
                return Err(async_graphql::Error::new(format!(
                    "cannot order by unknown column \"{}\" on \"{}\"",
                    field, table_name
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

/// Whether a to-one relationship always has a row at the other end.
///
/// It does when this row's own key columns cannot be null: a foreign key
/// guarantees a matching row for every value, so a `NOT NULL` key is a
/// relationship that cannot be absent, and the field is non-null. A client
/// generating types then gets `author` rather than `author | null`, which is
/// what the schema actually promises.
///
/// Only where a foreign key says so. A computed relationship is a function
/// call, and a one-to-one reached through the *child's* key is a row that may
/// simply not exist.
fn always_present(rel: &RelationshipField, table: &postrust_core::schema_cache::Table) -> bool {
    use postrust_core::schema_cache::{Cardinality, Relationship};
    let Relationship::ForeignKey { cardinality, .. } = &rel.relationship else {
        return false;
    };
    match cardinality {
        Cardinality::M2O { .. } => {}
        Cardinality::O2O {
            is_parent: false, ..
        } => {}
        _ => return false,
    }
    let columns = cardinality.columns();
    !columns.is_empty()
        && columns.iter().all(|(local, _)| {
            table
                .get_column(local)
                .is_some_and(|column| !column.nullable)
        })
}

/// Whether the row at the other end of a relationship carries the key.
///
/// If it does, this row is written first and its key is pushed down; if this
/// row's own column carries it, the other row is written first and its key is
/// read back. Cardinality alone does not answer it: a one-to-one is one row in
/// either direction, and which side holds the foreign key is the whole
/// difference between the two orders.
fn child_holds_the_key(relationship: &postrust_core::schema_cache::Relationship) -> bool {
    use postrust_core::schema_cache::{Cardinality, Relationship};
    match relationship {
        Relationship::ForeignKey { cardinality, .. } => match cardinality {
            Cardinality::M2O { .. } => false,
            Cardinality::O2O { is_parent, .. } => *is_parent,
            _ => true,
        },
        Relationship::Computed { .. } => true,
    }
}

/// The name of a function's session parameter, if it has one.
fn computed_session_argument(
    cache: &SchemaCache,
    function: &postrust_core::api_request::QualifiedIdentifier,
    row_argument: &str,
) -> Option<String> {
    let _ = row_argument;
    cache
        .routines
        .get(function)
        .into_iter()
        .flatten()
        .find(|routine| routine.name == function.name)?
        .params
        .iter()
        .find(|param| {
            param.name == "hasura_session" && matches!(param.param_type.as_str(), "json" | "jsonb")
        })
        .map(|param| param.name.clone())
}

/// The name of the input holding a computed relationship's own arguments.
fn computed_args_type_name(type_name: &str, field: &str) -> String {
    format!("{}_{}_args", type_name, field)
}

/// The argument list a computed relationship's function is called with.
///
/// `None` where the parent row is the only thing it takes -- the ordinary case,
/// and the positional call the REST surface writes. Anything else is written by
/// name: the row, the session where the function asks for one, and whatever the
/// client put under `args`.
fn computed_arguments(
    relationship: &RelationshipField,
    plan: &postrust_core::embed::EmbedPlan,
    parent_row: &str,
    given: Option<&serde_json::Value>,
    param_idx: &mut usize,
    values: &mut Vec<serde_json::Value>,
    schema_cache: &SchemaCache,
) -> Result<Option<String>, async_graphql::Error> {
    let (Some(function), Some(row_argument)) = (&plan.function, &plan.row_argument) else {
        return Ok(None);
    };
    let session = schema_cache
        .routines
        .get(function)
        .into_iter()
        .flatten()
        .find(|routine| routine.name == function.name)
        .and_then(|routine| {
            routine
                .params
                .iter()
                .find(|param| {
                    param.name == "hasura_session"
                        && matches!(param.param_type.as_str(), "json" | "jsonb")
                })
                .map(|param| param.name.clone())
        });
    if relationship.arguments.is_empty() && session.is_none() {
        return Ok(None);
    }

    let mut passed = vec![format!(
        "{} => {}",
        postrust_sql::escape_ident(row_argument),
        parent_row
    )];
    if let Some(session) = session {
        passed.push(format!(
            "{} => {}",
            postrust_sql::escape_ident(&session),
            SESSION_ARGUMENT
        ));
    }
    for (name, pg_type, required) in &relationship.arguments {
        match given.and_then(|given| given.get(name)) {
            None if *required => {
                return Err(async_graphql::Error::new(format!(
                    "{} needs the argument \"{}\"",
                    relationship.name, name
                )))
            }
            // Left out is left out, so the function's own default applies.
            None => continue,
            Some(value) => {
                passed.push(format!(
                    "{} => ${}::{}",
                    postrust_sql::escape_ident(name),
                    param_idx,
                    pg_type
                ));
                *param_idx += 1;
                values.push(value.clone());
            }
        }
    }
    Ok(Some(passed.join(", ")))
}

/// The arguments written on an embedded field, as JSON.
fn embed_arguments(field: async_graphql::SelectionField<'_>) -> HashMap<String, serde_json::Value> {
    field
        .arguments()
        .map(|args| {
            args.into_iter()
                .map(|(name, value)| {
                    (
                        name.to_string(),
                        plain_numbers(value.into_json().unwrap_or(serde_json::Value::Null)),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

/// What an embed's arguments narrow its child rows to: a `DISTINCT ON`
/// prefix, a predicate, an ordering, a limit and an offset, in the order the
/// SQL takes them.
type Narrowing = (String, Option<String>, Option<String>, Option<i64>, i64);

/// What an embed's arguments narrow its child rows to.
///
/// A list and an aggregate of that same list take the same four arguments and
/// mean the same thing by them; only what is done with the surviving rows
/// differs, so the reading of the arguments is shared.
#[allow(clippy::too_many_arguments)] // one parameter per SQL clause, plus the binding state
fn embed_narrowing(
    arguments: &HashMap<String, serde_json::Value>,
    plan: &postrust_core::embed::EmbedPlan,
    schema_cache: &SchemaCache,
    relationships: &HashMap<String, Vec<RelationshipField>>,
    target_type: &str,
    child_alias: &str,
    max_rows: Option<i64>,
    param_idx: &mut usize,
    values: &mut Vec<serde_json::Value>,
    names: &crate::names::NameOverrides,
    caller: &crate::role::Caller<'_>,
) -> Result<Narrowing, async_graphql::Error> {
    // The child's own permission, which is the whole reason an embed is a
    // place this has to be applied: `author { articles { ... } }` reads two
    // tables, and the rule on the second is not the rule on the first.
    let permission =
        permission_predicate(caller, names, &plan.foreign_schema, &plan.foreign_table)?;
    let requested = arguments.get("where").filter(|v| !v.is_null()).cloned();
    let child_where = match and_predicates(requested, permission) {
        Some(expression) => {
            let child_scope = WhereScope::for_alias(
                &plan.foreign_schema,
                &plan.foreign_table,
                child_alias,
                target_type,
                schema_cache,
                relationships,
                names,
            )
            .for_caller(*caller);
            let mut nested_alias = 0usize;
            build_condition(
                &expression,
                &child_scope,
                param_idx,
                values,
                &mut nested_alias,
            )?
        }
        None => None,
    };

    let child_order = match arguments.get("order_by") {
        Some(order) if !order.is_null() => order_terms(
            order,
            schema_cache,
            &plan.foreign_schema,
            &plan.foreign_table,
            child_alias,
            names,
        )?,
        _ => None,
    };

    // A limit written on the embed is what the client asked for; the
    // configured ceiling still applies as an upper bound, the same way it
    // does at the top level -- and so does the child's own permission, which
    // bounds rows per parent here rather than per query.
    let ceiling = match (
        max_rows,
        crate::role::read_limit(caller, names, &plan.foreign_schema, &plan.foreign_table),
    ) {
        (Some(configured), Some(granted)) => Some(configured.min(granted)),
        (only, None) | (None, only) => only,
    };
    let child_limit = match arguments.get("limit").and_then(|v| v.as_i64()) {
        Some(requested) => match ceiling {
            Some(ceiling) => Some(requested.min(ceiling)),
            None => Some(requested),
        },
        None => ceiling,
    };
    let child_offset = arguments
        .get("offset")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);

    // `author { articles(distinct_on: [title], order_by: {title: asc}) }` --
    // one article per title, the same question the root field answers and
    // under the same rule about what may be ordered by.
    let requested = match arguments.get("distinct_on") {
        Some(serde_json::Value::Array(items)) => items
            .iter()
            .filter_map(|item| item.as_str())
            .collect::<Vec<_>>(),
        Some(serde_json::Value::String(one)) => vec![one.as_str()],
        _ => Vec::new(),
    };
    let mut distinct_on = Vec::with_capacity(requested.len());
    if !requested.is_empty() {
        let qi = postrust_core::api_request::QualifiedIdentifier::new(
            &plan.foreign_schema,
            &plan.foreign_table,
        );
        let table = schema_cache.get_table(&qi).ok_or_else(|| {
            async_graphql::Error::new(format!("unknown table \"{}\"", plan.foreign_table))
        })?;
        for field in requested {
            let column = table_column_for(names, table, field).to_string();
            if table.get_column(&column).is_none() {
                return Err(async_graphql::Error::new(format!(
                    "cannot take distinct on unknown column \"{}\" of \"{}\"",
                    field, plan.foreign_table
                )));
            }
            distinct_on.push(format!(
                "{}.{}",
                postrust_sql::escape_ident(child_alias),
                postrust_sql::escape_ident(&column)
            ));
        }
    }
    let (distinct, child_order) = distinct_on_clause(&distinct_on, child_order.as_deref())?;

    Ok((
        distinct,
        child_where,
        child_order,
        child_limit,
        child_offset,
    ))
}

#[allow(clippy::too_many_arguments)] // one parameter per SQL clause, plus the binding state
fn build_embed_expressions(
    caller: &crate::role::Caller<'_>,
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

            // The same narrowing an embedded list takes. `where` here decides
            // which rows are counted, not which are returned, so it goes on the
            // set the aggregate reads rather than on the answer.
            let arguments = embed_arguments(field);
            let (child_distinct, child_where, child_order, child_limit, child_offset) =
                embed_narrowing(
                    &arguments,
                    &plan,
                    schema_cache,
                    relationships,
                    &rel.target_type,
                    &child_alias,
                    max_rows,
                    param_idx,
                    values,
                    names,
                    caller,
                )?;

            let child_table =
                schema_cache.get_table(&postrust_core::api_request::QualifiedIdentifier::new(
                    &plan.foreign_schema,
                    &plan.foreign_table,
                ));
            let select = nested_aggregate_select(
                caller,
                field,
                &child_alias,
                schema_cache,
                relationships,
                &rel.target_type,
                child_table,
                max_rows,
                alias_counter,
                param_idx,
                values,
                names,
            )?;

            let call = computed_arguments(
                rel,
                &plan,
                &postrust_sql::escape_ident(parent_alias),
                arguments.get("args"),
                param_idx,
                values,
                schema_cache,
            )?;
            let expression = plan
                .aggregate_expression(
                    parent_alias,
                    &postrust_sql::escape_ident(parent_alias),
                    &child_alias,
                    &select,
                    child_limit,
                    child_offset,
                    child_where.as_deref(),
                    child_order.as_deref(),
                    call.as_deref(),
                    &child_distinct,
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
            caller,
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
        let arguments = embed_arguments(field);
        let (child_distinct, child_where, child_order, child_limit, child_offset) =
            embed_narrowing(
                &arguments,
                &plan,
                schema_cache,
                relationships,
                &rel.target_type,
                &child_alias,
                max_rows,
                param_idx,
                values,
                names,
                caller,
            )?;

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
            let column = child_table
                .map(|t| table_column_for(names, t, name))
                .unwrap_or(name);
            let computed = child_table
                .filter(|t| t.get_column(column).is_none())
                .and_then(|t| {
                    let function = names
                        .computed_source(&t.schema, &t.name, name)
                        .unwrap_or(name);
                    t.get_computed_column(function)
                });
            match computed {
                Some(definition) => parts.push(format!(
                    "{} AS {}",
                    computed_call(
                        definition,
                        &format!("{}.*", postrust_sql::escape_ident(&child_alias)),
                        &postrust_sql::escape_ident(&child_alias),
                    ),
                    postrust_sql::escape_ident(name)
                )),
                // A column the child exposes under another name is selected as
                // that name, since the row it lands in is the client's.
                None if column != name => parts.push(format!(
                    "{} AS {}",
                    postrust_sql::escape_ident(column),
                    postrust_sql::escape_ident(name)
                )),
                None => parts.push(postrust_sql::escape_ident(name)),
            }
        }
        if parts.is_empty() && nested.is_empty() {
            parts.push(
                child_table
                    .and_then(|table| rename_projection(table, &child_alias, names))
                    .unwrap_or_else(|| format!("{}.*", postrust_sql::escape_ident(&child_alias))),
            );
        }
        for (field_name, expression) in nested {
            parts.push(format!(
                "{} AS {}",
                expression,
                postrust_sql::escape_ident(&field_name)
            ));
        }

        let computed_call_arguments = computed_arguments(
            rel,
            &plan,
            &postrust_sql::escape_ident(parent_alias),
            arguments.get("args"),
            param_idx,
            values,
            schema_cache,
        )?;
        let expression = plan
            .embed_expression(
                parent_alias,
                // A computed relationship is correlated by argument rather
                // than by a key: the function takes the parent row, and an
                // alias names that row.
                &postrust_sql::escape_ident(parent_alias),
                &child_alias,
                &format!("{}{}", child_distinct, parts.join(", ")),
                child_limit,
                child_offset,
                child_where.as_deref(),
                child_order.as_deref(),
                computed_call_arguments.as_deref(),
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

                let fetched = sqlx::query(&sql)
                    .bind(&keys)
                    .fetch_all(&mut *conn)
                    .await
                    .map_err(database_error)?;

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
    schema_name: &str,
    table_name: &str,
    pk_columns: &[(String, String)],
    names: &crate::names::NameOverrides,
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
        // Under the name the key column is exposed as, on both sides: the
        // argument the client wrote, and the predicate handed on to the same
        // builder every other `where` goes through.
        let field = names
            .column(schema_name, table_name, col_name)
            .unwrap_or(col_name);
        let value = match &from_object {
            Some(serde_json::Value::Object(map)) => map.get(field).cloned().ok_or_else(|| {
                async_graphql::Error::new(format!(
                    "pk_columns is missing the key column \"{}\"",
                    field
                ))
            })?,
            _ => {
                let arg = ctx.args.try_get(field).map_err(|_| {
                    async_graphql::Error::new(format!(
                        "missing required primary key argument \"{}\"",
                        field
                    ))
                })?;
                accessor_to_json(&arg)
            }
        };
        conditions.insert(field.to_string(), serde_json::json!({ "_eq": value }));
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

/// The key serde_json wraps a number in when it is keeping its digits.
///
/// This crate reads `serde_json` with `arbitrary_precision`, so a PostgreSQL
/// `numeric` survives a round trip through JSON with every digit it had. The
/// cost is on the way in: a fractional number in a *variable* is deserialized
/// into async-graphql's own value type as a one-key object holding the text,
/// because the generic path cannot know it is looking at a number. Integers
/// are unaffected, which is why this went unnoticed until a shape arrived by
/// variable and every coordinate in it read as zero.
const NUMBER_AS_TEXT: &str = "$serde_json::private::Number";

/// The number an arbitrary-precision wrapper is holding, if that is what this
/// object is.
fn number_kept_as_text(
    map: &serde_json::Map<String, serde_json::Value>,
) -> Option<serde_json::Value> {
    if map.len() != 1 {
        return None;
    }
    let text = map.get(NUMBER_AS_TEXT)?.as_str()?;
    serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .filter(|value| value.is_number())
}

/// The same, applied to a whole document that came in as one.
fn plain_numbers(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => match number_kept_as_text(&map) {
            Some(number) => number,
            None => serde_json::Value::Object(
                map.into_iter()
                    .map(|(k, v)| (k, plain_numbers(v)))
                    .collect(),
            ),
        },
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.into_iter().map(plain_numbers).collect())
        }
        other => other,
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
        match number_kept_as_text(&map) {
            Some(number) => number,
            None => serde_json::Value::Object(map),
        }
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
                always_generated: false,
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
                always_generated: false,
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

        let result = build_dynamic_schema(
            &generated,
            &cache,
            None,
            None,
            Arc::new(Default::default()),
            std::time::Duration::from_secs(30),
            None,
        );
        if let Err(ref e) = result {
            eprintln!("Schema build error: {:?}", e);
        }
        assert!(result.is_ok(), "Schema build failed: {:?}", result.err());
    }

    /// `_exists` becomes a subselect over the table it names, correlated with
    /// nothing -- which is the difference between it and a relationship.
    #[test]
    fn exists_is_a_subselect_over_another_table() {
        let cache = create_test_schema_cache();
        let names = crate::names::NameOverrides::default();
        let relationships: HashMap<String, Vec<RelationshipField>> = HashMap::new();
        let scope = WhereScope::table("public", "users", "users", &names)
            .with_resolution(&cache, &relationships);

        let mut param_idx = 1usize;
        let mut values: Vec<serde_json::Value> = Vec::new();
        let mut aliases = 0usize;
        let sql = build_condition(
            &serde_json::json!({
                "_exists": {"_table": "users", "_where": {"name": {"_eq": "Ann"}}}
            }),
            &scope,
            &mut param_idx,
            &mut values,
            &mut aliases,
        )
        .expect("compiles")
        .expect("constrains something");

        assert_eq!(
            sql,
            "EXISTS (SELECT 1 FROM \"public\".\"users\" AS \"pgrst_exists_1\" \
             WHERE \"pgrst_exists_1\".\"name\" = $1::text)"
        );
        assert_eq!(values, vec![serde_json::json!("Ann")]);
    }

    /// A `_table` nobody has is refused rather than written into the query.
    #[test]
    fn exists_names_a_table_the_server_has() {
        let cache = create_test_schema_cache();
        let names = crate::names::NameOverrides::default();
        let relationships: HashMap<String, Vec<RelationshipField>> = HashMap::new();
        let scope = WhereScope::table("public", "users", "users", &names)
            .with_resolution(&cache, &relationships);
        let error = build_condition(
            &serde_json::json!({"_exists": {"_table": "nowhere", "_where": {}}}),
            &scope,
            &mut 1,
            &mut Vec::new(),
            &mut 0,
        )
        .expect_err("a table that is not there is an error");
        assert!(
            error.message.contains("public.nowhere"),
            "{}",
            error.message
        );
    }

    /// A role granted "how many" and not "which" gets the count and no rows.
    ///
    /// A select permission naming no columns, with `allow_aggregations`, is
    /// Hasura's way of writing that. The table has an aggregate root and no
    /// row type, so `nodes` is not there to be a list of one -- and `count`
    /// takes no `columns`, because there are none to name.
    #[test]
    fn a_table_a_role_may_count_and_not_read_has_a_count_and_no_rows() {
        let cache = create_test_schema_cache();
        let names = crate::names::NameOverrides::parse(
            r#"{"tables": {"public.users": {"permissions":
                 {"counter": {"select": {"columns": [], "filter": {},
                                         "allow_aggregations": true}}}}}}"#,
        )
        .unwrap();
        let view = crate::role::cache_for_role(&cache, &names, "counter", false);
        assert!(!view.tables.is_empty(), "the table is there to be counted");

        let config = SchemaConfig {
            names: names.clone(),
            role: Some("counter".to_string()),
            ..SchemaConfig::default()
        };
        let generated = build_schema(&view, &config);
        let schema = build_dynamic_schema(
            &generated,
            &view,
            None,
            None,
            Arc::new(names),
            std::time::Duration::from_secs(30),
            Some("counter"),
        )
        .expect("a role that may only count still has a buildable schema");
        let sdl = schema.sdl();

        assert!(
            sdl.contains("users_aggregate"),
            "the count is there:\n{}",
            sdl
        );
        assert!(sdl.contains("count: Int!"), "and it is a count:\n{}", sdl);
        // No row type, so nothing that would answer with one.
        assert!(!sdl.contains("type users "), "no row type:\n{}", sdl);
        assert!(
            !sdl.contains("nodes"),
            "and no rows beside the numbers:\n{}",
            sdl
        );
        assert!(
            !sdl.contains("users_select_column"),
            "no column enum to order by or count over:\n{}",
            sdl
        );
        // The functions that read column data are absent with the columns.
        assert!(!sdl.contains("users_max_fields"), "no max:\n{}", sdl);
    }

    /// A role granted `insert` and no `select` gets the write and no type.
    ///
    /// Hasura's own corpus does this eight times over. What has to hold is
    /// that the schema still builds: `insert_users` is there, `users` is not,
    /// and the response the insert answers with has no `returning` to name a
    /// type that does not exist.
    #[test]
    fn a_table_a_role_may_only_write_gets_the_write_and_no_type() {
        let cache = create_test_schema_cache();
        let names = crate::names::NameOverrides::parse(
            r#"{"tables": {"public.users": {"permissions":
                 {"writer": {"insert": {"columns": "*", "check": {}}}}}}}"#,
        )
        .unwrap();
        let view = crate::role::cache_for_role(&cache, &names, "writer", false);
        let config = SchemaConfig {
            names: names.clone(),
            role: Some("writer".to_string()),
            ..SchemaConfig::default()
        };
        let generated = build_schema(&view, &config);

        let schema = build_dynamic_schema(
            &generated,
            &view,
            None,
            None,
            Arc::new(names),
            std::time::Duration::from_secs(30),
            Some("writer"),
        )
        .expect("a role that may only write still has a buildable schema");
        let sdl = schema.sdl();

        assert!(
            sdl.contains("insert_users("),
            "the write is there:\n{}",
            sdl
        );
        assert!(
            sdl.contains("users_insert_input"),
            "and its input:\n{}",
            sdl
        );
        // No row type, and so none of the three fields that answer with one.
        assert!(!sdl.contains("type users "), "no row type:\n{}", sdl);
        assert!(!sdl.contains("insert_users_one"), "no insert_one:\n{}", sdl);
        assert!(
            !sdl.contains("returning"),
            "and nothing to return:\n{}",
            sdl
        );
        assert!(
            sdl.contains("users_mutation_response"),
            "the count is still answered:\n{}",
            sdl
        );
        assert!(
            !sdl.contains("_mutation_response_mutation_response"),
            "and no type is named twice over:\n{}",
            sdl
        );
    }

    /// A column the role may write and not read is in the input and not in the
    /// type. The two column sets are the point: one schema cache, two answers.
    #[test]
    fn a_column_a_role_may_set_without_seeing_is_in_the_input_alone() {
        let cache = create_test_schema_cache();
        let names = crate::names::NameOverrides::parse(
            r#"{"tables": {"public.users": {"permissions":
                 {"user": {"select": {"columns": ["id"], "filter": {}},
                           "insert": {"columns": "*", "check": {}}}}}}}"#,
        )
        .unwrap();
        let view = crate::role::cache_for_role(&cache, &names, "user", false);
        let config = SchemaConfig {
            names: names.clone(),
            role: Some("user".to_string()),
            ..SchemaConfig::default()
        };
        let generated = build_schema(&view, &config);
        let object = &generated.object_types["users"];
        assert_eq!(
            object
                .fields
                .iter()
                .map(|f| f.name.as_str())
                .collect::<Vec<_>>(),
            vec!["id"],
            "the type shows what may be read"
        );
        assert!(
            object.writable_fields.iter().any(|f| f.name == "name"),
            "and the input keeps what may be written"
        );

        let schema = build_dynamic_schema(
            &generated,
            &view,
            None,
            None,
            Arc::new(names),
            std::time::Duration::from_secs(30),
            Some("user"),
        )
        .expect("schema builds");
        let sdl = schema.sdl();
        let input = sdl
            .split("input users_insert_input")
            .nth(1)
            .and_then(|rest| rest.split('}').next())
            .expect("the insert input is registered");
        assert!(input.contains("name:"), "settable without being readable");
    }

    /// A column PostgreSQL generates always is in the row type and in no
    /// write input. It is not a permission -- no role may name it -- so the
    /// refusal belongs to the schema rather than to the database.
    #[test]
    fn a_generated_column_is_readable_and_not_writable() {
        let mut cache = create_test_schema_cache();
        cache
            .tables
            .get_mut(&postrust_core::api_request::QualifiedIdentifier::new(
                "public", "users",
            ))
            .expect("the fixture has users")
            .columns
            .get_mut("id")
            .expect("with an id")
            .always_generated = true;

        let names = crate::names::NameOverrides::default();
        let config = SchemaConfig {
            names: names.clone(),
            ..SchemaConfig::default()
        };
        let generated = build_schema(&cache, &config);
        let schema = build_dynamic_schema(
            &generated,
            &cache,
            None,
            None,
            Arc::new(names),
            std::time::Duration::from_secs(30),
            None,
        )
        .expect("schema builds");
        let sdl = schema.sdl();

        let block = |header: &str| {
            sdl.split(header)
                .nth(1)
                .and_then(|rest| rest.split('}').next())
                .unwrap_or_else(|| panic!("{} is registered:\n{}", header, sdl))
                .to_string()
        };
        assert!(
            block("type users ").contains("id:"),
            "still readable:\n{}",
            sdl
        );
        for header in ["input users_insert_input", "input users_set_input"] {
            assert!(
                !block(header).contains("id:"),
                "{} may not name it:\n{}",
                header,
                block(header)
            );
        }
        // The table's only number was the generated column, so there is
        // nothing left to add to and no type for adding to it.
        assert!(
            !sdl.contains("users_inc_input"),
            "and nothing is left to increment:\n{}",
            sdl
        );
        assert!(
            block("input users_insert_input").contains("name:"),
            "and the rest of the table is untouched",
        );
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

        let _query = create_query_type(
            &generated,
            None,
            Arc::new(HashMap::new()),
            Arc::new(Default::default()),
        );
    }

    #[test]
    fn test_create_mutation_type() {
        let cache = create_test_schema_cache();
        let config = SchemaConfig::default();
        let generated = build_schema(&cache, &config);

        let _mutation = create_mutation_type(
            &generated,
            Arc::new(HashMap::new()),
            Arc::new(HashMap::new()),
            Arc::new(Default::default()),
            None,
            None,
        );
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

        let schema = build_dynamic_schema(
            &generated,
            &cache,
            None,
            None,
            Arc::new(Default::default()),
            std::time::Duration::from_secs(30),
            None,
        );
        assert!(schema.is_ok(), "{:?}", schema.err());

        let sdl = schema.unwrap().sdl();
        assert!(
            sdl.contains("_bool_exp"),
            "no boolean expressions in:\n{}",
            sdl
        );
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
        let result = build_dynamic_schema(
            &generated,
            &cache,
            Some(&sub_fields),
            None,
            Arc::new(Default::default()),
            std::time::Duration::from_secs(30),
            None,
        );
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
        let cache = create_test_schema_cache();
        let config = SchemaConfig::default();
        let generated = build_schema(&cache, &config);
        // A live query per queryable root: the subscription root mirrors the
        // query root, which is the whole contract.
        let _subscription = create_subscription_type(
            &generated,
            None,
            Arc::new(Default::default()),
            Arc::new(Default::default()),
            std::time::Duration::from_secs(30),
        );
    }
}

/// One step of a JSON path: a key of an object or an index of an array.
#[derive(Debug, PartialEq, Eq)]
enum PathStep {
    Key(String),
    Index(usize),
}

/// Read one part of a document, by the path spelling Hasura accepts.
///
/// `column(path: "objs[0]['你好']")` -- an optional leading `$`, then keys
/// written bare or after a dot, and indices or quoted keys written in
/// brackets. A step that names something the document does not have is null,
/// which is what `#>` answers to the same path.
fn walk_json_path(value: &Value, path: &str) -> Result<Value, async_graphql::Error> {
    let mut at = value;
    for step in parse_json_path(path)? {
        let next = match (&step, at) {
            (PathStep::Key(key), Value::Object(map)) => map.get(key.as_str()),
            (PathStep::Index(index), Value::List(items)) => items.get(*index),
            // An index of an object or a key of an array reads nothing, and so
            // does anything asked of a scalar.
            _ => None,
        };
        match next {
            Some(next) => at = next,
            None => return Ok(Value::Null),
        }
    }
    Ok(at.clone())
}

/// A path whose bracket never closes: written wrong rather than written oddly.
fn unclosed_bracket(path: &str) -> async_graphql::Error {
    let message = format!("\"{}\" is not a JSON path: a bracket is never closed", path);
    validation_error(&message)
}

/// Parse the path spelling Hasura accepts into its steps.
///
/// Deliberately tolerant about separators, because the corpus writes the same
/// path four ways: `objs[0]["x"]`, `objs[0].["x"]`, `objs.[0].["x"]` and
/// `.objs[0]['x']` all name one thing. What is not tolerated is a bracket that
/// never closes, which is a path the client got wrong rather than a spelling.
fn parse_json_path(path: &str) -> Result<Vec<PathStep>, async_graphql::Error> {
    let mut steps = Vec::new();
    let mut chars = path.chars().peekable();
    // `$` is the document itself, and naming it is optional.
    if chars.peek() == Some(&'$') {
        chars.next();
    }
    while let Some(&c) = chars.peek() {
        match c {
            // A separator carries no step of its own.
            '.' => {
                chars.next();
            }
            '[' => {
                chars.next();
                let quote = match chars.peek() {
                    Some(&q @ ('\'' | '"')) => {
                        chars.next();
                        Some(q)
                    }
                    _ => None,
                };
                let mut inner = String::new();
                loop {
                    let Some(c) = chars.next() else {
                        return Err(unclosed_bracket(path));
                    };
                    match (quote, c) {
                        (Some(quote), c) if c == quote => break,
                        (None, ']') => break,
                        _ => inner.push(c),
                    }
                }
                // A quoted key's closing bracket, which the quote ended before.
                if quote.is_some() {
                    match chars.next() {
                        Some(']') => {}
                        _ => return Err(unclosed_bracket(path)),
                    }
                }
                // An unquoted number is an index; anything else is a key,
                // since an object may be keyed by a word as well as a digit.
                match (quote, inner.parse::<usize>()) {
                    (None, Ok(index)) => steps.push(PathStep::Index(index)),
                    _ => steps.push(PathStep::Key(inner)),
                }
            }
            _ => {
                let mut key = String::new();
                while let Some(&c) = chars.peek() {
                    if matches!(c, '.' | '[') {
                        break;
                    }
                    key.push(c);
                    chars.next();
                }
                steps.push(PathStep::Key(key));
            }
        }
    }
    Ok(steps)
}

#[cfg(test)]
mod json_path_tests {
    use super::*;

    fn steps(path: &str) -> Vec<PathStep> {
        parse_json_path(path).expect("the path parses")
    }

    fn key(name: &str) -> PathStep {
        PathStep::Key(name.to_string())
    }

    #[test]
    fn the_document_itself_has_no_steps() {
        assert_eq!(steps("$"), vec![]);
        assert_eq!(steps(""), vec![]);
    }

    #[test]
    fn a_key_is_written_bare_or_after_a_dot() {
        assert_eq!(steps("a"), vec![key("a")]);
        assert_eq!(steps(".obj.c1"), vec![key("obj"), key("c1")]);
        assert_eq!(steps("._underscore"), vec![key("_underscore")]);
    }

    #[test]
    fn a_bracket_holds_an_index_or_a_quoted_key() {
        assert_eq!(steps("arr[0]"), vec![key("arr"), PathStep::Index(0)]);
        assert_eq!(steps("['!@#$%^']"), vec![key("!@#$%^")]);
        assert_eq!(
            steps("translations['hello world!']"),
            vec![key("translations"), key("hello world!")]
        );
    }

    /// The corpus writes one path four ways, and Hasura reads them all alike.
    #[test]
    fn a_dot_before_a_bracket_changes_nothing() {
        let expected = vec![key("objs"), PathStep::Index(0), key("\u{4f60}\u{597d}")];
        for spelling in [
            "objs[0]['\u{4f60}\u{597d}']",
            "objs[0][\"\u{4f60}\u{597d}\"]",
            "objs[0].[\"\u{4f60}\u{597d}\"]",
            "objs.[0].[\"\u{4f60}\u{597d}\"]",
        ] {
            assert_eq!(steps(spelling), expected, "{}", spelling);
        }
    }

    #[test]
    fn a_bracket_that_never_closes_is_refused() {
        assert!(parse_json_path("arr[0").is_err());
        assert!(parse_json_path("['key'").is_err());
    }

    #[test]
    fn a_step_the_document_does_not_have_reads_null() {
        let document = Value::from_json(serde_json::json!({
            "obj": {"c1": "c2"},
            "arr": [1, 2, 3],
        }))
        .expect("a document");
        let at = |path| walk_json_path(&document, path).expect("the path parses");
        assert_eq!(at(".obj.c1"), Value::String("c2".to_string()));
        assert_eq!(at("arr[1]"), Value::Number(2.into()));
        assert_eq!(at("arr[9]"), Value::Null);
        assert_eq!(at(".obj.nothing"), Value::Null);
        // An index of an object, and a key of an array: neither reads
        // anything, which is what `#>` answers.
        assert_eq!(at("obj[0]"), Value::Null);
        assert_eq!(at("arr.first"), Value::Null);
    }
}
