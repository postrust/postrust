//! Relationship types for resource embedding.

use crate::api_request::QualifiedIdentifier;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A relationship between tables.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Relationship {
    /// Foreign key relationship
    ForeignKey {
        /// Source table
        table: QualifiedIdentifier,
        /// Target table
        foreign_table: QualifiedIdentifier,
        /// Whether this is a self-referential relationship
        is_self: bool,
        /// Relationship cardinality
        cardinality: Cardinality,
        /// Whether the source is a view
        table_is_view: bool,
        /// Whether the target is a view
        foreign_table_is_view: bool,
        /// FK constraint name
        constraint_name: String,
    },
    /// Computed relationship (from a function)
    Computed {
        /// Function that computes the relationship
        function: QualifiedIdentifier,
        /// Source table
        table: QualifiedIdentifier,
        /// Target table
        foreign_table: QualifiedIdentifier,
        /// Alias for the relationship
        table_alias: QualifiedIdentifier,
        /// Whether this returns a single row
        to_one: bool,
        /// Whether this is self-referential
        is_self: bool,
    },
}

impl Relationship {
    /// Get the foreign table for this relationship.
    pub fn foreign_table(&self) -> &QualifiedIdentifier {
        match self {
            Self::ForeignKey { foreign_table, .. } => foreign_table,
            Self::Computed { foreign_table, .. } => foreign_table,
        }
    }

    /// Check if this relationship returns a single row.
    pub fn is_to_one(&self) -> bool {
        match self {
            Self::ForeignKey { cardinality, .. } => matches!(
                cardinality,
                Cardinality::M2O { .. } | Cardinality::O2O { .. }
            ),
            Self::Computed { to_one, .. } => *to_one,
        }
    }

    /// Get the join columns for this relationship.
    pub fn join_columns(&self) -> Vec<(String, String)> {
        match self {
            Self::ForeignKey { cardinality, .. } => cardinality.columns(),
            Self::Computed { .. } => vec![],
        }
    }
}

/// Relationship cardinality.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Cardinality {
    /// One-to-Many: parent has many children
    O2M {
        constraint: String,
        columns: Vec<(String, String)>,
    },
    /// Many-to-One: child has one parent
    M2O {
        constraint: String,
        columns: Vec<(String, String)>,
    },
    /// One-to-One
    O2O {
        constraint: String,
        columns: Vec<(String, String)>,
        /// Whether this table is the parent in the relationship
        is_parent: bool,
    },
    /// Many-to-Many (via junction table)
    M2M(Junction),
}

impl Cardinality {
    /// Get the join columns.
    pub fn columns(&self) -> Vec<(String, String)> {
        match self {
            Self::O2M { columns, .. } => columns.clone(),
            Self::M2O { columns, .. } => columns.clone(),
            Self::O2O { columns, .. } => columns.clone(),
            Self::M2M(junction) => junction.source_columns(),
        }
    }

    /// Get the constraint name.
    pub fn constraint_name(&self) -> &str {
        match self {
            Self::O2M { constraint, .. } => constraint,
            Self::M2O { constraint, .. } => constraint,
            Self::O2O { constraint, .. } => constraint,
            Self::M2M(junction) => &junction.constraint1,
        }
    }
}

/// Junction table for M2M relationships.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Junction {
    /// The junction table
    pub table: QualifiedIdentifier,
    /// FK constraint from junction to source
    pub constraint1: String,
    /// FK constraint from junction to target
    pub constraint2: String,
    /// Columns linking source to junction
    pub source_columns: Vec<(String, String)>,
    /// Columns linking junction to target
    pub target_columns: Vec<(String, String)>,
}

impl Junction {
    /// Get the source-side join columns.
    pub fn source_columns(&self) -> Vec<(String, String)> {
        self.source_columns.clone()
    }

    /// Get the target-side join columns.
    pub fn target_columns(&self) -> Vec<(String, String)> {
        self.target_columns.clone()
    }
}

/// Map of (table, schema) to relationships.
pub type RelationshipsMap = HashMap<(QualifiedIdentifier, String), Vec<Relationship>>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_relationship_foreign_table() {
        let rel = Relationship::ForeignKey {
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
        };

        assert_eq!(rel.foreign_table().name, "users");
        assert!(rel.is_to_one());
    }

    #[test]
    fn test_cardinality_columns() {
        let card = Cardinality::O2M {
            constraint: "users_id_fkey".into(),
            columns: vec![("id".into(), "user_id".into())],
        };

        let cols = card.columns();
        assert_eq!(cols.len(), 1);
        assert_eq!(cols[0], ("id".into(), "user_id".into()));
    }
}
