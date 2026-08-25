//! GraphQL schema generation from PostgreSQL schema cache.
//!
//! Builds a dynamic GraphQL schema from the database schema cache,
//! creating query and mutation types for each table.

pub mod aggregate;
pub mod object;
pub mod relationship;

use crate::input::mutation::{is_deletable, is_insertable, is_updatable};
use crate::schema::object::TableObjectType;
use crate::schema::relationship::RelationshipField;
use postrust_core::schema_cache::{SchemaCache, Table};
use std::collections::HashMap;

/// Configuration for schema generation.
#[derive(Debug, Clone)]
pub struct SchemaConfig {
    /// Schemas to expose in GraphQL (e.g., ["public"])
    pub exposed_schemas: Vec<String>,
    /// Whether to generate mutation types
    pub enable_mutations: bool,
    /// Whether to generate subscription types
    pub enable_subscriptions: bool,
    /// The members of each enum table, keyed by `schema.table`, as
    /// `(value, comment)`.
    ///
    /// These are rows rather than schema, so they cannot come from the schema
    /// cache: they are read once at startup, for the tables the configuration
    /// marks. A table marked as an enum whose values are absent is exposed as
    /// an ordinary table -- an empty GraphQL enum is not a legal type, and
    /// refusing to start over a table with no rows in it would be worse.
    pub enum_values: HashMap<String, Vec<(String, Option<String>)>>,
    /// Names given rather than derived.
    ///
    /// Empty by default, which is every name derived from the schema exactly
    /// as before.
    pub names: crate::names::NameOverrides,
    /// Ceiling on rows a single query may return (`PGRST_MAX_ROWS`).
    ///
    /// Applied when a query supplies no `limit` of its own, and as an upper
    /// bound when it supplies a larger one. `None` leaves queries unbounded.
    pub max_rows: Option<i64>,
}

impl Default for SchemaConfig {
    fn default() -> Self {
        Self {
            exposed_schemas: vec!["public".to_string()],
            enum_values: HashMap::new(),
            names: crate::names::NameOverrides::default(),
            enable_mutations: true,
            enable_subscriptions: false,
            max_rows: None,
        }
    }
}

impl SchemaConfig {
    /// Create a new schema config.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the exposed schemas.
    pub fn with_schemas(mut self, schemas: Vec<String>) -> Self {
        self.exposed_schemas = schemas;
        self
    }

    /// Enable or disable mutations.
    pub fn with_mutations(mut self, enable: bool) -> Self {
        self.enable_mutations = enable;
        self
    }

    /// Enable or disable subscriptions.
    pub fn with_subscriptions(mut self, enable: bool) -> Self {
        self.enable_subscriptions = enable;
        self
    }

    /// Check if a schema is exposed.
    pub fn is_schema_exposed(&self, schema: &str) -> bool {
        self.exposed_schemas.iter().any(|s| s == schema)
    }

    /// The schema whose tables get unqualified GraphQL names.
    ///
    /// This is the first exposed schema, mirroring how the REST surface treats
    /// the first entry of `PGRST_DB_SCHEMAS` as the default.
    pub fn default_schema(&self) -> &str {
        self.exposed_schemas
            .first()
            .map(|s| s.as_str())
            .unwrap_or("public")
    }
}

/// Primary key columns of a table, as `(column name, PostgreSQL type)`.
///
/// `nominal_type` (the underlying `udt_name`) is used because it is always a
/// castable type name.
fn pk_columns_of(table: &Table) -> Vec<(String, String)> {
    table
        .pk_cols
        .iter()
        .map(|col_name| {
            let pg_type = table
                .get_column(col_name)
                .map(|c| c.nominal_type.clone())
                .unwrap_or_else(|| "text".to_string());
            (col_name.clone(), pg_type)
        })
        .collect()
}

/// Every key a relationship may be named under, most specific first.
///
/// More than one, because there is more than one way to identify the same
/// relationship and a document may be written by hand or converted from
/// Hasura's metadata, which uses a different one. A constraint names exactly
/// one relationship even where two of them point at the same table; a computed
/// relationship has no constraint and is named by its function; and Hasura
/// names both directions by a column -- the foreign key column on this table
/// for a relationship to one row, and `table.column` on the far side for a
/// relationship to many. All of them are accepted, so a converted document
/// needs no database to turn a column into the constraint that carries it.
fn relationship_keys(rel: &postrust_core::schema_cache::Relationship) -> Vec<String> {
    use postrust_core::schema_cache::Relationship;

    if let Relationship::Computed { function, .. } = rel {
        return vec![function.name.clone()];
    }

    let mut keys = Vec::new();
    match rel.constraint_name() {
        "" => {}
        constraint => keys.push(constraint.to_string()),
    }
    // The column that carries the key, on whichever side carries it. A
    // one-to-one may have it on either, and two relationships between the same
    // pair of tables can share the local column while differing in the far one
    // -- so the more specific spelling is offered first and the bare column
    // last.
    if let Some(column) = rel.single_foreign_column() {
        keys.push(format!("{}.{}", rel.foreign_table().name, column));
    }
    if !rel.is_one_to_many() {
        if let Some(column) = rel.single_local_column() {
            keys.push(column.to_string());
        }
    }
    keys
}

/// A type name as the catalogue writes it, reduced to the table it names.
///
/// `pg_catalog.format_type` quotes anything that needs quoting and qualifies
/// anything outside the search path, so a function returning rows of a table
/// called `user` -- a reserved word -- reports `"user"`, and one returning rows
/// of `other.thing` reports `other.thing`. Neither is the table's name, and
/// comparing them against one finds nothing.
fn type_name_of(rendered: &str) -> (Option<String>, String) {
    let (schema, name) = match rendered.rsplit_once('.') {
        Some((schema, name)) => (Some(schema.trim_matches('"').to_string()), name),
        None => (None, rendered),
    };
    (schema, name.trim_matches('"').to_string())
}

/// Base name used to derive a table's GraphQL type and field names.
///
/// Tables in the default schema keep their bare name, so a single-schema
/// deployment is unaffected. Tables in any other exposed schema are prefixed
/// with the schema, because GraphQL has one flat namespace: without this, a
/// `users` table in both `public` and `api` would generate the same type and
/// field names and one would silently replace the other.
fn base_name_for(table: &Table, config: &SchemaConfig) -> String {
    // A name that was given is the answer; the rest of this is what to call a
    // table nobody named.
    if let Some(given) = config.names.base_name(&table.schema, &table.name) {
        return given.to_string();
    }
    if table.schema == config.default_schema() {
        table.name.clone()
    } else {
        format!("{}_{}", table.schema, table.name)
    }
}

/// Represents a generated GraphQL schema.
#[derive(Debug, Clone)]
pub struct GeneratedSchema {
    /// Object types for each table
    pub object_types: HashMap<String, TableObjectType>,
    /// Query fields
    pub query_fields: Vec<QueryField>,
    /// Mutation fields (if enabled)
    pub mutation_fields: Vec<MutationField>,
    /// Relationship fields for each type
    pub relationship_fields: HashMap<String, Vec<RelationshipField>>,
    /// GraphQL enums generated from enum tables: type name to
    /// `(member, description)`.
    pub enum_types: HashMap<String, Vec<(String, Option<String>)>>,
    /// Database functions exposed as root fields.
    pub function_fields: Vec<FunctionField>,
}

/// A database function returning rows of a table, exposed as a root field.
///
/// A function that returns `SETOF <table>` answers the same question a table
/// does, from a query somebody wrote in SQL rather than from the table
/// directly. It filters, orders and pages like the table it returns, and its
/// own arguments arrive under `args` so they cannot collide with those.
///
/// Where it appears follows from what PostgreSQL says it does: a function
/// declared VOLATILE may write, so it is a mutation; one declared STABLE or
/// IMMUTABLE may not, so it is a query. Nothing else is a safe place to draw
/// that line -- the alternative is trusting a name.
#[derive(Debug, Clone)]
pub struct FunctionField {
    /// The field's name, which is the function's.
    pub name: String,
    /// Schema the function lives in.
    pub schema_name: String,
    /// The function's own name, which the field is named after.
    pub function_name: String,
    /// The GraphQL type of the rows it returns.
    pub returns: String,
    /// The table those rows belong to, as `(schema, table)`.
    pub returns_table: (String, String),
    /// Its arguments, as `(name, PostgreSQL type, required)`.
    pub arguments: Vec<(String, String, bool)>,
    /// Whether it may write, and so belongs on the mutation root.
    pub volatile: bool,
    /// Description from the function's comment.
    pub description: Option<String>,
}

impl GeneratedSchema {
    /// Get an object type by name.
    pub fn get_object_type(&self, name: &str) -> Option<&TableObjectType> {
        self.object_types.get(name)
    }

    /// Get query fields for a table.
    pub fn get_query_field(&self, table_name: &str) -> Option<&QueryField> {
        self.query_fields
            .iter()
            .find(|f| f.table_name == table_name)
    }

    /// Get mutation fields for a table.
    pub fn get_mutation_fields(&self, table_name: &str) -> Vec<&MutationField> {
        self.mutation_fields
            .iter()
            .filter(|f| f.table_name == table_name)
            .collect()
    }

    /// Get relationship fields for a type.
    pub fn get_relationship_fields(&self, type_name: &str) -> Option<&Vec<RelationshipField>> {
        self.relationship_fields.get(type_name)
    }

    /// Get all table names.
    pub fn table_names(&self) -> Vec<&str> {
        self.object_types
            .values()
            .map(|t| t.table.name.as_str())
            .collect()
    }

    /// Get all type names.
    pub fn type_names(&self) -> Vec<&str> {
        self.object_types.keys().map(|s| s.as_str()).collect()
    }
}

/// A query field for a table (e.g., users, userByPk).
#[derive(Debug, Clone)]
pub struct QueryField {
    /// Field name (e.g., "users")
    pub name: String,
    /// Table name
    pub table_name: String,
    /// Schema the table lives in
    pub schema_name: String,
    /// GraphQL object type name (e.g., "Users")
    pub type_name: String,
    /// GraphQL return type
    pub return_type: String,
    /// Whether this returns a list
    pub is_list: bool,
    /// Whether this is a "by PK" query
    pub is_by_pk: bool,
    /// Primary key columns, as `(column name, PostgreSQL type)`.
    ///
    /// Populated for by-PK queries so the resolver can filter on the table's
    /// actual key rather than assuming a column called `id`. Empty for list
    /// queries.
    pub pk_columns: Vec<(String, String)>,
    /// Field description
    pub description: Option<String>,
    /// The name given to this table's aggregate root, if one was.
    ///
    /// The aggregate field is built beside the list field rather than being a
    /// `QueryField` of its own, so the name it was given rides along with the
    /// list field that spawns it.
    pub aggregate_name: Option<String>,
    /// The description given to that aggregate root, if one was. `Some("")`
    /// means metadata said it has none.
    pub aggregate_description: Option<String>,
}

impl QueryField {
    /// Create a list query field (e.g., users), named after the table.
    pub fn list(table: &Table) -> Self {
        Self::list_named(table, &table.name)
    }

    /// Create a list query field using an explicit base name.
    pub fn list_named(table: &Table, base_name: &str) -> Self {
        let type_name = base_name.to_string();
        let name = base_name.to_string();

        Self {
            name,
            table_name: table.name.clone(),
            schema_name: table.schema.clone(),
            type_name: type_name.clone(),
            return_type: format!("[{}!]!", type_name),
            is_list: true,
            is_by_pk: false,
            pk_columns: Vec::new(),
            description: Some(format!("fetch data from the table: \"{}\"", table.name)),
            aggregate_name: None,
            aggregate_description: None,
        }
    }

    /// Create a by-PK query field (e.g., userByPk), named after the table.
    pub fn by_pk(table: &Table) -> Option<Self> {
        Self::by_pk_named(table, &table.name)
    }

    /// Create a by-PK query field using an explicit base name.
    pub fn by_pk_named(table: &Table, base_name: &str) -> Option<Self> {
        if table.pk_cols.is_empty() {
            return None;
        }

        let type_name = base_name.to_string();
        let field_name = format!("{}_by_pk", base_name);

        // Carry the key columns and their types so the resolver can filter on
        // the real primary key.
        let pk_columns = pk_columns_of(table);

        Some(Self {
            name: field_name,
            table_name: table.name.clone(),
            schema_name: table.schema.clone(),
            type_name: type_name.clone(),
            return_type: type_name,
            is_list: false,
            is_by_pk: true,
            pk_columns,
            description: Some(format!(
                "fetch data from the table: \"{}\" using primary key columns",
                table.name
            )),
            aggregate_name: None,
            aggregate_description: None,
        })
    }
}

/// A mutation field for a table.
#[derive(Debug, Clone)]
pub struct MutationField {
    /// Field name (e.g., "insertUsers")
    pub name: String,
    /// Table name
    pub table_name: String,
    /// Schema the table lives in
    pub schema_name: String,
    /// Mutation type
    pub mutation_type: MutationType,
    /// Primary key columns, as `(column name, PostgreSQL type)`.
    ///
    /// Populated for by-PK mutations so the resolver can target the row by its
    /// key. Empty for bulk mutations.
    pub pk_columns: Vec<(String, String)>,
    /// GraphQL return type
    pub return_type: String,
    /// Field description
    pub description: Option<String>,
}

/// Types of mutations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MutationType {
    /// Insert multiple records
    Insert,
    /// Insert a single record
    InsertOne,
    /// Update records matching a filter
    Update,
    /// Update a single record by PK
    UpdateByPk,
    /// Several updates, each with its own filter, in one transaction
    UpdateMany,
    /// Delete records matching a filter
    Delete,
    /// Delete a single record by PK
    DeleteByPk,
}

impl MutationField {
    /// Create insert mutation fields for a table.
    pub fn insert_fields(table: &Table) -> Vec<Self> {
        Self::insert_fields_named(table, &table.name)
    }

    /// As [`Self::insert_fields`], with an explicit base name for the generated
    /// field and type names.
    pub fn insert_fields_named(table: &Table, base_name: &str) -> Vec<Self> {
        if !is_insertable(table) {
            return vec![];
        }

        let type_name = base_name.to_string();

        let mut fields = vec![];

        // insert_users (batch insert)
        let name = format!("insert_{}", base_name);
        fields.push(Self {
            name,
            table_name: table.name.clone(),
            schema_name: table.schema.clone(),
            mutation_type: MutationType::Insert,
            pk_columns: Vec::new(),
            return_type: format!("{}_mutation_response", type_name),
            description: Some(format!("insert data into the table: \"{}\"", table.name)),
        });

        // insert_user_one (single insert)
        let name = format!("insert_{}_one", base_name);
        fields.push(Self {
            name,
            table_name: table.name.clone(),
            schema_name: table.schema.clone(),
            mutation_type: MutationType::InsertOne,
            pk_columns: Vec::new(),
            return_type: type_name.clone(),
            description: Some(format!(
                "insert a single row into the table: \"{}\"",
                table.name
            )),
        });

        fields
    }

    /// Create update mutation fields for a table.
    pub fn update_fields(table: &Table) -> Vec<Self> {
        Self::update_fields_named(table, &table.name)
    }

    /// As [`Self::update_fields`], with an explicit base name for the generated
    /// field and type names.
    pub fn update_fields_named(table: &Table, base_name: &str) -> Vec<Self> {
        if !is_updatable(table) {
            return vec![];
        }

        let type_name = base_name.to_string();

        let mut fields = vec![];

        // update_users (batch update)
        let name = format!("update_{}", base_name);
        fields.push(Self {
            name,
            table_name: table.name.clone(),
            schema_name: table.schema.clone(),
            mutation_type: MutationType::Update,
            pk_columns: Vec::new(),
            return_type: format!("{}_mutation_response", type_name),
            description: Some(format!("update data of the table: \"{}\"", table.name)),
        });

        // update_users_many (several filters, each with its own values)
        let name = format!("update_{}_many", base_name);
        fields.push(Self {
            name,
            table_name: table.name.clone(),
            schema_name: table.schema.clone(),
            mutation_type: MutationType::UpdateMany,
            pk_columns: Vec::new(),
            return_type: format!("[{}_mutation_response]", type_name),
            description: Some(format!(
                "update multiples rows of table: \"{}\"",
                table.name
            )),
        });

        // update_user_by_pk (single update by PK)
        if !table.pk_cols.is_empty() {
            let name = format!("update_{}_by_pk", base_name);
            fields.push(Self {
                name,
                table_name: table.name.clone(),
                schema_name: table.schema.clone(),
                mutation_type: MutationType::UpdateByPk,
                pk_columns: pk_columns_of(table),
                return_type: type_name,
                description: Some(format!(
                    "update single row of the table: \"{}\"",
                    table.name
                )),
            });
        }

        fields
    }

    /// Create delete mutation fields for a table.
    pub fn delete_fields(table: &Table) -> Vec<Self> {
        Self::delete_fields_named(table, &table.name)
    }

    /// As [`Self::delete_fields`], with an explicit base name for the generated
    /// field and type names.
    pub fn delete_fields_named(table: &Table, base_name: &str) -> Vec<Self> {
        if !is_deletable(table) {
            return vec![];
        }

        let type_name = base_name.to_string();

        let mut fields = vec![];

        // delete_users (batch delete)
        let name = format!("delete_{}", base_name);
        fields.push(Self {
            name,
            table_name: table.name.clone(),
            schema_name: table.schema.clone(),
            mutation_type: MutationType::Delete,
            pk_columns: Vec::new(),
            return_type: format!("{}_mutation_response", type_name),
            description: Some(format!("delete data from the table: \"{}\"", table.name)),
        });

        // delete_user_by_pk (single delete by PK)
        if !table.pk_cols.is_empty() {
            let name = format!("delete_{}_by_pk", base_name);
            fields.push(Self {
                name,
                table_name: table.name.clone(),
                schema_name: table.schema.clone(),
                mutation_type: MutationType::DeleteByPk,
                pk_columns: pk_columns_of(table),
                return_type: type_name,
                description: Some(format!(
                    "delete single row from the table: \"{}\"",
                    table.name
                )),
            });
        }

        fields
    }
}

/// Build a GraphQL schema from a schema cache.
pub fn build_schema(schema_cache: &SchemaCache, config: &SchemaConfig) -> GeneratedSchema {
    let mut object_types = HashMap::new();
    let mut query_fields = Vec::new();
    let mut mutation_fields = Vec::new();
    let mut relationship_fields = HashMap::new();

    // Tables are visited in a stable order: the cache is a hash map, and any
    // name disambiguation below must not shift between restarts.
    let mut tables: Vec<&Table> = schema_cache
        .tables
        .values()
        .filter(|table| config.is_schema_exposed(&table.schema))
        .collect();
    tables.sort_by(|a, b| (&a.schema, &a.name).cmp(&(&b.schema, &b.name)));

    // Base names already carry the schema for non-default schemas, so a clash
    // here needs contrived naming (a `public.api_users` table alongside
    // `api.users`). Resolve it with a numeric suffix rather than letting one
    // table overwrite the other.
    let mut used_base_names: HashMap<String, u32> = HashMap::new();
    // (schema, table) -> resolved base name, needed when naming relationship
    // targets.
    let mut base_names: HashMap<(String, String), String> = HashMap::new();

    for table in &tables {
        let preferred = base_name_for(table, config);
        let base_name = match used_base_names.get_mut(&preferred) {
            None => {
                used_base_names.insert(preferred.clone(), 1);
                preferred
            }
            Some(count) => {
                *count += 1;
                let disambiguated = format!("{}_{}", preferred, count);
                tracing::warn!(
                    "GraphQL name collision: {}.{} would generate the same names as \
                     an earlier table; exposing it as \"{}\" instead",
                    table.schema,
                    table.name,
                    disambiguated
                );
                disambiguated
            }
        };

        base_names.insert(
            (table.schema.clone(), table.name.clone()),
            base_name.clone(),
        );
    }

    for table in tables {
        let base_name = base_names
            .get(&(table.schema.clone(), table.name.clone()))
            .expect("every visited table has a resolved base name")
            .clone();

        // Create object type
        let obj_type = TableObjectType::from_table_named(table, &base_name, &config.names);
        let type_name = obj_type.name.clone();

        // Add query fields. Each root may be named separately -- Hasura names
        // them one at a time, and a set that does not agree on a base name has
        // nowhere else to be written down.
        // A root may be given a name, a description, or both. An empty
        // description is one that was given and is empty: metadata said the
        // field has none.
        let rename = |field: &mut QueryField, kind: &str| {
            if let Some(given) = config.names.root(&table.schema, &table.name, kind) {
                field.name = given.to_string();
            }
            if let Some(comment) = config.names.root_comment(&table.schema, &table.name, kind) {
                field.description = Some(comment.to_string()).filter(|c| !c.is_empty());
            }
        };
        let mut list = QueryField::list_named(table, &base_name);
        rename(&mut list, "select");
        // The aggregate root is generated from the type name rather than being
        // a `QueryField` of its own, so its name travels with the list field
        // that spawns it.
        list.aggregate_name = config
            .names
            .root(&table.schema, &table.name, "select_aggregate")
            .map(str::to_string);
        list.aggregate_description = config
            .names
            .root_comment(&table.schema, &table.name, "select_aggregate")
            .map(str::to_string);
        query_fields.push(list);
        if let Some(mut by_pk) = QueryField::by_pk_named(table, &base_name) {
            rename(&mut by_pk, "select_by_pk");
            query_fields.push(by_pk);
        }

        // Add mutation fields if enabled
        if config.enable_mutations {
            let mut fields: Vec<MutationField> = Vec::new();
            fields.extend(MutationField::insert_fields_named(table, &base_name));
            fields.extend(MutationField::update_fields_named(table, &base_name));
            fields.extend(MutationField::delete_fields_named(table, &base_name));
            for field in &mut fields {
                let kind = match field.mutation_type {
                    MutationType::Insert => "insert",
                    MutationType::InsertOne => "insert_one",
                    MutationType::Update => "update",
                    MutationType::UpdateByPk => "update_by_pk",
                    MutationType::UpdateMany => "update_many",
                    MutationType::Delete => "delete",
                    MutationType::DeleteByPk => "delete_by_pk",
                };
                if let Some(given) = config.names.root(&table.schema, &table.name, kind) {
                    field.name = given.to_string();
                }
                if let Some(comment) =
                    config.names.root_comment(&table.schema, &table.name, kind)
                {
                    field.description =
                        Some(comment.to_string()).filter(|c| !c.is_empty());
                }
            }
            mutation_fields.extend(fields);
        }

        // Add relationship fields
        let rels: Vec<RelationshipField> = schema_cache
            .get_relationships(&table.qualified_identifier(), &table.schema)
            .map(|relationships| {
                relationships
                    .iter()
                    .filter_map(|rel| {
                        // A relationship whose target is not exposed would
                        // reference a GraphQL type that was never registered.
                        let foreign = rel.foreign_table();
                        let target_base =
                            base_names.get(&(foreign.schema.clone(), foreign.name.clone()))?;
                        Some(RelationshipField::from_relationship_named(
                            rel,
                            target_base,
                            relationship_keys(rel).iter().find_map(|key| {
                                config.names.relationship(&table.schema, &table.name, key)
                            }),
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default();

        if !rels.is_empty() {
            relationship_fields.insert(type_name.clone(), rels);
        }

        object_types.insert(type_name, obj_type);
    }

    // A table marked as a set of allowed values becomes a GraphQL enum, and
    // every column with a foreign key to it is typed as that enum rather than
    // as text. The values are rows, so they were read at startup rather than
    // reflected.
    let mut enum_types: HashMap<String, Vec<(String, Option<String>)>> = HashMap::new();
    let mut enum_type_of: HashMap<(String, String), String> = HashMap::new();

    for (schema_name, table_name) in config.names.enum_tables() {
        let Some(base) = base_names.get(&(schema_name.clone(), table_name.clone())) else {
            continue;
        };
        let key = format!("{}.{}", schema_name, table_name);
        let members: Vec<(String, Option<String>)> = config
            .enum_values
            .get(&key)
            .map(|values| {
                values
                    .iter()
                    .filter(|(value, _)| is_graphql_name(value))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();

        if members.is_empty() {
            // No rows, or none whose value is a legal GraphQL name. An empty
            // enum is not a legal type, so the table stays an ordinary one.
            tracing::warn!(
                "{}.{} is marked as an enumeration but has no values that can name an \
                 enum member; exposing it as an ordinary table",
                schema_name,
                table_name
            );
            continue;
        }

        let type_name = format!("{}_enum", base);
        enum_type_of.insert((schema_name.clone(), table_name.clone()), type_name.clone());
        enum_types.insert(type_name, members);
    }

    // Retype the columns that point at one.
    if !enum_type_of.is_empty() {
        for (type_name, relationships) in &relationship_fields {
            for relationship in relationships {
                if relationship.is_list {
                    continue;
                }
                let foreign = relationship.relationship.foreign_table();
                let Some(enum_type) =
                    enum_type_of.get(&(foreign.schema.clone(), foreign.name.clone()))
                else {
                    continue;
                };
                let Some(column) = relationship.relationship.single_local_column() else {
                    continue;
                };
                if let Some(object) = object_types.get_mut(type_name) {
                    // Under the name the column is exposed as, which is not
                    // always its own.
                    let exposed = config
                        .names
                        .column(&object.table.schema, &object.table.name, &column)
                        .unwrap_or(&column)
                        .to_string();
                    for field in &mut object.fields {
                        if field.name == exposed {
                            field.graphql_type =
                                crate::types::GraphQLType::Custom(enum_type.clone());
                        }
                    }
                }
            }
        }
    }

    // A table marked as a set of allowed values is a set of *values*: the
    // column typed as the enum is the whole of what a client needs, and there
    // is nothing at the other end of a relationship to it worth traversing to.
    // Hasura exposes none, and a stray `user { color { ... } }` beside
    // `favorite_color` is a second spelling of the same fact.
    if !enum_type_of.is_empty() {
        for relationships in relationship_fields.values_mut() {
            relationships.retain(|relationship| {
                let foreign = relationship.relationship.foreign_table();
                !enum_type_of.contains_key(&(foreign.schema.clone(), foreign.name.clone()))
            });
        }
    }

    // Functions that answer with rows of a table it already exposes. One
    // returning anything else has no type here to return: a scalar-returning
    // function is not a root field, it is an RPC, which the REST surface
    // already offers.
    let mut function_fields = Vec::new();
    for routines in schema_cache.routines.values() {
        for routine in routines {
            if !config.is_schema_exposed(&routine.schema) || routine.is_procedure {
                continue;
            }
            let postrust_core::schema_cache::RetType::SetOf(returned) = &routine.return_type else {
                continue;
            };
            // A function taking a table's row is that table's computed field,
            // not a root field: `fetch_articles(author_row author)` is asked
            // of an author, and a row type is not something a client can send
            // as an argument. It is already exposed where it belongs.
            if routine.params.iter().any(|param| {
                let (_, named) = type_name_of(&param.param_type);
                base_names.keys().any(|(_, table)| table == &named)
            }) {
                continue;
            }
            // `SETOF <table>`, where the table is one this schema exposes. The
            // catalogue names the returned type without a schema, so the
            // function's own schema is tried first -- a function and the table
            // it returns usually live together, and two schemas may both have
            // a table of that name.
            let (returned_schema, returned_table) = type_name_of(returned);
            let found = base_names
                .iter()
                .find(|((schema, table), _)| {
                    table == &returned_table
                        && schema == returned_schema.as_ref().unwrap_or(&routine.schema)
                })
                .or_else(|| {
                    base_names
                        .iter()
                        .find(|((_, table), _)| table == &returned_table)
                });
            let Some(((target_schema, target_table), base)) = found else {
                continue;
            };
            let target = (target_schema.clone(), target_table.clone());

            function_fields.push(FunctionField {
                name: routine.name.clone(),
                schema_name: routine.schema.clone(),
                function_name: routine.name.clone(),
                returns: base.clone(),
                returns_table: target,
                arguments: routine
                    .params
                    .iter()
                    .filter(|param| !param.name.is_empty())
                    .map(|param| {
                        (param.name.clone(), param.param_type.clone(), param.required)
                    })
                    .collect(),
                volatile: matches!(
                    routine.volatility,
                    postrust_core::schema_cache::FuncVolatility::Volatile
                ),
                description: routine.description.clone(),
            });
        }
    }
    function_fields.sort_by(|a, b| a.name.cmp(&b.name));

    GeneratedSchema {
        object_types,
        query_fields,
        mutation_fields,
        relationship_fields,
        enum_types,
        function_fields,
    }
}

/// Whether a value can name a GraphQL enum member.
///
/// `[_A-Za-z][_0-9A-Za-z]*`, which is what the specification allows. A value
/// that cannot is left out rather than mangled into one: a client asking for
/// `light blue` should be told there is no such member, not handed
/// `light_blue` and left to wonder which row it means.
fn is_graphql_name(value: &str) -> bool {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) if first == '_' || first.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

#[cfg(test)]
mod tests {
    use super::*;
    use indexmap::IndexMap;
    use postrust_core::schema_cache::Column;
    use pretty_assertions::assert_eq;

    fn create_test_table(name: &str, insertable: bool, updatable: bool, deletable: bool) -> Table {
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
            insertable,
            updatable,
            deletable,
            pk_cols: vec!["id".into()],
            unique_constraints: Vec::new(),
            columns,
            computed_columns: Default::default(),
            is_partitioned: false,
        }
    }

    fn create_test_schema_cache() -> SchemaCache {
        use std::collections::{HashMap, HashSet};

        let mut tables = HashMap::new();

        let users = create_test_table("users", true, true, true);
        let posts = create_test_table("posts", true, true, true);
        let comments = create_test_table("comments", true, false, false);

        tables.insert(users.qualified_identifier(), users);
        tables.insert(posts.qualified_identifier(), posts);
        tables.insert(comments.qualified_identifier(), comments);

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
    // SchemaConfig Tests
    // ============================================================================

    #[test]
    fn test_schema_config_default() {
        let config = SchemaConfig::default();
        assert!(config.is_schema_exposed("public"));
        assert!(!config.is_schema_exposed("private"));
        assert!(config.enable_mutations);
        assert!(!config.enable_subscriptions);
    }

    #[test]
    fn test_schema_config_with_schemas() {
        let config =
            SchemaConfig::new().with_schemas(vec!["api".to_string(), "public".to_string()]);
        assert!(config.is_schema_exposed("api"));
        assert!(config.is_schema_exposed("public"));
        assert!(!config.is_schema_exposed("private"));
    }

    #[test]
    fn test_schema_config_mutations_disabled() {
        let config = SchemaConfig::new().with_mutations(false);
        assert!(!config.enable_mutations);
    }

    // ============================================================================
    // QueryField Tests
    // ============================================================================

    #[test]
    fn test_query_field_list() {
        let table = create_test_table("users", true, true, true);
        let field = QueryField::list(&table);

        assert_eq!(field.name, "users");
        assert_eq!(field.return_type, "[users!]!");
        assert!(field.is_list);
        assert!(!field.is_by_pk);
    }

    #[test]
    fn test_query_field_by_pk() {
        let table = create_test_table("users", true, true, true);
        let field = QueryField::by_pk(&table).unwrap();

        assert_eq!(field.name, "users_by_pk");
        assert_eq!(field.return_type, "users");
        assert!(!field.is_list);
        assert!(field.is_by_pk);
    }

    #[test]
    fn test_query_field_by_pk_no_pk() {
        let mut table = create_test_table("users", true, true, true);
        table.pk_cols = vec![];
        let field = QueryField::by_pk(&table);

        assert!(field.is_none());
    }

    // ============================================================================
    // MutationField Tests
    // ============================================================================

    #[test]
    fn test_mutation_field_insert() {
        let table = create_test_table("users", true, true, true);
        let fields = MutationField::insert_fields(&table);

        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].name, "insert_users");
        assert_eq!(fields[0].mutation_type, MutationType::Insert);
        assert_eq!(fields[1].name, "insert_users_one");
        assert_eq!(fields[1].mutation_type, MutationType::InsertOne);
    }

    #[test]
    fn test_mutation_field_insert_not_insertable() {
        let table = create_test_table("users", false, true, true);
        let fields = MutationField::insert_fields(&table);

        assert!(fields.is_empty());
    }

    #[test]
    fn test_mutation_field_update() {
        let table = create_test_table("users", true, true, true);
        let fields = MutationField::update_fields(&table);

        assert_eq!(fields.len(), 3);
        assert_eq!(fields[0].name, "update_users");
        assert_eq!(fields[0].mutation_type, MutationType::Update);
        assert_eq!(fields[1].name, "update_users_many");
        assert_eq!(fields[1].mutation_type, MutationType::UpdateMany);
        assert_eq!(fields[2].name, "update_users_by_pk");
        assert_eq!(fields[2].mutation_type, MutationType::UpdateByPk);
    }

    #[test]
    fn test_mutation_field_update_not_updatable() {
        let table = create_test_table("users", true, false, true);
        let fields = MutationField::update_fields(&table);

        assert!(fields.is_empty());
    }

    #[test]
    fn test_mutation_field_delete() {
        let table = create_test_table("users", true, true, true);
        let fields = MutationField::delete_fields(&table);

        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].name, "delete_users");
        assert_eq!(fields[0].mutation_type, MutationType::Delete);
        assert_eq!(fields[1].name, "delete_users_by_pk");
        assert_eq!(fields[1].mutation_type, MutationType::DeleteByPk);
    }

    #[test]
    fn test_mutation_field_delete_not_deletable() {
        let table = create_test_table("users", true, true, false);
        let fields = MutationField::delete_fields(&table);

        assert!(fields.is_empty());
    }

    // ============================================================================
    // Singularize Tests
    // ============================================================================

    // ============================================================================
    // Build Schema Tests
    // ============================================================================

    #[test]
    fn test_build_schema_object_types() {
        let cache = create_test_schema_cache();
        let config = SchemaConfig::default();
        let schema = build_schema(&cache, &config);

        assert_eq!(schema.object_types.len(), 3);
        assert!(schema.get_object_type("users").is_some());
        assert!(schema.get_object_type("posts").is_some());
        assert!(schema.get_object_type("comments").is_some());
    }

    #[test]
    fn test_build_schema_query_fields() {
        let cache = create_test_schema_cache();
        let config = SchemaConfig::default();
        let schema = build_schema(&cache, &config);

        // 3 tables * 2 (list + byPk) = 6 query fields
        assert_eq!(schema.query_fields.len(), 6);

        // Check users query fields
        let users_field = schema.get_query_field("users").unwrap();
        assert_eq!(users_field.name, "users");
        assert!(users_field.is_list);
    }

    #[test]
    fn test_build_schema_mutation_fields() {
        let cache = create_test_schema_cache();
        let config = SchemaConfig::default();
        let schema = build_schema(&cache, &config);

        // users: 2 insert + 3 update + 2 delete = 7
        // posts: 2 insert + 3 update + 2 delete = 7
        // comments: 2 insert + 0 update + 0 delete = 2
        // Total: 16
        assert_eq!(schema.mutation_fields.len(), 16);

        let users_mutations = schema.get_mutation_fields("users");
        assert_eq!(users_mutations.len(), 7);
    }

    #[test]
    fn test_build_schema_mutations_disabled() {
        let cache = create_test_schema_cache();
        let config = SchemaConfig::new().with_mutations(false);
        let schema = build_schema(&cache, &config);

        assert!(schema.mutation_fields.is_empty());
    }

    #[test]
    fn test_build_schema_table_names() {
        let cache = create_test_schema_cache();
        let config = SchemaConfig::default();
        let schema = build_schema(&cache, &config);

        let names = schema.table_names();
        assert_eq!(names.len(), 3);
        assert!(names.contains(&"users"));
        assert!(names.contains(&"posts"));
        assert!(names.contains(&"comments"));
    }

    #[test]
    fn test_build_schema_type_names() {
        let cache = create_test_schema_cache();
        let config = SchemaConfig::default();
        let schema = build_schema(&cache, &config);

        let names = schema.type_names();
        assert_eq!(names.len(), 3);
        assert!(names.contains(&"users"));
        assert!(names.contains(&"posts"));
        assert!(names.contains(&"comments"));
    }

    #[test]
    fn test_build_schema_exposed_schemas() {
        let mut cache = create_test_schema_cache();

        // Add a table in a different schema
        let private_table = Table {
            schema: "private".into(),
            name: "secrets".into(),
            description: None,
            is_view: false,
            insertable: true,
            updatable: true,
            deletable: true,
            pk_cols: vec!["id".into()],
            unique_constraints: Vec::new(),
            columns: indexmap::IndexMap::new(),
            computed_columns: Default::default(),
            is_partitioned: false,
        };
        cache
            .tables
            .insert(private_table.qualified_identifier(), private_table);

        let config = SchemaConfig::default(); // Only exposes "public"
        let schema = build_schema(&cache, &config);

        // Should only have 3 tables from public schema
        assert_eq!(schema.object_types.len(), 3);
        assert!(schema.get_object_type("Secrets").is_none());
    }

    // ============================================================================
    // GeneratedSchema Tests
    // ============================================================================

    #[test]
    fn test_generated_schema_get_object_type() {
        let cache = create_test_schema_cache();
        let config = SchemaConfig::default();
        let schema = build_schema(&cache, &config);

        let users = schema.get_object_type("users").unwrap();
        assert_eq!(users.table.name, "users");
    }

    #[test]
    fn test_generated_schema_get_query_field() {
        let cache = create_test_schema_cache();
        let config = SchemaConfig::default();
        let schema = build_schema(&cache, &config);

        let field = schema.get_query_field("posts").unwrap();
        assert_eq!(field.table_name, "posts");
    }

    #[test]
    fn test_generated_schema_get_mutation_fields() {
        let cache = create_test_schema_cache();
        let config = SchemaConfig::default();
        let schema = build_schema(&cache, &config);

        let fields = schema.get_mutation_fields("comments");
        // comments is only insertable
        assert_eq!(fields.len(), 2); // insert_comments + insert_comments_one
    }
}
