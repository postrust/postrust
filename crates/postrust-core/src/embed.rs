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
    pub fn children_sql(&self, limit: Option<i64>) -> Result<String> {
        let type_name = castable_type_name(&self.foreign_column_type).ok_or_else(|| {
            Error::EmbeddingError(format!(
                "cannot embed \"{}\": join column type \"{}\" is not a plain type name",
                self.foreign_table, self.foreign_column_type
            ))
        })?;

        let mut inner = format!(
            "SELECT * FROM {}.{} WHERE {} = ANY($1::{}[])",
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
        let sql = plan(true).children_sql(None).unwrap();
        assert_eq!(
            sql,
            "SELECT row_to_json(t) FROM (SELECT * FROM \"public\".\"posts\" \
             WHERE \"user_id\" = ANY($1::int4[])) t"
        );
    }

    #[test]
    fn children_sql_applies_a_limit() {
        let sql = plan(true).children_sql(Some(25)).unwrap();
        assert!(sql.contains("LIMIT 25"), "{}", sql);
    }

    #[test]
    fn children_sql_rejects_a_non_plain_type_name() {
        let mut p = plan(true);
        p.foreign_column_type = "int4; DROP TABLE users".into();
        assert!(p.children_sql(None).is_err());
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
}
