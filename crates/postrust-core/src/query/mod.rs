//! SQL query generation from execution plans.
//!
//! This module converts execution plans into parameterized SQL queries.

mod builder;

pub use builder::QueryBuilder;

use crate::error::Result;
use crate::plan::{ActionPlan, DbActionPlan};
use postrust_sql::{SqlFragment, SqlParam};

/// Build SQL from an action plan.
pub fn build_query(plan: &ActionPlan, role: Option<&str>) -> Result<MainQuery> {
    match plan {
        ActionPlan::Db(db_plan) => build_db_query(db_plan, role),
        ActionPlan::Info(_) => Ok(MainQuery::empty()),
    }
}

/// The name the rows a mutation affected are read from.
pub const MUTATION_RESULT: &str = "pgrst_mutation_result";

/// Build SQL from a database action plan.
fn build_db_query(plan: &DbActionPlan, role: Option<&str>) -> Result<MainQuery> {
    let mut query = MainQuery::new();

    // Add role switch if specified
    if let Some(role) = role {
        query.pre_statements.push(format!(
            "SET LOCAL ROLE {}",
            postrust_sql::escape_ident(role)
        ));
    }

    match plan {
        DbActionPlan::Read(read_tree) => {
            query.main = QueryBuilder::build_read(read_tree)?;
        }
        DbActionPlan::MutateRead { mutate, read } => {
            let mutation = QueryBuilder::build_mutate(mutate)?;

            // The rows a mutation affected are a relation like any other, so
            // `?select=` is answered by reading from them -- which is also
            // what lets a mutation embed, alias, cast and compute exactly as a
            // read does. Without the wrapper the `RETURNING` list was the
            // whole answer, and none of that reached it.
            query.main = match read {
                Some(read_tree) => {
                    let read_sql = QueryBuilder::build_read(read_tree)?;
                    let mut frag = SqlFragment::new();
                    frag.push("WITH ");
                    frag.push(MUTATION_RESULT);
                    frag.push(" AS (");
                    frag.append(mutation);
                    frag.push(") ");
                    frag.append(read_sql);
                    frag
                }
                None => mutation,
            };
        }
        DbActionPlan::Call { call, read } => {
            query.main = QueryBuilder::build_call(call, read.as_ref())?;
        }
    }

    Ok(query)
}

/// A complete query with setup and main statement.
#[derive(Clone, Debug, Default)]
pub struct MainQuery {
    /// Pre-query statements (SET commands)
    pub pre_statements: Vec<String>,
    /// Main query
    pub main: SqlFragment,
    /// Read query (for mutations with RETURNING)
    pub read: Option<SqlFragment>,
    /// Count query (for pagination)
    pub count: Option<SqlFragment>,
}

impl MainQuery {
    /// Create a new empty query.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create an empty query (for info-only plans).
    pub fn empty() -> Self {
        Self::default()
    }

    /// Check if this query has a main statement.
    pub fn has_main(&self) -> bool {
        !self.main.is_empty()
    }

    /// Get the main SQL and parameters.
    pub fn build_main(self) -> (String, Vec<SqlParam>) {
        self.main.build()
    }
}
