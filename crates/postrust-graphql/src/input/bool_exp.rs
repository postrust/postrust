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

/// Spatial relations taking one other shape, for `geometry` columns.
///
/// PostGIS names them `ST_Contains` and so on; Hasura spells each in the same
/// lower-cased shape as every other comparison, and a client sends what Hasura
/// spelled.
const GEOMETRY: &[&str] = &[
    "_st_contains",
    "_st_crosses",
    "_st_equals",
    "_st_intersects",
    "_st_3d_intersects",
    "_st_overlaps",
    "_st_touches",
    "_st_within",
];

/// The PostGIS function behind each of those, and behind the ones that take
/// more than a shape.
pub fn postgis_function(operator: &str) -> Option<&'static str> {
    Some(match operator {
        "_st_contains" => "ST_Contains",
        "_st_crosses" => "ST_Crosses",
        "_st_equals" => "ST_Equals",
        "_st_intersects" => "ST_Intersects",
        "_st_3d_intersects" => "ST_3DIntersects",
        "_st_overlaps" => "ST_Overlaps",
        "_st_touches" => "ST_Touches",
        "_st_within" => "ST_Within",
        "_st_d_within" => "ST_DWithin",
        "_st_3d_d_within" => "ST_3DDWithin",
        "_st_intersects_rast" => "ST_Intersects",
        "_st_intersects_geom_nband" => "ST_Intersects",
        "_st_intersects_nband_geom" => "ST_Intersects",
        _ => return None,
    })
}

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
    // A shape is compared by how it lies against another shape, not by
    // ordering: `_gt` on a polygon means nothing, and PostGIS answers every
    // real question with a function instead.
    if scalar == "geometry" || scalar == "geography" {
        if scalar == "geometry" {
            for op in GEOMETRY {
                input = input.field(InputValue::new(*op, TypeRef::named("geometry")));
            }
        } else {
            input = input.field(InputValue::new("_st_intersects", TypeRef::named("geography")));
        }
        input = input.field(InputValue::new(
            "_st_d_within",
            TypeRef::named(format!("st_d_within_{}_input", scalar)),
        ));
        // A geometry and a geography answer different questions about the same
        // shape -- one on a plane, one on a sphere -- so a client asks for the
        // other by casting the column rather than by keeping two of them.
        input = input.field(InputValue::new(
            "_cast",
            TypeRef::named(format!("{}_cast_exp", scalar)),
        ));
        if scalar == "geometry" {
            input = input.field(InputValue::new(
                "_st_3d_d_within",
                TypeRef::named("st_d_within_geometry_input"),
            ));
        }
    }
    if scalar == "raster" {
        input = input
            .field(InputValue::new("_st_intersects_rast", TypeRef::named("raster")))
            .field(InputValue::new(
                "_st_intersects_geom_nband",
                TypeRef::named("st_intersects_geom_nband_input"),
            ))
            .field(InputValue::new(
                "_st_intersects_nband_geom",
                TypeRef::named("st_intersects_nband_geom_input"),
            ));
    }
    if scalar == "jsonb" || scalar == "json" {
        for op in JSON {
            input = input.field(InputValue::new(*op, TypeRef::named(scalar)));
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
/// `text[]` takes text, not a list of lists. Everything else compares as
/// itself, including the types that carry their own PostgreSQL name -- a
/// `geometry` column takes a `geometry`, and a client that declares
/// `$area: geometry!` is naming the type this produces.
fn scalar_for(graphql_type: &GraphQLType) -> String {
    match graphql_type {
        GraphQLType::List(inner) => scalar_for(inner),
        other => other.to_string(),
    }
}

/// Every input type the boolean expressions need, ready to register, and every
/// scalar those inputs name.
///
/// The second half is not a convenience. A comparison input names scalars its
/// own table may not have a column of -- a cast from a geometry names
/// `geography`, a raster comparison names `geometry` -- and a type the schema
/// mentions and never registers makes the whole schema unbuildable. Reading
/// the names off what was actually generated is the only way that cannot drift
/// from the truth; twice it was patched case by case, and twice it broke again
/// one operator later.
///
/// The inputs come back with the comparisons first and the table expressions
/// after, though registration order does not matter to the schema builder --
/// the types refer to each other by name and are resolved once the whole set
/// is present, which is also what makes a recursive `_and: [author_bool_exp!]`
/// expressible at all.
pub fn build_inputs(
    object_types: &HashMap<String, TableObjectType>,
    relationship_fields: &HashMap<String, Vec<RelationshipField>>,
) -> (Vec<InputObject>, HashSet<String>) {
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

    // A cast from one shape names the other's comparison input, so a schema
    // with only one of them still needs both.
    if scalars.contains("geometry") {
        scalars.insert("geography".to_string());
    }
    if scalars.contains("geography") {
        scalars.insert("geometry".to_string());
    }

    // Every scalar named by anything generated here, which is more than the
    // scalars the columns are: a comparison may name another shape's type.
    let mut named: HashSet<String> = scalars.clone();
    if scalars.contains("raster") {
        named.insert("geometry".to_string());
    }

    let mut comparisons: Vec<InputObject> =
        scalars.iter().map(|s| comparison_input(s)).collect();

    // The comparisons that take more than one value need an input of their
    // own. Registered only where a column of that shape exists, so a schema
    // without PostGIS carries none of them.
    if scalars.contains("geometry") || scalars.contains("geography") {
        for shape in ["geometry", "geography"] {
            if !scalars.contains(shape) {
                continue;
            }
            comparisons.push(
                InputObject::new(format!("st_d_within_{}_input", shape))
                    .description("Within a distance of another shape.")
                    .field(InputValue::new(
                        "distance",
                        TypeRef::named_nn(TypeRef::FLOAT),
                    ))
                    .field(InputValue::new("from", TypeRef::named_nn(shape))),
            );
        }
    }
    // What each shape may be compared as.
    if scalars.contains("geometry") {
        comparisons.push(
            InputObject::new("geometry_cast_exp")
                .description("Compare a geometry column as another type.")
                .field(InputValue::new(
                    "geography",
                    TypeRef::named(comparison_type_name("geography")),
                )),
        );
    }
    if scalars.contains("geography") {
        comparisons.push(
            InputObject::new("geography_cast_exp")
                .description("Compare a geography column as another type.")
                .field(InputValue::new(
                    "geometry",
                    TypeRef::named(comparison_type_name("geometry")),
                )),
        );
    }
    if scalars.contains("raster") {
        comparisons.push(
            InputObject::new("st_intersects_geom_nband_input")
                .description("Intersecting a shape, optionally in one band.")
                .field(InputValue::new("geommin", TypeRef::named_nn("geometry")))
                .field(InputValue::new("nband", TypeRef::named(TypeRef::INT))),
        );
        comparisons.push(
            InputObject::new("st_intersects_nband_geom_input")
                .description("Intersecting a shape in a given band.")
                .field(InputValue::new("nband", TypeRef::named_nn(TypeRef::INT)))
                .field(InputValue::new("geommin", TypeRef::named_nn("geometry"))),
        );
    }
    comparisons.extend(inputs);
    (comparisons, named)
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
    fn a_type_with_its_own_name_compares_as_itself() {
        let custom = GraphQLType::Custom("geography".to_string());
        assert_eq!(scalar_for(&custom), "geography");
    }
}
