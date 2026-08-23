//! GraphQL schema generation from PostgreSQL schema cache.
//!
//! Builds a dynamic GraphQL schema from the database schema cache,
//! creating query and mutation types for each table.

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

/// Base name used to derive a table's GraphQL type and field names.
///
/// Tables in the default schema keep their bare name, so a single-schema
/// deployment is unaffected. Tables in any other exposed schema are prefixed
/// with the schema, because GraphQL has one flat namespace: without this, a
/// `users` table in both `public` and `api` would generate the same type and
/// field names and one would silently replace the other.
fn base_name_for(table: &Table, config: &SchemaConfig) -> String {
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
            description: Some(format!("Query {} records", table.name)),
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
            description: Some(format!("Get a single {} by primary key", base_name)),
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
            return_type: format!("[{}!]!", type_name),
            description: Some(format!("Insert multiple {} records", table.name)),
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
            description: Some(format!("Insert a single {} record", base_name)),
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
            return_type: format!("[{}!]!", type_name),
            description: Some(format!("Update {} records", table.name)),
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
                description: Some(format!("Update a single {} by primary key", base_name)),
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
            return_type: format!("[{}!]!", type_name),
            description: Some(format!("Delete {} records", table.name)),
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
                description: Some(format!("Delete a single {} by primary key", base_name)),
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
        let obj_type = TableObjectType::from_table_named(table, &base_name);
        let type_name = obj_type.name.clone();

        // Add query fields
        query_fields.push(QueryField::list_named(table, &base_name));
        if let Some(by_pk) = QueryField::by_pk_named(table, &base_name) {
            query_fields.push(by_pk);
        }

        // Add mutation fields if enabled
        if config.enable_mutations {
            mutation_fields.extend(MutationField::insert_fields_named(table, &base_name));
            mutation_fields.extend(MutationField::update_fields_named(table, &base_name));
            mutation_fields.extend(MutationField::delete_fields_named(table, &base_name));
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
                        Some(RelationshipField::from_relationship_named(rel, target_base))
                    })
                    .collect()
            })
            .unwrap_or_default();

        if !rels.is_empty() {
            relationship_fields.insert(type_name.clone(), rels);
        }

        object_types.insert(type_name, obj_type);
    }

    GeneratedSchema {
        object_types,
        query_fields,
        mutation_fields,
        relationship_fields,
    }
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

        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].name, "update_users");
        assert_eq!(fields[0].mutation_type, MutationType::Update);
        assert_eq!(fields[1].name, "update_users_by_pk");
        assert_eq!(fields[1].mutation_type, MutationType::UpdateByPk);
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

        // users: 2 insert + 2 update + 2 delete = 6
        // posts: 2 insert + 2 update + 2 delete = 6
        // comments: 2 insert + 0 update + 0 delete = 2
        // Total: 14
        assert_eq!(schema.mutation_fields.len(), 14);

        let users_mutations = schema.get_mutation_fields("users");
        assert_eq!(users_mutations.len(), 6);
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
