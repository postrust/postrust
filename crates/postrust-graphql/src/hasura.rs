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
    if message.contains("permission") || message.contains("not allowed") {
        "permission-error"
    } else if message.contains("violates") || message.contains("constraint") {
        "constraint-violation"
    } else if message.contains("unknown argument")
        || message.contains("cannot query field")
        || message.contains("expected")
        || message.contains("unknown field")
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
