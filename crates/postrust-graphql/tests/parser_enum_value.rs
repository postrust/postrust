//! An enum value is "Name but not `true`, `false` or `null`" (GraphQL spec,
//! EnumValue): the three names, not names that begin with them. The published
//! async-graphql-parser 7.0.17 refused both; this workspace builds a patched
//! copy (vendor/async-graphql-parser). Hasura's
//! `graphql_mutation/insert/nullprefixcolumn` upserts on a constraint called
//! `nullPrefixTestTable_pkey`.

use async_graphql::parser::parse_query;

fn upsert(constraint: &str) -> String {
    format!(
        "mutation {{ insert_t(objects: [{{id: 1}}], on_conflict: \
         {{constraint: {constraint}, update_columns: []}}) {{ affected_rows }} }}"
    )
}

#[test]
fn enum_values_that_merely_begin_with_a_literal_parse() {
    for name in [
        "nullPrefixTestTable_pkey",
        "nullable",
        "null_",
        "true_x",
        "trueish",
        "truestore",
        "falsey",
        "false1",
        "ACTIVE",
        "_x",
    ] {
        parse_query(upsert(name)).unwrap_or_else(|e| panic!("{name} should parse: {e}"));
    }
}

#[test]
fn the_three_literals_are_still_not_enum_values() {
    // As an argument value these are the boolean and null literals, so they
    // parse as such; what must stay refused is an enum value *definition*.
    for name in ["null", "true", "false"] {
        let sdl = format!("enum E {{ {name} }}");
        assert!(
            async_graphql::parser::parse_schema(&sdl).is_err(),
            "{name} must not be an enum value"
        );
    }
    for name in ["nullable", "truestore", "falsey"] {
        let sdl = format!("enum E {{ {name} }}");
        async_graphql::parser::parse_schema(&sdl)
            .unwrap_or_else(|e| panic!("enum member {name} should parse: {e}"));
    }
}
