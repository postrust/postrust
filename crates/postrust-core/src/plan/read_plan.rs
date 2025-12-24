//! Read (SELECT) query planning.

use super::types::*;
use crate::api_request::{
    ApiRequest, JoinType, QualifiedIdentifier, Range, SelectItem,
};
use crate::error::{Error, Result};
use crate::schema_cache::{Relationship, SchemaCache, Table};
use serde::{Deserialize, Serialize};

/// A read plan for a single table/view.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReadPlan {
    /// Columns to select
    pub select: Vec<CoercibleSelectField>,
    /// Source table
    pub from: QualifiedIdentifier,
    /// Table alias
    pub from_alias: Option<String>,
    /// WHERE conditions
    pub where_clauses: Vec<CoercibleLogicTree>,
    /// ORDER BY terms
    pub order: Vec<CoercibleOrderTerm>,
    /// Pagination range
    pub range: Range,
    /// Relation name (for embedding)
    pub rel_name: String,
    /// Relationship to parent (if embedded)
    pub rel_to_parent: Option<Relationship>,
    /// Join conditions
    pub rel_join_conds: Vec<JoinCondition>,
    /// Join type
    pub rel_join_type: Option<JoinType>,
    /// Embedded relations to select
    pub rel_select: Vec<RelSelectField>,
    /// Nesting depth
    pub depth: u32,
}

impl ReadPlan {
    /// Create a read plan from an API request.
    pub fn from_request(
        request: &ApiRequest,
        table: &Table,
        schema_cache: &SchemaCache,
    ) -> Result<Self> {
        let qi = table.qualified_identifier();

        // Build select fields
        let select = build_select_fields(&request.query_params.select, table)?;

        // Build where clauses from filters
        let where_clauses = build_where_clauses(request, table)?;

        // Build order terms
        let order = build_order_terms(request, table)?;

        // Build relation selects for embedding
        let rel_select = build_relation_selects(&request.query_params.select, table, schema_cache)?;

        Ok(Self {
            select,
            from: qi,
            from_alias: None,
            where_clauses,
            order,
            range: request.top_level_range.clone(),
            rel_name: table.name.clone(),
            rel_to_parent: None,
            rel_join_conds: vec![],
            rel_join_type: None,
            rel_select,
            depth: 0,
        })
    }

    /// Create a read plan for returning mutation results.
    pub fn for_mutation(
        request: &ApiRequest,
        table: &Table,
        schema_cache: &SchemaCache,
    ) -> Result<Self> {
        let mut plan = Self::from_request(request, table, schema_cache)?;
        // For mutations, we select from the CTE result
        plan.from_alias = Some("pgrst_mutation_result".to_string());
        Ok(plan)
    }

    /// Check if this plan has any where clauses.
    pub fn has_where(&self) -> bool {
        !self.where_clauses.is_empty()
    }

    /// Check if this plan has any order terms.
    pub fn has_order(&self) -> bool {
        !self.order.is_empty()
    }

    /// Check if this plan has pagination.
    pub fn has_pagination(&self) -> bool {
        self.range.limit.is_some() || self.range.offset > 0
    }
}

/// Build select fields from select items.
fn build_select_fields(
    items: &[SelectItem],
    table: &Table,
) -> Result<Vec<CoercibleSelectField>> {
    if items.is_empty() {
        // Default: select all columns
        return Ok(table
            .columns
            .iter()
            .map(|(name, col)| CoercibleSelectField::simple(name, &col.data_type))
            .collect());
    }

    let mut fields = Vec::new();

    for item in items {
        match item {
            SelectItem::Field {
                field,
                aggregate,
                aggregate_cast,
                cast,
                alias,
            } => {
                let column = table
                    .get_column(&field.name)
                    .ok_or_else(|| Error::ColumnNotFound(field.name.clone()))?;

                fields.push(CoercibleSelectField {
                    field: CoercibleField::from_field(field, &column.data_type),
                    aggregate: aggregate.clone(),
                    aggregate_cast: aggregate_cast.clone(),
                    cast: cast.clone(),
                    alias: alias.clone(),
                });
            }
            // Relations are handled separately
            SelectItem::Relation { .. } | SelectItem::SpreadRelation { .. } => {}
        }
    }

    Ok(fields)
}

/// Build where clauses from request filters.
fn build_where_clauses(
    request: &ApiRequest,
    table: &Table,
) -> Result<Vec<CoercibleLogicTree>> {
    let type_resolver = |name: &str| -> String {
        table
            .get_column(name)
            .map(|c| c.data_type.clone())
            .unwrap_or_else(|| "text".to_string())
    };

    let mut clauses = Vec::new();

    // Add root filters
    for filter in &request.query_params.filters_root {
        let pg_type = type_resolver(&filter.field.name);
        clauses.push(CoercibleLogicTree::Stmt(CoercibleFilter::from_filter(
            filter, &pg_type,
        )));
    }

    // Add logic trees
    for (path, tree) in &request.query_params.logic {
        if path.is_empty() {
            clauses.push(CoercibleLogicTree::from_logic_tree(tree, type_resolver));
        }
    }

    Ok(clauses)
}

/// Build order terms from request.
fn build_order_terms(
    request: &ApiRequest,
    table: &Table,
) -> Result<Vec<CoercibleOrderTerm>> {
    let mut terms = Vec::new();

    for (path, order_terms) in &request.query_params.order {
        if path.is_empty() {
            for term in order_terms {
                let field_name = match term {
                    crate::api_request::OrderTerm::Field { field, .. } => &field.name,
                    crate::api_request::OrderTerm::Relation { field, .. } => &field.name,
                };

                let pg_type = table
                    .get_column(field_name)
                    .map(|c| c.data_type.as_str())
                    .unwrap_or("text");

                terms.push(CoercibleOrderTerm::from_order_term(term, pg_type));
            }
        }
    }

    Ok(terms)
}

/// Build relation select fields for embedding.
fn build_relation_selects(
    items: &[SelectItem],
    table: &Table,
    schema_cache: &SchemaCache,
) -> Result<Vec<RelSelectField>> {
    let mut rel_selects = Vec::new();

    for item in items {
        match item {
            SelectItem::Relation {
                relation,
                alias,
                hint: _,
                join_type,
            } => {
                // Verify relationship exists
                let _rel = schema_cache
                    .find_relationship(&table.qualified_identifier(), relation, &table.schema)
                    .ok_or_else(|| Error::RelationshipNotFound(relation.clone()))?;

                rel_selects.push(RelSelectField {
                    name: relation.clone(),
                    agg_alias: alias.clone().unwrap_or_else(|| format!("pgrst_{}", relation)),
                    join_type: join_type.clone().unwrap_or_default(),
                    is_spread: false,
                });
            }
            SelectItem::SpreadRelation {
                relation,
                hint: _,
                join_type,
            } => {
                let _rel = schema_cache
                    .find_relationship(&table.qualified_identifier(), relation, &table.schema)
                    .ok_or_else(|| Error::RelationshipNotFound(relation.clone()))?;

                rel_selects.push(RelSelectField {
                    name: relation.clone(),
                    agg_alias: format!("pgrst_spread_{}", relation),
                    join_type: join_type.clone().unwrap_or_default(),
                    is_spread: true,
                });
            }
            _ => {}
        }
    }

    Ok(rel_selects)
}

/// A tree of read plans (for nested embedding).
#[derive(Clone, Debug)]
pub struct ReadPlanTree {
    /// Root plan
    pub root: ReadPlan,
    /// Child plans (embedded resources)
    pub children: Vec<ReadPlanTree>,
}

impl ReadPlanTree {
    /// Create an empty tree.
    pub fn empty() -> Self {
        Self {
            root: ReadPlan {
                select: vec![],
                from: QualifiedIdentifier::unqualified(""),
                from_alias: None,
                where_clauses: vec![],
                order: vec![],
                range: Range::default(),
                rel_name: String::new(),
                rel_to_parent: None,
                rel_join_conds: vec![],
                rel_join_type: None,
                rel_select: vec![],
                depth: 0,
            },
            children: vec![],
        }
    }

    /// Create a leaf tree (no children).
    pub fn leaf(plan: ReadPlan) -> Self {
        Self {
            root: plan,
            children: vec![],
        }
    }

    /// Add a child tree.
    pub fn add_child(&mut self, child: ReadPlanTree) {
        self.children.push(child);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_read_plan_tree_empty() {
        let tree = ReadPlanTree::empty();
        assert!(tree.root.select.is_empty());
        assert!(tree.children.is_empty());
    }
}
