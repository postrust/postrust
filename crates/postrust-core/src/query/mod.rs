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
            query.main = QueryBuilder::build_mutate(mutate)?;
            if let Some(read_tree) = read {
                query.read = Some(QueryBuilder::build_read(read_tree)?);
            }
        }
        DbActionPlan::Call { call, read } => {
            query.main = QueryBuilder::build_call(call)?;
            if let Some(read_tree) = read {
                query.read = Some(QueryBuilder::build_read(read_tree)?);
            }
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
