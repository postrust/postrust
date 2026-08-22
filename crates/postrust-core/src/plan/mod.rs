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
            let plan = create_db_plan(request, db_action, schema_cache)?;
            Ok(ActionPlan::Db(plan))
        }
        Action::RelationInfo(qi) => Ok(ActionPlan::Info(InfoPlan::RelationInfo(qi.clone()))),
        Action::RoutineInfo { qi, .. } => Ok(ActionPlan::Info(InfoPlan::RoutineInfo(qi.clone()))),
        Action::SchemaInfo => Ok(ActionPlan::Info(InfoPlan::OpenApiSpec)),
    }
}

/// Choose which signature of a function the supplied arguments call.
///
/// A name may carry several, and taking whichever happened to be loaded first
/// calls the wrong one -- or calls one the arguments do not fit, which
/// PostgreSQL then rejects with a message about a function that does not
/// exist, where the truth is that none of them matched.
///
/// A signature fits when every parameter it requires is supplied. A key that
/// names no parameter is not disqualifying: on a function call an unrecognised
/// key filters the result rather than arguing with the signature, which is why
/// `/rpc/getallprojects?id=gt.1` calls a function taking nothing at all.
///
/// Among those that fit, the one matching most of its parameters wins, and the
/// shorter signature breaks a tie -- it leaves least to defaults the caller
/// did not ask for.
fn select_overload<'a>(routines: &'a [Routine], request: &ApiRequest) -> Option<&'a Routine> {
    // Arguments in a body are not inspected here. The payload is parsed
    // further down, and a mismatch there is PostgreSQL's to report.
    if request.payload.is_some() {
        return routines.first();
    }

    let supplied: HashSet<&str> = request
        .query_params
        .params
        .iter()
        .map(|(name, _)| name.as_str())
        .collect();

    routines
        .iter()
        .filter(|routine| {
            routine
                .params
                .iter()
                .filter(|p| p.required)
                .all(|p| supplied.contains(p.name.as_str()))
        })
        .max_by_key(|routine| {
            let matched = routine
                .params
                .iter()
                .filter(|p| supplied.contains(p.name.as_str()))
                .count();
            (matched, std::cmp::Reverse(routine.params.len()))
        })
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
            let mutate_plan = MutatePlan::from_request(request, table, mutation)?;

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

        DbAction::Routine {
            qi,
            invoke_method: _,
        } => {
            let supplied: Vec<String> = request
                .query_params
                .params
                .iter()
                .map(|(name, _)| name.clone())
                .collect();

            // The name is reported as it was called, arguments and all, so the
            // client can see which signature was looked for.
            let not_found = |candidate: Option<String>| Error::FunctionNotFound {
                name: qi.to_string(),
                params: supplied.clone(),
                candidate,
            };

            let routines = schema_cache
                .get_routines(qi)
                .ok_or_else(|| not_found(None))?;

            let routine = select_overload(routines, request).ok_or_else(|| {
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
