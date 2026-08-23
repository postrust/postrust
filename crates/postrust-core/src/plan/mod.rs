//! Query planning module.
//!
//! Converts parsed API requests into execution plans that can be
//! translated to SQL queries.

mod call_plan;
mod mutate_plan;
mod read_plan;
mod types;

pub use call_plan::{CallParams, CallPlan};
pub use mutate_plan::MutatePlan;
pub use read_plan::{ReadPlan, ReadPlanTree};
pub use types::*;

use crate::api_request::{Action, ApiRequest, DbAction, QualifiedIdentifier};
use crate::error::{Error, Result};
use crate::schema_cache::{Routine, SchemaCache};
use std::collections::HashSet;

/// The execution plan for an API request.
#[derive(Clone, Debug)]
pub enum ActionPlan {
    /// Plan that requires database access
    Db(DbActionPlan),
    /// Plan that doesn't need database (OPTIONS, OpenAPI)
    Info(InfoPlan),
}

/// Database action plan.
#[derive(Clone, Debug)]
pub enum DbActionPlan {
    /// Read operation (SELECT)
    Read(ReadPlanTree),
    /// Mutation operation (INSERT/UPDATE/DELETE)
    MutateRead {
        mutate: MutatePlan,
        read: Option<ReadPlanTree>,
    },
    /// RPC call
    Call {
        call: CallPlan,
        read: Option<ReadPlanTree>,
    },
}

/// Info-only plan (no database access needed).
#[derive(Clone, Debug)]
pub enum InfoPlan {
    /// OPTIONS on a table
    RelationInfo(QualifiedIdentifier),
    /// OPTIONS on a function
    RoutineInfo(QualifiedIdentifier),
    /// OpenAPI spec
    OpenApiSpec,
}

/// Create an action plan from an API request.
pub fn create_action_plan(request: &ApiRequest, schema_cache: &SchemaCache) -> Result<ActionPlan> {
    match &request.action {
        Action::Db(db_action) => {
            // SchemaRead is a special case - it returns OpenAPI spec, not a DB query
            if matches!(db_action, DbAction::SchemaRead { .. }) {
                return Ok(ActionPlan::Info(InfoPlan::OpenApiSpec));
            }
            validate_embed_paths(request)?;
            let plan = create_db_plan(request, db_action, schema_cache)?;
            Ok(ActionPlan::Db(plan))
        }
        Action::RelationInfo(qi) => Ok(ActionPlan::Info(InfoPlan::RelationInfo(qi.clone()))),
        Action::RoutineInfo { qi, .. } => Ok(ActionPlan::Info(InfoPlan::RoutineInfo(qi.clone()))),
        Action::SchemaInfo => Ok(ActionPlan::Info(InfoPlan::OpenApiSpec)),
    }
}

/// The overload whose parameters most resemble the ones supplied.
///
/// Compared as one string of sorted names -- `a, b, c` -- because what a
/// client gets wrong is usually one name out of several, and comparing the
/// lists as a whole is what notices that. `None` when nothing is close enough
/// to be worth saying; a bad guess is worse than silence.
fn nearest_overload(
    qi: &QualifiedIdentifier,
    routines: &[crate::schema_cache::Routine],
    supplied: &[String],
) -> Option<String> {
    // No floor at all, unlike a name: the nearest signature that shares any
    // character sequence with what was asked for is offered. A client that got
    // one parameter of three wrong is not close by any similarity measure --
    // `(a, b)` against `(a, b, smthelse)` scores a third -- and naming a
    // parameter list discloses nothing, since the client is trying to write
    // one. A name is different: there a bad guess names an object the client
    // did not know about, which is why that hint keeps its floor.
    //
    // Fitted against every hint the reference gives rather than reasoned from:
    // a floor of a third answers seven of the eight correctly, which is
    // exactly the kind of nearly-right that looks settled.
    const MIN_SIMILARITY: f64 = 0.0;

    // Compared as the parameter list is written -- parentheses included --
    // because that is what the client sees and what it would have to change.
    let listed = |names: &mut Vec<String>| -> String {
        names.sort();
        format!("({})", names.join(", "))
    };
    let asked = listed(&mut supplied.to_vec());

    let candidates: Vec<String> = routines
        .iter()
        .map(|routine| listed(&mut routine.params.iter().map(|p| p.name.clone()).collect()))
        .collect();

    crate::schema_cache::closest(
        candidates.iter().map(String::as_str),
        &asked,
        MIN_SIMILARITY,
    )
    .map(|params| format!("{}{}", qi, params))
}

/// Check that every dotted filter, order or range names a resource that was
/// embedded.
///
/// `?non_existent_projects.name=like.*x*` looks like a filter on an embedded
/// resource, and answering it as though the prefix meant nothing would return
/// the whole table with no sign that the filter was dropped. PostgREST reports
/// the first segment it cannot account for, which is the one the client got
/// wrong.
fn validate_embed_paths(request: &ApiRequest) -> Result<()> {
    use crate::api_request::SelectItem;

    // An embed answers to the alias the request gave it and to the relation's
    // own name: `the_tasks:tasks(*)` is addressed by either.
    fn descend<'a>(items: &'a [SelectItem], segment: &str) -> Option<&'a [SelectItem]> {
        items.iter().find_map(|item| match item {
            SelectItem::Relation {
                relation,
                alias,
                select,
                ..
            } if relation == segment || alias.as_deref() == Some(segment) => {
                Some(select.as_slice())
            }
            SelectItem::SpreadRelation {
                relation, select, ..
            } if relation == segment => Some(select.as_slice()),
            _ => None,
        })
    }

    let check = |path: &[String]| -> Result<()> {
        let mut items = request.query_params.select.as_slice();
        for segment in path {
            items = descend(items, segment)
                .ok_or_else(|| Error::NotAnEmbeddedResource(segment.clone()))?;
        }
        Ok(())
    };

    for (path, _) in &request.query_params.filters {
        check(path)?;
    }
    for (path, _) in &request.query_params.order {
        check(path)?;
    }
    for (path, _) in &request.query_params.logic {
        check(path)?;
    }
    for path in request.query_params.ranges.keys() {
        if !path.is_empty() {
            check(&path.split('.').map(String::from).collect::<Vec<_>>())?;
        }
    }

    Ok(())
}

/// The arguments a request supplies to a function.
///
/// On a function call every query parameter is a candidate argument, but a
/// value carrying an operator -- `id=gt.1` -- is a filter over the result
/// instead. That is what lets `/rpc/getallprojects?id=gt.1` call a function
/// taking nothing at all, while `/rpc/add_them?a=1&b=2&smthelse=x` is a call
/// with an argument no signature declares.
fn supplied_arguments(request: &ApiRequest) -> Vec<String> {
    use crate::api_request::Payload;

    // Over POST the arguments are the body's keys, and the query string holds
    // filters over the result. Reading only the query string there picked the
    // signature that takes nothing at all.
    //
    // `?columns=` overrides them: it says which of the body's keys are
    // arguments, so a body carrying more than the call needs still names one
    // signature rather than none.
    let is_post = matches!(
        request.action,
        Action::Db(DbAction::Routine {
            invoke_method: crate::api_request::InvokeMethod::Inv,
            ..
        })
    );
    if is_post {
        if let Some(columns) = &request.query_params.columns {
            let mut names: Vec<String> = columns.iter().cloned().collect();
            names.sort();
            return names;
        }
    }

    // A name is reported and matched as a set, so the order the body or the
    // query string happened to use is not part of it.
    let sorted = |mut names: Vec<String>| -> Vec<String> {
        names.sort();
        names.dedup();
        names
    };

    match &request.payload {
        Some(Payload::ProcessedJson { keys, .. }) => return sorted(keys.iter().cloned().collect()),
        Some(Payload::ProcessedUrlEncoded { data, .. }) => {
            return sorted(data.iter().map(|(name, _)| name.clone()).collect())
        }
        // A raw body is one unnamed argument, so it names no parameter at all.
        Some(Payload::RawJson(_)) | Some(Payload::RawPayload(_)) => return Vec::new(),
        None if is_post => return Vec::new(),
        _ => {}
    }

    // A filter on an embedded resource is keyed by its path -- `clients.id`
    // -- and is no more an argument than a filter on the result itself.
    let embedded: HashSet<String> = request
        .query_params
        .filters
        .iter()
        .map(|(path, filter)| {
            let mut key = path.join(".");
            key.push('.');
            key.push_str(&filter.field.name);
            key
        })
        .collect();

    let mut names: Vec<String> = Vec::new();
    for (name, value) in &request.query_params.params {
        if crate::api_request::value_is_filter(value) || embedded.contains(name) {
            continue;
        }
        if !names.contains(name) {
            names.push(name.clone());
        }
    }
    names.sort();
    names
}

/// Choose which signature of a function the supplied arguments call.
///
/// A name may carry several, and taking whichever happened to be loaded first
/// calls the wrong one -- or calls one the arguments do not fit, which
/// PostgreSQL then rejects with a message about a function that does not
/// exist, where the truth is that none of them matched.
///
/// Two kinds of signature can answer a call. One names its parameters, and
/// fits when the arguments supplied are exactly its required ones plus any of
/// its optional ones. The other has a single parameter with no name, and takes
/// the whole request body as that one argument -- so it fits whatever the body
/// says, and only when the body's media type matches the parameter's type.
///
/// A named match always wins; the unnamed one is what a call falls back to. If
/// nothing is left the function was not found, and if more than one remains at
/// the same level the call is ambiguous -- saying so is more use than picking
/// one.
fn select_overload<'a>(
    routines: &'a [Routine],
    request: &ApiRequest,
) -> Result<Option<&'a Routine>> {
    let supplied = supplied_arguments(request);
    let is_post = matches!(
        request.action,
        Action::Db(DbAction::Routine {
            invoke_method: crate::api_request::InvokeMethod::Inv,
            ..
        })
    );

    let (named, unnamed): (Vec<&Routine>, Vec<&Routine>) =
        routines
            .iter()
            .fold((Vec::new(), Vec::new()), |mut acc, r| {
                if matches_params(r, &supplied, is_post, &request.content_media_type) {
                    acc.0.push(r);
                } else if takes_whole_body(r, is_post, &request.content_media_type) {
                    acc.1.push(r);
                }
                acc
            });

    let candidates = match named.is_empty() {
        true => unnamed,
        false => named,
    };

    match candidates.len() {
        0 => Ok(None),
        1 => Ok(candidates.first().copied()),
        _ => Err(Error::AmbiguousFunction {
            candidates: candidates.iter().map(|r| signature_of(r)).collect(),
        }),
    }
}

/// Whether a signature's parameters are exactly what the request supplied.
///
/// Required parameters must all be given and optional ones may be; an argument
/// naming no parameter at all disqualifies the signature, which is what makes
/// `add_them(a, b)` not answer a call passing `a`, `b` and `smthelse`.
fn matches_params(
    routine: &Routine,
    supplied: &[String],
    is_post: bool,
    content: &crate::api_request::MediaType,
) -> bool {
    if routine.params.is_empty() {
        // A body posted as text, xml or bytes is one unnamed argument. A
        // signature taking nothing cannot receive it, however empty the
        // argument list looks.
        return supplied.is_empty() && !(is_post && is_raw_body_type(content));
    }

    let declares = |name: &String, required: bool| {
        routine
            .params
            .iter()
            .any(|p| &p.name == name && p.required == required)
    };

    routine
        .params
        .iter()
        .filter(|p| p.required)
        .all(|p| supplied.contains(&p.name))
        && supplied
            .iter()
            .all(|name| declares(name, true) || declares(name, false))
}

/// Whether a signature is the single unnamed parameter that takes the body.
///
/// Only over `POST`, and only where the body's media type is the one that
/// parameter's type can receive: a `json` parameter takes a JSON body, a
/// `text` one a plain-text body, and so on. A GET has no body to pass.
fn takes_whole_body(
    routine: &Routine,
    is_post: bool,
    content: &crate::api_request::MediaType,
) -> bool {
    use crate::api_request::MediaType;

    if !is_post {
        return false;
    }
    let [param] = routine.params.as_slice() else {
        return false;
    };
    if !param.name.is_empty() {
        return false;
    }
    matches!(
        (content, param.param_type.as_str()),
        (MediaType::ApplicationJson, "json" | "jsonb")
            | (MediaType::TextPlain, "text")
            | (MediaType::TextXml, "xml")
            | (MediaType::OctetStream, "bytea")
    )
}

/// The type a single unnamed parameter would have, for a body of this type.
///
/// `None` where no such parameter could take the body at all, which is every
/// media type that is neither JSON nor one of the three that carry a single
/// value.
fn single_param_type(content: &crate::api_request::MediaType) -> Option<String> {
    use crate::api_request::MediaType;
    match content {
        MediaType::ApplicationJson => Some(crate::error::JSON_PARAM.to_string()),
        MediaType::TextPlain => Some("text".to_string()),
        MediaType::TextXml => Some("xml".to_string()),
        MediaType::OctetStream => Some("bytea".to_string()),
        _ => None,
    }
}

/// Media types whose body is one value rather than a set of named arguments.
fn is_raw_body_type(content: &crate::api_request::MediaType) -> bool {
    use crate::api_request::MediaType;
    matches!(
        content,
        MediaType::TextPlain | MediaType::TextXml | MediaType::OctetStream
    )
}

/// A function's signature as PostgREST prints it when naming candidates.
fn signature_of(routine: &Routine) -> String {
    format!(
        "{}.{}({})",
        routine.schema,
        routine.name,
        routine
            .params
            .iter()
            .map(|p| format!("{} => {}", p.name, p.param_type))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

/// Create a database action plan.
fn create_db_plan(
    request: &ApiRequest,
    action: &DbAction,
    schema_cache: &SchemaCache,
) -> Result<DbActionPlan> {
    match action {
        DbAction::RelationRead { qi, .. } => {
            let table = schema_cache.require_table(qi)?;
            let read_plan = ReadPlan::from_request(request, table, schema_cache)?;
            Ok(DbActionPlan::Read(ReadPlanTree::leaf(read_plan)))
        }

        DbAction::RelationMut { qi, mutation } => {
            let table = schema_cache.require_table(qi)?;
            let mutate_plan = MutatePlan::from_request(request, table, mutation, schema_cache)?;

            let read_plan = if request.preferences.representation.needs_body() {
                let rp = ReadPlan::for_mutation(request, table, schema_cache)?;
                Some(ReadPlanTree::leaf(rp))
            } else {
                None
            };

            Ok(DbActionPlan::MutateRead {
                mutate: mutate_plan,
                read: read_plan,
            })
        }

        DbAction::Routine { qi, invoke_method } => {
            let supplied = supplied_arguments(request);

            // Over POST the whole body can be one unnamed argument, and the
            // body's media type decides which type that parameter could be.
            // The message says which signatures were looked for, so it says
            // so too.
            let single_param = match invoke_method {
                crate::api_request::InvokeMethod::Inv => {
                    single_param_type(&request.content_media_type)
                }
                _ => None,
            };

            // The name is reported as it was called, arguments and all, so the
            // client can see which signature was looked for.
            let not_found = |candidate: Option<String>| Error::FunctionNotFound {
                name: qi.to_string(),
                params: supplied.clone(),
                candidate,
                single_param: single_param.clone(),
            };

            // Nothing of that name: the client most likely misspelled it, so
            // the nearest name in the schema is worth offering.
            let routines = schema_cache
                .get_routines(qi)
                .ok_or_else(|| not_found(schema_cache.similar_routine(qi)))?;

            let routine = select_overload(routines, request)?.ok_or_else(|| {
                // The name exists but nothing takes these arguments, so the
                // overload whose parameters most resemble the ones supplied is
                // the one to offer -- not simply the first, which says nothing
                // about what the client asked for.
                not_found(nearest_overload(qi, routines, &supplied))
            })?;

            let call_plan = CallPlan::from_request(request, routine)?;

            // The rows a function returns are shaped like any other read:
            // selected, filtered, ordered, paged and embedded on. That needs a
            // table to resolve columns against, so it applies exactly when the
            // function returns rows of one this cache knows.
            //
            // The table is the one *this* overload returns: a name may carry
            // several signatures returning different things, and taking the
            // first would shape the result of one call by the columns of
            // another.
            let read = schema_cache
                .returned_table(routine)
                .map(|table| ReadPlan::from_request(request, table, schema_cache))
                .transpose()?
                .map(ReadPlanTree::leaf);

            Ok(DbActionPlan::Call {
                call: call_plan,
                read,
            })
        }

        DbAction::SchemaRead { .. } => {
            // This case is handled in create_action_plan before calling create_db_plan
            unreachable!("SchemaRead should be handled in create_action_plan")
        }
    }
}

impl crate::api_request::PreferRepresentation {
    /// Check if response body is needed.
    pub fn needs_body(&self) -> bool {
        matches!(self, Self::Full)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_info_plan() {
        let qi = QualifiedIdentifier::new("public", "users");
        let plan = ActionPlan::Info(InfoPlan::RelationInfo(qi.clone()));

        match plan {
            ActionPlan::Info(InfoPlan::RelationInfo(q)) => {
                assert_eq!(q.name, "users");
            }
            _ => panic!("Expected RelationInfo"),
        }
    }
}
