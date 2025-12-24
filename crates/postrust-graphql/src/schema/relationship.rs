//! Relationship to GraphQL field conversion.

use crate::schema::object::to_pascal_case;
use postrust_core::schema_cache::Relationship;

/// Extract constraint name from a Relationship.
fn get_constraint_name(rel: &Relationship) -> &str {
    match rel {
        Relationship::ForeignKey { constraint_name, .. } => constraint_name,
        Relationship::Computed { function, .. } => &function.name,
    }
}

/// Represents a GraphQL field derived from a database relationship.
#[derive(Debug, Clone)]
pub struct RelationshipField {
    /// Field name (derived from foreign table name).
    pub name: String,
    /// Target GraphQL type name.
    pub target_type: String,
    /// Whether this returns a list (O2M, M2M) or single object (M2O, O2O).
    pub is_list: bool,
    /// The original relationship.
    pub relationship: Relationship,
    /// Description for the field.
    pub description: Option<String>,
}

impl RelationshipField {
    /// Create a GraphQL field from a database relationship.
    pub fn from_relationship(rel: &Relationship) -> Self {
        let foreign_table = rel.foreign_table();
        let is_list = !rel.is_to_one();

        // Generate field name from foreign table
        let name = if is_list {
            // Plural for lists (simple pluralization)
            pluralize(&foreign_table.name)
        } else {
            // Singular for single objects
            singularize(&foreign_table.name)
        };

        let target_type = to_pascal_case(&foreign_table.name);

        let description = Some(format!(
            "Related {} via {}",
            if is_list { "records" } else { "record" },
            get_constraint_name(rel)
        ));

        Self {
            name,
            target_type,
            is_list,
            relationship: rel.clone(),
            description,
        }
    }

    /// Get the GraphQL type string.
    pub fn type_string(&self) -> String {
        if self.is_list {
            format!("[{}!]!", self.target_type)
        } else {
            self.target_type.clone()
        }
    }

    /// Get the join columns for this relationship.
    pub fn join_columns(&self) -> Vec<(String, String)> {
        self.relationship.join_columns()
    }
}

/// Simple pluralization (adds 's' or 'es').
fn pluralize(s: &str) -> String {
    // If already ends with 's' (but not 'ss'), assume it's already plural
    if s.ends_with('s') && !s.ends_with("ss") {
        return s.to_string();
    }

    if s.ends_with('x') || s.ends_with("ch") || s.ends_with("sh") || s.ends_with("ss") {
        format!("{}es", s)
    } else if s.ends_with('y') && !s.ends_with("ey") && !s.ends_with("ay") && !s.ends_with("oy") {
        format!("{}ies", &s[..s.len() - 1])
    } else {
        format!("{}s", s)
    }
}

/// Simple singularization (removes trailing 's').
fn singularize(s: &str) -> String {
    if s.ends_with("ies") {
        format!("{}y", &s[..s.len() - 3])
    } else if s.ends_with("es") && (s.ends_with("ses") || s.ends_with("xes") || s.ends_with("ches") || s.ends_with("shes")) {
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
    use postrust_core::api_request::QualifiedIdentifier;
    use postrust_core::schema_cache::Cardinality;
    use pretty_assertions::assert_eq;

    fn create_m2o_relationship() -> Relationship {
        // orders.user_id -> users.id (Many-to-One)
        Relationship::ForeignKey {
            table: QualifiedIdentifier::new("public", "orders"),
            foreign_table: QualifiedIdentifier::new("public", "users"),
            is_self: false,
            cardinality: Cardinality::M2O {
                constraint: "orders_user_id_fkey".into(),
                columns: vec![("user_id".into(), "id".into())],
            },
            table_is_view: false,
            foreign_table_is_view: false,
            constraint_name: "orders_user_id_fkey".into(),
        }
    }

    fn create_o2m_relationship() -> Relationship {
        // users.id -> orders.user_id (One-to-Many)
        Relationship::ForeignKey {
            table: QualifiedIdentifier::new("public", "users"),
            foreign_table: QualifiedIdentifier::new("public", "orders"),
            is_self: false,
            cardinality: Cardinality::O2M {
                constraint: "orders_user_id_fkey".into(),
                columns: vec![("id".into(), "user_id".into())],
            },
            table_is_view: false,
            foreign_table_is_view: false,
            constraint_name: "orders_user_id_fkey".into(),
        }
    }

    fn create_o2o_relationship() -> Relationship {
        // users.id -> user_profiles.user_id (One-to-One)
        Relationship::ForeignKey {
            table: QualifiedIdentifier::new("public", "users"),
            foreign_table: QualifiedIdentifier::new("public", "user_profiles"),
            is_self: false,
            cardinality: Cardinality::O2O {
                constraint: "user_profiles_user_id_fkey".into(),
                columns: vec![("id".into(), "user_id".into())],
                is_parent: true,
            },
            table_is_view: false,
            foreign_table_is_view: false,
            constraint_name: "user_profiles_user_id_fkey".into(),
        }
    }

    #[test]
    fn test_pluralize() {
        assert_eq!(pluralize("user"), "users");
        assert_eq!(pluralize("order"), "orders");
        assert_eq!(pluralize("category"), "categories");
        assert_eq!(pluralize("box"), "boxes");
        assert_eq!(pluralize("match"), "matches");
        assert_eq!(pluralize("dish"), "dishes");
        assert_eq!(pluralize("key"), "keys"); // 'ey' ending
        assert_eq!(pluralize("day"), "days"); // 'ay' ending
    }

    #[test]
    fn test_singularize() {
        assert_eq!(singularize("users"), "user");
        assert_eq!(singularize("orders"), "order");
        assert_eq!(singularize("categories"), "category");
        assert_eq!(singularize("boxes"), "box");
        assert_eq!(singularize("matches"), "match");
        assert_eq!(singularize("class"), "class"); // ends with 'ss'
    }

    #[test]
    fn test_m2o_relationship_field() {
        let rel = create_m2o_relationship();
        let field = RelationshipField::from_relationship(&rel);

        assert_eq!(field.name, "user"); // Singular for M2O
        assert_eq!(field.target_type, "Users");
        assert!(!field.is_list); // Returns single object
    }

    #[test]
    fn test_o2m_relationship_field() {
        let rel = create_o2m_relationship();
        let field = RelationshipField::from_relationship(&rel);

        assert_eq!(field.name, "orders"); // Plural for O2M
        assert_eq!(field.target_type, "Orders");
        assert!(field.is_list); // Returns list
    }

    #[test]
    fn test_o2o_relationship_field() {
        let rel = create_o2o_relationship();
        let field = RelationshipField::from_relationship(&rel);

        assert_eq!(field.name, "user_profile"); // Singular for O2O
        assert_eq!(field.target_type, "UserProfiles");
        assert!(!field.is_list); // Returns single object
    }

    #[test]
    fn test_relationship_type_string_list() {
        let rel = create_o2m_relationship();
        let field = RelationshipField::from_relationship(&rel);

        assert_eq!(field.type_string(), "[Orders!]!");
    }

    #[test]
    fn test_relationship_type_string_single() {
        let rel = create_m2o_relationship();
        let field = RelationshipField::from_relationship(&rel);

        assert_eq!(field.type_string(), "Users");
    }

    #[test]
    fn test_relationship_join_columns() {
        let rel = create_m2o_relationship();
        let field = RelationshipField::from_relationship(&rel);

        let columns = field.join_columns();
        assert_eq!(columns.len(), 1);
        assert_eq!(columns[0], ("user_id".into(), "id".into()));
    }

    #[test]
    fn test_relationship_description() {
        let rel = create_m2o_relationship();
        let field = RelationshipField::from_relationship(&rel);

        assert!(field.description.is_some());
        assert!(field.description.as_ref().unwrap().contains("orders_user_id_fkey"));
    }

    #[test]
    fn test_m2m_relationship_field() {
        // users -> tags via user_tags junction
        let rel = Relationship::ForeignKey {
            table: QualifiedIdentifier::new("public", "users"),
            foreign_table: QualifiedIdentifier::new("public", "tags"),
            is_self: false,
            cardinality: Cardinality::M2M(postrust_core::schema_cache::Junction {
                table: QualifiedIdentifier::new("public", "user_tags"),
                constraint1: "user_tags_user_id_fkey".into(),
                constraint2: "user_tags_tag_id_fkey".into(),
                source_columns: vec![("id".into(), "user_id".into())],
                target_columns: vec![("tag_id".into(), "id".into())],
            }),
            table_is_view: false,
            foreign_table_is_view: false,
            constraint_name: "user_tags_user_id_fkey".into(),
        };

        let field = RelationshipField::from_relationship(&rel);

        assert_eq!(field.name, "tags"); // Plural for M2M
        assert_eq!(field.target_type, "Tags");
        assert!(field.is_list);
    }
}
