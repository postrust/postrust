//! Boolean expression input types.
//!
//! Hasura's filter argument is `where: <table>_bool_exp` -- a generated input
//! object with one field per column, one per relationship, and `_and`, `_or`
//! and `_not` for structure:
//!
//! ```graphql
//! author(where: {_and: [{id: {_gt: 2}},
//!                       {articles: {title: {_ilike: "%rust%"}}}]}) { id }
//! ```
//!
//! The shape matters more than it looks. A `JSON` argument accepts the same
//! text, but nothing downstream can read it: a client cannot generate types
//! from it, an editor cannot complete it, and a typo in an operator name
//! reaches the server instead of being refused by the client. Every case in
//! Hasura's introspection corpus turns on these types existing, and so does
//! every `graphql-codegen` run a migrating client has in its build.
//!
//! Two naming rules, both taken from what a client has already compiled
//! against rather than from taste:
//!
//! * The table's expression is `<table>_bool_exp`, named for the table
//!   exactly as it is spelled in PostgreSQL. It appears in the client's own
//!   source wherever a query declares a variable.
//! * A column's expression is named for the column's scalar type, so every
//!   `text` column in the schema shares one `String_comparison_exp`.

use crate::schema::relationship::RelationshipField;
use crate::schema::object::TableObjectType;
use crate::types::GraphQLType;
use async_graphql::dynamic::{InputObject, InputValue, TypeRef};
use std::collections::{HashMap, HashSet};

/// Comparisons every type supports.
const UNIVERSAL: &[&str] = &["_eq", "_neq", "_gt", "_gte", "_lt", "_lte"];

/// Comparisons taking a list of the column's own type.
const LIST_VALUED: &[&str] = &["_in", "_nin"];

/// Pattern matching, for text-shaped columns only.
const TEXT: &[&str] = &[
    "_like", "_nlike", "_ilike", "_nilike",
    "_similar", "_nsimilar",
    "_regex", "_iregex", "_nregex", "_niregex",
];

/// Containment and key tests, for `json` and `jsonb` columns only.
const JSON: &[&str] = &["_contains", "_contained_in"];

/// The name of the comparison input for a scalar.
pub fn comparison_type_name(scalar: &str) -> String {
    format!("{}_comparison_exp", scalar)
}

/// The name of a table's boolean expression input.
pub fn bool_exp_type_name(base_name: &str) -> String {
    format!("{}_bool_exp", base_name)
}

/// Build the comparison input for one scalar type.
///
/// `_is_null` takes a Boolean whatever the column is, and the list-valued
/// comparisons take a list of the column's type. Everything else takes one
/// value of it.
fn comparison_input(scalar: &str) -> InputObject {
    let mut input = InputObject::new(comparison_type_name(scalar)).description(format!(
        "Comparisons against a {} column. All fields are combined with AND.",
        scalar
    ));

    for op in UNIVERSAL {
        input = input.field(InputValue::new(*op, TypeRef::named(scalar)));
    }
    for op in LIST_VALUED {
        input = input.field(InputValue::new(*op, TypeRef::named_list(scalar)));
    }
    input = input.field(InputValue::new("_is_null", TypeRef::named("Boolean")));

    if scalar == "String" {
        for op in TEXT {
            input = input.field(InputValue::new(*op, TypeRef::named("String")));
        }
    }
    if scalar == "JSON" {
        for op in JSON {
            input = input.field(InputValue::new(*op, TypeRef::named("JSON")));
        }
        input = input.field(InputValue::new("_has_key", TypeRef::named("String")));
        input = input.field(InputValue::new("_has_keys_any", TypeRef::named_list("String")));
        input = input.field(InputValue::new("_has_keys_all", TypeRef::named_list("String")));
    }

    input
}

/// The scalar a column is compared against.
///
/// An array column is compared against its element type: `_contains` on a
/// `text[]` takes text, not a list of lists. A column of a type with no
/// scalar of its own is compared as a string, which is what the SQL cast in
/// the resolver does with it anyway.
fn scalar_for(graphql_type: &GraphQLType) -> String {
    match graphql_type {
        GraphQLType::List(inner) => scalar_for(inner),
        GraphQLType::Custom(_) => "String".to_string(),
        other => other.to_string(),
    }
}

/// Every input type the boolean expressions need, ready to register.
///
/// Returns the comparison inputs first and the table expressions after, though
/// registration order does not matter to the schema builder -- the types refer
/// to each other by name and are resolved once the whole set is present. That
/// is also what makes a recursive `_and: [author_bool_exp!]` expressible at
/// all.
pub fn build_inputs(
    object_types: &HashMap<String, TableObjectType>,
    relationship_fields: &HashMap<String, Vec<RelationshipField>>,
) -> Vec<InputObject> {
    let mut scalars: HashSet<String> = HashSet::new();
    let mut inputs = Vec::new();

    for (type_name, object) in object_types {
        let bool_exp = bool_exp_type_name(type_name);
        let mut input = InputObject::new(&bool_exp).description(format!(
            "Filter rows of {}. Fields are combined with AND unless _or says otherwise.",
            type_name
        ));

        input = input
            .field(InputValue::new("_and", TypeRef::named_nn_list(&bool_exp)))
            .field(InputValue::new("_or", TypeRef::named_nn_list(&bool_exp)))
            .field(InputValue::new("_not", TypeRef::named(&bool_exp)));

        // Two fields of one name abort the process rather than returning an
        // error, and a foreign key column named after its target -- the
        // ordinary `pizza.crust references crust` -- produces exactly that
        // clash. The column wins here for the same reason it wins in the
        // object type: it is the table's own data, and the two have to agree
        // about which fields exist.
        let mut taken: HashSet<&str> = HashSet::new();

        for field in &object.fields {
            if !taken.insert(field.name.as_str()) {
                continue;
            }
            let scalar = scalar_for(&field.graphql_type);
            input = input.field(InputValue::new(
                &field.name,
                TypeRef::named(comparison_type_name(&scalar)),
            ));
            scalars.insert(scalar);
        }

        // A relationship is filtered by filtering the rows at its other end:
        // `where: {articles: {…}}` keeps the authors that have a matching
        // article.
        for relationship in relationship_fields.get(type_name).into_iter().flatten() {
            if !taken.insert(relationship.name.as_str()) {
                continue;
            }
            input = input.field(InputValue::new(
                &relationship.name,
                TypeRef::named(bool_exp_type_name(&relationship.target_type)),
            ));
        }

        inputs.push(input);
    }

    let mut comparisons: Vec<InputObject> =
        scalars.iter().map(|s| comparison_input(s)).collect();
    comparisons.extend(inputs);
    comparisons
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn a_table_expression_is_named_after_the_table() {
        assert_eq!(bool_exp_type_name("author"), "author_bool_exp");
    }

    #[test]
    fn a_comparison_is_named_after_the_scalar() {
        assert_eq!(comparison_type_name("String"), "String_comparison_exp");
    }

    #[test]
    fn an_array_column_is_compared_against_its_element_type() {
        let list = GraphQLType::List(Box::new(GraphQLType::String));
        assert_eq!(scalar_for(&list), "String");
    }

    #[test]
    fn a_type_with_no_scalar_of_its_own_is_compared_as_text() {
        let custom = GraphQLType::Custom("geography".to_string());
        assert_eq!(scalar_for(&custom), "String");
    }
}
