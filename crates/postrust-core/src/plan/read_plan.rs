//! Read (SELECT) query planning.

use super::types::*;
use crate::api_request::{ApiRequest, JoinType, QualifiedIdentifier, Range, SelectItem};
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
        let mut select = build_select_fields(&request.query_params.select, table)?;

        // A schema may declare how a value of one of its domains is written on
        // the wire -- a `color` as `"#01E240"` rather than as the integer it is
        // stored as. The cast it declared does the rendering, in the database,
        // which is also the only place that knows about it.
        for field in select.iter_mut() {
            if field.field.transform.is_some() || field.aggregate.is_some() {
                continue;
            }
            let Some(column) = table.get_column(&field.field.name) else {
                continue;
            };
            if let Some(function) =
                schema_cache.representation(column.representation_type(), "json")
            {
                field.field.transform = Some(function.to_string());
            }
        }

        // `Prefer: timezone` changes how a `timestamptz` reads, and only
        // PostgreSQL knows the session's zone -- this process would render
        // every one of them in UTC. Handing the rendering to the database is
        // what PostgREST does for every value; here it is done only where the
        // answer depends on it.
        if request.preferences.timezone.is_some() {
            for field in select.iter_mut() {
                if field.cast.is_none()
                    && field.aggregate.is_none()
                    && field.field.json_path.is_empty()
                    && matches!(
                        field.field.ir_type.as_str(),
                        "timestamptz"
                            | "timetz"
                            | "timestamp with time zone"
                            | "time with time zone"
                    )
                {
                    field.field.transform = Some("to_jsonb".to_string());
                }
            }
        }

        // Build where clauses from filters
        let mut where_clauses = build_where_clauses(request, table)?;
        for clause in where_clauses.iter_mut() {
            attach_representation_tree(clause, table, schema_cache);
        }

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
            range: resolve_top_level_range(request),
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

        // The rows read are the ones the mutation returned, not the table's.
        // The filters chose which rows to affect and have already done their
        // work; applying them again here would be harmless for an update and
        // wrong for a delete, whose rows are gone by then.
        plan.from = QualifiedIdentifier::unqualified(crate::query::MUTATION_RESULT);
        plan.from_alias = None;
        plan.where_clauses.clear();

        // A computed field is a function of the row it is read from, and the
        // rows here are the statement's result rather than the table's.
        for field in plan.select.iter_mut() {
            if let Some(computed) = field.field.computed.as_mut() {
                computed.relation = crate::query::MUTATION_RESULT.to_string();
            }
        }

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
fn build_select_fields(items: &[SelectItem], table: &Table) -> Result<Vec<CoercibleSelectField>> {
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
            // `*` stands for every column, and may sit alongside others.
            SelectItem::Field { field, .. } if field.name == "*" => {
                fields.extend(
                    table
                        .columns
                        .iter()
                        .map(|(name, col)| CoercibleSelectField::simple(name, &col.data_type)),
                );
            }
            SelectItem::Field {
                field,
                aggregate,
                aggregate_cast,
                cast,
                alias,
            } => {
                // A bare `count` means COUNT(*) -- PostgREST's original
                // spelling, still accepted and, unlike `count()`, not gated
                // behind db-aggregates-enabled. A table that really has a
                // column of that name wins, which is why this is decided here
                // rather than in the parser.
                let legacy_count = aggregate.is_none()
                    && field.name == "count"
                    && field.json_path.is_empty()
                    && table.get_column("count").is_none();

                // `count()` has no column behind it, so there is nothing to
                // resolve and no type to carry.
                // A name the table has no column for may still be a function
                // of its row type, which reads as a column.
                let computed = match table.get_column(&field.name) {
                    Some(_) => None,
                    // A computed column that takes anything besides the row
                    // is not callable here: this surface has nowhere to write
                    // an argument and no session document to supply.
                    None => table
                        .get_computed_column(&field.name)
                        .filter(|c| !c.takes_arguments),
                };

                let pg_type = if field.name.is_empty() || legacy_count {
                    String::new()
                } else if let Some(computed) = computed {
                    computed.data_type.clone()
                } else {
                    table
                        .get_column(&field.name)
                        .ok_or_else(|| Error::ColumnNotFound(field.name.clone()))?
                        .data_type
                        .clone()
                };

                let (field, aggregate) = if legacy_count {
                    (
                        &crate::api_request::Field::simple(""),
                        &Some(crate::api_request::AggregateFunction::Count),
                    )
                } else {
                    (field, aggregate)
                };

                let mut resolved = CoercibleField::from_field(field, &pg_type);
                resolved.computed = computed.map(|computed| crate::plan::ComputedRef {
                    function: computed.function.clone(),
                    relation: table.name.clone(),
                    row_type: table.qualified_identifier(),
                });

                fields.push(CoercibleSelectField {
                    field: resolved,
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

/// Resolve the effective top-level range.
///
/// The `Range` header supplies the base range; the `limit` and `offset` query
/// parameters take precedence over it when present, matching PostgREST. The
/// server-configured `max_rows` is then applied as a ceiling, so a request that
/// asks for no limit -- or for more rows than the server permits -- cannot pull
/// an unbounded result set into memory.
pub fn resolve_top_level_range(request: &ApiRequest) -> Range {
    let mut range = request.top_level_range.clone();

    if let Some(from_params) = request.query_params.ranges.get("") {
        if from_params.limit.is_some() {
            range.limit = from_params.limit;
        }
        if from_params.offset_explicit {
            range.offset = from_params.offset;
        }
    }

    if let Some(max_rows) = request.max_rows {
        range.limit = Some(match range.limit {
            Some(requested) => requested.min(max_rows),
            None => max_rows,
        });
    }

    range
}

/// Build where clauses from request filters.
fn build_where_clauses(request: &ApiRequest, table: &Table) -> Result<Vec<CoercibleLogicTree>> {
    // `nominal_type` (the underlying `udt_name`) is used rather than `data_type`
    // because it is always castable: `information_schema` reports arrays as
    // `ARRAY` and enums as `USER-DEFINED`, neither valid in a `::type` cast.
    let type_resolver = |name: &str| -> String {
        table
            .get_column(name)
            .map(|c| c.nominal_type.clone())
            .unwrap_or_else(|| "text".to_string())
    };

    // `?clients=is.null` names an embedded resource, not a column: it asks
    // whether the embed matched. That is decided where the embed exists, which
    // is above this query, so it is left out here. A real column of the same
    // name wins, since then the filter means what it usually means.
    let embedded: std::collections::HashSet<&str> = request
        .query_params
        .select
        .iter()
        .filter_map(|item| match item {
            SelectItem::Relation {
                relation, alias, ..
            } => Some(alias.as_deref().unwrap_or(relation)),
            SelectItem::SpreadRelation { relation, .. } => Some(relation.as_str()),
            SelectItem::Field { .. } => None,
        })
        .filter(|name| table.get_column(name).is_none())
        .collect();

    let mut clauses = Vec::new();

    // Add root filters
    for filter in &request.query_params.filters_root {
        if embedded.contains(filter.field.name.as_str()) {
            continue;
        }
        let pg_type = type_resolver(&filter.field.name);
        clauses.push(CoercibleLogicTree::Stmt(CoercibleFilter::from_filter(
            filter, &pg_type,
        )));
    }

    // Add logic trees.
    //
    // One made entirely of embedded resource names is about whether those
    // embeds matched, which nothing here can answer -- the embeds are not in
    // scope until they have been joined. It is applied out there instead.
    let embeds = crate::api_request::embedded_names(&request.query_params.select);
    for (path, tree) in &request.query_params.logic {
        if path.is_empty() && !tree.names_only(&embeds) {
            clauses.push(CoercibleLogicTree::from_logic_tree(tree, type_resolver));
        }
    }

    for clause in clauses.iter_mut() {
        attach_computed_tree(clause, table);
    }

    Ok(clauses)
}

/// Point a filter's value at the function that parses it.
///
/// The mirror of the output representation: where a schema declares a cast
/// from `text` to one of its domains, a filter value written in the domain's
/// own spelling is read by that cast rather than by PostgreSQL's input
/// function for the underlying type.
fn attach_representation(field: &mut CoercibleField, table: &Table, schema_cache: &SchemaCache) {
    let Some(column) = table.get_column(&field.name) else {
        return;
    };
    if let Some(function) = schema_cache.representation("text", column.representation_type()) {
        field.transform = Some(function.to_string());
    }
}

/// Walk a logic tree, giving every filter in it its input representation.
fn attach_representation_tree(
    tree: &mut CoercibleLogicTree,
    table: &Table,
    schema_cache: &SchemaCache,
) {
    match tree {
        CoercibleLogicTree::Expr { children, .. } => {
            for child in children {
                attach_representation_tree(child, table, schema_cache);
            }
        }
        CoercibleLogicTree::Stmt(filter) => {
            attach_representation(&mut filter.field, table, schema_cache)
        }
        CoercibleLogicTree::NullEmbed { .. } => {}
    }
}

/// Point a field at the function behind it, where the table has no such column.
///
/// A filter or an order term names a field the same way a select does, so a
/// computed field has to be recognised in all three -- `?always_true=is.true`
/// and `?order=anti_id.desc` are as much a part of PostgREST's contract as
/// `?select=always_true`.
fn attach_computed(field: &mut CoercibleField, table: &Table) {
    if table.get_column(&field.name).is_some() {
        return;
    }
    if let Some(computed) = table
        .get_computed_column(&field.name)
        .filter(|c| !c.takes_arguments)
    {
        field.ir_type = computed.data_type.clone();
        field.base_type = computed.data_type.clone();
        field.computed = Some(crate::plan::ComputedRef {
            function: computed.function.clone(),
            relation: table.name.clone(),
            row_type: table.qualified_identifier(),
        });
    }
}

/// Walk a logic tree, resolving every field it names.
fn attach_computed_tree(tree: &mut CoercibleLogicTree, table: &Table) {
    match tree {
        CoercibleLogicTree::Expr { children, .. } => {
            for child in children {
                attach_computed_tree(child, table);
            }
        }
        CoercibleLogicTree::Stmt(filter) => attach_computed(&mut filter.field, table),
        CoercibleLogicTree::NullEmbed { .. } => {}
    }
}

/// Build order terms from request.
fn build_order_terms(request: &ApiRequest, table: &Table) -> Result<Vec<CoercibleOrderTerm>> {
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

                let mut resolved = CoercibleOrderTerm::from_order_term(term, pg_type);
                attach_computed(&mut resolved.field, table);
                terms.push(resolved);
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
                hint,
                join_type,
                select: _,
            } => {
                // Verify relationship exists
                let _rel = schema_cache
                    .find_relationship(
                        &table.qualified_identifier(),
                        relation,
                        hint.as_deref(),
                        &table.schema,
                    )?
                    .ok_or_else(|| {
                        schema_cache.relationship_not_found(
                            &table.qualified_identifier(),
                            relation,
                            hint.as_deref(),
                            &table.schema,
                        )
                    })?;

                rel_selects.push(RelSelectField {
                    name: relation.clone(),
                    agg_alias: alias
                        .clone()
                        .unwrap_or_else(|| format!("pgrst_{}", relation)),
                    join_type: join_type.clone().unwrap_or_default(),
                    is_spread: false,
                });
            }
            SelectItem::SpreadRelation {
                relation,
                hint,
                join_type,
                select: _,
            } => {
                let _rel = schema_cache
                    .find_relationship(
                        &table.qualified_identifier(),
                        relation,
                        hint.as_deref(),
                        &table.schema,
                    )?
                    .ok_or_else(|| {
                        schema_cache.relationship_not_found(
                            &table.qualified_identifier(),
                            relation,
                            hint.as_deref(),
                            &table.schema,
                        )
                    })?;

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
