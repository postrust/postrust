//! Query planning module.
//!
//! Converts parsed API requests into execution plans that can be
//! translated to SQL queries.

mod read_plan;
mod mutate_plan;
mod call_plan;
mod types;

pub use read_plan::{ReadPlan, ReadPlanTree};
pub use mutate_plan::MutatePlan;
pub use call_plan::{CallPlan, CallParams};
pub use types::*;

use crate::api_request::{
    Action, ApiRequest, DbAction,
    QualifiedIdentifier,
};
use crate::error::{Error, Result};
use crate::schema_cache::SchemaCache;

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
pub fn create_action_plan(
    request: &ApiRequest,
    schema_cache: &SchemaCache,
) -> Result<ActionPlan> {
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

        DbAction::Routine { qi, invoke_method: _ } => {
            let routines = schema_cache
                .get_routines(qi)
                .ok_or_else(|| Error::FunctionNotFound(qi.to_string()))?;

            let routine = routines
                .first()
                .ok_or_else(|| Error::FunctionNotFound(qi.to_string()))?;

            let call_plan = CallPlan::from_request(request, routine)?;

            Ok(DbActionPlan::Call {
                call: call_plan,
                read: None,
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
    use crate::api_request::Action;

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
