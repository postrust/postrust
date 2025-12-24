//! Coercible types for query planning.
//!
//! These types carry additional type information needed for
//! proper SQL generation with type coercion.

use crate::api_request::{
    AggregateFunction, Field, Filter, JoinType, JsonPath, LogicOperator,
    LogicTree, OpExpr, OrderDirection, OrderNulls, OrderTerm, QualifiedIdentifier,
};
use serde::{Deserialize, Serialize};

/// A field with type coercion information.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CoercibleField {
    /// Field name
    pub name: String,
    /// JSON path (for JSON columns)
    pub json_path: JsonPath,
    /// Whether to convert to JSON
    pub to_json: bool,
    /// Full-text search configuration
    pub to_tsvector: Option<String>,
    /// PostgreSQL type
    pub ir_type: String,
    /// Base type (for domains)
    pub base_type: String,
    /// Type transformer function
    pub transform: Option<String>,
    /// Default value expression
    pub default: Option<String>,
    /// Whether to select full row
    pub full_row: bool,
}

impl CoercibleField {
    /// Create from a simple field name.
    pub fn simple(name: impl Into<String>, pg_type: impl Into<String>) -> Self {
        let type_str = pg_type.into();
        Self {
            name: name.into(),
            json_path: vec![],
            to_json: false,
            to_tsvector: None,
            ir_type: type_str.clone(),
            base_type: type_str,
            transform: None,
            default: None,
            full_row: false,
        }
    }

    /// Create from an API field with type info.
    pub fn from_field(field: &Field, pg_type: &str) -> Self {
        Self {
            name: field.name.clone(),
            json_path: field.json_path.clone(),
            to_json: false,
            to_tsvector: None,
            ir_type: pg_type.to_string(),
            base_type: pg_type.to_string(),
            transform: None,
            default: None,
            full_row: false,
        }
    }
}

/// A select field with coercion and aggregation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CoercibleSelectField {
    /// The field
    pub field: CoercibleField,
    /// Aggregate function
    pub aggregate: Option<AggregateFunction>,
    /// Cast for aggregate result
    pub aggregate_cast: Option<String>,
    /// Output cast
    pub cast: Option<String>,
    /// Output alias
    pub alias: Option<String>,
}

impl CoercibleSelectField {
    /// Create a simple select field.
    pub fn simple(name: &str, pg_type: &str) -> Self {
        Self {
            field: CoercibleField::simple(name, pg_type),
            aggregate: None,
            aggregate_cast: None,
            cast: None,
            alias: None,
        }
    }

    /// Create with alias.
    pub fn with_alias(name: &str, pg_type: &str, alias: &str) -> Self {
        Self {
            field: CoercibleField::simple(name, pg_type),
            aggregate: None,
            aggregate_cast: None,
            cast: None,
            alias: Some(alias.to_string()),
        }
    }
}

/// A filter with coercion information.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CoercibleFilter {
    /// The field
    pub field: CoercibleField,
    /// The operation
    pub op_expr: OpExpr,
}

impl CoercibleFilter {
    /// Create from a filter with type info.
    pub fn from_filter(filter: &Filter, pg_type: &str) -> Self {
        Self {
            field: CoercibleField::from_field(&filter.field, pg_type),
            op_expr: filter.op_expr.clone(),
        }
    }
}

/// A logic tree with coercion information.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum CoercibleLogicTree {
    /// Boolean expression
    Expr {
        negated: bool,
        op: LogicOperator,
        children: Vec<CoercibleLogicTree>,
    },
    /// Leaf filter
    Stmt(CoercibleFilter),
    /// NULL check for embedding
    NullEmbed {
        negated: bool,
        field_name: String,
    },
}

impl CoercibleLogicTree {
    /// Create from a logic tree with a type resolver.
    pub fn from_logic_tree<F>(tree: &LogicTree, resolver: F) -> Self
    where
        F: Fn(&str) -> String + Copy,
    {
        match tree {
            LogicTree::Expr { negated, op, children } => Self::Expr {
                negated: *negated,
                op: op.clone(),
                children: children
                    .iter()
                    .map(|c| Self::from_logic_tree(c, resolver))
                    .collect(),
            },
            LogicTree::Stmt(filter) => {
                let pg_type = resolver(&filter.field.name);
                Self::Stmt(CoercibleFilter::from_filter(filter, &pg_type))
            }
        }
    }
}

/// An ORDER BY term with coercion.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CoercibleOrderTerm {
    /// The field
    pub field: CoercibleField,
    /// Sort direction
    pub direction: Option<OrderDirection>,
    /// NULL ordering
    pub nulls: Option<OrderNulls>,
    /// Relation (for embedded ordering)
    pub relation: Option<String>,
}

impl CoercibleOrderTerm {
    /// Create from an order term with type info.
    pub fn from_order_term(term: &OrderTerm, pg_type: &str) -> Self {
        match term {
            OrderTerm::Field { field, direction, nulls } => Self {
                field: CoercibleField::from_field(field, pg_type),
                direction: direction.clone(),
                nulls: nulls.clone(),
                relation: None,
            },
            OrderTerm::Relation { relation, field, direction, nulls } => Self {
                field: CoercibleField::from_field(field, pg_type),
                direction: direction.clone(),
                nulls: nulls.clone(),
                relation: Some(relation.clone()),
            },
        }
    }
}

/// Join condition between tables.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JoinCondition {
    /// Left side (table.column)
    pub left: (QualifiedIdentifier, String),
    /// Right side (table.column)
    pub right: (QualifiedIdentifier, String),
}

/// Relation select field (for embedding).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RelSelectField {
    /// Relation name
    pub name: String,
    /// Aggregate alias
    pub agg_alias: String,
    /// Join type
    pub join_type: JoinType,
    /// Whether this is a spread relation
    pub is_spread: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_coercible_field_simple() {
        let field = CoercibleField::simple("id", "integer");
        assert_eq!(field.name, "id");
        assert_eq!(field.ir_type, "integer");
        assert!(field.json_path.is_empty());
    }

    #[test]
    fn test_coercible_select_field() {
        let sel = CoercibleSelectField::with_alias("created_at", "timestamptz", "created");
        assert_eq!(sel.alias, Some("created".into()));
    }
}
