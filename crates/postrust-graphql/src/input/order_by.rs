//! Ordering and distinct input types.
//!
//! Hasura orders with `order_by: [<table>_order_by!]` where each entry maps a
//! column to a direction drawn from one shared enum:
//!
//! ```graphql
//! author(order_by: [{name: asc_nulls_last}, {id: desc}]) { id }
//! ```
//!
//! A list rather than a single object, because ordering is ordered: `{name:
//! asc, id: desc}` is one object whose two keys have no defined precedence,
//! and the client that wrote it meant name first.
//!
//! The direction enum carries null placement, which the previous
//! `orderBy: ["name.asc"]` strings could not express at all. In PostgreSQL
//! nulls sort last ascending and first descending, so `asc` and
//! `asc_nulls_last` mean the same thing and `asc_nulls_first` does not --
//! which is exactly the case a client reaches for the enum to say.
//!
//! `distinct_on` takes columns from a generated enum rather than strings, for
//! the same reason the boolean expressions do: an unknown column should be
//! refused by the client, not by the database.

use crate::schema::object::TableObjectType;
use crate::schema::relationship::RelationshipField;
use async_graphql::dynamic::{Enum, EnumItem, InputObject, InputValue, TypeRef};
use std::collections::{HashMap, HashSet};

/// The name of the shared direction enum.
pub const DIRECTION_ENUM: &str = "order_by";

/// Every direction, with its SQL.
const DIRECTIONS: &[(&str, &str)] = &[
    ("asc", "ASC"),
    ("asc_nulls_first", "ASC NULLS FIRST"),
    ("asc_nulls_last", "ASC NULLS LAST"),
    ("desc", "DESC"),
    ("desc_nulls_first", "DESC NULLS FIRST"),
    ("desc_nulls_last", "DESC NULLS LAST"),
];

/// The name of a table's ordering input.
pub fn order_by_type_name(base_name: &str) -> String {
    format!("{}_order_by", base_name)
}

/// The name of a table's column enum.
pub fn select_column_type_name(base_name: &str) -> String {
    format!("{}_select_column", base_name)
}

/// The name of the input for ordering a row by an aggregate of its children.
pub fn aggregate_order_by_type_name(base_name: &str) -> String {
    format!("{}_aggregate_order_by", base_name)
}

/// The name of the input for ordering by one aggregate function's results.
pub fn function_order_by_type_name(base_name: &str, function: &str) -> String {
    format!("{}_{}_order_by", base_name, function)
}

/// The SQL for a direction, or `None` if it is not one.
pub fn direction_sql(name: &str) -> Option<&'static str> {
    DIRECTIONS
        .iter()
        .find(|(enum_name, _)| *enum_name == name)
        .map(|(_, sql)| *sql)
}

/// The shared direction enum.
pub fn direction_enum() -> Enum {
    let mut direction = Enum::new(DIRECTION_ENUM)
        .description("Sort direction, with where nulls go. `asc` is `asc_nulls_last`.");
    for (name, _) in DIRECTIONS {
        direction = direction.item(EnumItem::new(*name));
    }
    direction
}

/// The ordering input and column enum for every table.
pub fn build_inputs(
    object_types: &HashMap<String, TableObjectType>,
    relationship_fields: &HashMap<String, Vec<RelationshipField>>,
) -> (Vec<InputObject>, Vec<Enum>) {
    let mut inputs = Vec::new();
    let mut enums = vec![direction_enum()];

    for (type_name, object) in object_types {
        // A table with no columns would produce an enum with no members, which
        // no GraphQL schema may contain.
        if object.fields.is_empty() {
            continue;
        }

        let mut input = InputObject::new(order_by_type_name(type_name)).description(format!(
            "Order rows of {} by a column, by a related row's column, or by an \
             aggregate of its children.",
            type_name
        ));
        let mut columns = Enum::new(select_column_type_name(type_name))
            .description(format!("A column of {}.", type_name));

        let mut taken: HashSet<String> = HashSet::new();
        for field in &object.fields {
            if !taken.insert(field.name.clone()) {
                continue;
            }
            input = input.field(InputValue::new(
                &field.name,
                TypeRef::named(DIRECTION_ENUM),
            ));
            columns = columns.item(EnumItem::new(&field.name));
        }

        // Ordering by something the row points at. One row contributes a
        // column, so ordering by it is ordering by that column; many rows
        // contribute a count or a statistic, so ordering by them is ordering
        // by an aggregate. That is the whole difference, and it is why the two
        // sides take different inputs.
        for relationship in relationship_fields.get(type_name).into_iter().flatten() {
            if relationship.is_list {
                let field = format!("{}_aggregate", relationship.name);
                if taken.insert(field.clone()) {
                    input = input.field(InputValue::new(
                        &field,
                        TypeRef::named(aggregate_order_by_type_name(&relationship.target_type)),
                    ));
                }
            } else if taken.insert(relationship.name.clone()) {
                input = input.field(InputValue::new(
                    &relationship.name,
                    TypeRef::named(order_by_type_name(&relationship.target_type)),
                ));
            }
        }

        inputs.push(input);
        enums.push(columns);

        // What a parent may order its children by.
        let mut aggregate = InputObject::new(aggregate_order_by_type_name(type_name))
            .description(format!("Order by an aggregate of {} rows.", type_name))
            .field(InputValue::new("count", TypeRef::named(DIRECTION_ENUM)));

        for (function, _, columns) in crate::schema::aggregate::functions_for(object) {
            let function_type = function_order_by_type_name(type_name, function);
            let mut per_column = InputObject::new(&function_type).description(format!(
                "Order by the `{}` of a {} column.",
                function, type_name
            ));
            for column in &columns {
                per_column = per_column
                    .field(InputValue::new(column, TypeRef::named(DIRECTION_ENUM)));
            }
            inputs.push(per_column);
            aggregate = aggregate
                .field(InputValue::new(function, TypeRef::named(&function_type)));
        }
        inputs.push(aggregate);
    }

    (inputs, enums)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn a_tables_ordering_is_named_after_it() {
        assert_eq!(order_by_type_name("author"), "author_order_by");
        assert_eq!(select_column_type_name("author"), "author_select_column");
    }

    #[test]
    fn plain_asc_places_nulls_where_postgres_does() {
        // Not `ASC NULLS LAST` spelled out: the point is that the default and
        // the explicit spelling produce the same order, so the plain form is
        // left to the database.
        assert_eq!(direction_sql("asc"), Some("ASC"));
        assert_eq!(direction_sql("asc_nulls_last"), Some("ASC NULLS LAST"));
        assert_eq!(direction_sql("asc_nulls_first"), Some("ASC NULLS FIRST"));
    }

    #[test]
    fn a_direction_that_is_not_one_has_no_sql() {
        assert_eq!(direction_sql("sideways"), None);
        assert_eq!(direction_sql("ASC"), None);
    }
}
