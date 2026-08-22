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
}

impl EmbedPlan {
    /// Resolve a relationship into an embed plan.
    ///
    /// Returns an error for relationships this cannot express yet, rather than
    /// silently omitting the embedded data.
    pub fn resolve(relationship: &Relationship, schema_cache: &SchemaCache) -> Result<Self> {
        let foreign_table_qi = relationship.foreign_table().clone();

        let columns = match relationship {
            Relationship::ForeignKey { cardinality, .. } => cardinality.columns(),
            Relationship::Computed { .. } => {
                return Err(Error::EmbeddingError(
                    "embedding a computed relationship is not supported yet".into(),
                ))
            }
        };

        if columns.len() != 1 {
            return Err(Error::EmbeddingError(format!(
                "embedding \"{}\" is not supported yet: it joins on {} columns and \
                 only single-column joins are implemented",
                foreign_table_qi.name,
                columns.len()
            )));
        }

        let (local_column, foreign_column) = columns[0].clone();

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
        })
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

    /// A correlated subselect that yields this relationship as one JSON column.
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
    /// Parent columns are deliberately left alone: they stay ordinary typed
    /// columns and are converted to JSON by the same code as an unembedded
    /// request, so embedding does not change how a NUMERIC or a timestamp is
    /// rendered. Only the relationship column arrives as JSON, which is what
    /// the separate child query already returned.
    pub fn embed_expression(
        &self,
        parent_alias: &str,
        child_alias: &str,
        inner_select: &str,
        limit: Option<i64>,
        child_where: Option<&str>,
    ) -> Result<String> {
        // The child table is aliased rather than referred to by name. A
        // self-referential relationship would otherwise make the correlation
        // ambiguous, since the parent and the child are the same table.
        let mut inner = format!(
            "SELECT {} FROM {}.{} AS {} WHERE {}.{} = {}.{}",
            inner_select,
            postrust_sql::escape_ident(&self.foreign_schema),
            postrust_sql::escape_ident(&self.foreign_table),
            postrust_sql::escape_ident(child_alias),
            postrust_sql::escape_ident(child_alias),
            postrust_sql::escape_ident(&self.foreign_column),
            postrust_sql::escape_ident(parent_alias),
            postrust_sql::escape_ident(&self.local_column),
        );

        // Filters written against the embedded resource (`clients.id=eq.1`)
        // narrow the children, exactly as they would if the child had been
        // requested on its own.
        if let Some(child_where) = child_where {
            inner.push_str(" AND ");
            inner.push_str(child_where);
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

        let alias = postrust_sql::escape_ident(&format!("{}_j", child_alias));

        Ok(if self.is_list {
            // An empty array rather than null, so the shape does not depend on
            // whether the parent happens to have children.
            format!(
                "COALESCE((SELECT json_agg(row_to_json({alias})) FROM ({inner}) {alias}), '[]'::json)",
                alias = alias,
                inner = inner
            )
        } else {
            format!(
                "(SELECT row_to_json({alias}) FROM ({inner}) {alias})",
                alias = alias,
                inner = inner
            )
        })
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
        child_alias: &str,
        child_where: Option<&str>,
    ) -> String {
        let mut predicate = format!(
            "EXISTS (SELECT 1 FROM {}.{} AS {} WHERE {}.{} = {}.{}",
            postrust_sql::escape_ident(&self.foreign_schema),
            postrust_sql::escape_ident(&self.foreign_table),
            postrust_sql::escape_ident(child_alias),
            postrust_sql::escape_ident(child_alias),
            postrust_sql::escape_ident(&self.foreign_column),
            postrust_sql::escape_ident(parent_alias),
            postrust_sql::escape_ident(&self.local_column),
        );

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
/// `columns` names the columns the client asked for, in order. It is what lets
/// a parent with no matching child still carry the right keys with null
/// values, since there is no child row to read the names off. A spread with no
/// explicit column list has no such list to fall back on, so in that case a
/// parent with no match gains nothing.
pub fn spread_into_parent(
    parent: &mut serde_json::Value,
    plan: &EmbedPlan,
    grouped: &HashMap<String, Vec<serde_json::Value>>,
    columns: &[String],
) {
    let key = parent.get(&plan.local_column).and_then(key_to_text);
    let matches = key
        .as_ref()
        .and_then(|k| grouped.get(k))
        .cloned()
        .unwrap_or_default();

    // Which keys to write, and in which order. An explicit column list wins,
    // so the shape does not depend on which rows happened to match; otherwise
    // take the union of the keys the matched rows carry, first seen first.
    let mut names: Vec<String> = columns.to_vec();
    if names.is_empty() {
        for child in &matches {
            if let Some(object) = child.as_object() {
                for name in object.keys() {
                    if !names.iter().any(|existing| existing == name) {
                        names.push(name.clone());
                    }
                }
            }
        }
    }

    let Some(object) = parent.as_object_mut() else {
        return;
    };

    for name in names {
        let value = if plan.is_list {
            serde_json::Value::Array(
                matches
                    .iter()
                    .map(|child| child.get(&name).cloned().unwrap_or(serde_json::Value::Null))
                    .collect(),
            )
        } else {
            matches
                .first()
                .and_then(|child| child.get(&name).cloned())
                .unwrap_or(serde_json::Value::Null)
        };
        object.insert(name, value);
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
            local_column: "id".into(),
            foreign_column: "user_id".into(),
            foreign_column_type: "int4".into(),
            foreign_schema: "public".into(),
            foreign_table: "posts".into(),
            is_list,
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
            .embed_expression("p", "posts", r#""id", "title""#, None, None)
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
            .embed_expression("p", "author", r#""id""#, None, None)
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
            .embed_expression("p", "posts", r#""id""#, Some(25), None)
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
            &["first_name".to_string(), "last_name".to_string()],
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
            &["first_name".to_string()],
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
        spread_into_parent(&mut parent, &plan(true), &grouped, &["title".to_string()]);

        assert_eq!(parent["title"], serde_json::json!(["a", "b"]));
    }

    #[test]
    fn spread_without_a_column_list_takes_the_child_keys() {
        let mut grouped = HashMap::new();
        grouped.insert("1".to_string(), vec![serde_json::json!({"a": 1, "b": 2})]);

        let mut parent = serde_json::json!({"id": 1});
        spread_into_parent(&mut parent, &plan(false), &grouped, &[]);

        assert_eq!(parent["a"], 1);
        assert_eq!(parent["b"], 2);
    }
}
