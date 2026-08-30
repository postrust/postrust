//! Relationship embedding: fetching related rows for a set of parent rows.
//!
//! Both the REST and GraphQL surfaces embed related resources, and both do it
//! the same way: take the parent rows already fetched, collect the values of
//! the join column, and issue **one** query for all of them rather than one
//! query per parent row.
//!
//! ```text
//! SELECT row_to_json(t) FROM (
//!     SELECT * FROM "public"."posts" WHERE "user_id" = ANY($1::int4[])
//! ) t
//! ```
//!
//! The children are then grouped by that column and attached to the parent
//! rows, so a request embedding two relationships across a page of 25 parents
//! costs three queries, not fifty-one.

use crate::error::{Error, Result};
use crate::schema_cache::{Relationship, SchemaCache, Table};
use std::collections::HashMap;

/// Everything needed to fetch one relationship's rows for a set of parents.
#[derive(Clone, Debug)]
pub struct EmbedPlan {
    /// Column on the parent row whose value identifies the parent.
    pub local_column: String,
    /// Column on the related table that points back at the parent.
    pub foreign_column: String,
    /// PostgreSQL type of the foreign column, used to cast the bound array.
    pub foreign_column_type: String,
    /// Schema of the related table.
    pub foreign_schema: String,
    /// Name of the related table.
    pub foreign_table: String,
    /// Whether the relationship yields many rows per parent.
    pub is_list: bool,
    /// Every column pair the join is on, parent side first.
    ///
    /// Usually one. A foreign key over several columns joins on all of them,
    /// and the pairs are ordered so each parent column sits with the child
    /// column it actually references.
    pub columns: Vec<(String, String)>,
    /// The junction of a many-to-many relationship, if that is what this is.
    ///
    /// The parent and the child share no key; each points at a table that
    /// exists to join them, so reaching the child means going through it.
    pub junction: Option<EmbedJunction>,
    /// The function behind a computed relationship, if that is what this is.
    ///
    /// A computed relationship is a function taking the parent row and
    /// returning rows of the related table. There is no key to join on -- the
    /// parent row is the argument -- so the columns above are empty and the
    /// relationship can only be expressed as a call correlated to the parent.
    pub function: Option<crate::api_request::QualifiedIdentifier>,
    /// The name of the parameter that takes the parent's row, where the
    /// function has more than one and the call has to be written by name.
    pub row_argument: Option<String>,
}

/// The table a many-to-many relationship is joined through.
#[derive(Clone, Debug)]
pub struct EmbedJunction {
    pub schema: String,
    pub table: String,
    /// `(parent column, junction column)` for each column the parent joins on.
    ///
    /// Usually one. A junction may be keyed over several columns on each side
    /// -- `touched_files` joins `files(project_id, filename)` to
    /// `users_tasks(user_id, task_id)` -- and joining on the first of them
    /// alone would relate rows that are not related.
    pub parent_columns: Vec<(String, String)>,
    /// `(junction column, child column)` for each column the child joins on.
    pub child_columns: Vec<(String, String)>,
}

impl EmbedJunction {
    /// The junction's join to the child, one equality per column pair.
    fn child_join(&self, junction_alias: &str, child_alias: &str) -> String {
        Self::equalities(&self.child_columns, junction_alias, child_alias, true)
    }

    /// The junction's correlation to one parent row, one equality per pair.
    fn parent_correlation(&self, junction_alias: &str, parent_alias: &str) -> String {
        Self::equalities(&self.parent_columns, junction_alias, parent_alias, false)
    }

    /// `junction.column = other.column` for each pair, joined with `AND`.
    ///
    /// `junction_first` says which half of the pair is the junction's, the two
    /// lists running in opposite directions: the parent's own column comes
    /// first on the way in, and the child's second on the way out.
    fn equalities(
        pairs: &[(String, String)],
        junction_alias: &str,
        other_alias: &str,
        junction_first: bool,
    ) -> String {
        pairs
            .iter()
            .map(|(left, right)| {
                let (junction_column, other_column) = match junction_first {
                    true => (left, right),
                    false => (right, left),
                };
                format!(
                    "{}.{} = {}.{}",
                    postrust_sql::escape_ident(junction_alias),
                    postrust_sql::escape_ident(junction_column),
                    postrust_sql::escape_ident(other_alias),
                    postrust_sql::escape_ident(other_column),
                )
            })
            .collect::<Vec<_>>()
            .join(" AND ")
    }
}

impl EmbedPlan {
    /// Resolve a relationship into an embed plan.
    ///
    /// Returns an error for relationships this cannot express yet, rather than
    /// silently omitting the embedded data.
    pub fn resolve(relationship: &Relationship, schema_cache: &SchemaCache) -> Result<Self> {
        let foreign_table_qi = relationship.foreign_table().clone();

        // A computed relationship takes the parent row as its argument, so
        // there is nothing to look up and nothing to join on.
        if let Relationship::Computed {
            function,
            foreign_table,
            to_one,
            row_argument,
            ..
        } = relationship
        {
            return Ok(Self {
                local_column: String::new(),
                foreign_column: String::new(),
                foreign_column_type: String::new(),
                foreign_schema: foreign_table.schema.clone(),
                foreign_table: foreign_table.name.clone(),
                is_list: !to_one,
                columns: Vec::new(),
                junction: None,
                function: Some(function.clone()),
                row_argument: row_argument.clone(),
            });
        }

        if let Relationship::ForeignKey {
            cardinality: crate::schema_cache::Cardinality::M2M(junction),
            ..
        } = relationship
        {
            // The join itself is written from every column pair below. These
            // two are the parent's and the child's first key column, which is
            // what the batched path groups by -- and it refuses a junction
            // keyed on more than one, rather than grouping by half a key.
            let (local_column, _) =
                junction.source_columns.first().cloned().ok_or_else(|| {
                    Error::EmbeddingError("junction has no source columns".into())
                })?;
            let (_, foreign_column) =
                junction.target_columns.first().cloned().ok_or_else(|| {
                    Error::EmbeddingError("junction has no target columns".into())
                })?;

            return Ok(Self {
                local_column,
                foreign_column,
                foreign_column_type: String::new(),
                foreign_schema: foreign_table_qi.schema.clone(),
                foreign_table: foreign_table_qi.name.clone(),
                // A junction always yields a set: that is what it is for.
                is_list: true,
                columns: Vec::new(),
                row_argument: None,
                junction: Some(EmbedJunction {
                    schema: junction.table.schema.clone(),
                    table: junction.table.name.clone(),
                    parent_columns: junction.source_columns.clone(),
                    child_columns: junction.target_columns.clone(),
                }),
                function: None,
            });
        }

        let columns = match relationship {
            Relationship::ForeignKey { cardinality, .. } => cardinality.columns(),
            Relationship::Computed { .. } => unreachable!("handled above"),
        };

        let (local_column, foreign_column) = columns
            .first()
            .cloned()
            .ok_or_else(|| Error::EmbeddingError("relationship joins on no columns".into()))?;

        let foreign_table: &Table = schema_cache.get_table(&foreign_table_qi).ok_or_else(|| {
            Error::EmbeddingError(format!(
                "cannot embed \"{}\": it is not in an exposed schema",
                foreign_table_qi
            ))
        })?;

        let foreign_column_type = foreign_table
            .get_column(&foreign_column)
            .map(|c| c.nominal_type.clone())
            .ok_or_else(|| {
                Error::EmbeddingError(format!(
                    "cannot embed \"{}\": join column \"{}\" not found",
                    foreign_table_qi, foreign_column
                ))
            })?;

        Ok(Self {
            local_column,
            foreign_column,
            foreign_column_type,
            foreign_schema: foreign_table_qi.schema.clone(),
            foreign_table: foreign_table_qi.name.clone(),
            is_list: !relationship.is_to_one(),
            columns,
            junction: None,
            function: None,
            row_argument: None,
        })
    }

    /// The predicate correlating child rows to one parent row.
    ///
    /// A foreign key over several columns joins on all of them, so this is one
    /// equality per pair rather than a single one.
    fn correlation(&self, parent_alias: &str, child_alias: &str) -> String {
        let pairs = if self.columns.is_empty() {
            vec![(self.local_column.clone(), self.foreign_column.clone())]
        } else {
            self.columns.clone()
        };

        pairs
            .iter()
            .map(|(local, foreign)| {
                format!(
                    "{}.{} = {}.{}",
                    postrust_sql::escape_ident(child_alias),
                    postrust_sql::escape_ident(foreign),
                    postrust_sql::escape_ident(parent_alias),
                    postrust_sql::escape_ident(local),
                )
            })
            .collect::<Vec<_>>()
            .join(" AND ")
    }

    /// SQL that fetches every related row for the given parent key values.
    ///
    /// The keys are bound as a single text array and cast to the foreign
    /// column's type, so the column itself is never wrapped in a cast and an
    /// index on it remains usable. `limit` bounds the rows per query, not per
    /// parent.
    ///
    /// `columns` is the set of columns the client asked for. An empty set means
    /// every column. Projecting here rather than discarding columns after the
    /// fact matters: an unprojected column is read from the heap, serialised to
    /// JSON by PostgreSQL, sent over the socket and parsed, before being thrown
    /// away. The join column is always included even when it was not requested,
    /// because grouping needs it; the caller strips it afterwards.
    pub fn children_sql(&self, limit: Option<i64>, columns: &[String]) -> Result<String> {
        let type_name = castable_type_name(&self.foreign_column_type).ok_or_else(|| {
            Error::EmbeddingError(format!(
                "cannot embed \"{}\": join column type \"{}\" is not a plain type name",
                self.foreign_table, self.foreign_column_type
            ))
        })?;

        let projection = self.projection(columns);

        let mut inner = format!(
            "SELECT {} FROM {}.{} WHERE {} = ANY($1::{}[])",
            projection,
            postrust_sql::escape_ident(&self.foreign_schema),
            postrust_sql::escape_ident(&self.foreign_table),
            postrust_sql::escape_ident(&self.foreign_column),
            type_name
        );

        if let Some(limit) = limit {
            inner.push_str(&format!(" LIMIT {}", limit));
        }

        Ok(format!("SELECT row_to_json(t) FROM ({}) t", inner))
    }

    /// SQL that fetches related rows already grouped by their join key.
    ///
    /// One row comes back per distinct key: the key itself and a JSON array of
    /// that key's children. Grouping in PostgreSQL rather than in this process
    /// removes a per-child-row JSON parse, the per-row hash insert, and the
    /// clone of each group onto its parent.
    ///
    /// The key is returned as JSON rather than cast to text, so it is rendered
    /// by the same code that renders the parents' keys. Casting to text in SQL
    /// would agree for integers and uuids and disagree for a NUMERIC join
    /// column, where PostgreSQL and serde_json format differently.
    ///
    /// `limit` still bounds the rows scanned, not the rows per parent, so it is
    /// applied to the inner select exactly as the ungrouped form does.
    pub fn children_grouped_sql(&self, limit: Option<i64>, columns: &[String]) -> Result<String> {
        if self
            .junction
            .as_ref()
            .is_some_and(|j| j.parent_columns.len() > 1)
        {
            return Err(Error::EmbeddingError(format!(
                "embedding \"{}\" this way is not supported: it joins through a junction \
                 keyed on several columns, and grouping children by key needs a single one",
                self.foreign_table,
            )));
        }

        if self.columns.len() > 1 {
            return Err(Error::EmbeddingError(format!(
                "embedding \"{}\" this way is not supported: it joins on {} columns, and \
                 grouping children by key needs a single one",
                self.foreign_table,
                self.columns.len()
            )));
        }

        let type_name = castable_type_name(&self.foreign_column_type).ok_or_else(|| {
            Error::EmbeddingError(format!(
                "cannot embed \"{}\": join column type \"{}\" is not a plain type name",
                self.foreign_table, self.foreign_column_type
            ))
        })?;

        let key = postrust_sql::escape_ident(&self.foreign_column);

        let mut inner = format!(
            "SELECT {} FROM {}.{} WHERE {} = ANY($1::{}[])",
            self.projection(columns),
            postrust_sql::escape_ident(&self.foreign_schema),
            postrust_sql::escape_ident(&self.foreign_table),
            key,
            type_name
        );

        if let Some(limit) = limit {
            inner.push_str(&format!(" LIMIT {}", limit));
        }

        Ok(format!(
            "SELECT to_jsonb(c.{key}) AS k, json_agg(row_to_json(c)) AS v \
             FROM ({inner}) c GROUP BY c.{key}",
            key = key,
            inner = inner
        ))
    }

    /// The child rows belonging to one parent row, as a SELECT correlated
    /// against the parent's alias.
    ///
    /// This is the single-query form of embedding: instead of fetching parents,
    /// collecting their keys and issuing a second query, the relationship is
    /// attached to the parent query as an expression, so PostgreSQL builds the
    /// array while it already has the parent row.
    ///
    /// `inner_select` is the child's SELECT list, which the caller assembles --
    /// its columns, plus any deeper relationship expressions built by calling
    /// this again. Only the caller knows the shape of its own selection tree, so
    /// the recursion lives there and the SQL assembly lives here.
    ///
    /// What is done with those rows is the caller's: [`embed_expression`]
    /// renders them as JSON, [`aggregate_expression`] summarises them.
    #[allow(clippy::too_many_arguments)] // one parameter per SQL clause
    pub fn correlated_rows(
        &self,
        parent_alias: &str,
        parent_row: &str,
        child_alias: &str,
        inner_select: &str,
        limit: Option<i64>,
        // `clients.offset=1` skips rows inside the embed. A to-one embed that
        // is skipped past yields no row at all, which is null -- the same
        // answer as no match, and the one PostgREST gives.
        offset: i64,
        child_where: Option<&str>,
        // `clients.order=name.desc` orders the rows inside the embed, which
        // belongs to the child's own subselect -- and has to be applied before
        // the child's own limit, or the limit takes an unsorted window.
        order_by: Option<&str>,
        // The whole argument list of a computed relationship's function, where
        // the parent row is not the only thing it takes.
        function_arguments: Option<&str>,
    ) -> Result<String> {
        // The child table is aliased rather than referred to by name. A
        // self-referential relationship would otherwise make the correlation
        // ambiguous, since the parent and the child are the same table.
        //
        // A computed relationship is correlated by argument rather than by a
        // key: the function takes the parent row, so the parent alias is
        // passed to it and there is no predicate to write.
        let mut inner = match &self.function {
            Some(function) => format!(
                "SELECT {} FROM {}.{}({}) AS {} WHERE true",
                inner_select,
                postrust_sql::escape_ident(&function.schema),
                postrust_sql::escape_ident(&function.name),
                // The parent row alone, unless the caller wrote the whole
                // argument list: a function with more than the row -- a search
                // term, the caller's session -- is called by name, and only the
                // caller knows what to put in those.
                function_arguments.unwrap_or(parent_row),
                postrust_sql::escape_ident(child_alias),
            ),
            None => match &self.junction {
                // Two hops: the child is reached through the table that joins
                // it to the parent, so the correlation lands on the junction.
                Some(junction) => {
                    let junction_alias = format!("{}_j", child_alias);
                    format!(
                        "SELECT {} FROM {}.{} AS {} JOIN {}.{} AS {} ON {} WHERE {}",
                        inner_select,
                        postrust_sql::escape_ident(&self.foreign_schema),
                        postrust_sql::escape_ident(&self.foreign_table),
                        postrust_sql::escape_ident(child_alias),
                        postrust_sql::escape_ident(&junction.schema),
                        postrust_sql::escape_ident(&junction.table),
                        postrust_sql::escape_ident(&junction_alias),
                        junction.child_join(&junction_alias, child_alias),
                        junction.parent_correlation(&junction_alias, parent_alias),
                    )
                }
                None => format!(
                    "SELECT {} FROM {}.{} AS {} WHERE {}",
                    inner_select,
                    postrust_sql::escape_ident(&self.foreign_schema),
                    postrust_sql::escape_ident(&self.foreign_table),
                    postrust_sql::escape_ident(child_alias),
                    self.correlation(parent_alias, child_alias),
                ),
            },
        };

        // Filters written against the embedded resource (`clients.id=eq.1`)
        // narrow the children, exactly as they would if the child had been
        // requested on its own.
        if let Some(child_where) = child_where {
            inner.push_str(" AND ");
            inner.push_str(child_where);
        }

        if let Some(order_by) = order_by {
            inner.push_str(" ORDER BY ");
            inner.push_str(order_by);
        }

        // A to-one relationship takes the first row; a to-many takes them all.
        // The limit bounds rows per parent here, which is what a client asking
        // for a page of children means, and is stricter than the row cap the
        // two-query form could apply.
        if let Some(limit) = limit {
            inner.push_str(&format!(" LIMIT {}", limit));
        } else if !self.is_list {
            inner.push_str(" LIMIT 1");
        }

        if offset > 0 {
            inner.push_str(&format!(" OFFSET {}", offset));
        }

        Ok(inner)
    }

    /// A correlated subselect that yields this relationship as one JSON column.
    ///
    /// The rows come from [`correlated_rows`]; this decides how they are
    /// rendered -- an array for a to-many relationship, one object or null for
    /// a to-one.
    #[allow(clippy::too_many_arguments)] // one parameter per SQL clause
    pub fn embed_expression(
        &self,
        parent_alias: &str,
        parent_row: &str,
        child_alias: &str,
        inner_select: &str,
        limit: Option<i64>,
        offset: i64,
        child_where: Option<&str>,
        order_by: Option<&str>,
        function_arguments: Option<&str>,
    ) -> Result<String> {
        let inner = self.correlated_rows(
            parent_alias,
            parent_row,
            child_alias,
            inner_select,
            limit,
            offset,
            child_where,
            order_by,
            function_arguments,
        )?;

        let alias = postrust_sql::escape_ident(&format!("{}_j", child_alias));

        // Cast to `jsonb`, as PostgREST does -- `COALESCE(json_agg(..),'[]')::jsonb`.
        // Only visible where an embedded row is rendered as text rather than
        // as part of a JSON body: `jsonb` normalises what `json` keeps
        // verbatim, so a schema-declared handler that stringifies the whole
        // row wrote `{"id":1}` where PostgREST wrote `{"id": 1}`.
        Ok(if self.is_list {
            // An empty array rather than null, so the shape does not depend on
            // whether the parent happens to have children.
            format!(
                "COALESCE((SELECT json_agg(row_to_json({alias})) FROM ({inner}) {alias}), '[]'::json)::jsonb",
                alias = alias,
                inner = inner
            )
        } else {
            format!(
                "(SELECT row_to_json({alias}) FROM ({inner}) {alias})::jsonb",
                alias = alias,
                inner = inner
            )
        })
    }

    /// A correlated subselect that summarises this relationship as one JSON
    /// column.
    ///
    /// `author { articles_aggregate { aggregate { count } } }`. The rows are
    /// the same correlated set an embed would render, but a `limit` here bounds
    /// what is *counted* rather than what is returned -- so the aggregate reads
    /// from the rows as a subquery instead of selecting over them directly.
    /// The subquery carries the child's own alias, so a predicate or an
    /// ordering written against it goes on reading the same name.
    ///
    /// Parent columns are deliberately left alone: they stay ordinary typed
    /// columns and are converted to JSON by the same code as an unembedded
    /// request, so embedding does not change how a NUMERIC or a timestamp is
    /// rendered.
    #[allow(clippy::too_many_arguments)] // one parameter per SQL clause
    pub fn aggregate_expression(
        &self,
        parent_alias: &str,
        parent_row: &str,
        child_alias: &str,
        aggregate_select: &str,
        limit: Option<i64>,
        offset: i64,
        child_where: Option<&str>,
        order_by: Option<&str>,
        function_arguments: Option<&str>,
        // A `DISTINCT ON (...) ` prefix, or empty. It belongs to the rows
        // being summarised rather than to the summary: counting the distinct
        // titles means counting the rows that survive it.
        distinct_on: &str,
    ) -> Result<String> {
        let child = postrust_sql::escape_ident(child_alias);
        let rows = self.correlated_rows(
            parent_alias,
            parent_row,
            child_alias,
            &format!("{}{}.*", distinct_on, child),
            limit,
            offset,
            child_where,
            order_by,
            function_arguments,
        )?;
        let alias = postrust_sql::escape_ident(&format!("{}_a", child_alias));
        Ok(format!(
            "(SELECT row_to_json({alias}) FROM \
             (SELECT {select} FROM ({rows}) AS {child}) {alias})::jsonb",
            alias = alias,
            select = aggregate_select,
            rows = rows,
            child = child
        ))
    }

    /// A scalar subselect yielding one column of the related row.
    ///
    /// This is what `order=clients(name)` orders by: the parent has no such
    /// column, so the value is fetched per parent row exactly as the embed
    /// itself is. A to-many relationship has no single value to order by, so
    /// the first row wins -- which is what `LIMIT 1` says and what PostgREST
    /// does.
    pub fn order_expression(
        &self,
        parent_alias: &str,
        parent_row: &str,
        child_alias: &str,
        column_sql: &str,
    ) -> String {
        let source = match (&self.function, &self.junction) {
            (Some(function), _) => format!(
                "{}.{}({}) AS {} WHERE true",
                postrust_sql::escape_ident(&function.schema),
                postrust_sql::escape_ident(&function.name),
                parent_row,
                postrust_sql::escape_ident(child_alias),
            ),
            (None, Some(junction)) => {
                let junction_alias = format!("{}_j", child_alias);
                format!(
                    "{}.{} AS {} JOIN {}.{} AS {} ON {} WHERE {}",
                    postrust_sql::escape_ident(&self.foreign_schema),
                    postrust_sql::escape_ident(&self.foreign_table),
                    postrust_sql::escape_ident(child_alias),
                    postrust_sql::escape_ident(&junction.schema),
                    postrust_sql::escape_ident(&junction.table),
                    postrust_sql::escape_ident(&junction_alias),
                    junction.child_join(&junction_alias, child_alias),
                    junction.parent_correlation(&junction_alias, parent_alias),
                )
            }
            (None, None) => format!(
                "{}.{} AS {} WHERE {}",
                postrust_sql::escape_ident(&self.foreign_schema),
                postrust_sql::escape_ident(&self.foreign_table),
                postrust_sql::escape_ident(child_alias),
                self.correlation(parent_alias, child_alias),
            ),
        };

        format!("(SELECT {} FROM {} LIMIT 1)", column_sql, source)
    }

    /// An `EXISTS` predicate restricting parents to those with a matching child.
    ///
    /// This is what `!inner` means. The embed expression on its own only
    /// decides what the relationship column contains; without this the parent
    /// row survives even when the relationship matched nothing, which is a
    /// left join. `child_where` should be the same predicate given to
    /// [`Self::embed_expression`] -- placeholders may be referenced from both
    /// places, so no parameter needs binding twice.
    pub fn inner_join_predicate(
        &self,
        parent_alias: &str,
        parent_row: &str,
        child_alias: &str,
        child_where: Option<&str>,
    ) -> String {
        let mut predicate = match &self.function {
            Some(function) => format!(
                "EXISTS (SELECT 1 FROM {}.{}({}) AS {} WHERE true",
                postrust_sql::escape_ident(&function.schema),
                postrust_sql::escape_ident(&function.name),
                parent_row,
                postrust_sql::escape_ident(child_alias),
            ),
            None => match &self.junction {
                Some(junction) => {
                    let junction_alias = format!("{}_j", child_alias);
                    format!(
                        "EXISTS (SELECT 1 FROM {}.{} AS {} JOIN {}.{} AS {} ON {} WHERE {}",
                        postrust_sql::escape_ident(&self.foreign_schema),
                        postrust_sql::escape_ident(&self.foreign_table),
                        postrust_sql::escape_ident(child_alias),
                        postrust_sql::escape_ident(&junction.schema),
                        postrust_sql::escape_ident(&junction.table),
                        postrust_sql::escape_ident(&junction_alias),
                        junction.child_join(&junction_alias, child_alias),
                        junction.parent_correlation(&junction_alias, parent_alias),
                    )
                }
                None => format!(
                    "EXISTS (SELECT 1 FROM {}.{} AS {} WHERE {}",
                    postrust_sql::escape_ident(&self.foreign_schema),
                    postrust_sql::escape_ident(&self.foreign_table),
                    postrust_sql::escape_ident(child_alias),
                    self.correlation(parent_alias, child_alias),
                ),
            },
        };

        if let Some(child_where) = child_where {
            predicate.push_str(" AND ");
            predicate.push_str(child_where);
        }

        predicate.push(')');
        predicate
    }

    /// The child projection list: the requested columns plus the join column.
    ///
    /// Column names come from the client, so each is escaped rather than
    /// interpolated bare. Anything that is not a plain column reference -- a
    /// nested relation, say -- is not a column and is skipped.
    fn projection(&self, columns: &[String]) -> String {
        if columns.is_empty() {
            return "*".to_string();
        }

        let mut wanted: Vec<&str> = Vec::with_capacity(columns.len() + 1);
        for column in columns {
            if !wanted.contains(&column.as_str()) {
                wanted.push(column);
            }
        }
        if !wanted.contains(&self.foreign_column.as_str()) {
            wanted.push(&self.foreign_column);
        }

        wanted
            .into_iter()
            .map(postrust_sql::escape_ident)
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// Reject anything that is not a bare type name, since it is interpolated.
fn castable_type_name(pg_type: &str) -> Option<&str> {
    if pg_type.is_empty() {
        return None;
    }
    if pg_type
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        Some(pg_type)
    } else {
        None
    }
}

/// Render a JSON value as the text form used to match join keys.
///
/// Keys are compared as text and cast by PostgreSQL, so a numeric key must not
/// arrive quoted the way `to_string` would render a JSON string.
pub fn key_to_text(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Null => None,
        serde_json::Value::String(s) => Some(s.clone()),
        other => Some(other.to_string()),
    }
}

/// Group related rows by the value of their join column.
/// Build the grouping from rows PostgreSQL has already grouped.
///
/// Each row is the join key as JSON and a JSON array of that key's children.
/// The key goes through `key_to_text`, the same rendering the parent side uses,
/// so the two agree for every column type.
pub fn group_from_aggregated(
    rows: Vec<(serde_json::Value, serde_json::Value)>,
) -> HashMap<String, Vec<serde_json::Value>> {
    let mut grouped: HashMap<String, Vec<serde_json::Value>> = HashMap::with_capacity(rows.len());

    for (key, children) in rows {
        let key = key_to_text(&key).unwrap_or_default();
        let children = match children {
            serde_json::Value::Array(items) => items,
            // json_agg only ever yields an array or null.
            _ => Vec::new(),
        };
        grouped.entry(key).or_default().extend(children);
    }

    grouped
}

pub fn group_by_key(
    children: Vec<serde_json::Value>,
    foreign_column: &str,
) -> HashMap<String, Vec<serde_json::Value>> {
    let mut grouped: HashMap<String, Vec<serde_json::Value>> = HashMap::new();

    for child in children {
        let key = child
            .get(foreign_column)
            .and_then(key_to_text)
            .unwrap_or_default();
        grouped.entry(key).or_default().push(child);
    }

    grouped
}

/// Attach grouped children onto a parent row under `field_name`.
///
/// A to-one relationship yields the first match or `null`; a to-many yields an
/// array, empty when there are no matches, so the shape of the response does
/// not depend on whether data happens to exist.
pub fn attach_to_parent(
    parent: &mut serde_json::Value,
    field_name: &str,
    plan: &EmbedPlan,
    grouped: &HashMap<String, Vec<serde_json::Value>>,
) {
    let key = parent.get(&plan.local_column).and_then(key_to_text);

    let matches = key
        .as_ref()
        .and_then(|k| grouped.get(k))
        .cloned()
        .unwrap_or_default();

    let value = if plan.is_list {
        serde_json::Value::Array(matches)
    } else {
        matches
            .into_iter()
            .next()
            .unwrap_or(serde_json::Value::Null)
    };

    if let Some(object) = parent.as_object_mut() {
        object.insert(field_name.to_string(), value);
    }
}

/// Merge a related resource's columns into the parent row itself.
///
/// This is what the `...` of `select=title,...directors(name)` means. Where
/// [`attach_to_parent`] puts the related rows under a key of their own, a
/// spread lifts their columns into the parent object, so the result stays flat.
///
/// Spreading a to-many relationship gives each column an array of that
/// column's value across the matched rows, rather than an array of objects.
///
/// `columns` pairs each requested column with the key it should appear under,
/// which differ when the client aliased it: `...clients(client_name:name)`
/// reads `name` and writes `client_name`. Listing them explicitly is also what
/// lets a parent with no matching child still carry the right keys with null
/// values, since there is no child row to read the names off.
///
/// An empty list spreads nothing, which is what `...clients()` means. Falling
/// back to every column the child happens to have would be actively wrong: the
/// related table's own `id` would land on top of the parent's.
pub fn spread_into_parent(
    parent: &mut serde_json::Value,
    plan: &EmbedPlan,
    grouped: &HashMap<String, Vec<serde_json::Value>>,
    columns: &[(String, String)],
) {
    let key = parent.get(&plan.local_column).and_then(key_to_text);
    let matches = key
        .as_ref()
        .and_then(|k| grouped.get(k))
        .cloned()
        .unwrap_or_default();

    let Some(object) = parent.as_object_mut() else {
        return;
    };

    for (source, output) in columns {
        let value = if plan.is_list {
            serde_json::Value::Array(
                matches
                    .iter()
                    .map(|child| {
                        child
                            .get(source)
                            .cloned()
                            .unwrap_or(serde_json::Value::Null)
                    })
                    .collect(),
            )
        } else {
            matches
                .first()
                .and_then(|child| child.get(source).cloned())
                .unwrap_or(serde_json::Value::Null)
        };
        object.insert(output.clone(), value);
    }
}

/// Collect the distinct, non-null join keys of a set of parent rows.
pub fn parent_keys(parents: &[serde_json::Value], local_column: &str) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut keys = Vec::new();

    for parent in parents {
        if let Some(key) = parent.get(local_column).and_then(key_to_text) {
            if seen.insert(key.clone()) {
                keys.push(key);
            }
        }
    }

    keys
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(is_list: bool) -> EmbedPlan {
        EmbedPlan {
            row_argument: None,
            local_column: "id".into(),
            foreign_column: "user_id".into(),
            foreign_column_type: "int4".into(),
            foreign_schema: "public".into(),
            foreign_table: "posts".into(),
            is_list,
            columns: vec![("id".into(), "user_id".into())],
            junction: None,
            function: None,
        }
    }

    #[test]
    fn children_sql_binds_keys_as_a_cast_array() {
        let sql = plan(true).children_sql(None, &[]).unwrap();
        assert_eq!(
            sql,
            "SELECT row_to_json(t) FROM (SELECT * FROM \"public\".\"posts\" \
             WHERE \"user_id\" = ANY($1::int4[])) t"
        );
    }

    #[test]
    fn embed_expression_aggregates_a_to_many_relation() {
        let sql = plan(true)
            .embed_expression(
                "p",
                "\"p\"",
                "posts",
                r#""id", "title""#,
                None,
                0,
                None,
                None,
                None,
            )
            .unwrap();

        assert!(sql.starts_with("COALESCE((SELECT json_agg("), "{}", sql);
        // Correlated on the parent, so there is no second query and no bound
        // array of keys.
        assert!(sql.contains(r#""posts"."user_id" = "p"."id""#), "{}", sql);
        assert!(
            sql.contains(r#"AS "posts""#),
            "the child table is aliased: {}",
            sql
        );
        assert!(!sql.contains("ANY("), "{}", sql);
        // An absent relation is an empty array, not null.
        assert!(sql.contains("'[]'::json"), "{}", sql);
    }

    #[test]
    fn embed_expression_takes_one_row_for_a_to_one_relation() {
        let sql = plan(false)
            .embed_expression("p", "\"p\"", "author", r#""id""#, None, 0, None, None, None)
            .unwrap();

        assert!(sql.contains("row_to_json"), "{}", sql);
        assert!(!sql.contains("json_agg"), "{}", sql);
        assert!(
            sql.contains("LIMIT 1"),
            "a to-one relation yields one row: {}",
            sql
        );
    }

    #[test]
    fn embed_expression_limits_rows_per_parent() {
        let sql = plan(true)
            .embed_expression(
                "p",
                "\"p\"",
                "posts",
                r#""id""#,
                Some(25),
                0,
                None,
                None,
                None,
            )
            .unwrap();
        assert!(sql.contains("LIMIT 25"), "{}", sql);
    }

    #[test]
    fn children_sql_projects_only_the_requested_columns() {
        let sql = plan(true)
            .children_sql(None, &["title".to_string(), "body".to_string()])
            .unwrap();

        assert!(
            sql.contains(r#"SELECT "title", "body", "user_id" FROM"#),
            "{}",
            sql
        );
        assert!(
            !sql.contains("SELECT *"),
            "an unrequested column should not be read at all: {}",
            sql
        );
    }

    #[test]
    fn children_sql_always_includes_the_join_column() {
        // The grouping keys off the join column, so it has to come back even
        // when the client did not ask for it.
        let sql = plan(true)
            .children_sql(None, &["title".to_string()])
            .unwrap();
        assert!(sql.contains(r#""user_id""#), "{}", sql);
    }

    #[test]
    fn children_sql_does_not_repeat_the_join_column() {
        let sql = plan(true)
            .children_sql(None, &["user_id".to_string(), "title".to_string()])
            .unwrap();
        assert_eq!(
            sql.matches(r#""user_id""#).count(),
            2,
            "expected the column once in the projection and once in the WHERE: {}",
            sql
        );
    }

    #[test]
    fn children_sql_escapes_column_names() {
        // Column names reach here from the client.
        let sql = plan(true)
            .children_sql(None, &[r#"ev"il"#.to_string()])
            .unwrap();
        assert!(sql.contains(r#""ev""il""#), "{}", sql);
    }

    #[test]
    fn children_sql_falls_back_to_every_column() {
        let sql = plan(true).children_sql(None, &[]).unwrap();
        assert!(sql.contains("SELECT * FROM"), "{}", sql);
    }

    #[test]
    fn children_sql_applies_a_limit() {
        let sql = plan(true).children_sql(Some(25), &[]).unwrap();
        assert!(sql.contains("LIMIT 25"), "{}", sql);
    }

    #[test]
    fn children_sql_rejects_a_non_plain_type_name() {
        let mut p = plan(true);
        p.foreign_column_type = "int4; DROP TABLE users".into();
        assert!(p.children_sql(None, &[]).is_err());
    }

    #[test]
    fn keys_are_rendered_without_json_quoting() {
        assert_eq!(key_to_text(&serde_json::json!(7)), Some("7".to_string()));
        assert_eq!(
            key_to_text(&serde_json::json!("abc")),
            Some("abc".to_string())
        );
        assert_eq!(key_to_text(&serde_json::Value::Null), None);
    }

    #[test]
    fn parent_keys_are_distinct_and_skip_nulls() {
        let parents = vec![
            serde_json::json!({"id": 1}),
            serde_json::json!({"id": 2}),
            serde_json::json!({"id": 1}),
            serde_json::json!({"id": null}),
        ];
        assert_eq!(parent_keys(&parents, "id"), vec!["1", "2"]);
    }

    #[test]
    fn to_many_attaches_an_array_and_empty_when_absent() {
        let grouped = group_by_key(vec![serde_json::json!({"id": 10, "user_id": 1})], "user_id");

        let mut matched = serde_json::json!({"id": 1});
        attach_to_parent(&mut matched, "posts", &plan(true), &grouped);
        assert_eq!(matched["posts"].as_array().map(|a| a.len()), Some(1));

        let mut unmatched = serde_json::json!({"id": 2});
        attach_to_parent(&mut unmatched, "posts", &plan(true), &grouped);
        assert_eq!(
            unmatched["posts"],
            serde_json::json!([]),
            "a to-many with no matches must still be an array"
        );
    }

    #[test]
    fn to_one_attaches_an_object_or_null() {
        let grouped = group_by_key(vec![serde_json::json!({"id": 10, "user_id": 1})], "user_id");

        let mut matched = serde_json::json!({"id": 1});
        attach_to_parent(&mut matched, "author", &plan(false), &grouped);
        assert_eq!(matched["author"]["id"], serde_json::json!(10));

        let mut unmatched = serde_json::json!({"id": 2});
        attach_to_parent(&mut unmatched, "author", &plan(false), &grouped);
        assert_eq!(unmatched["author"], serde_json::Value::Null);
    }

    #[test]
    fn spread_lifts_a_to_one_relation_into_the_parent() {
        let mut grouped = HashMap::new();
        grouped.insert(
            "1".to_string(),
            vec![serde_json::json!({"first_name": "Ada", "last_name": "L"})],
        );

        let mut parent = serde_json::json!({"id": 1, "title": "Notes"});
        spread_into_parent(
            &mut parent,
            &plan(false),
            &grouped,
            &[
                ("first_name".into(), "first_name".into()),
                ("last_name".into(), "last_name".into()),
            ],
        );

        // Flat, not nested under a key of its own.
        assert_eq!(parent["first_name"], "Ada");
        assert_eq!(parent["last_name"], "L");
        assert_eq!(parent["title"], "Notes");
        assert!(parent.get("posts").is_none());
    }

    #[test]
    fn spread_without_a_match_still_carries_the_requested_keys() {
        let mut parent = serde_json::json!({"id": 9});
        spread_into_parent(
            &mut parent,
            &plan(false),
            &HashMap::new(),
            &[("first_name".into(), "first_name".into())],
        );
        assert_eq!(parent["first_name"], serde_json::Value::Null);
    }

    #[test]
    fn spread_of_a_to_many_relation_gives_a_column_per_array() {
        let mut grouped = HashMap::new();
        grouped.insert(
            "1".to_string(),
            vec![
                serde_json::json!({"title": "a"}),
                serde_json::json!({"title": "b"}),
            ],
        );

        let mut parent = serde_json::json!({"id": 1});
        spread_into_parent(
            &mut parent,
            &plan(true),
            &grouped,
            &[("title".into(), "title".into())],
        );

        assert_eq!(parent["title"], serde_json::json!(["a", "b"]));
    }

    #[test]
    fn spread_without_a_column_list_adds_nothing() {
        // `...clients()` spreads no columns. Taking every column the child has
        // would drop the related table's `id` on top of the parent's.
        let mut grouped = HashMap::new();
        grouped.insert("1".to_string(), vec![serde_json::json!({"id": 7, "b": 2})]);

        let mut parent = serde_json::json!({"id": 1});
        spread_into_parent(&mut parent, &plan(false), &grouped, &[]);

        assert_eq!(parent, serde_json::json!({"id": 1}));
    }
}
