//! SQL query generation from execution plans.
//!
//! This module converts execution plans into parameterized SQL queries.

mod builder;

pub use builder::{QueryBuilder, INSERTED_COLUMN, PARENT_ROW_COLUMN};

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api_request::{QualifiedIdentifier, Range};
    use crate::plan::{CoercibleField, CoercibleSelectField, MutatePlan, ReadPlan, ReadPlanTree};

    /// The whole statement, parentheses included.
    ///
    /// A missing one here is a syntax error on every write with a body, and
    /// nothing short of executing it against PostgreSQL -- or reading it --
    /// will say so, since the fragments that build it are each well-formed.
    #[test]
    fn a_write_reads_its_body_and_is_read_from() {
        let mutate = MutatePlan::Insert {
            target: QualifiedIdentifier::new("test", "items"),
            columns: vec![CoercibleField::simple("id", "int8")],
            body: Some(bytes::Bytes::from(r#"{"id":9001}"#)),
            on_conflict: None,
            where_clauses: vec![],
            returning: vec!["id".into()],
            pk_cols: vec!["id".into()],
            apply_defaults: true,
            reports_inserted: false,
        };
        let read = ReadPlan {
            select: vec![CoercibleSelectField::simple("id", "bigint")],
            from: QualifiedIdentifier::unqualified(MUTATION_RESULT),
            from_alias: None,
            where_clauses: vec![],
            order: vec![],
            range: Range::default(),
            rel_name: String::new(),
            rel_to_parent: None,
            rel_join_conds: vec![],
            rel_join_type: None,
            rel_select: vec![],
            depth: 0,
        };

        let plan = ActionPlan::Db(DbActionPlan::MutateRead {
            mutate,
            read: Some(ReadPlanTree::leaf(read)),
        });
        let (sql, _) = build_query(&plan, None).unwrap().build_main();

        assert_eq!(
            sql,
            "WITH pgrst_mutation_result AS (\
             INSERT INTO \"test\".\"items\" (\"id\") \
             SELECT pgrst_body.\"id\" \
             FROM (SELECT $1::json AS json_data) pgrst_payload, \
             LATERAL (SELECT \"id\" FROM json_to_record(pgrst_payload.json_data) \
             AS _(\"id\" int8)) pgrst_body \
             RETURNING \"items\".\"id\") \
             SELECT \"id\" FROM \"pgrst_mutation_result\""
        );
    }
}
