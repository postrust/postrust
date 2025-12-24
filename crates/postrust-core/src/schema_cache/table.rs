//! Table and column types.

use crate::api_request::QualifiedIdentifier;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A database table or view.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Table {
    /// Schema name
    pub schema: String,
    /// Table/view name
    pub name: String,
    /// Description from comment
    pub description: Option<String>,
    /// Whether this is a view (vs a table)
    pub is_view: bool,
    /// Whether INSERT is allowed
    pub insertable: bool,
    /// Whether UPDATE is allowed
    pub updatable: bool,
    /// Whether DELETE is allowed
    pub deletable: bool,
    /// Primary key column names
    pub pk_cols: Vec<String>,
    /// Columns indexed by name
    pub columns: ColumnMap,
}

impl Table {
    /// Get a column by name.
    pub fn get_column(&self, name: &str) -> Option<&Column> {
        self.columns.get(name)
    }

    /// Check if the table has a column.
    pub fn has_column(&self, name: &str) -> bool {
        self.columns.contains_key(name)
    }

    /// Get the qualified identifier for this table.
    pub fn qualified_identifier(&self) -> QualifiedIdentifier {
        QualifiedIdentifier::new(&self.schema, &self.name)
    }

    /// Get column names in order.
    pub fn column_names(&self) -> impl Iterator<Item = &str> {
        self.columns.keys().map(|s| s.as_str())
    }

    /// Check if this is a read-only view.
    pub fn is_readonly(&self) -> bool {
        !self.insertable && !self.updatable && !self.deletable
    }
}

/// A table column.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Column {
    /// Column name
    pub name: String,
    /// Description from comment
    pub description: Option<String>,
    /// Whether NULL is allowed
    pub nullable: bool,
    /// PostgreSQL data type
    pub data_type: String,
    /// Base type (for domains)
    pub nominal_type: String,
    /// Maximum length (for varchar, etc.)
    pub max_len: Option<i32>,
    /// Default value expression
    pub default: Option<String>,
    /// Enum values (for enum types)
    pub enum_values: Vec<String>,
    /// Whether this is part of the primary key
    pub is_pk: bool,
    /// Column position (1-based)
    pub position: i32,
}

impl Column {
    /// Check if this column has a default value.
    pub fn has_default(&self) -> bool {
        self.default.is_some()
    }

    /// Check if this is an auto-generated column.
    pub fn is_auto(&self) -> bool {
        self.default
            .as_ref()
            .map(|d| d.contains("nextval(") || d.contains("gen_random_uuid()"))
            .unwrap_or(false)
    }

    /// Check if this is a JSON/JSONB column.
    pub fn is_json(&self) -> bool {
        self.data_type == "json" || self.data_type == "jsonb"
    }

    /// Check if this is an array type.
    pub fn is_array(&self) -> bool {
        self.data_type.starts_with('_') || self.data_type.ends_with("[]")
    }

    /// Check if this is a range type.
    pub fn is_range(&self) -> bool {
        self.data_type.ends_with("range")
    }
}

/// Map of column name to column.
pub type ColumnMap = IndexMap<String, Column>;

/// Map of qualified identifier to table.
pub type TablesMap = HashMap<QualifiedIdentifier, Table>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_table_qualified_identifier() {
        let table = Table {
            schema: "public".into(),
            name: "users".into(),
            description: None,
            is_view: false,
            insertable: true,
            updatable: true,
            deletable: true,
            pk_cols: vec!["id".into()],
            columns: IndexMap::new(),
        };

        let qi = table.qualified_identifier();
        assert_eq!(qi.schema, "public");
        assert_eq!(qi.name, "users");
    }

    #[test]
    fn test_column_is_auto() {
        let col1 = Column {
            name: "id".into(),
            description: None,
            nullable: false,
            data_type: "integer".into(),
            nominal_type: "integer".into(),
            max_len: None,
            default: Some("nextval('users_id_seq'::regclass)".into()),
            enum_values: vec![],
            is_pk: true,
            position: 1,
        };
        assert!(col1.is_auto());

        let col2 = Column {
            name: "uuid".into(),
            description: None,
            nullable: false,
            data_type: "uuid".into(),
            nominal_type: "uuid".into(),
            max_len: None,
            default: Some("gen_random_uuid()".into()),
            enum_values: vec![],
            is_pk: false,
            position: 2,
        };
        assert!(col2.is_auto());

        let col3 = Column {
            name: "name".into(),
            description: None,
            nullable: false,
            data_type: "text".into(),
            nominal_type: "text".into(),
            max_len: None,
            default: None,
            enum_values: vec![],
            is_pk: false,
            position: 3,
        };
        assert!(!col3.is_auto());
    }
}
