//! Query builder implementation.

use crate::error::Result;
use crate::plan::{
    CallParams, CallPlan, CoercibleFilter, CoercibleLogicTree, CoercibleOrderTerm,
    CoercibleSelectField, MutatePlan, ReadPlan, ReadPlanTree,
};
use postrust_sql::{
    escape_ident, from_qi, DeleteBuilder, InsertBuilder, OrderExpr, SelectBuilder, SqlFragment,
    SqlParam, UpdateBuilder,
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
            builder = builder.from_table(&postrust_sql::identifier::QualifiedIdentifier::new(
                &qi.schema, &qi.name,
            ));
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

        // GROUP BY. Selecting an aggregate alongside plain columns means one
        // row per distinct combination of those columns, so every one of them
        // has to be grouped -- PostgreSQL rejects the query otherwise.
        if plan.select.iter().any(|f| f.aggregate.is_some()) {
            for field in plan.select.iter().filter(|f| f.aggregate.is_none()) {
                builder = builder.group_by(&field.field.name);
            }
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

        // `count()` counts rows rather than a column's non-null values.
        if field.aggregate.is_some() && field.field.name.is_empty() {
            frag.push("*)");
            if let Some(cast) = &field.aggregate_cast {
                frag.push("::");
                frag.push(cast);
            }
            frag.push(" AS ");
            frag.push(&escape_ident(field.alias.as_deref().unwrap_or("count")));
            return Ok(frag);
        }

        // Column name with JSON path.
        //
        // `::` binds tighter than `->>`, so a cast over a JSON path has to be
        // parenthesised: `settings ->> 'foo'::json` would cast the *key*
        // rather than the extracted value.
        let needs_parens = field.cast.is_some() && !field.field.json_path.is_empty();
        if needs_parens {
            frag.push("(");
        }
        frag.push(&escape_ident(&field.field.name));
        push_json_path(&mut frag, &field.field.json_path);
        if needs_parens {
            frag.push(")");
        }

        // Cast on the column, inside the aggregate.
        if let Some(cast) = &field.cast {
            frag.push("::");
            frag.push(cast);
        }

        // Close aggregate, then any cast applied to its result.
        if let Some(agg) = &field.aggregate {
            frag.push(")");
            if let Some(cast) = &field.aggregate_cast {
                frag.push("::");
                frag.push(cast);
            }
            // An aggregate is an expression, so it needs a name. PostgREST
            // uses the function's own, lowercased.
            frag.push(" AS ");
            frag.push(&escape_ident(
                field
                    .alias
                    .as_deref()
                    .unwrap_or(&agg.to_sql().to_lowercase()),
            ));
            return Ok(frag);
        }

        // Alias
        //
        // A JSON path always needs one: `data -> 'a' ->> 'b'` is an expression,
        // and PostgreSQL would label the column `?column?`. PostgREST names it
        // after the last key in the path, falling back to the column itself
        // when the path ends in an array index.
        let implicit_alias = if field.alias.is_none() && !field.field.json_path.is_empty() {
            Some(json_path_alias(&field.field.name, &field.field.json_path))
        } else {
            None
        };

        if let Some(alias) = field.alias.as_deref().or(implicit_alias.as_deref()) {
            frag.push(" AS ");
            frag.push(&escape_ident(alias));
        }

        Ok(frag)
    }

    /// Build a logic tree.
    fn build_logic_tree(tree: &CoercibleLogicTree) -> Result<SqlFragment> {
        match tree {
            CoercibleLogicTree::Expr {
                negated,
                op,
                children,
            } => {
                let sep = match op {
                    crate::api_request::LogicOperator::And => " AND ",
                    crate::api_request::LogicOperator::Or => " OR ",
                };

                let child_frags: Result<Vec<_>> =
                    children.iter().map(Self::build_logic_tree).collect();

                let mut combined = SqlFragment::join(sep, child_frags?).parens();

                if *negated {
                    let mut neg = SqlFragment::raw("NOT ");
                    neg.append(combined);
                    combined = neg;
                }

                Ok(combined)
            }
            CoercibleLogicTree::Stmt(filter) => Self::build_filter(filter),
            CoercibleLogicTree::NullEmbed {
                negated,
                field_name,
            } => {
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

    /// Build the SQL for a single filter against a column of `pg_type`.
    ///
    /// Exposed for the embedding path, which applies filters inside a child
    /// subquery it assembles itself rather than through a full plan. Column
    /// names are left unqualified: inside the child's subselect the innermost
    /// `FROM` wins, so a bare name binds to the child. The caller is
    /// responsible for having checked the column exists there -- otherwise the
    /// name would resolve outward to the correlated parent instead.
    pub fn filter_sql(filter: &crate::api_request::Filter, pg_type: &str) -> Result<SqlFragment> {
        Self::build_filter(&CoercibleFilter::from_filter(filter, pg_type))
    }

    fn build_filter(filter: &CoercibleFilter) -> Result<SqlFragment> {
        let mut frag = SqlFragment::new();

        // Negation wraps the whole comparison. Placing `NOT` between the column
        // and the operator only parses for a few operators -- `col NOT LIKE $1`
        // is valid but `col NOT = $1` is a syntax error -- so the comparison is
        // parenthesised instead, which is correct for every operator.
        if filter.op_expr.negated {
            frag.push("NOT (");
        }

        // Column name.
        //
        // A text-search operator wants a `tsvector` on the left. A column that
        // is not already one is wrapped, so `text_search=fts.x` searches the
        // text instead of failing to find an operator. A domain over tsvector
        // is already one, hence the prefix test rather than an equality.
        let to_tsvector = matches!(
            filter.op_expr.operation,
            crate::api_request::Operation::Fts { .. }
        ) && !filter.field.ir_type.starts_with("tsvector");
        if to_tsvector {
            frag.push("to_tsvector(");
        }
        frag.push(&escape_ident(&filter.field.name));
        push_json_path(&mut frag, &filter.field.json_path);
        if to_tsvector {
            frag.push(")");
        }

        // Filter values are always bound as text, so a comparison against a
        // non-text column needs an explicit cast on the placeholder -- without
        // it PostgreSQL rejects the query with `operator does not exist:
        // integer = text`. A JSON path already yields text, so it is left as-is.
        let cast = if filter.field.json_path.is_empty() {
            castable_type(&filter.field.ir_type)
        } else {
            None
        };
        let push_value = |frag: &mut SqlFragment, value: String| match cast {
            Some(pg_type) => {
                frag.push_typed_param(value, pg_type);
            }
            None => {
                frag.push_param(value);
            }
        };

        // Operation
        match &filter.op_expr.operation {
            crate::api_request::Operation::Simple { op, value } => {
                frag.push(" ");
                frag.push(op.to_sql());
                frag.push(" ");
                push_value(&mut frag, value.clone());
            }
            crate::api_request::Operation::Quant {
                op,
                quantifier,
                value,
            } => {
                // PostgREST spells the LIKE wildcard `*` rather than `%`, and
                // maps it unconditionally -- `*` is never a literal asterisk in
                // a like/ilike operand. Other operators take the value as-is,
                // so `match`/`imatch` regexes keep their own `*`.
                let value = &match op {
                    crate::api_request::QuantOperator::Like
                    | crate::api_request::QuantOperator::ILike => value.replace('*', "%"),
                    _ => value.clone(),
                };
                frag.push(" ");
                frag.push(op.to_sql());
                frag.push(" ");
                if let Some(q) = quantifier {
                    match q {
                        crate::api_request::OpQuantifier::Any => frag.push("ANY("),
                        crate::api_request::OpQuantifier::All => frag.push("ALL("),
                    };
                    // A quantified comparison takes an array of the column's
                    // type. Array-typed columns are already handled by the
                    // element cast, so they are left alone.
                    match cast.filter(|t| !t.starts_with('_')) {
                        Some(pg_type) => {
                            frag.push_typed_param(value.clone(), &format!("{}[]", pg_type));
                        }
                        None => {
                            frag.push_param(value.clone());
                        }
                    }
                    frag.push(")");
                } else {
                    push_value(&mut frag, value.clone());
                }
            }
            crate::api_request::Operation::In(values) => {
                frag.push(" IN (");
                for (i, v) in values.iter().enumerate() {
                    if i > 0 {
                        frag.push(", ");
                    }
                    push_value(&mut frag, v.clone());
                }
                frag.push(")");
            }
            crate::api_request::Operation::Is(is_val) => {
                frag.push(" IS ");
                frag.push(is_val.to_sql());
            }
            crate::api_request::Operation::IsDistinctFrom(value) => {
                frag.push(" IS DISTINCT FROM ");
                push_value(&mut frag, value.clone());
            }
            crate::api_request::Operation::Fts {
                op,
                language,
                value,
            } => {
                frag.push(" @@ ");
                frag.push(op.to_function());
                frag.push("(");
                if let Some(lang) = language {
                    // The text-search functions take their configuration as a
                    // `regconfig`, and there is no `to_tsquery(text, text)` to
                    // fall back on -- bound as plain text the call resolves to
                    // no function at all.
                    frag.push_typed_param(lang.clone(), "regconfig");
                    frag.push(", ");
                }
                // The query is text whatever the column holds, so it never
                // takes the column's own cast.
                frag.push_param(value.clone());
                frag.push(")");
            }
        }

        if filter.op_expr.negated {
            frag.push(")");
        }

        Ok(frag)
    }

    /// Build an ORDER BY term.
    fn build_order_term(term: &CoercibleOrderTerm) -> OrderExpr {
        let mut order = if term.field.json_path.is_empty() {
            OrderExpr::new(&term.field.name)
        } else {
            let mut frag = SqlFragment::new();
            frag.push(&escape_ident(&term.field.name));
            push_json_path(&mut frag, &term.field.json_path);
            OrderExpr::raw(frag.sql())
        };

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

                if let Some(body_bytes) = body {
                    let body_str = String::from_utf8_lossy(body_bytes);

                    // `json_populate_recordset` only accepts an array, but a
                    // single-row insert posts a bare object. Wrapping it here
                    // keeps one code path for both shapes -- checking the
                    // first token is enough, since the payload has already
                    // been validated as JSON by this point.
                    let rows = if body_str.trim_start().starts_with('{') {
                        format!("[{body_str}]")
                    } else {
                        body_str.into_owned()
                    };

                    let mut frag = SqlFragment::new();
                    frag.push("SELECT * FROM json_populate_recordset(NULL::");
                    frag.push(&from_qi(&qi));
                    frag.push(", ");
                    frag.push_param(rows);
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
                            builder =
                                builder.on_conflict_do_update(conflict_cols.clone(), set_cols);
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

        // Arguments come off the wire as strings. Casting each one to the type
        // the function actually declares is what lets PostgreSQL resolve the
        // signature; binding everything as `text` only works for text-taking
        // functions. An argument the schema cache doesn't know is left
        // untyped, so PostgreSQL applies its own inference rather than failing.
        let declared_type = |name: &str| {
            plan.param_types
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, t)| t.clone())
        };

        match &plan.params {
            CallParams::Named(params) => {
                for (i, (name, value)) in params.iter().enumerate() {
                    if i > 0 {
                        frag.push(", ");
                    }
                    frag.push(&escape_ident(name));
                    frag.push(" => ");
                    match declared_type(name) {
                        Some(pg_type) => {
                            frag.push_typed_param(SqlParam::Text(value.clone()), &pg_type)
                        }
                        None => frag.push_param(SqlParam::Text(value.clone())),
                    };
                }
            }
            CallParams::Positional(values) => {
                for (i, value) in values.iter().enumerate() {
                    if i > 0 {
                        frag.push(", ");
                    }
                    match plan.param_types.get(i) {
                        Some((_, pg_type)) => {
                            frag.push_typed_param(SqlParam::Text(value.clone()), pg_type)
                        }
                        None => frag.push_param(SqlParam::Text(value.clone())),
                    };
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

/// Return the type to cast a bound filter value to, if it is safe to do so.
///
/// The type name is interpolated into SQL, so only bare type names are
/// accepted: anything else (an empty type, a parameterised type such as
/// `character varying(255)`, or the `ARRAY`/`USER-DEFINED` placeholders that
/// `information_schema` reports) yields `None` and the value is bound
/// uncast, preserving the previous behaviour.
/// Append a JSON path to a column reference: `"data" -> 'a' ->> 'b'`.
///
/// Keys are emitted as string literals and indices as bare integers, which is
/// what distinguishes `data->'1'` from `data->1` in PostgreSQL. Operands reach
/// us already restricted to alphanumerics and underscores by the parser; the
/// quote doubling is belt-and-braces so this stays safe if that ever loosens.
fn push_json_path(frag: &mut SqlFragment, path: &crate::api_request::JsonPath) {
    use crate::api_request::{JsonOperand, JsonOperation};

    for operation in path {
        let (arrow, operand) = match operation {
            JsonOperation::Arrow(operand) => ("->", operand),
            JsonOperation::DoubleArrow(operand) => ("->>", operand),
        };

        frag.push(" ");
        frag.push(arrow);
        frag.push(" ");

        match operand {
            JsonOperand::Key(key) => {
                frag.push(&format!("'{}'", key.replace('\'', "''")));
            }
            JsonOperand::Idx(index) => {
                frag.push(&index.to_string());
            }
        }
    }
}

/// The name PostgREST gives an unaliased JSON path expression.
///
/// The last key in the path wins -- `data->a->>b` is reported as `b`. A path
/// that ends in an array index has no name to take, so it keeps the column's.
fn json_path_alias(column: &str, path: &crate::api_request::JsonPath) -> String {
    use crate::api_request::{JsonOperand, JsonOperation};

    path.iter()
        .rev()
        .find_map(|operation| match operation {
            JsonOperation::Arrow(JsonOperand::Key(key))
            | JsonOperation::DoubleArrow(JsonOperand::Key(key)) => Some(key.clone()),
            _ => None,
        })
        .unwrap_or_else(|| column.to_string())
}

fn castable_type(pg_type: &str) -> Option<&str> {
    if pg_type.is_empty() {
        return None;
    }

    let is_bare_name = pg_type
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_');

    if is_bare_name {
        Some(pg_type)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn castable_type_accepts_bare_type_names() {
        assert_eq!(castable_type("int4"), Some("int4"));
        assert_eq!(castable_type("timestamptz"), Some("timestamptz"));
        assert_eq!(castable_type("_text"), Some("_text"));
    }

    #[test]
    fn castable_type_rejects_unsafe_names() {
        assert_eq!(castable_type(""), None);
        assert_eq!(castable_type("character varying"), None);
        assert_eq!(castable_type("USER-DEFINED"), None);
        assert_eq!(castable_type("int4; DROP TABLE users"), None);
    }

    /// Build the SQL for the single root filter in `query`.
    fn filter_sql_for(query: &str, pg_type: &str) -> (String, Vec<String>) {
        let params = crate::api_request::parse_query_params(query, false).unwrap();
        let frag = QueryBuilder::filter_sql(&params.filters_root[0], pg_type).unwrap();
        let bound = frag
            .params()
            .iter()
            .map(|p| format!("{:?}", p))
            .collect::<Vec<_>>();
        (frag.sql().to_string(), bound)
    }

    #[test]
    fn quantified_comparison_binds_an_array() {
        let (sql, params) = filter_sql_for("id=eq(any).{1,2,3}", "int4");
        assert!(sql.contains("= ANY("), "got {sql}");
        assert!(sql.contains("int4[]"), "got {sql}");
        assert_eq!(params.len(), 1);
        assert!(params[0].contains("{1,2,3}"), "got {:?}", params[0]);
    }

    #[test]
    fn quantified_all_renders_all() {
        let (sql, _) = filter_sql_for("id=lt(all).{4,5}", "int4");
        assert!(sql.contains("< ALL("), "got {sql}");
    }

    #[test]
    fn quantified_like_maps_the_wildcard_inside_the_array() {
        let (_, params) = filter_sql_for("name=like(any).{foo*,*bar}", "text");
        assert!(params[0].contains("{foo%,%bar}"), "got {:?}", params[0]);
    }

    #[test]
    fn array_column_is_not_double_arrayed() {
        // An array-typed column already compares element-wise, so the cast is
        // left alone rather than becoming `_text[]`.
        let (sql, _) = filter_sql_for("tags=eq(any).{a,b}", "_text");
        assert!(sql.contains("= ANY("), "got {sql}");
        assert!(!sql.contains("_text[]"), "got {sql}");
    }
}
