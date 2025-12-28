//! GraphQL schema generation from PostgreSQL schema cache.
//!
//! Builds a dynamic GraphQL schema from the database schema cache,
//! creating query and mutation types for each table.

pub mod object;
pub mod relationship;

use crate::input::mutation::{is_deletable, is_insertable, is_updatable};
use crate::schema::object::{to_camel_case, to_pascal_case, TableObjectType};
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
    /// Prefix for query fields (e.g., "all" -> "allUsers")
    pub query_prefix: Option<String>,
    /// Suffix for query fields (e.g., "Collection" -> "usersCollection")
    pub query_suffix: Option<String>,
    /// Whether to use camelCase for field names
    pub use_camel_case: bool,
}

impl Default for SchemaConfig {
    fn default() -> Self {
        Self {
            exposed_schemas: vec!["public".to_string()],
            enable_mutations: true,
            enable_subscriptions: false,
            query_prefix: None,
            query_suffix: None,
            use_camel_case: true,
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
        self.query_fields.iter().find(|f| f.table_name == table_name)
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
        self.object_types.values().map(|t| t.table.name.as_str()).collect()
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
    /// GraphQL object type name (e.g., "Users")
    pub type_name: String,
    /// GraphQL return type
    pub return_type: String,
    /// Whether this returns a list
    pub is_list: bool,
    /// Whether this is a "by PK" query
    pub is_by_pk: bool,
    /// Field description
    pub description: Option<String>,
}

impl QueryField {
    /// Create a list query field (e.g., users).
    pub fn list(table: &Table, config: &SchemaConfig) -> Self {
        let type_name = to_pascal_case(&table.name);
        let field_name = if config.use_camel_case {
            to_camel_case(&table.name)
        } else {
            table.name.clone()
        };

        let name = match (&config.query_prefix, &config.query_suffix) {
            (Some(prefix), None) => format!("{}{}", prefix, to_pascal_case(&field_name)),
            (None, Some(suffix)) => format!("{}{}", field_name, suffix),
            (Some(prefix), Some(suffix)) => {
                format!("{}{}{}", prefix, to_pascal_case(&field_name), suffix)
            }
            (None, None) => field_name,
        };

        Self {
            name,
            table_name: table.name.clone(),
            type_name: type_name.clone(),
            return_type: format!("[{}!]!", type_name),
            is_list: true,
            is_by_pk: false,
            description: Some(format!("Query {} records", table.name)),
        }
    }

    /// Create a by-PK query field (e.g., userByPk).
    pub fn by_pk(table: &Table, config: &SchemaConfig) -> Option<Self> {
        if table.pk_cols.is_empty() {
            return None;
        }

        let type_name = to_pascal_case(&table.name);
        let singular = singularize(&table.name);
        let field_name = if config.use_camel_case {
            format!("{}ByPk", to_camel_case(&singular))
        } else {
            format!("{}_by_pk", singular)
        };

        Some(Self {
            name: field_name,
            table_name: table.name.clone(),
            type_name: type_name.clone(),
            return_type: type_name,
            is_list: false,
            is_by_pk: true,
            description: Some(format!("Get a single {} by primary key", singular)),
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
    /// Mutation type
    pub mutation_type: MutationType,
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
    pub fn insert_fields(table: &Table, config: &SchemaConfig) -> Vec<Self> {
        if !is_insertable(table) {
            return vec![];
        }

        let type_name = to_pascal_case(&table.name);
        let singular = singularize(&table.name);

        let mut fields = vec![];

        // insert_users (batch insert)
        let name = if config.use_camel_case {
            format!("insert{}", to_pascal_case(&table.name))
        } else {
            format!("insert_{}", table.name)
        };
        fields.push(Self {
            name,
            table_name: table.name.clone(),
            mutation_type: MutationType::Insert,
            return_type: format!("[{}!]!", type_name),
            description: Some(format!("Insert multiple {} records", table.name)),
        });

        // insert_user_one (single insert)
        let name = if config.use_camel_case {
            format!("insert{}One", to_pascal_case(&singular))
        } else {
            format!("insert_{}_one", singular)
        };
        fields.push(Self {
            name,
            table_name: table.name.clone(),
            mutation_type: MutationType::InsertOne,
            return_type: type_name.clone(),
            description: Some(format!("Insert a single {} record", singular)),
        });

        fields
    }

    /// Create update mutation fields for a table.
    pub fn update_fields(table: &Table, config: &SchemaConfig) -> Vec<Self> {
        if !is_updatable(table) {
            return vec![];
        }

        let type_name = to_pascal_case(&table.name);
        let singular = singularize(&table.name);

        let mut fields = vec![];

        // update_users (batch update)
        let name = if config.use_camel_case {
            format!("update{}", to_pascal_case(&table.name))
        } else {
            format!("update_{}", table.name)
        };
        fields.push(Self {
            name,
            table_name: table.name.clone(),
            mutation_type: MutationType::Update,
            return_type: format!("[{}!]!", type_name),
            description: Some(format!("Update {} records", table.name)),
        });

        // update_user_by_pk (single update by PK)
        if !table.pk_cols.is_empty() {
            let name = if config.use_camel_case {
                format!("update{}ByPk", to_pascal_case(&singular))
            } else {
                format!("update_{}_by_pk", singular)
            };
            fields.push(Self {
                name,
                table_name: table.name.clone(),
                mutation_type: MutationType::UpdateByPk,
                return_type: type_name,
                description: Some(format!("Update a single {} by primary key", singular)),
            });
        }

        fields
    }

    /// Create delete mutation fields for a table.
    pub fn delete_fields(table: &Table, config: &SchemaConfig) -> Vec<Self> {
        if !is_deletable(table) {
            return vec![];
        }

        let type_name = to_pascal_case(&table.name);
        let singular = singularize(&table.name);

        let mut fields = vec![];

        // delete_users (batch delete)
        let name = if config.use_camel_case {
            format!("delete{}", to_pascal_case(&table.name))
        } else {
            format!("delete_{}", table.name)
        };
        fields.push(Self {
            name,
            table_name: table.name.clone(),
            mutation_type: MutationType::Delete,
            return_type: format!("[{}!]!", type_name),
            description: Some(format!("Delete {} records", table.name)),
        });

        // delete_user_by_pk (single delete by PK)
        if !table.pk_cols.is_empty() {
            let name = if config.use_camel_case {
                format!("delete{}ByPk", to_pascal_case(&singular))
            } else {
                format!("delete_{}_by_pk", singular)
            };
            fields.push(Self {
                name,
                table_name: table.name.clone(),
                mutation_type: MutationType::DeleteByPk,
                return_type: type_name,
                description: Some(format!("Delete a single {} by primary key", singular)),
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

    // Process each table in the schema cache
    for table in schema_cache.tables.values() {
        // Skip tables not in exposed schemas
        if !config.is_schema_exposed(&table.schema) {
            continue;
        }

        // Create object type
        let obj_type = TableObjectType::from_table(table);
        let type_name = obj_type.name.clone();

        // Add query fields
        query_fields.push(QueryField::list(table, config));
        if let Some(by_pk) = QueryField::by_pk(table, config) {
            query_fields.push(by_pk);
        }

        // Add mutation fields if enabled
        if config.enable_mutations {
            mutation_fields.extend(MutationField::insert_fields(table, config));
            mutation_fields.extend(MutationField::update_fields(table, config));
            mutation_fields.extend(MutationField::delete_fields(table, config));
        }

        // Add relationship fields
        let rels: Vec<RelationshipField> = schema_cache
            .get_relationships(&table.qualified_identifier(), &table.schema)
            .map(|relationships| {
                relationships
                    .iter()
                    .map(|r| RelationshipField::from_relationship(r))
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

/// Simple singularization for field names.
fn singularize(s: &str) -> String {
    if s.ends_with("ies") {
        format!("{}y", &s[..s.len() - 3])
    } else if s.ends_with("es")
        && (s.ends_with("ses") || s.ends_with("xes") || s.ends_with("ches") || s.ends_with("shes"))
    {
        s[..s.len() - 2].to_string()
    } else if s.ends_with('s') && !s.ends_with("ss") {
        s[..s.len() - 1].to_string()
    } else {
        s.to_string()
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
            insertable,
            updatable,
            deletable,
            pk_cols: vec!["id".into()],
            columns,
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
            pg_version: 150000,
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
        let config = SchemaConfig::new()
            .with_schemas(vec!["api".to_string(), "public".to_string()]);
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
        let config = SchemaConfig::default();
        let field = QueryField::list(&table, &config);

        assert_eq!(field.name, "users");
        assert_eq!(field.return_type, "[Users!]!");
        assert!(field.is_list);
        assert!(!field.is_by_pk);
    }

    #[test]
    fn test_query_field_list_with_prefix() {
        let table = create_test_table("users", true, true, true);
        let config = SchemaConfig {
            query_prefix: Some("all".to_string()),
            ..Default::default()
        };
        let field = QueryField::list(&table, &config);

        assert_eq!(field.name, "allUsers");
    }

    #[test]
    fn test_query_field_list_with_suffix() {
        let table = create_test_table("users", true, true, true);
        let config = SchemaConfig {
            query_suffix: Some("Collection".to_string()),
            ..Default::default()
        };
        let field = QueryField::list(&table, &config);

        assert_eq!(field.name, "usersCollection");
    }

    #[test]
    fn test_query_field_by_pk() {
        let table = create_test_table("users", true, true, true);
        let config = SchemaConfig::default();
        let field = QueryField::by_pk(&table, &config).unwrap();

        assert_eq!(field.name, "userByPk");
        assert_eq!(field.return_type, "Users");
        assert!(!field.is_list);
        assert!(field.is_by_pk);
    }

    #[test]
    fn test_query_field_by_pk_no_pk() {
        let mut table = create_test_table("users", true, true, true);
        table.pk_cols = vec![];
        let config = SchemaConfig::default();
        let field = QueryField::by_pk(&table, &config);

        assert!(field.is_none());
    }

    // ============================================================================
    // MutationField Tests
    // ============================================================================

    #[test]
    fn test_mutation_field_insert() {
        let table = create_test_table("users", true, true, true);
        let config = SchemaConfig::default();
        let fields = MutationField::insert_fields(&table, &config);

        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].name, "insertUsers");
        assert_eq!(fields[0].mutation_type, MutationType::Insert);
        assert_eq!(fields[1].name, "insertUserOne");
        assert_eq!(fields[1].mutation_type, MutationType::InsertOne);
    }

    #[test]
    fn test_mutation_field_insert_not_insertable() {
        let table = create_test_table("users", false, true, true);
        let config = SchemaConfig::default();
        let fields = MutationField::insert_fields(&table, &config);

        assert!(fields.is_empty());
    }

    #[test]
    fn test_mutation_field_update() {
        let table = create_test_table("users", true, true, true);
        let config = SchemaConfig::default();
        let fields = MutationField::update_fields(&table, &config);

        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].name, "updateUsers");
        assert_eq!(fields[0].mutation_type, MutationType::Update);
        assert_eq!(fields[1].name, "updateUserByPk");
        assert_eq!(fields[1].mutation_type, MutationType::UpdateByPk);
    }

    #[test]
    fn test_mutation_field_update_not_updatable() {
        let table = create_test_table("users", true, false, true);
        let config = SchemaConfig::default();
        let fields = MutationField::update_fields(&table, &config);

        assert!(fields.is_empty());
    }

    #[test]
    fn test_mutation_field_delete() {
        let table = create_test_table("users", true, true, true);
        let config = SchemaConfig::default();
        let fields = MutationField::delete_fields(&table, &config);

        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].name, "deleteUsers");
        assert_eq!(fields[0].mutation_type, MutationType::Delete);
        assert_eq!(fields[1].name, "deleteUserByPk");
        assert_eq!(fields[1].mutation_type, MutationType::DeleteByPk);
    }

    #[test]
    fn test_mutation_field_delete_not_deletable() {
        let table = create_test_table("users", true, true, false);
        let config = SchemaConfig::default();
        let fields = MutationField::delete_fields(&table, &config);

        assert!(fields.is_empty());
    }

    // ============================================================================
    // Singularize Tests
    // ============================================================================

    #[test]
    fn test_singularize() {
        assert_eq!(singularize("users"), "user");
        assert_eq!(singularize("posts"), "post");
        assert_eq!(singularize("categories"), "category");
        assert_eq!(singularize("boxes"), "box");
        assert_eq!(singularize("matches"), "match");
        assert_eq!(singularize("class"), "class");
    }

    // ============================================================================
    // Build Schema Tests
    // ============================================================================

    #[test]
    fn test_build_schema_object_types() {
        let cache = create_test_schema_cache();
        let config = SchemaConfig::default();
        let schema = build_schema(&cache, &config);

        assert_eq!(schema.object_types.len(), 3);
        assert!(schema.get_object_type("Users").is_some());
        assert!(schema.get_object_type("Posts").is_some());
        assert!(schema.get_object_type("Comments").is_some());
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
        assert!(names.contains(&"Users"));
        assert!(names.contains(&"Posts"));
        assert!(names.contains(&"Comments"));
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
        };
        cache.tables.insert(private_table.qualified_identifier(), private_table);

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

        let users = schema.get_object_type("Users").unwrap();
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
        assert_eq!(fields.len(), 2); // insertComments + insertCommentOne
    }
}
