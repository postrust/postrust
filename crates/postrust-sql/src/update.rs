//! UPDATE statement builder.

use crate::{
    builder::SqlFragment,
    expr::Expr,
    identifier::{escape_ident, from_qi, QualifiedIdentifier},
};

/// Builder for UPDATE statements.
#[derive(Clone, Debug, Default)]
pub struct UpdateBuilder {
    table: Option<SqlFragment>,
    set: Vec<(String, SqlFragment)>,
    where_clauses: Vec<SqlFragment>,
    returning: Vec<SqlFragment>,
}

impl UpdateBuilder {
    /// Create a new UPDATE builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the target table.
    pub fn table(mut self, qi: &QualifiedIdentifier) -> Self {
        self.table = Some(SqlFragment::raw(from_qi(qi)));
        self
    }

    /// Set the target table with alias.
    pub fn table_as(mut self, qi: &QualifiedIdentifier, alias: &str) -> Self {
        self.table = Some(SqlFragment::raw(format!(
            "{} AS {}",
            from_qi(qi),
            escape_ident(alias)
        )));
        self
    }

    /// Add a SET clause with parameterized value.
    pub fn set<V: Into<crate::param::SqlParam>>(mut self, column: &str, value: V) -> Self {
        let mut frag = SqlFragment::new();
        frag.push_param(value);
        self.set.push((column.to_string(), frag));
        self
    }

    /// Add a SET clause with raw SQL.
    pub fn set_raw(mut self, column: &str, value: SqlFragment) -> Self {
        self.set.push((column.to_string(), value));
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

    /// Build the UPDATE statement.
    pub fn build(self) -> SqlFragment {
        let mut result = SqlFragment::new();

        result.push("UPDATE ");

        if let Some(table) = self.table {
            result.append(table);
        }

        // SET
        if !self.set.is_empty() {
            result.push(" SET ");
            for (i, (col, val)) in self.set.into_iter().enumerate() {
                if i > 0 {
                    result.push(", ");
                }
                result.push(&escape_ident(&col));
                result.push(" = ");
                result.append(val);
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
    use crate::param::SqlParam;

    #[test]
    fn test_simple_update() {
        let qi = QualifiedIdentifier::new("public", "users");
        let sql = UpdateBuilder::new()
            .table(&qi)
            .set("name", SqlParam::text("Jane"))
            .where_expr(Expr::eq("id", 1i64))
            .build();

        assert!(sql.sql().contains("UPDATE"));
        assert!(sql.sql().contains("SET"));
        assert!(sql.sql().contains("WHERE"));
        assert_eq!(sql.params().len(), 2);
    }

    #[test]
    fn test_update_returning() {
        let qi = QualifiedIdentifier::unqualified("users");
        let sql = UpdateBuilder::new()
            .table(&qi)
            .set("status", SqlParam::text("active"))
            .returning_all()
            .build();

        assert!(sql.sql().contains("RETURNING *"));
    }

    #[test]
    fn test_update_multiple_sets() {
        let qi = QualifiedIdentifier::unqualified("users");
        let sql = UpdateBuilder::new()
            .table(&qi)
            .set("name", SqlParam::text("John"))
            .set("email", SqlParam::text("john@new.com"))
            .set("updated_at", SqlParam::text("now()"))
            .where_expr(Expr::eq("id", 5i64))
            .build();

        assert_eq!(sql.params().len(), 4); // 3 sets + 1 where
    }
}
