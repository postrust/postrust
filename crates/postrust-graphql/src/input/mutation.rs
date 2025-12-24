//! Mutation input types for inserts and updates.
//!
//! Provides input type generation for GraphQL mutations based on table metadata.

use crate::types::{pg_type_to_graphql, GraphQLType};
use postrust_core::schema_cache::{Column, Table};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Represents a field in an insert input type.
#[derive(Debug, Clone)]
pub struct InsertField {
    /// Field name
    pub name: String,
    /// GraphQL type
    pub graphql_type: GraphQLType,
    /// Whether the field is required (no default value and not nullable)
    pub required: bool,
    /// Field description
    pub description: Option<String>,
}

impl InsertField {
    /// Create an InsertField from a column.
    pub fn from_column(column: &Column) -> Self {
        let graphql_type = pg_type_to_graphql(&column.nominal_type);

        // A field is required if:
        // 1. It's not nullable AND
        // 2. It has no default value AND
        // 3. It's not a primary key with serial/auto-increment default
        let has_auto_default = column.default.as_ref().map_or(false, |d| {
            d.contains("nextval") || d.contains("gen_random_uuid")
        });

        let required = !column.nullable && column.default.is_none() && !has_auto_default;

        Self {
            name: column.name.clone(),
            description: column.description.clone(),
            graphql_type,
            required,
        }
    }

    /// Get the GraphQL type string for input.
    pub fn type_string(&self) -> String {
        let base = format!("{}", self.graphql_type);
        if self.required {
            format!("{}!", base)
        } else {
            base
        }
    }
}

/// Represents a field in an update input type.
#[derive(Debug, Clone)]
pub struct UpdateField {
    /// Field name
    pub name: String,
    /// GraphQL type
    pub graphql_type: GraphQLType,
    /// Field description
    pub description: Option<String>,
    /// Whether this is a primary key (cannot be updated)
    pub is_pk: bool,
}

impl UpdateField {
    /// Create an UpdateField from a column.
    pub fn from_column(column: &Column) -> Self {
        let graphql_type = pg_type_to_graphql(&column.nominal_type);

        Self {
            name: column.name.clone(),
            description: column.description.clone(),
            graphql_type,
            is_pk: column.is_pk,
        }
    }

    /// Get the GraphQL type string for input (always nullable for updates).
    pub fn type_string(&self) -> String {
        format!("{}", self.graphql_type)
    }

    /// Check if this field can be updated (non-PK fields only).
    pub fn is_updatable(&self) -> bool {
        !self.is_pk
    }
}

/// Represents an insert input type for a table.
#[derive(Debug, Clone)]
pub struct InsertInput {
    /// Table being inserted into
    pub table_name: String,
    /// GraphQL type name (e.g., "UsersInsertInput")
    pub type_name: String,
    /// Fields that can be inserted
    pub fields: Vec<InsertField>,
}

impl InsertInput {
    /// Create an InsertInput from a table.
    pub fn from_table(table: &Table) -> Self {
        let type_name = format!("{}InsertInput", to_pascal_case(&table.name));

        let fields = table
            .columns
            .values()
            .map(InsertField::from_column)
            .collect();

        Self {
            table_name: table.name.clone(),
            type_name,
            fields,
        }
    }

    /// Get required fields.
    pub fn required_fields(&self) -> Vec<&InsertField> {
        self.fields.iter().filter(|f| f.required).collect()
    }

    /// Get optional fields.
    pub fn optional_fields(&self) -> Vec<&InsertField> {
        self.fields.iter().filter(|f| !f.required).collect()
    }

    /// Check if the table has any required fields.
    pub fn has_required_fields(&self) -> bool {
        self.fields.iter().any(|f| f.required)
    }
}

/// Represents an update input type for a table.
#[derive(Debug, Clone)]
pub struct UpdateInput {
    /// Table being updated
    pub table_name: String,
    /// GraphQL type name (e.g., "UsersSetInput")
    pub type_name: String,
    /// Fields that can be updated
    pub fields: Vec<UpdateField>,
}

impl UpdateInput {
    /// Create an UpdateInput from a table.
    pub fn from_table(table: &Table) -> Self {
        let type_name = format!("{}SetInput", to_pascal_case(&table.name));

        let fields = table
            .columns
            .values()
            .filter(|c| !c.is_pk) // Exclude primary keys from update
            .map(UpdateField::from_column)
            .collect();

        Self {
            table_name: table.name.clone(),
            type_name,
            fields,
        }
    }

    /// Get updatable fields.
    pub fn updatable_fields(&self) -> Vec<&UpdateField> {
        self.fields.iter().filter(|f| f.is_updatable()).collect()
    }
}

/// A dynamic input value that can hold different types.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum InputValue {
    /// Null value
    Null,
    /// Boolean value
    Bool(bool),
    /// Integer value
    Int(i64),
    /// Float value
    Float(f64),
    /// String value
    String(String),
    /// JSON object value
    Object(HashMap<String, InputValue>),
    /// JSON array value
    Array(Vec<InputValue>),
}

impl InputValue {
    /// Check if this is null.
    pub fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }

    /// Try to get as string.
    pub fn as_string(&self) -> Option<&str> {
        match self {
            Self::String(s) => Some(s),
            _ => None,
        }
    }

    /// Try to get as i64.
    pub fn as_int(&self) -> Option<i64> {
        match self {
            Self::Int(i) => Some(*i),
            _ => None,
        }
    }

    /// Try to get as f64.
    pub fn as_float(&self) -> Option<f64> {
        match self {
            Self::Float(f) => Some(*f),
            Self::Int(i) => Some(*i as f64),
            _ => None,
        }
    }

    /// Try to get as bool.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// Convert to SQL string representation.
    pub fn to_sql_string(&self) -> String {
        match self {
            Self::Null => "NULL".to_string(),
            Self::Bool(b) => if *b { "true" } else { "false" }.to_string(),
            Self::Int(i) => i.to_string(),
            Self::Float(f) => f.to_string(),
            Self::String(s) => s.clone(),
            Self::Object(o) => serde_json::to_string(o).unwrap_or_default(),
            Self::Array(a) => serde_json::to_string(a).unwrap_or_default(),
        }
    }
}

/// Helper to convert snake_case to PascalCase.
fn to_pascal_case(s: &str) -> String {
    s.split('_')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect()
}

/// Check if a table is insertable based on its permissions.
pub fn is_insertable(table: &Table) -> bool {
    table.insertable
}

/// Check if a table is updatable based on its permissions.
pub fn is_updatable(table: &Table) -> bool {
    table.updatable
}

/// Check if a table is deletable based on its permissions.
pub fn is_deletable(table: &Table) -> bool {
    table.deletable
}

#[cfg(test)]
mod tests {
    use super::*;
    use indexmap::IndexMap;
    use pretty_assertions::assert_eq;

    fn create_test_table() -> Table {
        let mut columns = IndexMap::new();
        columns.insert(
            "id".into(),
            Column {
                name: "id".into(),
                description: Some("Primary key".into()),
                nullable: false,
                data_type: "integer".into(),
                nominal_type: "int4".into(),
                max_len: None,
                default: Some("nextval('users_id_seq')".into()),
                enum_values: vec![],
                is_pk: true,
                position: 1,
            },
        );
        columns.insert(
            "name".into(),
            Column {
                name: "name".into(),
                description: Some("User name".into()),
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
        columns.insert(
            "email".into(),
            Column {
                name: "email".into(),
                description: None,
                nullable: true,
                data_type: "text".into(),
                nominal_type: "text".into(),
                max_len: None,
                default: None,
                enum_values: vec![],
                is_pk: false,
                position: 3,
            },
        );
        columns.insert(
            "created_at".into(),
            Column {
                name: "created_at".into(),
                description: None,
                nullable: false,
                data_type: "timestamptz".into(),
                nominal_type: "timestamptz".into(),
                max_len: None,
                default: Some("now()".into()),
                enum_values: vec![],
                is_pk: false,
                position: 4,
            },
        );

        Table {
            schema: "public".into(),
            name: "users".into(),
            description: Some("User accounts".into()),
            is_view: false,
            insertable: true,
            updatable: true,
            deletable: true,
            pk_cols: vec!["id".into()],
            columns,
        }
    }

    fn create_readonly_table() -> Table {
        let mut table = create_test_table();
        table.insertable = false;
        table.updatable = false;
        table.deletable = false;
        table
    }

    // ============================================================================
    // InsertField Tests
    // ============================================================================

    #[test]
    fn test_insert_field_required() {
        let table = create_test_table();
        let name_col = table.columns.get("name").unwrap();
        let field = InsertField::from_column(name_col);

        assert_eq!(field.name, "name");
        assert!(field.required); // Not nullable, no default
        assert_eq!(field.type_string(), "String!");
    }

    #[test]
    fn test_insert_field_optional_nullable() {
        let table = create_test_table();
        let email_col = table.columns.get("email").unwrap();
        let field = InsertField::from_column(email_col);

        assert_eq!(field.name, "email");
        assert!(!field.required); // Nullable
        assert_eq!(field.type_string(), "String");
    }

    #[test]
    fn test_insert_field_optional_with_default() {
        let table = create_test_table();
        let created_at_col = table.columns.get("created_at").unwrap();
        let field = InsertField::from_column(created_at_col);

        assert_eq!(field.name, "created_at");
        assert!(!field.required); // Has default
        assert_eq!(field.type_string(), "DateTime");
    }

    #[test]
    fn test_insert_field_auto_pk() {
        let table = create_test_table();
        let id_col = table.columns.get("id").unwrap();
        let field = InsertField::from_column(id_col);

        assert_eq!(field.name, "id");
        assert!(!field.required); // Has auto-increment default
    }

    // ============================================================================
    // UpdateField Tests
    // ============================================================================

    #[test]
    fn test_update_field_non_pk() {
        let table = create_test_table();
        let name_col = table.columns.get("name").unwrap();
        let field = UpdateField::from_column(name_col);

        assert_eq!(field.name, "name");
        assert!(!field.is_pk);
        assert!(field.is_updatable());
        assert_eq!(field.type_string(), "String"); // All update fields are nullable
    }

    #[test]
    fn test_update_field_pk() {
        let table = create_test_table();
        let id_col = table.columns.get("id").unwrap();
        let field = UpdateField::from_column(id_col);

        assert_eq!(field.name, "id");
        assert!(field.is_pk);
        assert!(!field.is_updatable());
    }

    // ============================================================================
    // InsertInput Tests
    // ============================================================================

    #[test]
    fn test_insert_input_from_table() {
        let table = create_test_table();
        let input = InsertInput::from_table(&table);

        assert_eq!(input.table_name, "users");
        assert_eq!(input.type_name, "UsersInsertInput");
        assert_eq!(input.fields.len(), 4);
    }

    #[test]
    fn test_insert_input_required_fields() {
        let table = create_test_table();
        let input = InsertInput::from_table(&table);

        let required = input.required_fields();
        assert_eq!(required.len(), 1); // Only "name" is required
        assert_eq!(required[0].name, "name");
    }

    #[test]
    fn test_insert_input_optional_fields() {
        let table = create_test_table();
        let input = InsertInput::from_table(&table);

        let optional = input.optional_fields();
        assert_eq!(optional.len(), 3); // id, email, created_at
    }

    #[test]
    fn test_insert_input_has_required_fields() {
        let table = create_test_table();
        let input = InsertInput::from_table(&table);

        assert!(input.has_required_fields());
    }

    // ============================================================================
    // UpdateInput Tests
    // ============================================================================

    #[test]
    fn test_update_input_from_table() {
        let table = create_test_table();
        let input = UpdateInput::from_table(&table);

        assert_eq!(input.table_name, "users");
        assert_eq!(input.type_name, "UsersSetInput");
        assert_eq!(input.fields.len(), 3); // Excludes PK
    }

    #[test]
    fn test_update_input_excludes_pk() {
        let table = create_test_table();
        let input = UpdateInput::from_table(&table);

        let field_names: Vec<_> = input.fields.iter().map(|f| f.name.as_str()).collect();
        assert!(!field_names.contains(&"id"));
    }

    #[test]
    fn test_update_input_updatable_fields() {
        let table = create_test_table();
        let input = UpdateInput::from_table(&table);

        let updatable = input.updatable_fields();
        assert_eq!(updatable.len(), 3);
    }

    // ============================================================================
    // InputValue Tests
    // ============================================================================

    #[test]
    fn test_input_value_null() {
        let value = InputValue::Null;
        assert!(value.is_null());
        assert_eq!(value.to_sql_string(), "NULL");
    }

    #[test]
    fn test_input_value_bool() {
        let value = InputValue::Bool(true);
        assert_eq!(value.as_bool(), Some(true));
        assert_eq!(value.to_sql_string(), "true");

        let value = InputValue::Bool(false);
        assert_eq!(value.to_sql_string(), "false");
    }

    #[test]
    fn test_input_value_int() {
        let value = InputValue::Int(42);
        assert_eq!(value.as_int(), Some(42));
        assert_eq!(value.as_float(), Some(42.0)); // Can coerce to float
        assert_eq!(value.to_sql_string(), "42");
    }

    #[test]
    fn test_input_value_float() {
        let value = InputValue::Float(3.14);
        assert_eq!(value.as_float(), Some(3.14));
        assert_eq!(value.to_sql_string(), "3.14");
    }

    #[test]
    fn test_input_value_string() {
        let value = InputValue::String("hello".to_string());
        assert_eq!(value.as_string(), Some("hello"));
        assert_eq!(value.to_sql_string(), "hello");
    }

    // ============================================================================
    // Table Permission Tests
    // ============================================================================

    #[test]
    fn test_is_insertable() {
        let table = create_test_table();
        assert!(is_insertable(&table));

        let readonly = create_readonly_table();
        assert!(!is_insertable(&readonly));
    }

    #[test]
    fn test_is_updatable() {
        let table = create_test_table();
        assert!(is_updatable(&table));

        let readonly = create_readonly_table();
        assert!(!is_updatable(&readonly));
    }

    #[test]
    fn test_is_deletable() {
        let table = create_test_table();
        assert!(is_deletable(&table));

        let readonly = create_readonly_table();
        assert!(!is_deletable(&readonly));
    }

    // ============================================================================
    // PascalCase Tests
    // ============================================================================

    #[test]
    fn test_to_pascal_case() {
        assert_eq!(to_pascal_case("users"), "Users");
        assert_eq!(to_pascal_case("user_accounts"), "UserAccounts");
        assert_eq!(to_pascal_case("my_table_name"), "MyTableName");
    }
}
