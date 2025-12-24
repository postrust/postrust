//! Query resolvers for GraphQL table queries.
//!
//! Converts GraphQL query arguments into ReadPlan structures that can be executed.

use crate::input::filter::{
    combine_with_and, filters_to_logic_tree, BooleanFilterInput, FloatFilterInput, IntFilterInput,
    StringFilterInput, UuidFilterInput,
};
use crate::input::order::{OrderByField, PaginationInput};
use postrust_core::api_request::{Filter, LogicTree, Range};
use postrust_core::plan::{CoercibleLogicTree, CoercibleOrderTerm, CoercibleSelectField, ReadPlan};
use postrust_core::schema_cache::Table;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Arguments for a GraphQL table query.
#[derive(Debug, Clone, Default)]
pub struct QueryArgs {
    /// Fields to select (column names)
    pub select: Vec<String>,
    /// Filter conditions
    pub filter: Option<TableFilter>,
    /// Order by specifications
    pub order_by: Vec<OrderByField>,
    /// Limit number of results
    pub limit: Option<i64>,
    /// Offset into results
    pub offset: Option<i64>,
    /// Nested relation queries
    pub relations: HashMap<String, QueryArgs>,
}

impl QueryArgs {
    /// Create empty query args.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the select fields.
    pub fn with_select(mut self, fields: Vec<String>) -> Self {
        self.select = fields;
        self
    }

    /// Set the filter.
    pub fn with_filter(mut self, filter: TableFilter) -> Self {
        self.filter = Some(filter);
        self
    }

    /// Set order by.
    pub fn with_order_by(mut self, order_by: Vec<OrderByField>) -> Self {
        self.order_by = order_by;
        self
    }

    /// Set pagination.
    pub fn with_pagination(mut self, pagination: PaginationInput) -> Self {
        self.limit = pagination.limit;
        self.offset = pagination.offset;
        self
    }

    /// Set limit.
    pub fn with_limit(mut self, limit: i64) -> Self {
        self.limit = Some(limit);
        self
    }

    /// Set offset.
    pub fn with_offset(mut self, offset: i64) -> Self {
        self.offset = Some(offset);
        self
    }

    /// Add a nested relation query.
    pub fn with_relation(mut self, name: String, args: QueryArgs) -> Self {
        self.relations.insert(name, args);
        self
    }

    /// Get the pagination range.
    pub fn to_range(&self) -> Range {
        Range {
            offset: self.offset.unwrap_or(0),
            limit: self.limit,
        }
    }

    /// Check if any select fields are specified.
    pub fn has_select(&self) -> bool {
        !self.select.is_empty()
    }

    /// Check if any filters are specified.
    pub fn has_filter(&self) -> bool {
        self.filter.is_some()
    }

    /// Check if any ordering is specified.
    pub fn has_order_by(&self) -> bool {
        !self.order_by.is_empty()
    }

    /// Check if pagination is specified.
    pub fn has_pagination(&self) -> bool {
        self.limit.is_some() || self.offset.is_some()
    }
}

/// A dynamic filter for any table field.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TableFilter {
    /// Field-specific filters
    #[serde(flatten)]
    pub fields: HashMap<String, FieldFilter>,
    /// AND combined filters
    #[serde(rename = "_and")]
    pub and: Option<Vec<TableFilter>>,
    /// OR combined filters
    #[serde(rename = "_or")]
    pub or: Option<Vec<TableFilter>>,
    /// Negated filter
    #[serde(rename = "_not")]
    pub not: Option<Box<TableFilter>>,
}

impl TableFilter {
    /// Create an empty filter.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a field filter.
    pub fn with_field(mut self, name: impl Into<String>, filter: FieldFilter) -> Self {
        self.fields.insert(name.into(), filter);
        self
    }

    /// Add AND filters.
    pub fn with_and(mut self, filters: Vec<TableFilter>) -> Self {
        self.and = Some(filters);
        self
    }

    /// Add OR filters.
    pub fn with_or(mut self, filters: Vec<TableFilter>) -> Self {
        self.or = Some(filters);
        self
    }

    /// Add NOT filter.
    pub fn with_not(mut self, filter: TableFilter) -> Self {
        self.not = Some(Box::new(filter));
        self
    }

    /// Check if filter is empty.
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty() && self.and.is_none() && self.or.is_none() && self.not.is_none()
    }

    /// Convert to a LogicTree.
    pub fn to_logic_tree(&self) -> Option<LogicTree> {
        let mut trees = Vec::new();

        // Add field filters
        for (field_name, field_filter) in &self.fields {
            let filters = field_filter.to_filters(field_name);
            if let Some(tree) = filters_to_logic_tree(filters) {
                trees.push(tree);
            }
        }

        // Add AND filters
        if let Some(and_filters) = &self.and {
            let and_trees: Vec<LogicTree> = and_filters
                .iter()
                .filter_map(|f| f.to_logic_tree())
                .collect();
            if let Some(tree) = combine_with_and(and_trees) {
                trees.push(tree);
            }
        }

        // Add OR filters
        if let Some(or_filters) = &self.or {
            let or_trees: Vec<LogicTree> = or_filters
                .iter()
                .filter_map(|f| f.to_logic_tree())
                .collect();
            if !or_trees.is_empty() {
                trees.push(LogicTree::or(or_trees));
            }
        }

        // Add NOT filter
        if let Some(not_filter) = &self.not {
            if let Some(tree) = not_filter.to_logic_tree() {
                let negated = match tree {
                    LogicTree::Expr { op, children, .. } => LogicTree::Expr {
                        negated: true,
                        op,
                        children,
                    },
                    LogicTree::Stmt(filter) => {
                        let negated_expr = postrust_core::api_request::OpExpr {
                            negated: !filter.op_expr.negated,
                            operation: filter.op_expr.operation,
                        };
                        LogicTree::Stmt(Filter::new(filter.field, negated_expr))
                    }
                };
                trees.push(negated);
            }
        }

        combine_with_and(trees)
    }
}

/// A filter for a single field that can handle different types.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FieldFilter {
    /// String filter operations
    String(StringFilterInput),
    /// Integer filter operations
    Int(IntFilterInput),
    /// Float filter operations
    Float(FloatFilterInput),
    /// Boolean filter operations
    Boolean(BooleanFilterInput),
    /// UUID filter operations
    Uuid(UuidFilterInput),
}

impl FieldFilter {
    /// Create a string filter.
    pub fn string(filter: StringFilterInput) -> Self {
        Self::String(filter)
    }

    /// Create an int filter.
    pub fn int(filter: IntFilterInput) -> Self {
        Self::Int(filter)
    }

    /// Create a float filter.
    pub fn float(filter: FloatFilterInput) -> Self {
        Self::Float(filter)
    }

    /// Create a boolean filter.
    pub fn boolean(filter: BooleanFilterInput) -> Self {
        Self::Boolean(filter)
    }

    /// Create a UUID filter.
    pub fn uuid(filter: UuidFilterInput) -> Self {
        Self::Uuid(filter)
    }

    /// Convert to a list of Filters.
    pub fn to_filters(&self, field_name: &str) -> Vec<Filter> {
        match self {
            Self::String(f) => f.to_filters(field_name),
            Self::Int(f) => f.to_filters(field_name),
            Self::Float(f) => f.to_filters(field_name),
            Self::Boolean(f) => f.to_filters(field_name),
            Self::Uuid(f) => f.to_filters(field_name),
        }
    }
}

/// Build select fields from column names.
pub fn build_select_fields(columns: &[String], table: &Table) -> Vec<CoercibleSelectField> {
    if columns.is_empty() {
        // Default: select all columns
        return table
            .columns
            .iter()
            .map(|(name, col)| CoercibleSelectField::simple(name, &col.data_type))
            .collect();
    }

    columns
        .iter()
        .filter_map(|name| {
            table
                .columns
                .get(name)
                .map(|col| CoercibleSelectField::simple(name, &col.data_type))
        })
        .collect()
}

/// Build order terms from OrderByFields.
pub fn build_order_terms(order_by: &[OrderByField], table: &Table) -> Vec<CoercibleOrderTerm> {
    order_by
        .iter()
        .filter_map(|ob| {
            table.columns.get(&ob.field).map(|col| {
                let order_term = ob.to_order_term();
                CoercibleOrderTerm::from_order_term(&order_term, &col.data_type)
            })
        })
        .collect()
}

/// Build where clauses from a TableFilter.
pub fn build_where_clauses(filter: &Option<TableFilter>, table: &Table) -> Vec<CoercibleLogicTree> {
    let Some(filter) = filter else {
        return vec![];
    };

    let type_resolver = |name: &str| -> String {
        table
            .get_column(name)
            .map(|c| c.data_type.clone())
            .unwrap_or_else(|| "text".to_string())
    };

    filter
        .to_logic_tree()
        .map(|tree| vec![CoercibleLogicTree::from_logic_tree(&tree, type_resolver)])
        .unwrap_or_default()
}

/// Build a ReadPlan from GraphQL query arguments.
pub fn build_read_plan(args: &QueryArgs, table: &Table) -> ReadPlan {
    let select = build_select_fields(&args.select, table);
    let order = build_order_terms(&args.order_by, table);
    let where_clauses = build_where_clauses(&args.filter, table);

    ReadPlan {
        select,
        from: table.qualified_identifier(),
        from_alias: None,
        where_clauses,
        order,
        range: args.to_range(),
        rel_name: table.name.clone(),
        rel_to_parent: None,
        rel_join_conds: vec![],
        rel_join_type: None,
        rel_select: vec![],
        depth: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::filter::IntFilterInput;
    use indexmap::IndexMap;
    use postrust_core::schema_cache::Column;
    use pretty_assertions::assert_eq;

    fn create_test_table() -> Table {
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
            "age".into(),
            Column {
                name: "age".into(),
                description: None,
                nullable: true,
                data_type: "integer".into(),
                nominal_type: "int4".into(),
                max_len: None,
                default: None,
                enum_values: vec![],
                is_pk: false,
                position: 4,
            },
        );

        Table {
            schema: "public".into(),
            name: "users".into(),
            description: None,
            is_view: false,
            insertable: true,
            updatable: true,
            deletable: true,
            pk_cols: vec!["id".into()],
            columns,
        }
    }

    // ============================================================================
    // QueryArgs Tests
    // ============================================================================

    #[test]
    fn test_query_args_default() {
        let args = QueryArgs::new();
        assert!(!args.has_select());
        assert!(!args.has_filter());
        assert!(!args.has_order_by());
        assert!(!args.has_pagination());
    }

    #[test]
    fn test_query_args_with_select() {
        let args = QueryArgs::new().with_select(vec!["id".to_string(), "name".to_string()]);
        assert!(args.has_select());
        assert_eq!(args.select.len(), 2);
    }

    #[test]
    fn test_query_args_with_filter() {
        let filter = TableFilter::new().with_field(
            "name",
            FieldFilter::string(StringFilterInput {
                eq: Some("test".to_string()),
                ..Default::default()
            }),
        );
        let args = QueryArgs::new().with_filter(filter);
        assert!(args.has_filter());
    }

    #[test]
    fn test_query_args_with_order_by() {
        let args = QueryArgs::new().with_order_by(vec![OrderByField::desc("created_at")]);
        assert!(args.has_order_by());
        assert_eq!(args.order_by.len(), 1);
    }

    #[test]
    fn test_query_args_with_pagination() {
        let args = QueryArgs::new().with_limit(10).with_offset(20);
        assert!(args.has_pagination());
        assert_eq!(args.limit, Some(10));
        assert_eq!(args.offset, Some(20));
    }

    #[test]
    fn test_query_args_to_range() {
        let args = QueryArgs::new().with_limit(10).with_offset(5);
        let range = args.to_range();
        assert_eq!(range.offset, 5);
        assert_eq!(range.limit, Some(10));
    }

    #[test]
    fn test_query_args_with_relation() {
        let child_args = QueryArgs::new().with_limit(5);
        let args = QueryArgs::new().with_relation("orders".to_string(), child_args);
        assert!(args.relations.contains_key("orders"));
    }

    // ============================================================================
    // TableFilter Tests
    // ============================================================================

    #[test]
    fn test_table_filter_empty() {
        let filter = TableFilter::new();
        assert!(filter.is_empty());
        assert!(filter.to_logic_tree().is_none());
    }

    #[test]
    fn test_table_filter_single_field() {
        let filter = TableFilter::new().with_field(
            "name",
            FieldFilter::string(StringFilterInput {
                eq: Some("Alice".to_string()),
                ..Default::default()
            }),
        );
        assert!(!filter.is_empty());
        assert!(filter.to_logic_tree().is_some());
    }

    #[test]
    fn test_table_filter_multiple_fields() {
        let filter = TableFilter::new()
            .with_field(
                "name",
                FieldFilter::string(StringFilterInput {
                    eq: Some("Alice".to_string()),
                    ..Default::default()
                }),
            )
            .with_field(
                "age",
                FieldFilter::int(IntFilterInput {
                    gt: Some(18),
                    ..Default::default()
                }),
            );

        let tree = filter.to_logic_tree().unwrap();
        match tree {
            LogicTree::Expr { children, .. } => {
                assert_eq!(children.len(), 2);
            }
            _ => panic!("Expected Expr for multiple fields"),
        }
    }

    #[test]
    fn test_table_filter_with_and() {
        let filter1 = TableFilter::new().with_field(
            "a",
            FieldFilter::int(IntFilterInput {
                eq: Some(1),
                ..Default::default()
            }),
        );
        let filter2 = TableFilter::new().with_field(
            "b",
            FieldFilter::int(IntFilterInput {
                eq: Some(2),
                ..Default::default()
            }),
        );

        let combined = TableFilter::new().with_and(vec![filter1, filter2]);
        let tree = combined.to_logic_tree().unwrap();
        assert!(matches!(tree, LogicTree::Expr { .. }));
    }

    #[test]
    fn test_table_filter_with_or() {
        let filter1 = TableFilter::new().with_field(
            "status",
            FieldFilter::string(StringFilterInput {
                eq: Some("active".to_string()),
                ..Default::default()
            }),
        );
        let filter2 = TableFilter::new().with_field(
            "status",
            FieldFilter::string(StringFilterInput {
                eq: Some("pending".to_string()),
                ..Default::default()
            }),
        );

        let combined = TableFilter::new().with_or(vec![filter1, filter2]);
        let tree = combined.to_logic_tree().unwrap();

        match tree {
            LogicTree::Expr {
                op: postrust_core::api_request::LogicOperator::Or,
                ..
            } => {}
            _ => panic!("Expected OR expression"),
        }
    }

    #[test]
    fn test_table_filter_with_not() {
        let inner = TableFilter::new().with_field(
            "deleted",
            FieldFilter::boolean(BooleanFilterInput {
                eq: Some(true),
                ..Default::default()
            }),
        );

        let filter = TableFilter::new().with_not(inner);
        let tree = filter.to_logic_tree().unwrap();

        match tree {
            LogicTree::Expr { negated: true, .. } | LogicTree::Stmt(_) => {}
            _ => panic!("Expected negated expression"),
        }
    }

    // ============================================================================
    // FieldFilter Tests
    // ============================================================================

    #[test]
    fn test_field_filter_string() {
        let filter = FieldFilter::string(StringFilterInput {
            eq: Some("test".to_string()),
            ..Default::default()
        });
        let filters = filter.to_filters("name");
        assert_eq!(filters.len(), 1);
    }

    #[test]
    fn test_field_filter_int() {
        let filter = FieldFilter::int(IntFilterInput {
            gte: Some(18),
            lte: Some(65),
            ..Default::default()
        });
        let filters = filter.to_filters("age");
        assert_eq!(filters.len(), 2); // gte and lte
    }

    #[test]
    fn test_field_filter_boolean() {
        let filter = FieldFilter::boolean(BooleanFilterInput {
            eq: Some(true),
            ..Default::default()
        });
        let filters = filter.to_filters("active");
        assert_eq!(filters.len(), 1);
    }

    // ============================================================================
    // Build Functions Tests
    // ============================================================================

    #[test]
    fn test_build_select_fields_empty() {
        let table = create_test_table();
        let fields = build_select_fields(&[], &table);
        assert_eq!(fields.len(), 4); // All columns
    }

    #[test]
    fn test_build_select_fields_specific() {
        let table = create_test_table();
        let fields = build_select_fields(&["id".to_string(), "name".to_string()], &table);
        assert_eq!(fields.len(), 2);
    }

    #[test]
    fn test_build_select_fields_invalid_column() {
        let table = create_test_table();
        let fields = build_select_fields(&["nonexistent".to_string()], &table);
        assert_eq!(fields.len(), 0); // Invalid columns are skipped
    }

    #[test]
    fn test_build_order_terms() {
        let table = create_test_table();
        let order_by = vec![OrderByField::desc("name"), OrderByField::asc("id")];
        let terms = build_order_terms(&order_by, &table);
        assert_eq!(terms.len(), 2);
    }

    #[test]
    fn test_build_order_terms_invalid_column() {
        let table = create_test_table();
        let order_by = vec![OrderByField::desc("nonexistent")];
        let terms = build_order_terms(&order_by, &table);
        assert_eq!(terms.len(), 0); // Invalid columns are skipped
    }

    #[test]
    fn test_build_where_clauses_none() {
        let table = create_test_table();
        let clauses = build_where_clauses(&None, &table);
        assert!(clauses.is_empty());
    }

    #[test]
    fn test_build_where_clauses_with_filter() {
        let table = create_test_table();
        let filter = TableFilter::new().with_field(
            "name",
            FieldFilter::string(StringFilterInput {
                eq: Some("test".to_string()),
                ..Default::default()
            }),
        );
        let clauses = build_where_clauses(&Some(filter), &table);
        assert_eq!(clauses.len(), 1);
    }

    // ============================================================================
    // ReadPlan Building Tests
    // ============================================================================

    #[test]
    fn test_build_read_plan_basic() {
        let table = create_test_table();
        let args = QueryArgs::new();
        let plan = build_read_plan(&args, &table);

        assert_eq!(plan.from.name, "users");
        assert_eq!(plan.select.len(), 4); // All columns
        assert!(plan.where_clauses.is_empty());
        assert!(plan.order.is_empty());
    }

    #[test]
    fn test_build_read_plan_with_select() {
        let table = create_test_table();
        let args = QueryArgs::new().with_select(vec!["id".to_string(), "name".to_string()]);
        let plan = build_read_plan(&args, &table);

        assert_eq!(plan.select.len(), 2);
    }

    #[test]
    fn test_build_read_plan_with_filter() {
        let table = create_test_table();
        let filter = TableFilter::new().with_field(
            "age",
            FieldFilter::int(IntFilterInput {
                gte: Some(18),
                ..Default::default()
            }),
        );
        let args = QueryArgs::new().with_filter(filter);
        let plan = build_read_plan(&args, &table);

        assert!(!plan.where_clauses.is_empty());
    }

    #[test]
    fn test_build_read_plan_with_order() {
        let table = create_test_table();
        let args = QueryArgs::new().with_order_by(vec![OrderByField::desc("name")]);
        let plan = build_read_plan(&args, &table);

        assert_eq!(plan.order.len(), 1);
    }

    #[test]
    fn test_build_read_plan_with_pagination() {
        let table = create_test_table();
        let args = QueryArgs::new().with_limit(10).with_offset(20);
        let plan = build_read_plan(&args, &table);

        assert_eq!(plan.range.limit, Some(10));
        assert_eq!(plan.range.offset, 20);
    }

    #[test]
    fn test_build_read_plan_full() {
        let table = create_test_table();
        let filter = TableFilter::new()
            .with_field(
                "name",
                FieldFilter::string(StringFilterInput {
                    like: Some("%John%".to_string()),
                    ..Default::default()
                }),
            )
            .with_field(
                "age",
                FieldFilter::int(IntFilterInput {
                    gte: Some(21),
                    ..Default::default()
                }),
            );

        let args = QueryArgs::new()
            .with_select(vec!["id".to_string(), "name".to_string(), "email".to_string()])
            .with_filter(filter)
            .with_order_by(vec![OrderByField::asc("name")])
            .with_limit(50)
            .with_offset(0);

        let plan = build_read_plan(&args, &table);

        assert_eq!(plan.select.len(), 3);
        assert!(!plan.where_clauses.is_empty());
        assert_eq!(plan.order.len(), 1);
        assert_eq!(plan.range.limit, Some(50));
        assert_eq!(plan.range.offset, 0);
    }
}
