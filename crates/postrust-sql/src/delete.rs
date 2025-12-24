//! DELETE statement builder.

use crate::{
    builder::SqlFragment,
    expr::Expr,
    identifier::{escape_ident, from_qi, QualifiedIdentifier},
};

/// Builder for DELETE statements.
#[derive(Clone, Debug, Default)]
pub struct DeleteBuilder {
    table: Option<SqlFragment>,
    using: Vec<SqlFragment>,
    where_clauses: Vec<SqlFragment>,
    returning: Vec<SqlFragment>,
}

impl DeleteBuilder {
    /// Create a new DELETE builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the target table.
    pub fn from_table(mut self, qi: &QualifiedIdentifier) -> Self {
        self.table = Some(SqlFragment::raw(from_qi(qi)));
        self
    }

    /// Set the target table with alias.
    pub fn from_table_as(mut self, qi: &QualifiedIdentifier, alias: &str) -> Self {
        self.table = Some(SqlFragment::raw(format!(
            "{} AS {}",
            from_qi(qi),
            escape_ident(alias)
        )));
        self
    }

    /// Add a USING clause for joins.
    pub fn using(mut self, table: &str) -> Self {
        self.using.push(SqlFragment::raw(escape_ident(table)));
        self
    }

    /// Add a WHERE clause.
    pub fn where_expr(mut self, expr: Expr) -> Self {
        self.where_clauses.push(expr.into_fragment());
        self
    }

    /// Add a raw WHERE clause.
    pub fn where_raw(mut self, sql: SqlFragment) -> Self {
        self.where_clauses.push(sql);
        self
    }

    /// Add RETURNING clause.
    pub fn returning(mut self, column: &str) -> Self {
        self.returning
            .push(SqlFragment::raw(escape_ident(column)));
        self
    }

    /// Add RETURNING * clause.
    pub fn returning_all(mut self) -> Self {
        self.returning.push(SqlFragment::raw("*"));
        self
    }

    /// Build the DELETE statement.
    pub fn build(self) -> SqlFragment {
        let mut result = SqlFragment::new();

        result.push("DELETE FROM ");

        if let Some(table) = self.table {
            result.append(table);
        }

        // USING
        if !self.using.is_empty() {
            result.push(" USING ");
            for (i, table) in self.using.into_iter().enumerate() {
                if i > 0 {
                    result.push(", ");
                }
                result.append(table);
            }
        }

        // WHERE
        if !self.where_clauses.is_empty() {
            result.push(" WHERE ");
            for (i, clause) in self.where_clauses.into_iter().enumerate() {
                if i > 0 {
                    result.push(" AND ");
                }
                result.append(clause);
            }
        }

        // RETURNING
        if !self.returning.is_empty() {
            result.push(" RETURNING ");
            for (i, ret) in self.returning.into_iter().enumerate() {
                if i > 0 {
                    result.push(", ");
                }
                result.append(ret);
            }
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_delete() {
        let qi = QualifiedIdentifier::new("public", "users");
        let sql = DeleteBuilder::new()
            .from_table(&qi)
            .where_expr(Expr::eq("id", 1i64))
            .build();

        assert!(sql.sql().contains("DELETE FROM"));
        assert!(sql.sql().contains("WHERE"));
        assert_eq!(sql.params().len(), 1);
    }

    #[test]
    fn test_delete_all() {
        let qi = QualifiedIdentifier::unqualified("logs");
        let sql = DeleteBuilder::new().from_table(&qi).build();

        assert_eq!(sql.sql(), "DELETE FROM \"logs\"");
        assert!(sql.params().is_empty());
    }

    #[test]
    fn test_delete_returning() {
        let qi = QualifiedIdentifier::unqualified("users");
        let sql = DeleteBuilder::new()
            .from_table(&qi)
            .where_expr(Expr::is_not_null("deleted_at"))
            .returning("id")
            .returning("email")
            .build();

        assert!(sql.sql().contains("RETURNING"));
    }

    #[test]
    fn test_delete_with_using() {
        let qi = QualifiedIdentifier::unqualified("orders");
        let sql = DeleteBuilder::new()
            .from_table(&qi)
            .using("users")
            .where_raw(SqlFragment::raw(
                "\"orders\".\"user_id\" = \"users\".\"id\" AND \"users\".\"deleted\" = true",
            ))
            .build();

        assert!(sql.sql().contains("USING"));
    }
}
