//! Filter input types for GraphQL queries.
//!
//! Provides type-specific filter inputs (StringFilterInput, IntFilterInput, etc.)
//! that can be combined with AND/OR/NOT logic to form complex queries.

use postrust_core::api_request::{
    Field, Filter, LogicOperator, LogicTree, OpExpr, Operation, QuantOperator,
};
use serde::{Deserialize, Serialize};

/// Filter input for String fields.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StringFilterInput {
    /// Equals
    pub eq: Option<String>,
    /// Not equals
    pub neq: Option<String>,
    /// LIKE pattern match (case-sensitive)
    pub like: Option<String>,
    /// ILIKE pattern match (case-insensitive)
    pub ilike: Option<String>,
    /// In list
    #[serde(rename = "in")]
    pub in_list: Option<Vec<String>>,
    /// Is null check
    #[serde(rename = "isNull")]
    pub is_null: Option<bool>,
    /// Starts with
    #[serde(rename = "startsWith")]
    pub starts_with: Option<String>,
    /// Ends with
    #[serde(rename = "endsWith")]
    pub ends_with: Option<String>,
    /// Contains
    pub contains: Option<String>,
}

impl StringFilterInput {
    /// Convert to a list of Filters for a given field.
    pub fn to_filters(&self, field_name: &str) -> Vec<Filter> {
        let mut filters = Vec::new();
        let field = Field::simple(field_name);

        if let Some(ref value) = self.eq {
            filters.push(Filter::new(
                field.clone(),
                OpExpr::new(Operation::Quant {
                    op: QuantOperator::Equal,
                    quantifier: None,
                    value: value.clone(),
                }),
            ));
        }

        if let Some(ref value) = self.neq {
            filters.push(Filter::new(
                field.clone(),
                OpExpr::new(Operation::Quant {
                    op: QuantOperator::Equal,
                    quantifier: None,
                    value: value.clone(),
                })
                .with_negated(true),
            ));
        }

        if let Some(ref value) = self.like {
            filters.push(Filter::new(
                field.clone(),
                OpExpr::new(Operation::Quant {
                    op: QuantOperator::Like,
                    quantifier: None,
                    value: value.clone(),
                }),
            ));
        }

        if let Some(ref value) = self.ilike {
            filters.push(Filter::new(
                field.clone(),
                OpExpr::new(Operation::Quant {
                    op: QuantOperator::ILike,
                    quantifier: None,
                    value: value.clone(),
                }),
            ));
        }

        if let Some(ref values) = self.in_list {
            filters.push(Filter::new(
                field.clone(),
                OpExpr::new(Operation::In(values.clone())),
            ));
        }

        if let Some(is_null) = self.is_null {
            let op_expr = OpExpr::new(Operation::Is(
                postrust_core::api_request::IsValue::Null,
            ));
            filters.push(Filter::new(
                field.clone(),
                if is_null {
                    op_expr
                } else {
                    op_expr.with_negated(true)
                },
            ));
        }

        if let Some(ref value) = self.starts_with {
            filters.push(Filter::new(
                field.clone(),
                OpExpr::new(Operation::Quant {
                    op: QuantOperator::Like,
                    quantifier: None,
                    value: format!("{}%", value),
                }),
            ));
        }

        if let Some(ref value) = self.ends_with {
            filters.push(Filter::new(
                field.clone(),
                OpExpr::new(Operation::Quant {
                    op: QuantOperator::Like,
                    quantifier: None,
                    value: format!("%{}", value),
                }),
            ));
        }

        if let Some(ref value) = self.contains {
            filters.push(Filter::new(
                field.clone(),
                OpExpr::new(Operation::Quant {
                    op: QuantOperator::Like,
                    quantifier: None,
                    value: format!("%{}%", value),
                }),
            ));
        }

        filters
    }

    /// Check if any filter is set.
    pub fn is_empty(&self) -> bool {
        self.eq.is_none()
            && self.neq.is_none()
            && self.like.is_none()
            && self.ilike.is_none()
            && self.in_list.is_none()
            && self.is_null.is_none()
            && self.starts_with.is_none()
            && self.ends_with.is_none()
            && self.contains.is_none()
    }
}

/// Filter input for Int fields.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IntFilterInput {
    /// Equals
    pub eq: Option<i64>,
    /// Not equals
    pub neq: Option<i64>,
    /// Greater than
    pub gt: Option<i64>,
    /// Greater than or equal
    pub gte: Option<i64>,
    /// Less than
    pub lt: Option<i64>,
    /// Less than or equal
    pub lte: Option<i64>,
    /// In list
    #[serde(rename = "in")]
    pub in_list: Option<Vec<i64>>,
    /// Is null check
    #[serde(rename = "isNull")]
    pub is_null: Option<bool>,
}

impl IntFilterInput {
    /// Convert to a list of Filters for a given field.
    pub fn to_filters(&self, field_name: &str) -> Vec<Filter> {
        let mut filters = Vec::new();
        let field = Field::simple(field_name);

        if let Some(value) = self.eq {
            filters.push(Filter::new(
                field.clone(),
                OpExpr::new(Operation::Quant {
                    op: QuantOperator::Equal,
                    quantifier: None,
                    value: value.to_string(),
                }),
            ));
        }

        if let Some(value) = self.neq {
            filters.push(Filter::new(
                field.clone(),
                OpExpr::new(Operation::Quant {
                    op: QuantOperator::Equal,
                    quantifier: None,
                    value: value.to_string(),
                })
                .with_negated(true),
            ));
        }

        if let Some(value) = self.gt {
            filters.push(Filter::new(
                field.clone(),
                OpExpr::new(Operation::Quant {
                    op: QuantOperator::GreaterThan,
                    quantifier: None,
                    value: value.to_string(),
                }),
            ));
        }

        if let Some(value) = self.gte {
            filters.push(Filter::new(
                field.clone(),
                OpExpr::new(Operation::Quant {
                    op: QuantOperator::GreaterThanEqual,
                    quantifier: None,
                    value: value.to_string(),
                }),
            ));
        }

        if let Some(value) = self.lt {
            filters.push(Filter::new(
                field.clone(),
                OpExpr::new(Operation::Quant {
                    op: QuantOperator::LessThan,
                    quantifier: None,
                    value: value.to_string(),
                }),
            ));
        }

        if let Some(value) = self.lte {
            filters.push(Filter::new(
                field.clone(),
                OpExpr::new(Operation::Quant {
                    op: QuantOperator::LessThanEqual,
                    quantifier: None,
                    value: value.to_string(),
                }),
            ));
        }

        if let Some(ref values) = self.in_list {
            filters.push(Filter::new(
                field.clone(),
                OpExpr::new(Operation::In(
                    values.iter().map(|v| v.to_string()).collect(),
                )),
            ));
        }

        if let Some(is_null) = self.is_null {
            let op_expr = OpExpr::new(Operation::Is(
                postrust_core::api_request::IsValue::Null,
            ));
            filters.push(Filter::new(
                field.clone(),
                if is_null {
                    op_expr
                } else {
                    op_expr.with_negated(true)
                },
            ));
        }

        filters
    }

    /// Check if any filter is set.
    pub fn is_empty(&self) -> bool {
        self.eq.is_none()
            && self.neq.is_none()
            && self.gt.is_none()
            && self.gte.is_none()
            && self.lt.is_none()
            && self.lte.is_none()
            && self.in_list.is_none()
            && self.is_null.is_none()
    }
}

/// Filter input for Float fields.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FloatFilterInput {
    /// Equals
    pub eq: Option<f64>,
    /// Not equals
    pub neq: Option<f64>,
    /// Greater than
    pub gt: Option<f64>,
    /// Greater than or equal
    pub gte: Option<f64>,
    /// Less than
    pub lt: Option<f64>,
    /// Less than or equal
    pub lte: Option<f64>,
    /// Is null check
    #[serde(rename = "isNull")]
    pub is_null: Option<bool>,
}

impl FloatFilterInput {
    /// Convert to a list of Filters for a given field.
    pub fn to_filters(&self, field_name: &str) -> Vec<Filter> {
        let mut filters = Vec::new();
        let field = Field::simple(field_name);

        if let Some(value) = self.eq {
            filters.push(Filter::new(
                field.clone(),
                OpExpr::new(Operation::Quant {
                    op: QuantOperator::Equal,
                    quantifier: None,
                    value: value.to_string(),
                }),
            ));
        }

        if let Some(value) = self.neq {
            filters.push(Filter::new(
                field.clone(),
                OpExpr::new(Operation::Quant {
                    op: QuantOperator::Equal,
                    quantifier: None,
                    value: value.to_string(),
                })
                .with_negated(true),
            ));
        }

        if let Some(value) = self.gt {
            filters.push(Filter::new(
                field.clone(),
                OpExpr::new(Operation::Quant {
                    op: QuantOperator::GreaterThan,
                    quantifier: None,
                    value: value.to_string(),
                }),
            ));
        }

        if let Some(value) = self.gte {
            filters.push(Filter::new(
                field.clone(),
                OpExpr::new(Operation::Quant {
                    op: QuantOperator::GreaterThanEqual,
                    quantifier: None,
                    value: value.to_string(),
                }),
            ));
        }

        if let Some(value) = self.lt {
            filters.push(Filter::new(
                field.clone(),
                OpExpr::new(Operation::Quant {
                    op: QuantOperator::LessThan,
                    quantifier: None,
                    value: value.to_string(),
                }),
            ));
        }

        if let Some(value) = self.lte {
            filters.push(Filter::new(
                field.clone(),
                OpExpr::new(Operation::Quant {
                    op: QuantOperator::LessThanEqual,
                    quantifier: None,
                    value: value.to_string(),
                }),
            ));
        }

        if let Some(is_null) = self.is_null {
            let op_expr = OpExpr::new(Operation::Is(
                postrust_core::api_request::IsValue::Null,
            ));
            filters.push(Filter::new(
                field.clone(),
                if is_null {
                    op_expr
                } else {
                    op_expr.with_negated(true)
                },
            ));
        }

        filters
    }

    /// Check if any filter is set.
    pub fn is_empty(&self) -> bool {
        self.eq.is_none()
            && self.neq.is_none()
            && self.gt.is_none()
            && self.gte.is_none()
            && self.lt.is_none()
            && self.lte.is_none()
            && self.is_null.is_none()
    }
}

/// Filter input for Boolean fields.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BooleanFilterInput {
    /// Equals
    pub eq: Option<bool>,
    /// Is null check
    #[serde(rename = "isNull")]
    pub is_null: Option<bool>,
}

impl BooleanFilterInput {
    /// Convert to a list of Filters for a given field.
    pub fn to_filters(&self, field_name: &str) -> Vec<Filter> {
        let mut filters = Vec::new();
        let field = Field::simple(field_name);

        if let Some(value) = self.eq {
            filters.push(Filter::new(
                field.clone(),
                OpExpr::new(Operation::Is(if value {
                    postrust_core::api_request::IsValue::True
                } else {
                    postrust_core::api_request::IsValue::False
                })),
            ));
        }

        if let Some(is_null) = self.is_null {
            let op_expr = OpExpr::new(Operation::Is(
                postrust_core::api_request::IsValue::Null,
            ));
            filters.push(Filter::new(
                field.clone(),
                if is_null {
                    op_expr
                } else {
                    op_expr.with_negated(true)
                },
            ));
        }

        filters
    }

    /// Check if any filter is set.
    pub fn is_empty(&self) -> bool {
        self.eq.is_none() && self.is_null.is_none()
    }
}

/// Filter input for UUID fields.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UuidFilterInput {
    /// Equals
    pub eq: Option<String>,
    /// Not equals
    pub neq: Option<String>,
    /// In list
    #[serde(rename = "in")]
    pub in_list: Option<Vec<String>>,
    /// Is null check
    #[serde(rename = "isNull")]
    pub is_null: Option<bool>,
}

impl UuidFilterInput {
    /// Convert to a list of Filters for a given field.
    pub fn to_filters(&self, field_name: &str) -> Vec<Filter> {
        let mut filters = Vec::new();
        let field = Field::simple(field_name);

        if let Some(ref value) = self.eq {
            filters.push(Filter::new(
                field.clone(),
                OpExpr::new(Operation::Quant {
                    op: QuantOperator::Equal,
                    quantifier: None,
                    value: value.clone(),
                }),
            ));
        }

        if let Some(ref value) = self.neq {
            filters.push(Filter::new(
                field.clone(),
                OpExpr::new(Operation::Quant {
                    op: QuantOperator::Equal,
                    quantifier: None,
                    value: value.clone(),
                })
                .with_negated(true),
            ));
        }

        if let Some(ref values) = self.in_list {
            filters.push(Filter::new(
                field.clone(),
                OpExpr::new(Operation::In(values.clone())),
            ));
        }

        if let Some(is_null) = self.is_null {
            let op_expr = OpExpr::new(Operation::Is(
                postrust_core::api_request::IsValue::Null,
            ));
            filters.push(Filter::new(
                field.clone(),
                if is_null {
                    op_expr
                } else {
                    op_expr.with_negated(true)
                },
            ));
        }

        filters
    }

    /// Check if any filter is set.
    pub fn is_empty(&self) -> bool {
        self.eq.is_none()
            && self.neq.is_none()
            && self.in_list.is_none()
            && self.is_null.is_none()
    }
}

/// Convert a list of filters to a LogicTree with AND logic.
pub fn filters_to_logic_tree(filters: Vec<Filter>) -> Option<LogicTree> {
    if filters.is_empty() {
        return None;
    }

    if filters.len() == 1 {
        return Some(LogicTree::Stmt(filters.into_iter().next().unwrap()));
    }

    Some(LogicTree::Expr {
        negated: false,
        op: LogicOperator::And,
        children: filters.into_iter().map(LogicTree::Stmt).collect(),
    })
}

/// Combine multiple LogicTrees with AND logic.
pub fn combine_with_and(trees: Vec<LogicTree>) -> Option<LogicTree> {
    if trees.is_empty() {
        return None;
    }

    if trees.len() == 1 {
        return Some(trees.into_iter().next().unwrap());
    }

    Some(LogicTree::Expr {
        negated: false,
        op: LogicOperator::And,
        children: trees,
    })
}

/// Combine multiple LogicTrees with OR logic.
pub fn combine_with_or(trees: Vec<LogicTree>) -> Option<LogicTree> {
    if trees.is_empty() {
        return None;
    }

    if trees.len() == 1 {
        return Some(trees.into_iter().next().unwrap());
    }

    Some(LogicTree::Expr {
        negated: false,
        op: LogicOperator::Or,
        children: trees,
    })
}

/// Negate a LogicTree.
pub fn negate_tree(tree: LogicTree) -> LogicTree {
    match tree {
        LogicTree::Expr {
            negated,
            op,
            children,
        } => LogicTree::Expr {
            negated: !negated,
            op,
            children,
        },
        LogicTree::Stmt(filter) => {
            let negated_expr = OpExpr {
                negated: !filter.op_expr.negated,
                operation: filter.op_expr.operation,
            };
            LogicTree::Stmt(Filter::new(filter.field, negated_expr))
        }
    }
}

/// Extension trait to add with_negated to OpExpr.
trait OpExprExt {
    fn with_negated(self, negated: bool) -> Self;
}

impl OpExprExt for OpExpr {
    fn with_negated(mut self, negated: bool) -> Self {
        self.negated = negated;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    // ============================================================================
    // StringFilterInput Tests
    // ============================================================================

    #[test]
    fn test_string_filter_eq() {
        let filter = StringFilterInput {
            eq: Some("test".to_string()),
            ..Default::default()
        };

        let filters = filter.to_filters("name");
        assert_eq!(filters.len(), 1);
        assert_eq!(filters[0].field.name, "name");

        match &filters[0].op_expr.operation {
            Operation::Quant { op, value, .. } => {
                assert_eq!(*op, QuantOperator::Equal);
                assert_eq!(value, "test");
            }
            _ => panic!("Expected Quant operation"),
        }
        assert!(!filters[0].op_expr.negated);
    }

    #[test]
    fn test_string_filter_neq() {
        let filter = StringFilterInput {
            neq: Some("test".to_string()),
            ..Default::default()
        };

        let filters = filter.to_filters("name");
        assert_eq!(filters.len(), 1);

        match &filters[0].op_expr.operation {
            Operation::Quant { op, value, .. } => {
                assert_eq!(*op, QuantOperator::Equal);
                assert_eq!(value, "test");
            }
            _ => panic!("Expected Quant operation"),
        }
        assert!(filters[0].op_expr.negated);
    }

    #[test]
    fn test_string_filter_like() {
        let filter = StringFilterInput {
            like: Some("%test%".to_string()),
            ..Default::default()
        };

        let filters = filter.to_filters("name");
        assert_eq!(filters.len(), 1);

        match &filters[0].op_expr.operation {
            Operation::Quant { op, value, .. } => {
                assert_eq!(*op, QuantOperator::Like);
                assert_eq!(value, "%test%");
            }
            _ => panic!("Expected Quant operation"),
        }
    }

    #[test]
    fn test_string_filter_ilike() {
        let filter = StringFilterInput {
            ilike: Some("%TEST%".to_string()),
            ..Default::default()
        };

        let filters = filter.to_filters("name");
        assert_eq!(filters.len(), 1);

        match &filters[0].op_expr.operation {
            Operation::Quant { op, value, .. } => {
                assert_eq!(*op, QuantOperator::ILike);
                assert_eq!(value, "%TEST%");
            }
            _ => panic!("Expected Quant operation"),
        }
    }

    #[test]
    fn test_string_filter_in() {
        let filter = StringFilterInput {
            in_list: Some(vec!["a".to_string(), "b".to_string(), "c".to_string()]),
            ..Default::default()
        };

        let filters = filter.to_filters("status");
        assert_eq!(filters.len(), 1);

        match &filters[0].op_expr.operation {
            Operation::In(values) => {
                assert_eq!(values.len(), 3);
                assert_eq!(values[0], "a");
                assert_eq!(values[1], "b");
                assert_eq!(values[2], "c");
            }
            _ => panic!("Expected In operation"),
        }
    }

    #[test]
    fn test_string_filter_is_null_true() {
        let filter = StringFilterInput {
            is_null: Some(true),
            ..Default::default()
        };

        let filters = filter.to_filters("name");
        assert_eq!(filters.len(), 1);

        match &filters[0].op_expr.operation {
            Operation::Is(postrust_core::api_request::IsValue::Null) => {}
            _ => panic!("Expected Is Null operation"),
        }
        assert!(!filters[0].op_expr.negated);
    }

    #[test]
    fn test_string_filter_is_null_false() {
        let filter = StringFilterInput {
            is_null: Some(false),
            ..Default::default()
        };

        let filters = filter.to_filters("name");
        assert_eq!(filters.len(), 1);

        match &filters[0].op_expr.operation {
            Operation::Is(postrust_core::api_request::IsValue::Null) => {}
            _ => panic!("Expected Is Null operation"),
        }
        assert!(filters[0].op_expr.negated);
    }

    #[test]
    fn test_string_filter_starts_with() {
        let filter = StringFilterInput {
            starts_with: Some("hello".to_string()),
            ..Default::default()
        };

        let filters = filter.to_filters("name");
        assert_eq!(filters.len(), 1);

        match &filters[0].op_expr.operation {
            Operation::Quant { op, value, .. } => {
                assert_eq!(*op, QuantOperator::Like);
                assert_eq!(value, "hello%");
            }
            _ => panic!("Expected Quant operation"),
        }
    }

    #[test]
    fn test_string_filter_ends_with() {
        let filter = StringFilterInput {
            ends_with: Some("world".to_string()),
            ..Default::default()
        };

        let filters = filter.to_filters("name");
        assert_eq!(filters.len(), 1);

        match &filters[0].op_expr.operation {
            Operation::Quant { op, value, .. } => {
                assert_eq!(*op, QuantOperator::Like);
                assert_eq!(value, "%world");
            }
            _ => panic!("Expected Quant operation"),
        }
    }

    #[test]
    fn test_string_filter_contains() {
        let filter = StringFilterInput {
            contains: Some("foo".to_string()),
            ..Default::default()
        };

        let filters = filter.to_filters("name");
        assert_eq!(filters.len(), 1);

        match &filters[0].op_expr.operation {
            Operation::Quant { op, value, .. } => {
                assert_eq!(*op, QuantOperator::Like);
                assert_eq!(value, "%foo%");
            }
            _ => panic!("Expected Quant operation"),
        }
    }

    #[test]
    fn test_string_filter_multiple() {
        let filter = StringFilterInput {
            eq: Some("test".to_string()),
            starts_with: Some("t".to_string()),
            ..Default::default()
        };

        let filters = filter.to_filters("name");
        assert_eq!(filters.len(), 2);
    }

    #[test]
    fn test_string_filter_is_empty() {
        let filter = StringFilterInput::default();
        assert!(filter.is_empty());

        let filter = StringFilterInput {
            eq: Some("test".to_string()),
            ..Default::default()
        };
        assert!(!filter.is_empty());
    }

    // ============================================================================
    // IntFilterInput Tests
    // ============================================================================

    #[test]
    fn test_int_filter_eq() {
        let filter = IntFilterInput {
            eq: Some(42),
            ..Default::default()
        };

        let filters = filter.to_filters("age");
        assert_eq!(filters.len(), 1);

        match &filters[0].op_expr.operation {
            Operation::Quant { op, value, .. } => {
                assert_eq!(*op, QuantOperator::Equal);
                assert_eq!(value, "42");
            }
            _ => panic!("Expected Quant operation"),
        }
    }

    #[test]
    fn test_int_filter_neq() {
        let filter = IntFilterInput {
            neq: Some(42),
            ..Default::default()
        };

        let filters = filter.to_filters("age");
        assert_eq!(filters.len(), 1);
        assert!(filters[0].op_expr.negated);
    }

    #[test]
    fn test_int_filter_gt() {
        let filter = IntFilterInput {
            gt: Some(18),
            ..Default::default()
        };

        let filters = filter.to_filters("age");
        assert_eq!(filters.len(), 1);

        match &filters[0].op_expr.operation {
            Operation::Quant { op, value, .. } => {
                assert_eq!(*op, QuantOperator::GreaterThan);
                assert_eq!(value, "18");
            }
            _ => panic!("Expected Quant operation"),
        }
    }

    #[test]
    fn test_int_filter_gte() {
        let filter = IntFilterInput {
            gte: Some(18),
            ..Default::default()
        };

        let filters = filter.to_filters("age");
        assert_eq!(filters.len(), 1);

        match &filters[0].op_expr.operation {
            Operation::Quant { op, value, .. } => {
                assert_eq!(*op, QuantOperator::GreaterThanEqual);
                assert_eq!(value, "18");
            }
            _ => panic!("Expected Quant operation"),
        }
    }

    #[test]
    fn test_int_filter_lt() {
        let filter = IntFilterInput {
            lt: Some(65),
            ..Default::default()
        };

        let filters = filter.to_filters("age");
        assert_eq!(filters.len(), 1);

        match &filters[0].op_expr.operation {
            Operation::Quant { op, value, .. } => {
                assert_eq!(*op, QuantOperator::LessThan);
                assert_eq!(value, "65");
            }
            _ => panic!("Expected Quant operation"),
        }
    }

    #[test]
    fn test_int_filter_lte() {
        let filter = IntFilterInput {
            lte: Some(65),
            ..Default::default()
        };

        let filters = filter.to_filters("age");
        assert_eq!(filters.len(), 1);

        match &filters[0].op_expr.operation {
            Operation::Quant { op, value, .. } => {
                assert_eq!(*op, QuantOperator::LessThanEqual);
                assert_eq!(value, "65");
            }
            _ => panic!("Expected Quant operation"),
        }
    }

    #[test]
    fn test_int_filter_in() {
        let filter = IntFilterInput {
            in_list: Some(vec![1, 2, 3]),
            ..Default::default()
        };

        let filters = filter.to_filters("id");
        assert_eq!(filters.len(), 1);

        match &filters[0].op_expr.operation {
            Operation::In(values) => {
                assert_eq!(values.len(), 3);
                assert_eq!(values[0], "1");
                assert_eq!(values[1], "2");
                assert_eq!(values[2], "3");
            }
            _ => panic!("Expected In operation"),
        }
    }

    #[test]
    fn test_int_filter_range() {
        let filter = IntFilterInput {
            gte: Some(18),
            lte: Some(65),
            ..Default::default()
        };

        let filters = filter.to_filters("age");
        assert_eq!(filters.len(), 2);
    }

    #[test]
    fn test_int_filter_is_empty() {
        let filter = IntFilterInput::default();
        assert!(filter.is_empty());

        let filter = IntFilterInput {
            eq: Some(42),
            ..Default::default()
        };
        assert!(!filter.is_empty());
    }

    // ============================================================================
    // BooleanFilterInput Tests
    // ============================================================================

    #[test]
    fn test_boolean_filter_eq_true() {
        let filter = BooleanFilterInput {
            eq: Some(true),
            ..Default::default()
        };

        let filters = filter.to_filters("active");
        assert_eq!(filters.len(), 1);

        match &filters[0].op_expr.operation {
            Operation::Is(postrust_core::api_request::IsValue::True) => {}
            _ => panic!("Expected Is True operation"),
        }
    }

    #[test]
    fn test_boolean_filter_eq_false() {
        let filter = BooleanFilterInput {
            eq: Some(false),
            ..Default::default()
        };

        let filters = filter.to_filters("active");
        assert_eq!(filters.len(), 1);

        match &filters[0].op_expr.operation {
            Operation::Is(postrust_core::api_request::IsValue::False) => {}
            _ => panic!("Expected Is False operation"),
        }
    }

    #[test]
    fn test_boolean_filter_is_empty() {
        let filter = BooleanFilterInput::default();
        assert!(filter.is_empty());

        let filter = BooleanFilterInput {
            eq: Some(true),
            ..Default::default()
        };
        assert!(!filter.is_empty());
    }

    // ============================================================================
    // FloatFilterInput Tests
    // ============================================================================

    #[test]
    fn test_float_filter_eq() {
        let filter = FloatFilterInput {
            eq: Some(3.14),
            ..Default::default()
        };

        let filters = filter.to_filters("price");
        assert_eq!(filters.len(), 1);

        match &filters[0].op_expr.operation {
            Operation::Quant { op, value, .. } => {
                assert_eq!(*op, QuantOperator::Equal);
                assert_eq!(value, "3.14");
            }
            _ => panic!("Expected Quant operation"),
        }
    }

    #[test]
    fn test_float_filter_gt() {
        let filter = FloatFilterInput {
            gt: Some(100.0),
            ..Default::default()
        };

        let filters = filter.to_filters("price");
        assert_eq!(filters.len(), 1);

        match &filters[0].op_expr.operation {
            Operation::Quant { op, value, .. } => {
                assert_eq!(*op, QuantOperator::GreaterThan);
                assert_eq!(value, "100");
            }
            _ => panic!("Expected Quant operation"),
        }
    }

    #[test]
    fn test_float_filter_is_empty() {
        let filter = FloatFilterInput::default();
        assert!(filter.is_empty());
    }

    // ============================================================================
    // UuidFilterInput Tests
    // ============================================================================

    #[test]
    fn test_uuid_filter_eq() {
        let filter = UuidFilterInput {
            eq: Some("550e8400-e29b-41d4-a716-446655440000".to_string()),
            ..Default::default()
        };

        let filters = filter.to_filters("id");
        assert_eq!(filters.len(), 1);

        match &filters[0].op_expr.operation {
            Operation::Quant { op, value, .. } => {
                assert_eq!(*op, QuantOperator::Equal);
                assert_eq!(value, "550e8400-e29b-41d4-a716-446655440000");
            }
            _ => panic!("Expected Quant operation"),
        }
    }

    #[test]
    fn test_uuid_filter_in() {
        let filter = UuidFilterInput {
            in_list: Some(vec![
                "550e8400-e29b-41d4-a716-446655440000".to_string(),
                "550e8400-e29b-41d4-a716-446655440001".to_string(),
            ]),
            ..Default::default()
        };

        let filters = filter.to_filters("id");
        assert_eq!(filters.len(), 1);

        match &filters[0].op_expr.operation {
            Operation::In(values) => {
                assert_eq!(values.len(), 2);
            }
            _ => panic!("Expected In operation"),
        }
    }

    #[test]
    fn test_uuid_filter_is_empty() {
        let filter = UuidFilterInput::default();
        assert!(filter.is_empty());
    }

    // ============================================================================
    // LogicTree Tests
    // ============================================================================

    #[test]
    fn test_filters_to_logic_tree_empty() {
        let tree = filters_to_logic_tree(vec![]);
        assert!(tree.is_none());
    }

    #[test]
    fn test_filters_to_logic_tree_single() {
        let filter = Filter::new(
            Field::simple("name"),
            OpExpr::new(Operation::Quant {
                op: QuantOperator::Equal,
                quantifier: None,
                value: "test".to_string(),
            }),
        );

        let tree = filters_to_logic_tree(vec![filter]).unwrap();
        match tree {
            LogicTree::Stmt(_) => {}
            _ => panic!("Expected Stmt for single filter"),
        }
    }

    #[test]
    fn test_filters_to_logic_tree_multiple() {
        let filter1 = Filter::new(
            Field::simple("name"),
            OpExpr::new(Operation::Quant {
                op: QuantOperator::Equal,
                quantifier: None,
                value: "test".to_string(),
            }),
        );
        let filter2 = Filter::new(
            Field::simple("age"),
            OpExpr::new(Operation::Quant {
                op: QuantOperator::GreaterThan,
                quantifier: None,
                value: "18".to_string(),
            }),
        );

        let tree = filters_to_logic_tree(vec![filter1, filter2]).unwrap();
        match tree {
            LogicTree::Expr { op, children, .. } => {
                assert_eq!(op, LogicOperator::And);
                assert_eq!(children.len(), 2);
            }
            _ => panic!("Expected Expr for multiple filters"),
        }
    }

    #[test]
    fn test_combine_with_and() {
        let filter1 = Filter::new(
            Field::simple("a"),
            OpExpr::new(Operation::Quant {
                op: QuantOperator::Equal,
                quantifier: None,
                value: "1".to_string(),
            }),
        );
        let filter2 = Filter::new(
            Field::simple("b"),
            OpExpr::new(Operation::Quant {
                op: QuantOperator::Equal,
                quantifier: None,
                value: "2".to_string(),
            }),
        );

        let tree1 = LogicTree::Stmt(filter1);
        let tree2 = LogicTree::Stmt(filter2);

        let combined = combine_with_and(vec![tree1, tree2]).unwrap();
        match combined {
            LogicTree::Expr { op, children, .. } => {
                assert_eq!(op, LogicOperator::And);
                assert_eq!(children.len(), 2);
            }
            _ => panic!("Expected Expr"),
        }
    }

    #[test]
    fn test_combine_with_or() {
        let filter1 = Filter::new(
            Field::simple("a"),
            OpExpr::new(Operation::Quant {
                op: QuantOperator::Equal,
                quantifier: None,
                value: "1".to_string(),
            }),
        );
        let filter2 = Filter::new(
            Field::simple("b"),
            OpExpr::new(Operation::Quant {
                op: QuantOperator::Equal,
                quantifier: None,
                value: "2".to_string(),
            }),
        );

        let tree1 = LogicTree::Stmt(filter1);
        let tree2 = LogicTree::Stmt(filter2);

        let combined = combine_with_or(vec![tree1, tree2]).unwrap();
        match combined {
            LogicTree::Expr { op, children, .. } => {
                assert_eq!(op, LogicOperator::Or);
                assert_eq!(children.len(), 2);
            }
            _ => panic!("Expected Expr"),
        }
    }

    #[test]
    fn test_negate_tree_stmt() {
        let filter = Filter::new(
            Field::simple("a"),
            OpExpr::new(Operation::Quant {
                op: QuantOperator::Equal,
                quantifier: None,
                value: "1".to_string(),
            }),
        );

        let tree = LogicTree::Stmt(filter);
        let negated = negate_tree(tree);

        match negated {
            LogicTree::Stmt(f) => {
                assert!(f.op_expr.negated);
            }
            _ => panic!("Expected Stmt"),
        }
    }

    #[test]
    fn test_negate_tree_expr() {
        let filter = Filter::new(
            Field::simple("a"),
            OpExpr::new(Operation::Quant {
                op: QuantOperator::Equal,
                quantifier: None,
                value: "1".to_string(),
            }),
        );

        let tree = LogicTree::Expr {
            negated: false,
            op: LogicOperator::And,
            children: vec![LogicTree::Stmt(filter)],
        };

        let negated = negate_tree(tree);

        match negated {
            LogicTree::Expr { negated, .. } => {
                assert!(negated);
            }
            _ => panic!("Expected Expr"),
        }
    }
}
