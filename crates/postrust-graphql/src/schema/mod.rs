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
use std::collections::{HashMap, HashSet};

/// Configuration for schema generation.
#[derive(Debug, Clone)]
pub struct SchemaConfig {
    /// Schemas to expose in GraphQL (e.g., ["public"])
    pub exposed_schemas: Vec<String>,
    /// Whether to generate mutation types
    pub enable_mutations: bool,
    /// Whether to generate subscription types
    pub enable_subscriptions: bool,

    /// How often a live query re-reads itself when nothing has notified it.
    ///
    /// A subscription is woken by the trigger on the table it reads, which
    /// costs nothing while nothing is written. A trigger cannot see
    /// everything: a view has none, an embedded row may live in a table that
    /// carries none, and a predicate written against the clock changes with
    /// no write at all. This is how often those are noticed anyway. Seconds;
    /// zero turns the refresh off and leaves only the notifications.
    pub subscription_refresh_seconds: u64,
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

    /// Whose schema this is.
    ///
    /// `None` is the unrestricted one: what an administrator sees, and what
    /// every caller sees on a server with no permission document. `Some(role)`
    /// is built from a cache already reduced to what that role may see, so the
    /// only thing left for the builders to ask about is the aggregate root.
    pub role: Option<String>,
}

impl Default for SchemaConfig {
    fn default() -> Self {
        Self {
            exposed_schemas: vec!["public".to_string()],
            enum_values: HashMap::new(),
            names: crate::names::NameOverrides::default(),
            enable_mutations: true,
            enable_subscriptions: false,
            subscription_refresh_seconds: 30,
            max_rows: None,
            role: None,
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

    /// How often a live query re-reads itself when nothing has notified it.
    pub fn subscription_refresh(&self) -> std::time::Duration {
        match self.subscription_refresh_seconds {
            // A zero interval is not a tick every instant; it is no tick at
            // all, which leaves the notifications alone to wake it.
            0 => std::time::Duration::from_secs(60 * 60 * 24 * 365),
            seconds => std::time::Duration::from_secs(seconds),
        }
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
/// Whether the whole primary key is present on this table.
///
/// A `_by_pk` field addresses a row by its key, so a key the caller cannot see
/// is a field it cannot use: `books_by_pk(id: 1, book_name: "...")` for a role
/// granted `book_name` and not `id` asks for a row by half a key. Hasura does
/// not publish the field at all, and neither does this -- which is also what
/// stops `pk_columns_of` falling back to `text` for a column it cannot find
/// and typing the argument `String`.
fn has_whole_key(table: &Table) -> bool {
    !table.pk_cols.is_empty()
        && table
            .pk_cols
            .iter()
            .all(|column| table.get_column(column).is_some())
}

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
pub(crate) fn relationship_keys(rel: &postrust_core::schema_cache::Relationship) -> Vec<String> {
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
    // A key over more than one column, spelled as the columns in order. A
    // composite foreign key has no single column to be named by, and Hasura
    // names one by its columns rather than by its constraint -- so a
    // relationship over `(author_id1, author_id2)` is unreachable under any
    // of the spellings above. Sorted, because the order two sides list the
    // same columns in is not something either of them promises.
    let joined = |mut columns: Vec<String>| {
        columns.sort();
        columns.join(",")
    };
    let columns = rel.join_columns();
    if columns.len() > 1 {
        keys.push(format!(
            "{}.{}",
            rel.foreign_table().name,
            joined(columns.iter().map(|(_, foreign)| foreign.clone()).collect())
        ));
        if !rel.is_one_to_many() {
            keys.push(joined(
                columns.iter().map(|(local, _)| local.clone()).collect(),
            ));
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
    /// Its arguments, as `(name, PostgreSQL type, required)`. The session
    /// argument is not among them: the client does not supply it.
    pub arguments: Vec<(String, String, bool)>,
    /// The name of its session argument, if it has one.
    ///
    /// Hasura's convention: a `hasura_session json` parameter is filled from
    /// the caller's session variables rather than taken from the request, so a
    /// function can read who is asking. Exposing it as an argument would let
    /// the caller name its own identity, which is the opposite of the point.
    pub session_argument: Option<String>,
    /// Whether it may write, and so belongs on the mutation root.
    pub volatile: bool,
    /// Description from the function's comment.
    pub description: Option<String>,
}

/// What a computed relationship's function takes from the caller.
///
/// Everything but the parent row, which is what makes it a relationship, and
/// the session, which is the server's to supply. Read from the routine rather
/// than from the relationship because the relationship loader knows only which
/// argument is the row -- the rest are the same parameters any function has,
/// and they are already loaded.
/// One relationship field per foreign key the catalogue carries.
fn reflected_fields(
    table: &Table,
    schema_cache: &SchemaCache,
    config: &SchemaConfig,
    base_names: &HashMap<(String, String), String>,
) -> Vec<RelationshipField> {
    schema_cache
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
                    let mut field = RelationshipField::from_relationship_named(
                        rel,
                        target_base,
                        relationship_keys(rel).iter().find_map(|key| {
                            config.names.relationship(&table.schema, &table.name, key)
                        }),
                    );
                    field.arguments = caller_arguments(rel, schema_cache);
                    Some(field)
                })
                .collect()
        })
        .unwrap_or_default()
}

/// One relationship field per relationship metadata declares, and no others.
///
/// A declaration reached by a key names a relationship reflection already
/// found, so the catalogue's own cardinality and columns are used -- a key
/// says which side has one row and which has many, and guessing that from a
/// name would be guessing. One reached by a column mapping is not in the
/// catalogue at all and is built from the mapping, with the direction taken
/// from the declaration: Hasura writes object and array relationships
/// separately, which is where `to_one` comes from.
///
/// A declaration this server cannot resolve is left out rather than guessed
/// at. It is a field that would not work; the ones beside it still do.
fn declared_fields(
    declared: &[crate::names::DeclaredRelationship],
    table: &Table,
    schema_cache: &SchemaCache,
    config: &SchemaConfig,
    base_names: &HashMap<(String, String), String>,
) -> Vec<RelationshipField> {
    use postrust_core::schema_cache::Relationship;

    let reflected = schema_cache
        .get_relationships(&table.qualified_identifier(), &table.schema)
        .map(|found| found.to_vec())
        .unwrap_or_default();

    let mut fields = Vec::with_capacity(declared.len());
    for entry in declared {
        // Reached by a key reflection already found.
        if let Some(key) = entry.using.as_deref() {
            let Some(rel) = reflected
                .iter()
                .find(|rel| relationship_keys(rel).iter().any(|found| found == key))
            else {
                tracing::warn!(
                    "GraphQL: {}.{} declares the relationship \"{}\" through \"{}\", \
                     which no foreign key carries; it is left out",
                    table.schema,
                    table.name,
                    entry.name,
                    key
                );
                continue;
            };
            let foreign = rel.foreign_table();
            let Some(target_base) = base_names.get(&(foreign.schema.clone(), foreign.name.clone()))
            else {
                continue;
            };
            let mut field =
                RelationshipField::from_relationship_named(rel, target_base, Some(&entry.name));
            field.arguments = caller_arguments(rel, schema_cache);
            fields.push(field);
            continue;
        }

        // Reached by a column mapping, which is a join no key describes.
        let Some((schema, name)) = entry.target(config.default_schema()) else {
            tracing::warn!(
                "GraphQL: {}.{} declares the relationship \"{}\" with neither a key \
                 nor a table to join to; it is left out",
                table.schema,
                table.name,
                entry.name
            );
            continue;
        };
        let Some(target_base) = base_names.get(&(schema, name)) else {
            continue;
        };
        match mapped_relationship_field(
            entry,
            table,
            schema_cache,
            config.default_schema(),
            target_base,
        ) {
            Some(field) => fields.push(field),
            None => tracing::warn!(
                "GraphQL: {}.{} declares the relationship \"{}\" with no columns to \
                 join on; it is left out",
                table.schema,
                table.name,
                entry.name
            ),
        }
    }

    // A computed relationship is declared by `add_computed_field`, not by
    // `create_object_relationship`, so it is not in this list and is not what
    // the list is exhaustive about. Leaving it to reflection is what keeps
    // `author.get_articles` on a table whose foreign keys are all declared.
    let already: std::collections::HashSet<String> =
        fields.iter().map(|field| field.name.clone()).collect();
    for rel in reflected
        .iter()
        .filter(|rel| matches!(rel, Relationship::Computed { .. }))
    {
        let foreign = rel.foreign_table();
        let Some(target_base) = base_names.get(&(foreign.schema.clone(), foreign.name.clone()))
        else {
            continue;
        };
        let mut field = RelationshipField::from_relationship_named(
            rel,
            target_base,
            relationship_keys(rel)
                .iter()
                .find_map(|key| config.names.relationship(&table.schema, &table.name, key)),
        );
        if already.contains(&field.name) {
            continue;
        }
        field.arguments = caller_arguments(rel, schema_cache);
        fields.push(field);
    }
    fields
}

/// One relationship field for a declaration that maps column to column.
///
/// The join is the declaration's, the direction is the declaration's, and the
/// constraint name is this server's -- nothing reads it back, and a mapped
/// join has no constraint to borrow. `None` where the declaration names no
/// columns to join on, which is not a join.
pub(crate) fn mapped_relationship_field(
    entry: &crate::names::DeclaredRelationship,
    table: &Table,
    schema_cache: &SchemaCache,
    default_schema: &str,
    target_base: &str,
) -> Option<RelationshipField> {
    use postrust_core::schema_cache::{Cardinality, Relationship};

    let (schema, name) = entry.target(default_schema)?;
    if entry.columns.is_empty() {
        return None;
    }
    let foreign_table = postrust_core::api_request::QualifiedIdentifier::new(&schema, &name);
    let here = table.qualified_identifier();
    let columns: Vec<(String, String)> = entry
        .columns
        .iter()
        .map(|(mine, theirs)| (mine.clone(), theirs.clone()))
        .collect();
    let constraint = format!("{}.{}", table.name, entry.name);
    let cardinality = match entry.to_one {
        true => Cardinality::M2O {
            constraint: constraint.clone(),
            columns,
        },
        false => Cardinality::O2M {
            constraint: constraint.clone(),
            columns,
        },
    };
    let rel = Relationship::ForeignKey {
        is_self: here == foreign_table,
        table: here,
        foreign_table_is_view: schema_cache
            .get_table(&foreign_table)
            .is_some_and(|t| t.is_view),
        foreign_table,
        cardinality,
        table_is_view: table.is_view,
        constraint_name: constraint,
    };
    Some(RelationshipField::from_relationship_named(
        &rel,
        target_base,
        Some(&entry.name),
    ))
}

fn caller_arguments(
    relationship: &postrust_core::schema_cache::Relationship,
    schema_cache: &SchemaCache,
) -> Vec<(String, String, bool)> {
    use postrust_core::schema_cache::Relationship;
    let Relationship::Computed {
        function,
        row_argument,
        ..
    } = relationship
    else {
        return Vec::new();
    };
    let Some(row_argument) = row_argument else {
        return Vec::new();
    };
    schema_cache
        .routines
        .get(function)
        .into_iter()
        .flatten()
        .find(|routine| routine.name == function.name)
        .map(|routine| {
            routine
                .params
                .iter()
                .filter(|param| &param.name != row_argument)
                .filter(|param| !is_session_argument(param))
                .filter(|param| !param.name.is_empty())
                .map(|param| (param.name.clone(), param.param_type.clone(), param.required))
                .collect()
        })
        .unwrap_or_default()
}

/// What a computed *column*'s function takes from the caller.
///
/// The same rule as [`caller_arguments`], one field kind along: everything but
/// the row, which is what makes it a field of this table, and the session,
/// which is the server's to supply.
pub(crate) fn computed_caller_arguments(
    computed: &postrust_core::schema_cache::ComputedColumn,
    schema_cache: &SchemaCache,
) -> Vec<(String, String, bool)> {
    let Some(row_argument) = &computed.row_argument else {
        return Vec::new();
    };
    schema_cache
        .routines
        .get(&computed.function)
        .into_iter()
        .flatten()
        .find(|routine| routine.name == computed.function.name)
        .map(|routine| {
            routine
                .params
                .iter()
                .filter(|param| &param.name != row_argument)
                .filter(|param| !is_session_argument(param))
                .filter(|param| !param.name.is_empty())
                .map(|param| (param.name.clone(), param.param_type.clone(), param.required))
                .collect()
        })
        .unwrap_or_default()
}

/// Whether a parameter is the one Hasura fills from the session.
///
/// Recognised by name and type, which is the only thing the database records:
/// Hasura writes `session_argument: hasura_session` into metadata, and every
/// function that has one calls it that.
fn is_session_argument(param: &postrust_core::schema_cache::RoutineParam) -> bool {
    param.name == "hasura_session" && matches!(param.param_type.as_str(), "json" | "jsonb")
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
    /// Whether the aggregate root is built at all.
    ///
    /// The one thing a role's view of the schema cache cannot say. Dropping a
    /// table or a column is a fact about the database as this role sees it;
    /// whether that role may *count* the rows it can already read is a fact
    /// about the permission, and Hasura keeps the two apart because counting
    /// rows you cannot see is a way of seeing them. True unless a select
    /// permission says otherwise, which is what leaves an unconfigured server
    /// exactly as it was.
    pub aggregates: bool,
    /// Whether the rows themselves can be asked for.
    ///
    /// False for a table a role may count and not read: Hasura's way of
    /// granting "how many" without granting "which" is a select permission
    /// naming no columns, and such a table has an aggregate root and no row
    /// type to hang a list root on. The field is still carried so that the
    /// aggregate root beside it is built.
    pub rows: bool,
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
            aggregates: true,
            rows: true,
        }
    }

    /// Create a by-PK query field (e.g., userByPk), named after the table.
    pub fn by_pk(table: &Table) -> Option<Self> {
        Self::by_pk_named(table, &table.name)
    }

    /// Create a by-PK query field using an explicit base name.
    pub fn by_pk_named(table: &Table, base_name: &str) -> Option<Self> {
        if !has_whole_key(table) {
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
            aggregates: true,
            rows: true,
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
        if has_whole_key(table) {
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
        if has_whole_key(table) {
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

    // The tables this role may read, by base name. Needed after the loop, by
    // anything that would answer with a row of one.
    let mut readable_bases: HashSet<String> = HashSet::new();

    for table in &tables {
        let base_name = base_names
            .get(&(table.schema.clone(), table.name.clone()))
            .expect("every visited table has a resolved base name")
            .clone();

        // Create object type
        let mut obj_type = TableObjectType::from_table_named(table, &base_name, &config.names);
        // Whether this role can read the table at all. A table it may only
        // write has no GraphQL type -- a type with no fields is not a legal
        // one -- so nothing that would return that type is generated: no query
        // root, no `_by_pk`, no `insert_one`. The bulk write stays, and
        // answers with the count alone.
        //
        // Asked of the permission rather than of the fields, because the
        // fields are not split into the readable and the writable half until
        // every rewrite below has been applied to both.
        let readable = config.role.as_deref().is_none_or(|role| {
            config
                .names
                .permissions(&table.schema, &table.name, role)
                .is_none_or(|granted| crate::role::reads_anything(granted, table))
        });
        if readable {
            readable_bases.insert(base_name.clone());
        }
        // What each computed field takes from the caller. Read from the
        // routine, for the reason `caller_arguments` gives.
        for field in &mut obj_type.fields {
            let Some(function) = config
                .names
                .computed_source(&table.schema, &table.name, &field.name)
                .or(Some(field.name.as_str()))
            else {
                continue;
            };
            let Some(computed) = table.get_computed_column(function) else {
                continue;
            };
            field.arguments = computed_caller_arguments(computed, schema_cache);
        }
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
        // Whether this role may count what it can read. Only asked where a
        // role is building its own schema; the unrestricted one aggregates
        // whatever the database will.
        if let Some(role) = &config.role {
            list.aggregates =
                crate::role::allows_aggregations(&config.names, &table.schema, &table.name, role);
        }
        // A table with no readable field still gets its list entry when the
        // role may count it: the aggregate root is built beside that entry
        // rather than being one of its own, and `rows` is what says the list
        // root itself is not there. By key there is nothing to answer with at
        // all.
        list.rows = readable;
        if readable || list.aggregates {
            query_fields.push(list);
        }
        if readable {
            if let Some(mut by_pk) = QueryField::by_pk_named(table, &base_name) {
                rename(&mut by_pk, "select_by_pk");
                query_fields.push(by_pk);
            }
        }

        // Add mutation fields if enabled
        if config.enable_mutations {
            let mut fields: Vec<MutationField> = Vec::new();
            fields.extend(MutationField::insert_fields_named(table, &base_name));
            fields.extend(MutationField::update_fields_named(table, &base_name));
            fields.extend(MutationField::delete_fields_named(table, &base_name));
            // The three that answer with a row rather than with a count.
            if !readable {
                fields.retain(|field| {
                    !matches!(
                        field.mutation_type,
                        MutationType::InsertOne
                            | MutationType::UpdateByPk
                            | MutationType::DeleteByPk
                    )
                });
            }
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
                if let Some(comment) = config.names.root_comment(&table.schema, &table.name, kind) {
                    field.description = Some(comment.to_string()).filter(|c| !c.is_empty());
                }
            }
            mutation_fields.extend(fields);
        }

        // Add relationship fields. Where metadata declares them, they are
        // what the table has; otherwise reflection offers one per foreign
        // key, which is what a document saying nothing about a table has
        // always meant. See `TableNames::declared_relationships`.
        let rels = match config
            .names
            .declared_relationships(&table.schema, &table.name)
        {
            Some(declared) => declared_fields(declared, table, schema_cache, config, &base_names),
            None => reflected_fields(table, schema_cache, config, &base_names),
        };

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
        // A role that may not read the table's rows still sees the enum. The
        // values are a *type*, and a column typed by it is readable -- Hasura
        // publishes `colors_enum` to a role granted nothing on `colors`, and
        // the query `where: {favorite_color: {_eq: red}}` is one it answers.
        // So the name is derived rather than looked up, since a table the
        // role cannot read has no entry to look up.
        let base = match base_names.get(&(schema_name.clone(), table_name.clone())) {
            Some(found) => found.clone(),
            None => config
                .names
                .base_name(&schema_name, &table_name)
                .map(str::to_string)
                .unwrap_or_else(|| match schema_name == config.default_schema() {
                    true => table_name.clone(),
                    false => format!("{}_{}", schema_name, table_name),
                }),
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
    //
    // Read from the catalogue's foreign keys rather than from the
    // relationships this schema exposes, which are two different questions: a
    // relationship *to* an enum table is never exposed at all -- see just
    // below -- and a column's type has nothing to do with whether the row at
    // the other end can be traversed to. Keying one on the other meant a table
    // whose relationships metadata had declared away lost its enum columns'
    // type with them.
    if !enum_type_of.is_empty() {
        for table in &tables {
            let Some(type_name) = base_names.get(&(table.schema.clone(), table.name.clone()))
            else {
                continue;
            };
            let found = schema_cache
                .get_relationships(&table.qualified_identifier(), &table.schema)
                .map(|found| found.to_vec())
                .unwrap_or_default();
            for relationship in &found {
                if !relationship.is_to_one() {
                    continue;
                }
                let foreign = relationship.foreign_table();
                let Some(enum_type) =
                    enum_type_of.get(&(foreign.schema.clone(), foreign.name.clone()))
                else {
                    continue;
                };
                let Some(column) = relationship.single_local_column() else {
                    continue;
                };
                if let Some(object) = object_types.get_mut(type_name) {
                    // Under the name the column is exposed as, which is not
                    // always its own.
                    let exposed = config
                        .names
                        .column(&object.table.schema, &object.table.name, column)
                        .unwrap_or(column)
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

    // Split each type's fields into the half that may be read and the half
    // that may be written. Last, so that everything above -- the computed
    // fields' arguments, the retyping of an enum table's columns -- has been
    // applied to the whole list before either half is taken out of it.
    //
    // A role may be granted a column to set that it is not granted to see, so
    // the cache carries the union and this is where the two part company. See
    // [`crate::role`].
    for object in object_types.values_mut() {
        // Taken unconditionally, even where no role narrows anything: the
        // write inputs are built from this list, and leaving it as it was when
        // the type was first made would leave it behind every rewrite since.
        object.writable_fields = object.fields.clone();
        if let Some(role) = &config.role {
            let (schema_name, table_name) =
                (object.table.schema.clone(), object.table.name.clone());
            let Some(granted) = config.names.permissions(&schema_name, &table_name, role) else {
                continue;
            };
            let table = object.table.clone();
            object.fields.retain(|field| {
                let column = config
                    .names
                    .column_source(&schema_name, &table_name, &field.name)
                    .unwrap_or(&field.name);
                match table.get_column(column) {
                    // A computed field rather than a column, and those were
                    // reduced to the granted ones with the cache.
                    None => true,
                    Some(_) => granted
                        .select
                        .as_ref()
                        .is_some_and(|select| select.columns.allows(column)),
                }
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
            // A function returning rows of a table this role cannot read has
            // nowhere to put them: that table has no GraphQL type, and naming
            // one that was never registered is a schema that will not build.
            if !readable_bases.contains(base) {
                continue;
            }
            let target = (target_schema.clone(), target_table.clone());

            // Placed by what PostgreSQL says it does, unless metadata said
            // otherwise: `track_function` with `configuration: {exposed_as:
            // query}` puts a VOLATILE function on the query root, which is a
            // decision a person made and no catalogue remembers.
            let volatile = match config.names.exposed_as(&routine.schema, &routine.name) {
                Some("query") => false,
                Some("mutation") => true,
                _ => matches!(
                    routine.volatility,
                    postrust_core::schema_cache::FuncVolatility::Volatile
                ),
            };
            // A mutation a role was never granted. Hasura infers a query
            // function's permission from the select permission on the table it
            // returns -- which is the check just above -- and infers nothing
            // for a mutation: reading a table is not permission to change it,
            // so the role has to be named. A role granted no mutation at all
            // then has no mutation root, and a request for one is answered
            // `no mutations exist` rather than by a field that is missing.
            if volatile {
                if let Some(role) = config.role.as_deref() {
                    if !config
                        .names
                        .function_grants(&routine.schema, &routine.name, role)
                    {
                        continue;
                    }
                }
            }

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
                    .filter(|param| !is_session_argument(param))
                    .map(|param| (param.name.clone(), param.param_type.clone(), param.required))
                    .collect(),
                session_argument: routine
                    .params
                    .iter()
                    .find(|param| is_session_argument(param))
                    .map(|param| param.name.clone()),
                volatile,
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
