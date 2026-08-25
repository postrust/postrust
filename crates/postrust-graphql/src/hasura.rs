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
use std::collections::{HashMap, HashSet};

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
pub fn allow_unused_variables(request: async_graphql::Request) -> async_graphql::Request {
    prepare(None, request).unwrap_or_else(|(request, _)| request)
}

/// Ready a request for execution, and refuse it where async-graphql would not.
///
/// Two passes over the same parsed document, because parsing it twice to do
/// them separately would be the only reason to separate them:
///
/// - variable declarations nothing uses are dropped, since Hasura executes
///   such a document and the specification does not;
/// - variables used where their declared type does not fit are refused, since
///   the specification says so and async-graphql does not.
///
/// `schema` is what the second pass needs -- the types a variable is being
/// used against. Without one only the first pass runs.
///
/// `Err` carries the request back beside the errors, so the caller can answer
/// with them rather than executing.
// The error carries the request back, which is the whole point of it: the
// caller answers with the errors instead of executing, and needs the request
// to do anything else with. Boxing it would hide that.
#[allow(clippy::result_large_err)]
pub fn prepare(
    schema: Option<&async_graphql::dynamic::Schema>,
    request: async_graphql::Request,
) -> Result<async_graphql::Request, (async_graphql::Request, Vec<ServerError>)> {
    let mut request = drop_unused_variables(request);
    if let Some(schema) = schema {
        // Parsed again rather than threaded through: `set_parsed_query` takes
        // the document by value and gives nothing back, and re-parsing a
        // document that has already parsed once is not where the time goes.
        if let Ok(doc) = async_graphql::parser::parse_query(&request.query) {
            let errors = variable_errors(&doc, schema.registry(), &request.variables);
            if !errors.is_empty() {
                return Err((request, errors));
            }
        }
    }
    let _ = &mut request;
    Ok(request)
}

fn drop_unused_variables(mut request: async_graphql::Request) -> async_graphql::Request {
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
            Value::Object(fields) => fields.values().for_each(|field| from_value(field, used)),
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
        DocumentOperations::Multiple(operations) => operations.values_mut().for_each(prune),
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

/// Refuse a variable used where its declared type does not fit.
///
/// The specification's "All Variable Usages Are Allowed" rule: `query
/// ($limit: String) { author(limit: $limit) }` is invalid, because `limit` is
/// an `Int` and a `String` is not one. async-graphql carries exactly this rule
/// and it does not fire -- verified against 7.0.17 with a static schema, a
/// dynamic one, and a built-in directive, none of which report -- so the check
/// is made here instead. It is the one place where being lax is worse than
/// being wrong: the client is answered with data when what it wrote cannot
/// mean what it thinks, and it finds out from the rows.
///
/// The messages are Hasura's, since a client that shows them to a developer is
/// showing text it already ships.
fn variable_errors(
    doc: &async_graphql::parser::types::ExecutableDocument,
    registry: &async_graphql::registry::Registry,
    variables: &async_graphql::Variables,
) -> Vec<ServerError> {
    use async_graphql::parser::types::{DocumentOperations, OperationType};

    let mut errors: Vec<ServerError> = Vec::new();

    let operations: Vec<
        &async_graphql::Positioned<async_graphql::parser::types::OperationDefinition>,
    > = match &doc.operations {
        DocumentOperations::Single(operation) => vec![operation],
        DocumentOperations::Multiple(operations) => operations.values().collect(),
    };

    for operation in operations {
        let root = match operation.node.ty {
            OperationType::Query => Some(registry.query_type.as_str()),
            OperationType::Mutation => registry.mutation_type.as_deref(),
            OperationType::Subscription => registry.subscription_type.as_deref(),
        };
        let Some(root) = root else { continue };

        // What each variable was declared as, and what a null in it means.
        let mut declared: HashMap<&str, String> = HashMap::new();
        for definition in &operation.node.variable_definitions {
            let name = definition.node.name.node.as_str();
            let written = definition.node.var_type.node.to_string();
            // A nullable declaration with a default behaves as a non-null
            // one: the default stands in wherever the variable is left out.
            // A default *of* null does not -- `$author: author_insert_input =
            // null` still cannot be used where a non-null is expected, because
            // what it stands in with is a null.
            let defaulted = definition
                .node
                .default_value
                .as_ref()
                .is_some_and(|value| !matches!(value.node, async_graphql::Value::Null));
            let effective = match (definition.node.var_type.node.nullable, defaulted) {
                (true, true) => format!("{}!", written),
                _ => written.clone(),
            };
            declared.insert(name, effective);

            // An explicit null for a non-null variable. The default does not
            // save it: a default stands in for a variable that was not given,
            // not for one that was given as null.
            let given = variables.get(&async_graphql::Name::new(name));
            if !definition.node.var_type.node.nullable
                && matches!(given, Some(async_graphql::Value::Null))
            {
                errors.push(coded(
                    format!("null value found for non-nullable type: \"{}\"", written),
                    definition.pos,
                ));
            }
        }
        if declared.is_empty() {
            continue;
        }

        let mut scope = Usage {
            registry,
            fragments: &doc.fragments,
            declared: &declared,
            errors: &mut errors,
            seen: HashSet::new(),
        };
        scope.selection_set(&operation.node.selection_set.node, root);
    }

    errors
}

/// One walk of a document, checking every variable against where it is used.
struct Usage<'a> {
    registry: &'a async_graphql::registry::Registry,
    fragments: &'a std::collections::HashMap<
        async_graphql::Name,
        async_graphql::Positioned<async_graphql::parser::types::FragmentDefinition>,
    >,
    declared: &'a HashMap<&'a str, String>,
    errors: &'a mut Vec<ServerError>,
    /// Fragments already walked, so a cycle terminates. async-graphql refuses
    /// cyclic fragments too, but this runs first.
    seen: HashSet<String>,
}

impl Usage<'_> {
    fn selection_set(&mut self, set: &async_graphql::parser::types::SelectionSet, type_name: &str) {
        use async_graphql::parser::types::Selection;
        use async_graphql::registry::MetaTypeName;

        for selection in &set.items {
            match &selection.node {
                Selection::Field(field) => {
                    let meta = self
                        .registry
                        .types
                        .get(type_name)
                        .and_then(|ty| ty.field_by_name(field.node.name.node.as_str()));
                    for (name, value) in &field.node.arguments {
                        let Some(argument) =
                            meta.and_then(|meta| meta.args.get(name.node.as_str()))
                        else {
                            continue;
                        };
                        // A location with a default of its own accepts a
                        // nullable variable where it says non-null, since
                        // leaving the variable out is then the same as not
                        // writing the argument.
                        let expected = relax(&argument.ty, argument.default_value.is_some());
                        self.value(&value.node, &expected, value.pos);
                    }
                    self.directives(&field.node.directives);
                    if let Some(meta) = meta {
                        let inner = MetaTypeName::concrete_typename(&meta.ty).to_string();
                        self.selection_set(&field.node.selection_set.node, &inner);
                    }
                }
                Selection::InlineFragment(fragment) => {
                    let inner = fragment
                        .node
                        .type_condition
                        .as_ref()
                        .map(|condition| condition.node.on.node.to_string())
                        .unwrap_or_else(|| type_name.to_string());
                    self.directives(&fragment.node.directives);
                    self.selection_set(&fragment.node.selection_set.node, &inner);
                }
                Selection::FragmentSpread(spread) => {
                    let name = spread.node.fragment_name.node.to_string();
                    self.directives(&spread.node.directives);
                    if !self.seen.insert(name.clone()) {
                        continue;
                    }
                    if let Some(fragment) = self.fragments.get(&async_graphql::Name::new(&name)) {
                        let on = fragment.node.type_condition.node.on.node.to_string();
                        self.selection_set(&fragment.node.selection_set.node, &on);
                    }
                }
            }
        }
    }

    fn directives(
        &mut self,
        directives: &[async_graphql::Positioned<async_graphql::parser::types::Directive>],
    ) {
        for directive in directives {
            let meta = self
                .registry
                .directives
                .get(directive.node.name.node.as_str());
            for (name, value) in &directive.node.arguments {
                let Some(argument) = meta.and_then(|meta| meta.args.get(name.node.as_str())) else {
                    continue;
                };
                let expected = relax(&argument.ty, argument.default_value.is_some());
                self.value(&value.node, &expected, value.pos);
            }
        }
    }

    /// One written value, against the type of the place it was written in.
    fn value(
        &mut self,
        value: &async_graphql_value::Value,
        expected: &str,
        pos: async_graphql::Pos,
    ) {
        use async_graphql::registry::{MetaType, MetaTypeName};
        use async_graphql_value::Value;

        match value {
            Value::Variable(name) => {
                let Some(declared) = self.declared.get(name.as_str()) else {
                    // Undefined, which async-graphql reports itself.
                    return;
                };
                if !MetaTypeName::create(expected).is_subtype(&MetaTypeName::create(declared)) {
                    self.errors.push(coded(
                        format!(
                            "variable '{}' is declared as '{}', but used where '{}' is expected",
                            name, declared, expected
                        ),
                        pos,
                    ));
                }
            }
            Value::List(items) => {
                // A list may be written where one value is expected, in which
                // case each item is checked against that same type -- which is
                // what list input coercion means.
                let inner = match MetaTypeName::create(expected).unwrap_non_null() {
                    MetaTypeName::List(inner) => inner.to_string(),
                    _ => expected.to_string(),
                };
                for item in items {
                    self.value(item, &inner, pos);
                }
            }
            Value::Object(fields) => {
                let name = MetaTypeName::concrete_typename(expected);
                let Some(MetaType::InputObject { input_fields, .. }) =
                    self.registry.types.get(name)
                else {
                    return;
                };
                for (key, item) in fields {
                    let Some(field) = input_fields.get(key.as_str()) else {
                        continue;
                    };
                    let ty = relax(&field.ty, field.default_value.is_some());
                    self.value(item, &ty, pos);
                }
            }
            _ => {}
        }
    }
}

/// A location type as it accepts a variable, given whether it has a default.
///
/// A non-null location with a default takes a nullable variable: leaving that
/// variable out is the same as not writing the argument, and the default then
/// stands in. Without a default it does not.
fn relax(ty: &str, has_default: bool) -> String {
    match has_default {
        true => ty.strip_suffix('!').unwrap_or(ty).to_string(),
        false => ty.to_string(),
    }
}

/// A validation failure, coded as one.
fn coded(message: String, pos: async_graphql::Pos) -> ServerError {
    let mut error = ServerError::new(message, Some(pos));
    let mut extensions = async_graphql::ErrorExtensionValues::default();
    extensions.set("code", "validation-failed");
    error.extensions = Some(extensions);
    error
}

#[cfg(test)]
mod variable_position_tests {
    use super::*;
    use async_graphql::dynamic::*;

    /// A schema with the shapes the rule has to reason about: a scalar
    /// argument, a nested input object, a list, a non-null location, and a
    /// non-null location that has a default.
    fn schema() -> async_graphql::dynamic::Schema {
        let filter =
            InputObject::new("author_bool_exp").field(InputValue::new("id", TypeRef::named("Int")));
        let insert = InputObject::new("author_insert_input")
            .field(InputValue::new("name", TypeRef::named("String")));
        let query = Object::new("query_root").field(
            Field::new("author", TypeRef::named_nn_list_nn("author"), |_| {
                FieldFuture::new(async move { Ok(Some(async_graphql::Value::List(vec![]))) })
            })
            .argument(InputValue::new("limit", TypeRef::named("Int")))
            .argument(InputValue::new("where", TypeRef::named("author_bool_exp"))),
        );
        let mutation = Object::new("mutation_root")
            .field(
                Field::new("insert_author_one", TypeRef::named("author"), |_| {
                    FieldFuture::new(async move { Ok(None::<async_graphql::Value>) })
                })
                .argument(InputValue::new(
                    "object",
                    TypeRef::named_nn("author_insert_input"),
                )),
            )
            .field(
                Field::new("insert_author", TypeRef::named("author"), |_| {
                    FieldFuture::new(async move { Ok(None::<async_graphql::Value>) })
                })
                .argument(InputValue::new(
                    "objects",
                    TypeRef::named_nn_list_nn("author_insert_input"),
                ))
                .argument(
                    InputValue::new("update_columns", TypeRef::named_nn_list_nn("String"))
                        .default_value(async_graphql::Value::List(Vec::new())),
                ),
            );
        let author = Object::new("author").field(Field::new("id", TypeRef::named("Int"), |_| {
            FieldFuture::new(async move { Ok(None::<async_graphql::Value>) })
        }));
        Schema::build("query_root", Some("mutation_root"), None)
            .register(filter)
            .register(insert)
            .register(author)
            .register(query)
            .register(mutation)
            .finish()
            .expect("the test schema builds")
    }

    fn refusals(query: &str, variables: &str) -> Vec<String> {
        let request =
            async_graphql::Request::new(query).variables(async_graphql::Variables::from_json(
                serde_json::from_str(variables).expect("the variables are JSON"),
            ));
        match prepare(Some(&schema()), request) {
            Ok(_) => Vec::new(),
            Err((_, errors)) => errors.into_iter().map(|e| e.message).collect(),
        }
    }

    #[test]
    fn a_variable_of_the_wrong_type_is_refused() {
        assert_eq!(
            refusals("query ($s: String) { author(limit: $s) { id } }", "{}"),
            vec!["variable 's' is declared as 'String', but used where 'Int' is expected"]
        );
    }

    #[test]
    fn the_check_reaches_inside_an_input_object() {
        assert_eq!(
            refusals(
                "query ($s: String) { author(where: {id: $s}) { id } }",
                "{}"
            ),
            vec!["variable 's' is declared as 'String', but used where 'Int' is expected"]
        );
    }

    #[test]
    fn the_check_reaches_inside_a_list() {
        assert_eq!(
            refusals(
                "mutation ($n: Int) { insert_author(objects: [{name: $n}]) { id } }",
                "{}"
            ),
            vec!["variable 'n' is declared as 'Int', but used where 'String' is expected"]
        );
    }

    #[test]
    fn a_nullable_variable_cannot_fill_a_non_null_place() {
        assert_eq!(
            refusals(
                "mutation ($a: author_insert_input) { insert_author_one(object: $a) { id } }",
                "{}"
            ),
            vec![
                "variable 'a' is declared as 'author_insert_input', but used where \
                 'author_insert_input!' is expected"
            ]
        );
    }

    #[test]
    fn a_default_lets_it() {
        // The default stands in wherever the variable is left out, so the
        // place can never actually see a null.
        assert!(refusals(
            "mutation ($a: author_insert_input = {name: \"x\"}) \
             { insert_author_one(object: $a) { id } }",
            "{}"
        )
        .is_empty());
    }

    #[test]
    fn a_default_of_null_does_not() {
        assert_eq!(
            refusals(
                "mutation ($a: author_insert_input = null) \
                 { insert_author_one(object: $a) { id } }",
                "{}"
            ),
            vec![
                "variable 'a' is declared as 'author_insert_input', but used where \
                 'author_insert_input!' is expected"
            ]
        );
    }

    #[test]
    fn a_default_on_the_place_lets_it_too() {
        // `update_columns` is non-null with a default: not writing the
        // argument is allowed, so a variable that might be absent is too.
        assert!(refusals(
            "mutation ($c: [String!]) { insert_author(objects: [], update_columns: $c) { id } }",
            "{}"
        )
        .is_empty());
    }

    #[test]
    fn a_non_null_variable_fits_a_nullable_place() {
        assert!(refusals(
            "query ($n: Int!) { author(limit: $n) { id } }",
            "{\"n\": 1}"
        )
        .is_empty());
    }

    #[test]
    fn a_non_null_variable_given_null_is_refused() {
        assert_eq!(
            refusals(
                "query ($n: Int! = 1) { author(limit: $n) { id } }",
                "{\"n\": null}"
            ),
            vec!["null value found for non-nullable type: \"Int!\""]
        );
    }

    #[test]
    fn a_nullable_variable_given_null_is_not() {
        assert!(refusals(
            "query ($n: Int) { author(limit: $n) { id } }",
            "{\"n\": null}"
        )
        .is_empty());
    }

    #[test]
    fn a_variable_used_through_a_fragment_is_checked() {
        assert_eq!(
            refusals(
                "query ($s: String) { author { ...rows } } \
                 fragment rows on author { id }",
                "{}"
            ),
            Vec::<String>::new()
        );
        assert_eq!(
            refusals(
                "query ($s: String) { ...roots } \
                 fragment roots on query_root { author(limit: $s) { id } }",
                "{}"
            ),
            vec!["variable 's' is declared as 'String', but used where 'Int' is expected"]
        );
    }
}
