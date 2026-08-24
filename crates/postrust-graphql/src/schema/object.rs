//! Table to GraphQL ObjectType conversion.

use crate::types::{pg_type_to_graphql, GraphQLType};
use postrust_core::schema_cache::{Column, Table};

/// Represents a GraphQL field derived from a database column.
#[derive(Debug, Clone)]
pub struct GraphQLField {
    /// Field name (same as column name).
    pub name: String,
    /// Field description from column comment.
    pub description: Option<String>,
    /// GraphQL type for this field.
    pub graphql_type: GraphQLType,
    /// Whether the field is nullable.
    pub nullable: bool,
    /// Whether this is a primary key field.
    pub is_pk: bool,
}

impl GraphQLField {
    /// Create a GraphQL field from a database column.
    pub fn from_column(column: &Column) -> Self {
        let graphql_type = pg_type_to_graphql(&column.nominal_type);
        let nullable = column.nullable && !column.is_pk;

        Self {
            name: column.name.clone(),
            description: column.description.clone(),
            graphql_type,
            nullable,
            is_pk: column.is_pk,
        }
    }

    /// Get the GraphQL type string with nullability.
    pub fn type_string(&self) -> String {
        let base = format!("{}", self.graphql_type);
        if self.nullable {
            base
        } else {
            format!("{}!", base)
        }
    }
}

/// Represents a GraphQL ObjectType derived from a database table.
#[derive(Debug, Clone)]
pub struct TableObjectType {
    /// The original table.
    pub table: Table,
    /// GraphQL type name (PascalCase).
    pub name: String,
    /// Fields derived from columns.
    pub fields: Vec<GraphQLField>,
}

impl TableObjectType {
    /// Create a GraphQL ObjectType from a database table.
    ///
    /// The type is named after the table. Use [`Self::from_table_named`] when
    /// the name must be disambiguated (for example when the same table name
    /// appears in more than one exposed schema).
    pub fn from_table(table: &Table) -> Self {
        Self::from_table_named(table, &table.name)
    }

    /// Create a GraphQL ObjectType using an explicit base name.
    /// A field for a computed column.
    ///
    /// A function of one argument of the table's own row type, which
    /// PostgreSQL lets a query read as though it were a column. Always
    /// nullable: the function decides, and nothing in the catalogue says it
    /// cannot return null.
    fn computed_field(name: &str, computed: &postrust_core::schema_cache::ComputedColumn) -> GraphQLField {
        GraphQLField {
            name: name.to_string(),
            description: None,
            graphql_type: pg_type_to_graphql(&computed.data_type),
            nullable: true,
            is_pk: false,
        }
    }

    pub fn from_table_named(table: &Table, base_name: &str) -> Self {
        // Named exactly like the table. A GraphQL type is conventionally
        // PascalCase, but the schema a Hasura client was generated against
        // spells this `author`, and a client cannot be told otherwise: the
        // type name is in its generated types, its fragments and its cache
        // keys.
        let name = base_name.to_string();
        // Columns first, then the computed ones. A computed column named
        // after a real one is left out for the same reason a relationship is:
        // two fields of one name abort the schema build, and the column is the
        // table's own data.
        let mut fields: Vec<GraphQLField> = table
            .columns
            .values()
            .map(GraphQLField::from_column)
            .collect();
        let mut computed: Vec<(&String, &postrust_core::schema_cache::ComputedColumn)> =
            table.computed_columns.iter().collect();
        computed.sort_by_key(|(name, _)| (*name).clone());
        for (name, definition) in computed {
            if fields.iter().any(|f| &f.name == name) {
                continue;
            }
            fields.push(Self::computed_field(name, definition));
        }

        Self {
            table: table.clone(),
            name,
            fields,
        }
    }

    /// Get the GraphQL type name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Get the description from table comment.
    pub fn description(&self) -> Option<&str> {
        self.table.description.as_deref()
    }

    /// Get all fields.
    pub fn fields(&self) -> &[GraphQLField] {
        &self.fields
    }

    /// Get a field by name.
    pub fn get_field(&self, name: &str) -> Option<&GraphQLField> {
        self.fields.iter().find(|f| f.name == name)
    }

    /// Check if a field exists.
    pub fn has_field(&self, name: &str) -> bool {
        self.get_field(name).is_some()
    }

    /// Get primary key fields.
    pub fn pk_fields(&self) -> Vec<&GraphQLField> {
        self.fields.iter().filter(|f| f.is_pk).collect()
    }
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
                domain_type: None,
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
                domain_type: None,
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
                domain_type: None,
            },
        );
        columns.insert(
            "metadata".into(),
            Column {
                name: "metadata".into(),
                description: Some("JSON metadata".into()),
                nullable: true,
                data_type: "jsonb".into(),
                nominal_type: "jsonb".into(),
                max_len: None,
                default: None,
                enum_values: vec![],
                is_pk: false,
                position: 4,
                domain_type: None,
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
            computed_columns: Default::default(),
            is_partitioned: false,
        }
    }

    #[test]
    fn test_table_to_graphql_object_name() {
        let table = create_test_table();
        let obj = TableObjectType::from_table(&table);

        assert_eq!(obj.name(), "users"); // named exactly like the table
    }

    #[test]
    fn test_table_to_graphql_object_description() {
        let table = create_test_table();
        let obj = TableObjectType::from_table(&table);

        assert_eq!(obj.description(), Some("User accounts"));
    }

    #[test]
    fn test_table_to_graphql_object_fields() {
        let table = create_test_table();
        let obj = TableObjectType::from_table(&table);
        let fields = obj.fields();

        assert_eq!(fields.len(), 4);
        assert!(obj.has_field("id"));
        assert!(obj.has_field("name"));
        assert!(obj.has_field("email"));
        assert!(obj.has_field("metadata"));
    }

    #[test]
    fn test_field_types() {
        let table = create_test_table();
        let obj = TableObjectType::from_table(&table);

        let id_field = obj.get_field("id").unwrap();
        assert_eq!(id_field.graphql_type, GraphQLType::Int);

        let name_field = obj.get_field("name").unwrap();
        assert_eq!(name_field.graphql_type, GraphQLType::String);

        let metadata_field = obj.get_field("metadata").unwrap();
        assert_eq!(metadata_field.graphql_type, GraphQLType::Json);
    }

    #[test]
    fn test_field_nullability() {
        let table = create_test_table();
        let obj = TableObjectType::from_table(&table);

        let id_field = obj.get_field("id").unwrap();
        assert!(!id_field.nullable); // PK is never nullable

        let name_field = obj.get_field("name").unwrap();
        assert!(!name_field.nullable); // Not nullable in DB

        let email_field = obj.get_field("email").unwrap();
        assert!(email_field.nullable); // Nullable in DB
    }

    #[test]
    fn test_field_descriptions() {
        let table = create_test_table();
        let obj = TableObjectType::from_table(&table);

        let id_field = obj.get_field("id").unwrap();
        assert_eq!(id_field.description, Some("Primary key".into()));

        let email_field = obj.get_field("email").unwrap();
        assert_eq!(email_field.description, None);
    }

    #[test]
    fn test_field_type_string() {
        let table = create_test_table();
        let obj = TableObjectType::from_table(&table);

        let id_field = obj.get_field("id").unwrap();
        assert_eq!(id_field.type_string(), "Int!"); // Non-null

        let email_field = obj.get_field("email").unwrap();
        assert_eq!(email_field.type_string(), "String"); // Nullable
    }

    #[test]
    fn test_pk_fields() {
        let table = create_test_table();
        let obj = TableObjectType::from_table(&table);

        let pk_fields = obj.pk_fields();
        assert_eq!(pk_fields.len(), 1);
        assert_eq!(pk_fields[0].name, "id");
    }

    #[test]
    fn test_table_with_underscore_name() {
        let mut table = create_test_table();
        table.name = "user_accounts".into();

        let obj = TableObjectType::from_table(&table);
        assert_eq!(obj.name(), "user_accounts");
    }
}
