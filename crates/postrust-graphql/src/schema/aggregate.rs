//! Aggregate types.
//!
//! Every table gets a second root field alongside its rows:
//!
//! ```graphql
//! author_aggregate(where: {id: {_gt: 2}}) {
//!   aggregate { count, avg { salary }, max { salary } }
//!   nodes { id name }
//! }
//! ```
//!
//! `nodes` is the same list the plain root field returns, so one request can
//! ask for a page of rows and the count of the whole set the page came from --
//! which is the query behind every "showing 1-25 of 380" in a user interface,
//! and the reason the two live under one field instead of two.
//!
//! Which functions a column gets follows from what PostgreSQL will accept.
//! `sum` and `avg` are offered on numeric columns only; `max` and `min` on
//! anything that orders, which includes text and timestamps. Offering `avg`
//! over a text column would put an error in the schema rather than in the
//! client.

use crate::schema::object::TableObjectType;
use crate::types::GraphQLType;
use std::collections::HashMap;

/// Aggregates over numbers, and the GraphQL type each returns.
///
/// PostgreSQL widens `sum` and keeps the column's own scale, while the
/// statistical functions all return double precision whatever they were given.
pub const NUMERIC_AGGREGATES: &[(&str, Returns)] = &[
    ("sum", Returns::Column),
    ("avg", Returns::Float),
    ("stddev", Returns::Float),
    ("stddev_samp", Returns::Float),
    ("stddev_pop", Returns::Float),
    ("variance", Returns::Float),
    ("var_samp", Returns::Float),
    ("var_pop", Returns::Float),
];

/// Aggregates over anything that orders.
pub const ORDERED_AGGREGATES: &[(&str, Returns)] =
    &[("max", Returns::Column), ("min", Returns::Column)];

/// What an aggregate's field is typed as.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Returns {
    /// The column's own type: `max(name)` is still text.
    Column,
    /// Double precision, whatever went in.
    Float,
}

/// The name of a table's aggregate root type.
pub fn aggregate_type_name(base_name: &str) -> String {
    format!("{}_aggregate", base_name)
}

/// The name of a table's aggregate-function type.
pub fn aggregate_fields_type_name(base_name: &str) -> String {
    format!("{}_aggregate_fields", base_name)
}

/// The name of the type holding one aggregate's per-column results.
pub fn function_fields_type_name(base_name: &str, function: &str) -> String {
    format!("{}_{}_fields", base_name, function)
}

/// Whether a column can be added to.
///
/// Everything that can be summed, and `money` beside them: PostgreSQL adds to
/// an amount the way it adds to a number, and Hasura offers `_inc` on one.
/// It is not in [`is_numeric`] because the aggregates that set answers --
/// `stddev`, `variance` and their kin -- have no `money` form, and a schema
/// that offered them would be advertising a call the database refuses.
pub fn is_incrementable(graphql_type: &GraphQLType) -> bool {
    is_numeric(graphql_type) || matches!(graphql_type, GraphQLType::Custom(name) if name == "money")
}

/// Whether a column can be summed and averaged.
pub fn is_numeric(graphql_type: &GraphQLType) -> bool {
    matches!(
        graphql_type,
        GraphQLType::Int | GraphQLType::Float | GraphQLType::BigInt | GraphQLType::BigDecimal
    )
}

/// Whether a column can be ranked, which is what `max` and `min` need.
///
/// Excludes the types PostgreSQL has no ordering for. `json` is the one that
/// matters in practice: `max(details)` on a `jsonb` column is an error, not an
/// empty result.
pub fn is_ordered(graphql_type: &GraphQLType) -> bool {
    match graphql_type {
        GraphQLType::Json | GraphQLType::List(_) => false,
        // PostgreSQL has no `max(boolean)`: the question "the largest of these
        // flags" is asked as `bool_and`/`bool_or` instead, and Hasura leaves
        // a boolean column out of `min` and `max` for the same reason.
        GraphQLType::Boolean => false,
        // A named type orders unless PostgreSQL has no operator class for it.
        // `json` has none -- `max(details)` is an error, not an empty result.
        GraphQLType::Custom(name) => !matches!(name.as_str(), "json" | "raster"),
        _ => true,
    }
}

/// The columns each aggregate function applies to, for one table.
///
/// Returns `(function, returns, columns)` and omits any function with no
/// column to apply it to -- a GraphQL type may not have zero fields, so a
/// table of nothing but text has no `sum` type at all rather than an empty
/// one.
pub fn functions_for(object: &TableObjectType) -> Vec<(&'static str, Returns, Vec<String>)> {
    let mut out = Vec::new();

    for (function, returns) in NUMERIC_AGGREGATES {
        let columns: Vec<String> = object
            .fields
            .iter()
            .filter(|f| is_numeric(&f.graphql_type))
            .map(|f| f.name.clone())
            .collect();
        if !columns.is_empty() {
            out.push((*function, *returns, columns));
        }
    }

    for (function, returns) in ORDERED_AGGREGATES {
        let columns: Vec<String> = object
            .fields
            .iter()
            .filter(|f| is_ordered(&f.graphql_type))
            .map(|f| f.name.clone())
            .collect();
        if !columns.is_empty() {
            out.push((*function, *returns, columns));
        }
    }

    out
}

/// The GraphQL type name for one column under one aggregate.
pub fn field_type_for(object: &TableObjectType, column: &str, returns: Returns) -> String {
    match returns {
        Returns::Float => "Float".to_string(),
        Returns::Column => object
            .fields
            .iter()
            .find(|f| f.name == column)
            .map(|f| f.graphql_type.to_string())
            .unwrap_or_else(|| "String".to_string()),
    }
}

/// Which tables get aggregate types.
///
/// All of them: a table with no numeric column still has `count`, which is the
/// aggregate most requests actually want.
pub fn tables(object_types: &HashMap<String, TableObjectType>) -> Vec<&String> {
    let mut names: Vec<&String> = object_types.keys().collect();
    names.sort();
    names
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn names_follow_the_table() {
        assert_eq!(aggregate_type_name("author"), "author_aggregate");
        assert_eq!(
            aggregate_fields_type_name("author"),
            "author_aggregate_fields"
        );
        assert_eq!(
            function_fields_type_name("author", "sum"),
            "author_sum_fields"
        );
    }

    #[test]
    fn only_numbers_are_summed() {
        assert!(is_numeric(&GraphQLType::Int));
        assert!(is_numeric(&GraphQLType::BigDecimal));
        assert!(!is_numeric(&GraphQLType::String));
        assert!(!is_numeric(&GraphQLType::Json));
    }

    #[test]
    fn json_has_no_maximum() {
        // `max(details)` on a jsonb column is an error in PostgreSQL, so the
        // field is not offered rather than offered and failing.
        assert!(!is_ordered(&GraphQLType::Json));
        assert!(!is_ordered(&GraphQLType::Custom("json".to_string())));
        assert!(is_ordered(&GraphQLType::Custom("timestamp".to_string())));
        assert!(is_ordered(&GraphQLType::String));
        assert!(is_ordered(&GraphQLType::DateTime));
    }
}
