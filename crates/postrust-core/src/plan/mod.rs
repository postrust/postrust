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
    match &request.payload {
        Some(Payload::ProcessedJson { keys, .. }) => return keys.iter().cloned().collect(),
        Some(Payload::ProcessedUrlEncoded { data, .. }) => {
            return data.iter().map(|(name, _)| name.clone()).collect()
        }
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
    names
}

/// Choose which signature of a function the supplied arguments call.
///
/// A name may carry several, and taking whichever happened to be loaded first
/// calls the wrong one -- or calls one the arguments do not fit, which
/// PostgreSQL then rejects with a message about a function that does not
/// exist, where the truth is that none of them matched.
///
/// A signature fits when every argument names one of its parameters and every
/// parameter it requires is supplied. Among those that fit, the one matching
/// most of its parameters wins; where several match equally the call is
/// ambiguous and saying so is more use than picking one.
fn select_overload<'a>(
    routines: &'a [Routine],
    request: &ApiRequest,
) -> Result<Option<&'a Routine>> {
    // Arguments in a body are not inspected here. The payload is parsed
    // further down, and a mismatch there is PostgreSQL's to report.
    if request.payload.is_some() {
        return Ok(routines.first());
    }

    let supplied = supplied_arguments(request);

    let mut fitting: Vec<&Routine> = routines
        .iter()
        .filter(|routine| {
            supplied
                .iter()
                .all(|name| routine.params.iter().any(|p| &p.name == name))
                && routine
                    .params
                    .iter()
                    .filter(|p| p.required)
                    .all(|p| supplied.contains(&p.name))
        })
        .collect();

    let best = fitting
        .iter()
        .map(|routine| {
            routine
                .params
                .iter()
                .filter(|p| supplied.contains(&p.name))
                .count()
        })
        .max();

    let Some(best) = best else {
        return Ok(None);
    };

    fitting.retain(|routine| {
        routine
            .params
            .iter()
            .filter(|p| supplied.contains(&p.name))
            .count()
            == best
    });

    if fitting.len() > 1 {
        return Err(Error::AmbiguousFunction {
            candidates: fitting.iter().map(|r| signature_of(r)).collect(),
        });
    }

    Ok(fitting.first().copied())
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

            // Over POST the whole body can be one unnamed `json` argument, so
            // a signature taking that would also have matched. The message
            // says which signatures were looked for, so it says so too.
            let single_json = matches!(invoke_method, crate::api_request::InvokeMethod::Inv);

            // The name is reported as it was called, arguments and all, so the
            // client can see which signature was looked for.
            let not_found = |candidate: Option<String>| Error::FunctionNotFound {
                name: qi.to_string(),
                params: supplied.clone(),
                candidate,
                single_json,
            };

            let routines = schema_cache
                .get_routines(qi)
                .ok_or_else(|| not_found(None))?;

            let routine = select_overload(routines, request)?.ok_or_else(|| {
                // The name exists but nothing takes these arguments, so an
                // overload that does exist is worth naming.
                let candidate = routines.first().map(|r| {
                    format!(
                        "{}({})",
                        qi,
                        r.params
                            .iter()
                            .map(|p| p.name.clone())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                });
                not_found(candidate)
            })?;

            let call_plan = CallPlan::from_request(request, routine)?;

            // The rows a function returns are shaped like any other read:
            // selected, filtered, ordered, paged and embedded on. That needs a
            // table to resolve columns against, so it applies exactly when the
            // function returns rows of one this cache knows.
            let read = schema_cache
                .routine_returned_table(qi)
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
