//! Query builder implementation.

use crate::error::Result;
use crate::plan::{
    CallParams, CallPlan, CoercibleFilter, CoercibleLogicTree, CoercibleOrderTerm,
    CoercibleSelectField, MutatePlan, ReadPlan, ReadPlanTree,
};
use postrust_sql::{escape_ident, from_qi, OrderExpr, SelectBuilder, SqlFragment, SqlParam};

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
        //
        // A term naming an embedded resource is left for the caller: it orders
        // by a column of another table, which this query has no way to reach.
        for term in plan.order.iter().filter(|t| t.relation.is_none()) {
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

        // `*` reaches here only where the columns are not known ahead of time
        // -- a function's result, where the plan has no table to expand it
        // against. Quoting it would ask for a column literally named `*`.
        if field.field.name == "*" && field.aggregate.is_none() && field.cast.is_none() {
            frag.push("*");
            return Ok(frag);
        }

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

        // A type this process cannot decode is rendered by PostgreSQL instead.
        //
        // The row converter maps a fixed set of built-in types and falls back
        // to reading a column as text, which fails outright for a type like a
        // PostGIS geometry -- the column came back as null. `to_jsonb` gives
        // the database's own rendering, which for a geometry is the GeoJSON
        // PostgREST returns. PostgREST gets it for free by building its whole
        // response in SQL; this is the same rendering, asked for by name.
        let render_as_json = field.aggregate.is_none()
            && field.cast.is_none()
            && field.field.json_path.is_empty()
            && !decodable_type(&field.field.ir_type);
        if render_as_json {
            let mut inner = SqlFragment::new();
            push_field_ref(&mut inner, &field.field);
            frag.push("to_jsonb(");
            frag.push(inner.sql());
            frag.push(") AS ");
            frag.push(&escape_ident(
                field.alias.as_deref().unwrap_or(&field.field.name),
            ));
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
        // A transformer renders the column instead of this process: PostgREST
        // uses it for a schema's declared data representations, and it is also
        // how a value whose spelling depends on the session -- a `timestamptz`
        // under `Prefer: timezone` -- gets rendered where the session is.
        match &field.field.transform {
            Some(function) => {
                frag.push(function);
                frag.push("(");
                push_field_ref(&mut frag, &field.field);
                frag.push(")");
            }
            None => push_field_ref(&mut frag, &field.field),
        }
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
        let implicit_alias = if field.alias.is_some() {
            None
        } else if field.field.transform.is_some() {
            Some(field.field.name.clone())
        } else if !field.field.json_path.is_empty() {
            Some(json_path_alias(&field.field.name, &field.field.json_path))
        } else if field.field.computed.is_some() {
            // A call is an expression, and PostgreSQL would label it after the
            // function -- which is the right name, but only by coincidence of
            // the two agreeing. Naming it outright keeps a schema-qualified
            // call from arriving under some other label.
            Some(field.field.name.clone())
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

    /// A column reference with its JSON path, as it appears in SQL.
    ///
    /// Exposed for ordering by an embedded resource's column, which is
    /// assembled outside the read plan.
    pub fn column_sql(field: &crate::plan::CoercibleField) -> String {
        Self::qualified_column_sql(None, field)
    }

    /// The same, with the column qualified by a relation alias.
    ///
    /// The alias belongs to the column, not to the expression around it:
    /// `to_jsonb(e2."settings")->'a'`, never `e2.to_jsonb(...)`, which reads
    /// as a function in a schema called `e2`.
    pub fn qualified_column_sql(
        qualifier: Option<&str>,
        field: &crate::plan::CoercibleField,
    ) -> String {
        let mut frag = SqlFragment::new();
        push_qualified_field_ref(&mut frag, qualifier, field);
        frag.sql().to_string()
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

    /// Build the SQL for a logic tree against a table whose column types
    /// `type_resolver` supplies.
    ///
    /// Exposed for the same reason as `filter_sql`: an embedded resource's
    /// `and=`/`or=` is applied inside a subquery the embedding path assembles
    /// itself, without going through a full plan.
    pub fn logic_sql<F>(
        tree: &crate::api_request::LogicTree,
        type_resolver: F,
    ) -> Result<SqlFragment>
    where
        F: Fn(&str) -> String + Copy,
    {
        Self::build_logic_tree(&CoercibleLogicTree::from_logic_tree(tree, type_resolver))
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
        //
        // The language belongs on both sides: `to_tsvector('french', col) @@
        // to_tsquery('french', $1)`. Left off the left-hand side the column is
        // lexed with the default configuration -- English -- and a French
        // query then matches nothing, quietly and with a 200.
        let to_tsvector = match &filter.op_expr.operation {
            crate::api_request::Operation::Fts { language, .. }
                if !filter.field.ir_type.starts_with("tsvector") =>
            {
                Some(language.clone())
            }
            _ => None,
        };
        if let Some(language) = &to_tsvector {
            frag.push("to_tsvector(");
            if let Some(language) = language {
                frag.push_typed_param(language.clone(), "regconfig");
                frag.push(", ");
            }
        }
        push_field_ref(&mut frag, &filter.field);
        if to_tsvector.is_some() {
            frag.push(")");
        }

        // Filter values are always bound as text, so a comparison against a
        // non-text column needs an explicit cast on the placeholder -- without
        // it PostgreSQL rejects the query with `operator does not exist:
        // integer = text`. A JSON path already yields text, so it is left as-is.
        let cast = if filter.field.json_path.is_empty() {
            castable_type(&filter.field.ir_type)
        } else {
            // A JSON path leaves either `jsonb` (`->`) or `text` (`->>`) on the
            // left. Text needs no cast, but `jsonb = $1` with the placeholder
            // bound as text finds no operator, so the value is cast to match.
            json_path_result_cast(&filter.field.json_path)
        };

        // `match` is `~`, and on an `ltree` the right-hand side of `~` is an
        // `lquery` rather than another `ltree`: `path=match.*.Science` is a
        // pattern, not a path. Casting it to the column's own type -- which is
        // what every other operator wants -- finds no operator at all.
        let cast = match (&filter.op_expr.operation, filter.field.ir_type.as_str()) {
            (
                crate::api_request::Operation::Quant {
                    op:
                        crate::api_request::QuantOperator::Match
                        | crate::api_request::QuantOperator::IMatch,
                    quantifier: None,
                    ..
                },
                "ltree",
            ) => Some("lquery"),
            _ => cast,
        };
        // A schema that declared how one of its domains is written also
        // declared how it is read: the cast parses the value the client sent,
        // in its own spelling, rather than PostgreSQL's input function for the
        // type underneath.
        let parser = filter.field.transform.as_deref();
        let push_value = |frag: &mut SqlFragment, value: String| {
            if let Some(parser) = parser {
                frag.push(parser);
                frag.push("(");
                frag.push_param(value);
                frag.push(")");
                return;
            }
            match cast {
                Some(pg_type) => {
                    frag.push_typed_param(value, pg_type);
                }
                None => {
                    frag.push_param(value);
                }
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
            crate::api_request::Operation::In(values) if values.is_empty() => {
                // `IN ()` is a syntax error. The empty array says the same
                // thing and is a single expression, so it needs no parentheses
                // of its own wherever the filter ends up. It is false for
                // every value, nulls included, which is what makes `not.in.()`
                // match everything.
                frag.push(" = ANY('{}')");
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
            push_field_ref(&mut frag, &term.field);
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
                where_clauses,
                returning,
                ..
            } => {
                let qi = postrust_sql::identifier::QualifiedIdentifier::new(
                    &target.schema,
                    &target.name,
                );

                let mut frag = SqlFragment::new();
                frag.push("INSERT INTO ");
                frag.push(&from_qi(&qi));

                match (body, columns.is_empty()) {
                    // A body naming no columns -- `{}` -- inserts a row of
                    // defaults. There is no column list to write and no values
                    // to read out of the body.
                    (_, true) => {
                        frag.push(" DEFAULT VALUES");
                    }
                    (Some(body), false) => {
                        frag.push(" (");
                        push_column_list(&mut frag, columns);
                        frag.push(") SELECT ");
                        for (i, column) in columns.iter().enumerate() {
                            if i > 0 {
                                frag.push(", ");
                            }
                            frag.push("pgrst_body.");
                            frag.push(&escape_ident(&column.name));
                        }
                        frag.push(" ");
                        push_json_body(&mut frag, columns, body, false);

                        // PUT names the row in the URL as well as in the body,
                        // and the two have to agree -- the conditions are
                        // written against the body so that a mismatch inserts
                        // nothing rather than the wrong row.
                        if !where_clauses.is_empty() {
                            frag.push(" WHERE ");
                            for (i, clause) in where_clauses.iter().enumerate() {
                                if i > 0 {
                                    frag.push(" AND ");
                                }
                                frag.append(Self::build_logic_tree(clause)?);
                            }
                        }
                    }
                    (None, false) => {
                        frag.push(" DEFAULT VALUES");
                    }
                }

                if let Some((resolution, conflict_cols)) = on_conflict {
                    frag.push(" ON CONFLICT (");
                    for (i, column) in conflict_cols.iter().enumerate() {
                        if i > 0 {
                            frag.push(", ");
                        }
                        frag.push(&escape_ident(column));
                    }
                    frag.push(") DO ");
                    match resolution {
                        crate::api_request::PreferResolution::IgnoreDuplicates => {
                            frag.push("NOTHING");
                        }
                        crate::api_request::PreferResolution::MergeDuplicates
                            if columns.is_empty() =>
                        {
                            frag.push("NOTHING");
                        }
                        crate::api_request::PreferResolution::MergeDuplicates => {
                            frag.push("UPDATE SET ");
                            for (i, column) in columns.iter().enumerate() {
                                if i > 0 {
                                    frag.push(", ");
                                }
                                frag.push(&escape_ident(&column.name));
                                frag.push(" = EXCLUDED.");
                                frag.push(&escape_ident(&column.name));
                            }
                        }
                    }
                }

                push_returning(&mut frag, returning);
                Ok(frag)
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

                // An update that assigns nothing is not valid SQL, and it is
                // also not an error: `PATCH` with `{}` matches rows and
                // changes none of them. Selecting no rows from the table gives
                // the same answer with the same column names, which is what a
                // `?select=` over the result needs.
                if columns.is_empty() || body.is_none() {
                    let mut frag = SqlFragment::new();
                    frag.push("SELECT * FROM ");
                    frag.push(&from_qi(&qi));
                    frag.push(" WHERE false");
                    return Ok(frag);
                }

                let body = body.as_ref().expect("checked above");
                let mut frag = SqlFragment::new();
                frag.push("UPDATE ");
                frag.push(&from_qi(&qi));
                frag.push(" SET ");
                for (i, column) in columns.iter().enumerate() {
                    if i > 0 {
                        frag.push(", ");
                    }
                    frag.push(&escape_ident(&column.name));
                    frag.push(" = pgrst_body.");
                    frag.push(&escape_ident(&column.name));
                }
                frag.push(" ");
                push_json_body(&mut frag, columns, body, true);

                if !where_clauses.is_empty() {
                    frag.push(" WHERE ");
                    for (i, clause) in where_clauses.iter().enumerate() {
                        if i > 0 {
                            frag.push(" AND ");
                        }
                        frag.append(Self::build_logic_tree(clause)?);
                    }
                }

                push_returning(&mut frag, returning);
                Ok(frag)
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

                let mut frag = SqlFragment::new();
                frag.push("DELETE FROM ");
                frag.push(&from_qi(&qi));

                if !where_clauses.is_empty() {
                    frag.push(" WHERE ");
                    for (i, clause) in where_clauses.iter().enumerate() {
                        if i > 0 {
                            frag.push(" AND ");
                        }
                        frag.append(Self::build_logic_tree(clause)?);
                    }
                }

                push_returning(&mut frag, returning);
                Ok(frag)
            }
        }
    }

    /// Build an RPC call query.
    pub fn build_call(plan: &CallPlan, read: Option<&ReadPlanTree>) -> Result<SqlFragment> {
        let qi = postrust_sql::identifier::QualifiedIdentifier::new(
            &plan.function.schema,
            &plan.function.name,
        );

        // The call itself is the source of rows; the read plan, when there is
        // one, shapes them exactly as it would shape a table's.
        // A result this process cannot decode is rendered by the database,
        // exactly as a column of such a type is. Without it a function
        // returning `xml` or `bytea` answered null.
        // A composite return -- OUT parameters, `RETURNS TABLE`, a row type --
        // already expands to its own columns, and wrapping it would bury them
        // under the function's name.
        let render_as_json = read.is_none()
            && !plan.returns_composite
            && plan
                .return_type
                .as_deref()
                .map(|t| !decodable_type(t) && t != "record")
                .unwrap_or(false);

        let mut frag = SqlFragment::new();
        frag.push("SELECT ");
        if render_as_json {
            frag.push("to_jsonb(pgrst_scalar) AS ");
            frag.push(&escape_ident(&plan.function.name));
            frag.push(" FROM ");
            frag.push(&from_qi(&qi));
            frag.push("(");
        }
        match read.filter(|tree| !tree.root.select.is_empty()) {
            Some(tree) => {
                for (i, field) in tree.root.select.iter().enumerate() {
                    if i > 0 {
                        frag.push(", ");
                    }
                    let column = Self::build_select_field(field)?;
                    frag.append(column);
                }
            }
            None => {
                if !render_as_json {
                    frag.push("*");
                }
            }
        }
        if !render_as_json {
            frag.push(" FROM ");
            frag.push(&from_qi(&qi));
            frag.push("(");
        }

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
        if render_as_json {
            frag.push(" AS pgrst_scalar");
        }

        // Filters, ordering and paging over the returned rows.
        //
        // The call is left unaliased: a function returning a table's rows is
        // referred to by that table's name, which is what an unqualified
        // column in a filter resolves against.
        if let Some(tree) = read {
            for (i, clause) in tree.root.where_clauses.iter().enumerate() {
                frag.push(if i == 0 { " WHERE " } else { " AND " });
                let expr = Self::build_logic_tree(clause)?;
                frag.append(expr);
            }

            for (i, term) in tree.root.order.iter().enumerate() {
                frag.push(if i == 0 { " ORDER BY " } else { ", " });
                let order = Self::build_order_term(term).into_fragment();
                frag.append(order);
            }

            if let Some(limit) = tree.root.range.limit {
                frag.push(&format!(" LIMIT {}", limit));
            }
            if tree.root.range.offset > 0 {
                frag.push(&format!(" OFFSET {}", tree.root.range.offset));
            }
        }

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
/// The columns a mutation writes, as a comma-separated identifier list.
fn push_column_list(frag: &mut SqlFragment, columns: &[crate::plan::CoercibleField]) {
    for (i, column) in columns.iter().enumerate() {
        if i > 0 {
            frag.push(", ");
        }
        frag.push(&escape_ident(&column.name));
    }
}

/// The `FROM` clause that turns a JSON body into rows of the table's columns.
///
/// The body is read as a record set of exactly the columns being written, each
/// typed as the column is -- or as `json`, where a data representation is going
/// to parse it -- so PostgreSQL does the conversion and this process never has
/// to guess at a literal's spelling.
///
/// `single` reads one object rather than an array, which is what a `PATCH`
/// body is.
fn push_json_body(
    frag: &mut SqlFragment,
    columns: &[crate::plan::CoercibleField],
    body: &bytes::Bytes,
    single: bool,
) {
    let body = String::from_utf8_lossy(body).into_owned();
    let object = body.trim_start().starts_with('{');

    frag.push("FROM (SELECT ");
    frag.push_param(body);
    frag.push("::json AS json_data) pgrst_payload, LATERAL (SELECT ");
    for (i, column) in columns.iter().enumerate() {
        if i > 0 {
            frag.push(", ");
        }
        match &column.transform {
            Some(parser) => {
                frag.push(parser);
                frag.push("(");
                frag.push(&escape_ident(&column.name));
                frag.push(") AS ");
                frag.push(&escape_ident(&column.name));
            }
            None => {
                frag.push(&escape_ident(&column.name));
            }
        }
    }
    // `json_to_record` takes one object and `json_to_recordset` an array. An
    // update writes one set of values whatever it matches, so its body is one
    // object -- and a body that is not is an error PostgreSQL words better
    // than this could. An insert may be either, and the body says which.
    frag.push(match single || object {
        true => " FROM json_to_record(pgrst_payload.json_data) AS _(",
        false => " FROM json_to_recordset(pgrst_payload.json_data) AS _(",
    });
    for (i, column) in columns.iter().enumerate() {
        if i > 0 {
            frag.push(", ");
        }
        frag.push(&escape_ident(&column.name));
        frag.push(" ");
        frag.push(castable_type(&column.ir_type).unwrap_or("text"));
    }
    // Two parentheses: one closes the column definition list, one the lateral
    // subquery the alias names.
    frag.push(")) pgrst_body");
}

/// The `RETURNING` list, or nothing when there is none.
fn push_returning(frag: &mut SqlFragment, returning: &[String]) {
    if returning.is_empty() {
        return;
    }
    frag.push(" RETURNING ");
    for (i, column) in returning.iter().enumerate() {
        if i > 0 {
            frag.push(", ");
        }
        frag.push(&escape_ident(column));
    }
}

/// A column reference, converted to JSON first where the path requires it.
fn push_field_ref(frag: &mut SqlFragment, field: &crate::plan::CoercibleField) {
    push_qualified_field_ref(frag, None, field)
}

/// The same, with the column qualified by a relation alias.
fn push_qualified_field_ref(
    frag: &mut SqlFragment,
    qualifier: Option<&str>,
    field: &crate::plan::CoercibleField,
) {
    // A computed field is a function of the whole row. PostgreSQL would also
    // accept `items.always_true` and resolve it to the same call, but only
    // where the reference is qualified by the relation -- which a bare column
    // name here is not -- so the call is written out.
    let column = match &field.computed {
        Some(computed) => format!(
            "{}.{}({})",
            escape_ident(&computed.function.schema),
            escape_ident(&computed.function.name),
            escape_ident(&computed.relation),
        ),
        None => match qualifier {
            Some(alias) => format!("{}.{}", escape_ident(alias), escape_ident(&field.name)),
            None => escape_ident(&field.name),
        },
    };

    if field.to_json {
        frag.push("to_jsonb(");
        frag.push(&column);
        frag.push(")");
    } else {
        frag.push(&column);
    }
    push_json_path(frag, &field.json_path);
}

/// The type a JSON path leaves on the left of a comparison.
///
/// `None` for a path ending in `->>`, which is already text -- the type filter
/// values are bound as, so no cast is wanted.
fn json_path_result_cast(path: &crate::api_request::JsonPath) -> Option<&'static str> {
    use crate::api_request::JsonOperation;
    match path.last()? {
        JsonOperation::Arrow(_) => Some("jsonb"),
        JsonOperation::DoubleArrow(_) => None,
    }
}

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

/// Whether this process can decode a column of this type.
///
/// The row converter maps a fixed set of types by name and otherwise reads the
/// column as text, which only works for what PostgreSQL will hand over as
/// text. Anything else -- `xml`, `bytea`, an array, a PostGIS geometry -- fails
/// to decode and the column comes back null, so it is rendered in the database
/// instead. An empty type is a field with no column behind it, such as
/// `count()`, and never reaches this.
fn decodable_type(data_type: &str) -> bool {
    matches!(
        data_type,
        "" | "smallint"
            | "integer"
            | "bigint"
            | "numeric"
            | "decimal"
            | "real"
            | "double precision"
            | "boolean"
            | "text"
            | "character varying"
            | "character"
            | "name"
            | "date"
            | "time"
            | "time without time zone"
            | "time with time zone"
            | "timestamp"
            | "timestamp without time zone"
            | "timestamp with time zone"
            | "uuid"
            | "json"
            | "jsonb"
    )
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
