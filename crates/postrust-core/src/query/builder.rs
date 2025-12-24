//! Query builder implementation.

use crate::error::Result;
use crate::plan::{
    CallPlan, CallParams, CoercibleFilter, CoercibleLogicTree, CoercibleOrderTerm,
    CoercibleSelectField, MutatePlan, ReadPlan, ReadPlanTree,
};
use postrust_sql::{
    escape_ident, from_qi, DeleteBuilder, InsertBuilder, OrderExpr, SelectBuilder,
    SqlFragment, SqlParam, UpdateBuilder,
};

/// Query builder for converting plans to SQL.
pub struct QueryBuilder;

impl QueryBuilder {
    /// Build a SELECT query from a read plan tree.
    pub fn build_read(tree: &ReadPlanTree) -> Result<SqlFragment> {
        Self::build_read_plan(&tree.root)
    }

    /// Build a SELECT query from a read plan.
    fn build_read_plan(plan: &ReadPlan) -> Result<SqlFragment> {
        let mut builder = SelectBuilder::new();

        // FROM clause
        let qi = &plan.from;
        if let Some(alias) = &plan.from_alias {
            builder = builder.from_table_as(
                &postrust_sql::identifier::QualifiedIdentifier::new(&qi.schema, &qi.name),
                alias,
            );
        } else {
            builder = builder.from_table(
                &postrust_sql::identifier::QualifiedIdentifier::new(&qi.schema, &qi.name),
            );
        }

        // SELECT columns
        for field in &plan.select {
            let col_frag = Self::build_select_field(field)?;
            builder = builder.column_raw(col_frag);
        }

        // WHERE clauses
        for clause in &plan.where_clauses {
            let expr = Self::build_logic_tree(clause)?;
            builder = builder.where_raw(expr);
        }

        // ORDER BY
        for term in &plan.order {
            let order = Self::build_order_term(term);
            builder = builder.order_by(order);
        }

        // LIMIT/OFFSET
        if let Some(limit) = plan.range.limit {
            builder = builder.limit(limit);
        }
        if plan.range.offset > 0 {
            builder = builder.offset(plan.range.offset);
        }

        Ok(builder.build())
    }

    /// Build a SELECT field.
    fn build_select_field(field: &CoercibleSelectField) -> Result<SqlFragment> {
        let mut frag = SqlFragment::new();

        // Aggregate function
        if let Some(agg) = &field.aggregate {
            frag.push(agg.to_sql());
            frag.push("(");
        }

        // Column name with JSON path
        frag.push(&escape_ident(&field.field.name));

        // Close aggregate
        if field.aggregate.is_some() {
            frag.push(")");
        }

        // Cast
        if let Some(cast) = &field.cast {
            frag.push("::");
            frag.push(cast);
        }

        // Alias
        if let Some(alias) = &field.alias {
            frag.push(" AS ");
            frag.push(&escape_ident(alias));
        }

        Ok(frag)
    }

    /// Build a logic tree.
    fn build_logic_tree(tree: &CoercibleLogicTree) -> Result<SqlFragment> {
        match tree {
            CoercibleLogicTree::Expr { negated, op, children } => {
                let sep = match op {
                    crate::api_request::LogicOperator::And => " AND ",
                    crate::api_request::LogicOperator::Or => " OR ",
                };

                let child_frags: Result<Vec<_>> = children
                    .iter()
                    .map(|c| Self::build_logic_tree(c))
                    .collect();

                let mut combined = SqlFragment::join(sep, child_frags?).parens();

                if *negated {
                    let mut neg = SqlFragment::raw("NOT ");
                    neg.append(combined);
                    combined = neg;
                }

                Ok(combined)
            }
            CoercibleLogicTree::Stmt(filter) => Self::build_filter(filter),
            CoercibleLogicTree::NullEmbed { negated, field_name } => {
                let mut frag = SqlFragment::new();
                frag.push(&escape_ident(field_name));
                if *negated {
                    frag.push(" IS NOT NULL");
                } else {
                    frag.push(" IS NULL");
                }
                Ok(frag)
            }
        }
    }

    /// Build a filter expression.
    fn build_filter(filter: &CoercibleFilter) -> Result<SqlFragment> {
        let mut frag = SqlFragment::new();

        // Column name
        frag.push(&escape_ident(&filter.field.name));

        // Handle negation
        if filter.op_expr.negated {
            frag.push(" NOT");
        }

        // Operation
        match &filter.op_expr.operation {
            crate::api_request::Operation::Simple { op, value } => {
                frag.push(" ");
                frag.push(op.to_sql());
                frag.push(" ");
                frag.push_param(value.clone());
            }
            crate::api_request::Operation::Quant { op, quantifier, value } => {
                frag.push(" ");
                frag.push(op.to_sql());
                frag.push(" ");
                if let Some(q) = quantifier {
                    match q {
                        crate::api_request::OpQuantifier::Any => frag.push("ANY("),
                        crate::api_request::OpQuantifier::All => frag.push("ALL("),
                    };
                    frag.push_param(value.clone());
                    frag.push(")");
                } else {
                    frag.push_param(value.clone());
                }
            }
            crate::api_request::Operation::In(values) => {
                frag.push(" IN (");
                for (i, v) in values.iter().enumerate() {
                    if i > 0 {
                        frag.push(", ");
                    }
                    frag.push_param(v.clone());
                }
                frag.push(")");
            }
            crate::api_request::Operation::Is(is_val) => {
                frag.push(" IS ");
                frag.push(is_val.to_sql());
            }
            crate::api_request::Operation::IsDistinctFrom(value) => {
                frag.push(" IS DISTINCT FROM ");
                frag.push_param(value.clone());
            }
            crate::api_request::Operation::Fts { op, language, value } => {
                frag.push(" @@ ");
                frag.push(op.to_function());
                frag.push("(");
                if let Some(lang) = language {
                    frag.push_param(lang.clone());
                    frag.push(", ");
                }
                frag.push_param(value.clone());
                frag.push(")");
            }
        }

        Ok(frag)
    }

    /// Build an ORDER BY term.
    fn build_order_term(term: &CoercibleOrderTerm) -> OrderExpr {
        let mut order = OrderExpr::new(&term.field.name);

        if let Some(dir) = &term.direction {
            order = match dir {
                crate::api_request::OrderDirection::Asc => order.asc(),
                crate::api_request::OrderDirection::Desc => order.desc(),
            };
        }

        if let Some(nulls) = &term.nulls {
            order = match nulls {
                crate::api_request::OrderNulls::First => order.nulls_first(),
                crate::api_request::OrderNulls::Last => order.nulls_last(),
            };
        }

        order
    }

    /// Build a mutation query.
    pub fn build_mutate(plan: &MutatePlan) -> Result<SqlFragment> {
        match plan {
            MutatePlan::Insert {
                target,
                columns,
                body,
                on_conflict,
                returning,
                ..
            } => {
                let qi = postrust_sql::identifier::QualifiedIdentifier::new(
                    &target.schema,
                    &target.name,
                );

                let mut builder = InsertBuilder::new().into_table(&qi);

                // Column names
                let col_names: Vec<String> = columns.iter().map(|c| c.name.clone()).collect();
                builder = builder.columns(col_names);

                // For bulk insert, we'd use json_populate_recordset
                // For now, simplified single-row insert
                if let Some(body_bytes) = body {
                    // This would be expanded with proper JSON handling
                    let body_str = String::from_utf8_lossy(body_bytes);
                    let mut frag = SqlFragment::new();
                    frag.push("SELECT * FROM json_populate_recordset(NULL::");
                    frag.push(&from_qi(&qi));
                    frag.push(", ");
                    frag.push_param(body_str.to_string());
                    frag.push("::json)");
                    return Ok(frag);
                }

                // ON CONFLICT
                if let Some((resolution, conflict_cols)) = on_conflict {
                    match resolution {
                        crate::api_request::PreferResolution::IgnoreDuplicates => {
                            builder = builder.on_conflict_do_nothing();
                        }
                        crate::api_request::PreferResolution::MergeDuplicates => {
                            let set_cols: Vec<(String, SqlFragment)> = columns
                                .iter()
                                .map(|c| {
                                    let mut frag = SqlFragment::new();
                                    frag.push("EXCLUDED.");
                                    frag.push(&escape_ident(&c.name));
                                    (c.name.clone(), frag)
                                })
                                .collect();
                            builder = builder.on_conflict_do_update(conflict_cols.clone(), set_cols);
                        }
                    }
                }

                // RETURNING
                for col in returning {
                    builder = builder.returning(col);
                }

                Ok(builder.build())
            }

            MutatePlan::Update {
                target,
                columns,
                body,
                where_clauses,
                returning,
                ..
            } => {
                let qi = postrust_sql::identifier::QualifiedIdentifier::new(
                    &target.schema,
                    &target.name,
                );

                let builder = UpdateBuilder::new().table(&qi);

                // SET columns from body
                if let Some(body_bytes) = body {
                    let body_str = String::from_utf8_lossy(body_bytes);
                    // Simplified: would properly parse JSON and set columns
                    let mut frag = SqlFragment::new();
                    frag.push("UPDATE ");
                    frag.push(&from_qi(&qi));
                    frag.push(" SET ");

                    for (i, col) in columns.iter().enumerate() {
                        if i > 0 {
                            frag.push(", ");
                        }
                        frag.push(&escape_ident(&col.name));
                        frag.push(" = (");
                        frag.push_param(body_str.to_string());
                        frag.push("::json->>");
                        frag.push_param(col.name.clone());
                        frag.push(")::");
                        frag.push(&col.ir_type);
                    }

                    // WHERE
                    if !where_clauses.is_empty() {
                        frag.push(" WHERE ");
                        for (i, clause) in where_clauses.iter().enumerate() {
                            if i > 0 {
                                frag.push(" AND ");
                            }
                            frag.append(Self::build_logic_tree(clause)?);
                        }
                    }

                    // RETURNING
                    if !returning.is_empty() {
                        frag.push(" RETURNING ");
                        for (i, col) in returning.iter().enumerate() {
                            if i > 0 {
                                frag.push(", ");
                            }
                            frag.push(&escape_ident(col));
                        }
                    }

                    return Ok(frag);
                }

                Ok(builder.build())
            }

            MutatePlan::Delete {
                target,
                where_clauses,
                returning,
            } => {
                let qi = postrust_sql::identifier::QualifiedIdentifier::new(
                    &target.schema,
                    &target.name,
                );

                let mut builder = DeleteBuilder::new().from_table(&qi);

                // WHERE
                for clause in where_clauses {
                    let expr = Self::build_logic_tree(clause)?;
                    builder = builder.where_raw(expr);
                }

                // RETURNING
                for col in returning {
                    builder = builder.returning(col);
                }

                Ok(builder.build())
            }
        }
    }

    /// Build an RPC call query.
    pub fn build_call(plan: &CallPlan) -> Result<SqlFragment> {
        let qi = postrust_sql::identifier::QualifiedIdentifier::new(
            &plan.function.schema,
            &plan.function.name,
        );

        let mut frag = SqlFragment::new();
        frag.push("SELECT * FROM ");
        frag.push(&from_qi(&qi));
        frag.push("(");

        match &plan.params {
            CallParams::Named(params) => {
                for (i, (name, value)) in params.iter().enumerate() {
                    if i > 0 {
                        frag.push(", ");
                    }
                    frag.push(&escape_ident(name));
                    frag.push(" => ");
                    frag.push_param(SqlParam::Text(value.clone()));
                }
            }
            CallParams::Positional(values) => {
                for (i, value) in values.iter().enumerate() {
                    if i > 0 {
                        frag.push(", ");
                    }
                    frag.push_param(SqlParam::Text(value.clone()));
                }
            }
            CallParams::SingleObject(body) => {
                let body_str = String::from_utf8_lossy(body);
                frag.push_param(SqlParam::Text(body_str.to_string()));
            }
            CallParams::None => {}
        }

        frag.push(")");

        Ok(frag)
    }
}
