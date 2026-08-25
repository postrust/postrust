//! The response envelope Hasura clients expect.
//!
//! async-graphql answers in the shape the GraphQL specification describes:
//! `data` is always present, errors carry `locations` and a `path` that is a
//! list of segments. Hasura answers in a narrower shape of its own, and client
//! code branches on it:
//!
//! ```json
//! {"errors": [{"message": "...",
//!              "extensions": {"path": "$.selectionSet.author", "code": "validation-failed"}}]}
//! ```
//!
//! Two differences matter to a client. A failed request has no `data` key at
//! all, rather than `"data": null` -- code written as `if (body.data)` reads
//! the two the same way, but code written as `if ('data' in body)` does not.
//! And `extensions.code` is the machine-readable half: it is what a client
//! switches on to tell a permission failure from a constraint violation, and
//! the message text is only for a human.
//!
//! The status code is 200 for all of this. Of the 468 cases in Hasura's own
//! corpus that this server is measured against, 464 expect 200 -- including
//! every permission failure and every constraint violation. A GraphQL error
//! is a value in the response body, not a transport failure.

use async_graphql::{Response, ServerError};
use serde_json::{json, Map, Value};

/// Hasura's `extensions.code` for a server error.
///
/// The code is inferred from the error's own extensions where the resolver
/// set one, and otherwise from the message. Guessing from text is not
/// something to be proud of, but the alternative -- omitting the code -- is
/// worse: a client that switches on it would fall through to its default
/// branch for every error this server produces.
fn code_for(error: &ServerError) -> &'static str {
    if let Some(extensions) = &error.extensions {
        if let Some(Value::String(code)) = extensions.get("code").map(value_of) {
            return match code.as_str() {
                "validation-failed" => "validation-failed",
                "bad-request" => "bad-request",
                "permission-error" => "permission-error",
                "constraint-violation" => "constraint-violation",
                "data-exception" => "data-exception",
                "not-supported" => "not-supported",
                "parse-failed" => "parse-failed",
                _ => "unexpected",
            };
        }
    }

    let message = error.message.to_ascii_lowercase();
    // Validation is tested first: a rejected enum value says "enumeration type
    // ... does not contain the value", which mentions no constraint but reads
    // as one to a later test that only looks for the word.
    if message.contains("invalid value for argument")
        || message.contains("does not contain the value")
        || message.contains("is not defined by operation")
    {
        "validation-failed"
    } else if message.contains("permission") || message.contains("not allowed") {
        "permission-error"
    } else if message.contains("violates") || message.contains("constraint") {
        "constraint-violation"
    } else if message.contains("unknown argument")
        || message.contains("cannot query field")
        || message.contains("expected")
        || message.contains("unknown field")
        // Everything the variable rules say. A document that names a variable
        // it never declared is refused before a resolver runs, so calling it a
        // database error sends a client looking in the wrong place.
        || message.starts_with("variable \"")
    {
        "validation-failed"
    } else {
        "postgres-error"
    }
}

fn value_of(value: &async_graphql::Value) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}

/// Render an error's path the way Hasura spells it.
///
/// Hasura writes a JSONPath-ish string rooted at the operation --
/// `$.selectionSet.author.args.where` -- where the specification writes a list
/// of segments. A list index stays an index; everything else is a field name.
fn path_for(error: &ServerError) -> String {
    use async_graphql::PathSegment;

    if error.path.is_empty() {
        return "$".to_string();
    }
    let mut rendered = String::from("$.selectionSet");
    for segment in &error.path {
        match segment {
            PathSegment::Field(name) => {
                rendered.push('.');
                rendered.push_str(name);
            }
            PathSegment::Index(index) => {
                rendered.push_str(&format!("[{}]", index));
            }
        }
    }
    rendered
}

/// Convert an async-graphql response into Hasura's envelope.
pub fn envelope(response: Response) -> Value {
    if !response.errors.is_empty() {
        let errors: Vec<Value> = response
            .errors
            .iter()
            .map(|error| {
                json!({
                    "message": error.message,
                    "extensions": {
                        "path": path_for(error),
                        "code": code_for(error),
                    }
                })
            })
            .collect();
        // No `data` key. Hasura omits it entirely on failure.
        return json!({ "errors": errors });
    }

    let mut body = Map::new();
    body.insert("data".to_string(), value_of(&response.data));
    Value::Object(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_graphql::{PathSegment, ServerError};
    use pretty_assertions::assert_eq;

    fn error_at(message: &str, path: Vec<PathSegment>) -> ServerError {
        let mut error = ServerError::new(message, None);
        error.path = path;
        error
    }

    #[test]
    fn a_failed_response_has_no_data_key() {
        let mut response = Response::new(async_graphql::Value::Null);
        response.errors = vec![error_at("no", vec![])];
        let body = envelope(response);
        assert!(body.get("data").is_none());
        assert!(body.get("errors").is_some());
    }

    #[test]
    fn a_successful_response_has_no_errors_key() {
        let body = envelope(Response::new(async_graphql::Value::Null));
        assert!(body.get("errors").is_none());
        assert!(body.get("data").is_some());
    }

    #[test]
    fn path_is_rooted_at_the_selection_set() {
        let error = error_at(
            "boom",
            vec![
                PathSegment::Field("insert_author".to_string()),
                PathSegment::Index(0),
                PathSegment::Field("name".to_string()),
            ],
        );
        assert_eq!(path_for(&error), "$.selectionSet.insert_author[0].name");
    }

    #[test]
    fn an_error_with_no_path_is_rooted_at_the_document() {
        assert_eq!(path_for(&error_at("boom", vec![])), "$");
    }

    #[test]
    fn a_validation_message_is_coded_as_one() {
        let error = ServerError::new("Unknown argument \"filter\" on field \"author\"", None);
        assert_eq!(code_for(&error), "validation-failed");
    }

    #[test]
    fn a_constraint_message_is_coded_as_one() {
        let error = ServerError::new("duplicate key value violates unique constraint", None);
        assert_eq!(code_for(&error), "constraint-violation");
    }
}

/// Drop variable definitions the document never uses.
///
/// The specification's "All Variables Used" rule makes
/// `query ($tags: jsonb) { article(where: {tags: {_contains: "latest"}}) { id } }`
/// an invalid document, and async-graphql refuses it. Hasura executes it. A
/// client that has been sending that query for years -- because a filter was
/// edited and the declaration was left behind -- gets an answer from the server
/// it is migrating off and an error from this one, which is the kind of
/// difference a migration discovers in production.
///
/// So the document is parsed here, the unused declarations are removed, and
/// what validation sees has nothing to complain about. Every other rule still
/// runs: this makes one specific refusal go away, not validation in general.
/// A document that does not parse is handed on untouched, so the parse error is
/// reported by the executor with its own position rather than by this.
pub fn allow_unused_variables(mut request: async_graphql::Request) -> async_graphql::Request {
    use async_graphql::parser::types::{
        DocumentOperations, ExecutableDocument, Selection, SelectionSet,
    };
    use async_graphql::Name;
    // The executable `Value`, which has a `Variable` case; `async_graphql::Value`
    // is the constant one a variable has already been substituted into.
    use async_graphql_value::Value;

    let Ok(mut doc): Result<ExecutableDocument, _> =
        async_graphql::parser::parse_query(&request.query)
    else {
        return request;
    };

    fn from_value(value: &Value, used: &mut std::collections::HashSet<Name>) {
        match value {
            Value::Variable(name) => {
                used.insert(name.clone());
            }
            Value::List(items) => items.iter().for_each(|item| from_value(item, used)),
            Value::Object(fields) => {
                fields.values().for_each(|field| from_value(field, used))
            }
            _ => {}
        }
    }

    fn from_selection_set(set: &SelectionSet, used: &mut std::collections::HashSet<Name>) {
        for selection in &set.items {
            let (directives, arguments, nested) = match &selection.node {
                Selection::Field(field) => (
                    &field.node.directives,
                    Some(&field.node.arguments),
                    Some(&field.node.selection_set),
                ),
                Selection::FragmentSpread(spread) => (&spread.node.directives, None, None),
                Selection::InlineFragment(fragment) => (
                    &fragment.node.directives,
                    None,
                    Some(&fragment.node.selection_set),
                ),
            };
            for directive in directives {
                for (_, value) in &directive.node.arguments {
                    from_value(&value.node, used);
                }
            }
            for (_, value) in arguments.into_iter().flatten() {
                from_value(&value.node, used);
            }
            if let Some(nested) = nested {
                from_selection_set(&nested.node, used);
            }
        }
    }

    // Every variable named anywhere in the document, rather than per operation.
    // Over-counting only keeps a declaration that would have been kept before,
    // and a variable declared by one operation and used by another is exactly
    // the case this is here to stop refusing.
    let mut used = std::collections::HashSet::new();
    for (_, operation) in doc.operations.iter() {
        for directive in &operation.node.directives {
            for (_, value) in &directive.node.arguments {
                from_value(&value.node, &mut used);
            }
        }
        from_selection_set(&operation.node.selection_set.node, &mut used);
    }
    for fragment in doc.fragments.values() {
        from_selection_set(&fragment.node.selection_set.node, &mut used);
    }

    let mut dropped = false;
    let mut prune = |operation: &mut async_graphql::Positioned<
        async_graphql::parser::types::OperationDefinition,
    >| {
        let before = operation.node.variable_definitions.len();
        operation
            .node
            .variable_definitions
            .retain(|definition| used.contains(&definition.node.name.node));
        dropped |= operation.node.variable_definitions.len() != before;
    };
    match &mut doc.operations {
        DocumentOperations::Single(operation) => prune(operation),
        DocumentOperations::Multiple(operations) => {
            operations.values_mut().for_each(prune)
        }
    }

    if dropped {
        request.set_parsed_query(doc);
    }
    request
}

#[cfg(test)]
mod unused_variable_tests {
    use super::*;

    fn declarations(query: &str) -> Vec<String> {
        let request = allow_unused_variables(async_graphql::Request::new(query));
        // A request whose declarations were all used is handed on with nothing
        // parsed, so what the executor sees is the source text.
        let mut request = request;
        let doc = request.parsed_query().expect("the query parses");
        doc.operations
            .iter()
            .flat_map(|(_, operation)| operation.node.variable_definitions.iter())
            .map(|definition| definition.node.name.node.to_string())
            .collect()
    }

    #[test]
    fn a_variable_nothing_names_is_dropped() {
        assert_eq!(
            declarations("query ($tags: jsonb) { article(where: {id: {_eq: 1}}) { id } }"),
            Vec::<String>::new()
        );
    }

    #[test]
    fn a_variable_an_argument_names_is_kept() {
        assert_eq!(
            declarations("query ($id: Int) { article(where: {id: {_eq: $id}}) { id } }"),
            vec!["id".to_string()]
        );
    }

    #[test]
    fn a_variable_named_inside_a_list_is_kept() {
        assert_eq!(
            declarations("query ($id: Int) { article(where: {_or: [{id: {_eq: $id}}]}) { id } }"),
            vec!["id".to_string()]
        );
    }

    #[test]
    fn a_variable_only_a_fragment_names_is_kept() {
        assert_eq!(
            declarations(
                "query ($n: Int) { author { ...rows } } \
                 fragment rows on author { articles(limit: $n) { id } }"
            ),
            vec!["n".to_string()]
        );
    }

    #[test]
    fn a_document_that_does_not_parse_is_left_alone() {
        let request = allow_unused_variables(async_graphql::Request::new("query ("));
        assert_eq!(request.query, "query (");
    }
}
