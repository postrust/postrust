//! SELECT statement builder.

use crate::{
    builder::SqlFragment,
    expr::{Expr, OrderExpr},
    identifier::{escape_ident, from_qi, QualifiedIdentifier},
};

/// Builder for SELECT statements.
#[derive(Clone, Debug, Default)]
pub struct SelectBuilder {
    columns: Vec<SqlFragment>,
    from: Option<SqlFragment>,
    joins: Vec<SqlFragment>,
    where_clauses: Vec<SqlFragment>,
    group_by: Vec<SqlFragment>,
    having: Vec<SqlFragment>,
    order_by: Vec<SqlFragment>,
    limit: Option<i64>,
    offset: Option<i64>,
    distinct: bool,
    cte: Vec<(String, SqlFragment)>,
}

impl SelectBuilder {
    /// Create a new SELECT builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a CTE (WITH clause).
    pub fn with_cte(mut self, name: &str, query: SqlFragment) -> Self {
        self.cte.push((name.to_string(), query));
        self
    }

    /// Set DISTINCT.
    pub fn distinct(mut self) -> Self {
        self.distinct = true;
        self
    }

    /// Add a column to select.
    pub fn column(mut self, name: &str) -> Self {
        self.columns.push(SqlFragment::raw(escape_ident(name)));
        self
    }

    /// Add a column with alias.
    pub fn column_as(mut self, name: &str, alias: &str) -> Self {
        self.columns.push(SqlFragment::raw(format!(
            "{} AS {}",
            escape_ident(name),
            escape_ident(alias)
        )));
        self
    }

    /// Add a qualified column (table.column).
    pub fn qualified_column(mut self, table: &str, column: &str) -> Self {
        self.columns.push(SqlFragment::raw(format!(
            "{}.{}",
            escape_ident(table),
            escape_ident(column)
        )));
        self
    }

    /// Add a raw SQL column expression.
    pub fn column_raw(mut self, sql: SqlFragment) -> Self {
        self.columns.push(sql);
        self
    }

    /// Add all columns (*).
    pub fn all_columns(mut self) -> Self {
        self.columns.push(SqlFragment::raw("*"));
        self
    }

    /// Add all columns from a table (table.*).
    pub fn all_columns_from(mut self, table: &str) -> Self {
        self.columns
            .push(SqlFragment::raw(format!("{}.*", escape_ident(table))));
        self
    }

    /// Set the FROM table.
    pub fn from_table(mut self, qi: &QualifiedIdentifier) -> Self {
        self.from = Some(SqlFragment::raw(from_qi(qi)));
        self
    }

    /// Set FROM with alias.
    pub fn from_table_as(mut self, qi: &QualifiedIdentifier, alias: &str) -> Self {
        self.from = Some(SqlFragment::raw(format!(
            "{} AS {}",
            from_qi(qi),
            escape_ident(alias)
        )));
        self
    }

    /// Set FROM from raw SQL.
    pub fn from_raw(mut self, sql: SqlFragment) -> Self {
        self.from = Some(sql);
        self
    }

    /// Add an INNER JOIN.
    pub fn inner_join(mut self, table: &str, condition: &str) -> Self {
        self.joins.push(SqlFragment::raw(format!(
            " INNER JOIN {} ON {}",
            escape_ident(table),
            condition
        )));
        self
    }

    /// Add a LEFT JOIN.
    pub fn left_join(mut self, table: &str, condition: &str) -> Self {
        self.joins.push(SqlFragment::raw(format!(
            " LEFT JOIN {} ON {}",
            escape_ident(table),
            condition
        )));
        self
    }

    /// Add a LEFT JOIN LATERAL with subquery.
    pub fn left_join_lateral(mut self, subquery: SqlFragment, alias: &str, on: &str) -> Self {
        let mut join = SqlFragment::raw(" LEFT JOIN LATERAL (");
        join.append(subquery);
        join.push(") AS ");
        join.push(&escape_ident(alias));
        join.push(" ON ");
        join.push(on);
        self.joins.push(join);
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

    /// Add a GROUP BY column.
    pub fn group_by(mut self, column: &str) -> Self {
        self.group_by.push(SqlFragment::raw(escape_ident(column)));
        self
    }

    /// Add a HAVING clause.
    pub fn having(mut self, expr: Expr) -> Self {
        self.having.push(expr.into_fragment());
        self
    }

    /// Add an ORDER BY clause.
    pub fn order_by(mut self, expr: OrderExpr) -> Self {
        self.order_by.push(expr.into_fragment());
        self
    }

    /// Add ORDER BY from raw SQL.
    pub fn order_by_raw(mut self, sql: SqlFragment) -> Self {
        self.order_by.push(sql);
        self
    }

    /// Set LIMIT.
    pub fn limit(mut self, n: i64) -> Self {
        self.limit = Some(n);
        self
    }

    /// Set OFFSET.
    pub fn offset(mut self, n: i64) -> Self {
        self.offset = Some(n);
        self
    }

    /// Build the SELECT statement.
    pub fn build(self) -> SqlFragment {
        let mut result = SqlFragment::new();

        // CTEs
        if !self.cte.is_empty() {
            result.push("WITH ");
            for (i, (name, query)) in self.cte.into_iter().enumerate() {
                if i > 0 {
                    result.push(", ");
                }
                result.push(&escape_ident(&name));
                result.push(" AS (");
                result.append(query);
                result.push(")");
            }
            result.push(" ");
        }

        // SELECT
        result.push("SELECT ");
        if self.distinct {
            result.push("DISTINCT ");
        }

        // Columns
        if self.columns.is_empty() {
            result.push("*");
        } else {
            for (i, col) in self.columns.into_iter().enumerate() {
                if i > 0 {
                    result.push(", ");
                }
                result.append(col);
            }
        }

        // FROM
        if let Some(from) = self.from {
            result.push(" FROM ");
            result.append(from);
        }

        // JOINs
        for join in self.joins {
            result.append(join);
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

        // GROUP BY
        if !self.group_by.is_empty() {
            result.push(" GROUP BY ");
            for (i, col) in self.group_by.into_iter().enumerate() {
                if i > 0 {
                    result.push(", ");
                }
                result.append(col);
            }
        }

        // HAVING
        if !self.having.is_empty() {
            result.push(" HAVING ");
            for (i, clause) in self.having.into_iter().enumerate() {
                if i > 0 {
                    result.push(" AND ");
                }
                result.append(clause);
            }
        }

        // ORDER BY
        if !self.order_by.is_empty() {
            result.push(" ORDER BY ");
            for (i, order) in self.order_by.into_iter().enumerate() {
                if i > 0 {
                    result.push(", ");
                }
                result.append(order);
            }
        }

        // LIMIT
        if let Some(limit) = self.limit {
            result.push(" LIMIT ");
            result.push(&limit.to_string());
        }

        // OFFSET
        if let Some(offset) = self.offset {
            result.push(" OFFSET ");
            result.push(&offset.to_string());
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_select() {
        let qi = QualifiedIdentifier::new("public", "users");
        let sql = SelectBuilder::new()
            .column("id")
            .column("name")
            .from_table(&qi)
            .build();

        assert_eq!(
            sql.sql(),
            "SELECT \"id\", \"name\" FROM \"public\".\"users\""
        );
    }

    #[test]
    fn test_select_with_where() {
        let qi = QualifiedIdentifier::new("public", "users");
        let sql = SelectBuilder::new()
            .all_columns()
            .from_table(&qi)
            .where_expr(Expr::eq("id", 1i64))
            .build();

        assert!(sql.sql().contains("WHERE"));
        assert!(sql.sql().contains("$1"));
    }

    #[test]
    fn test_select_with_order_limit() {
        let qi = QualifiedIdentifier::new("public", "users");
        let sql = SelectBuilder::new()
            .all_columns()
            .from_table(&qi)
            .order_by(OrderExpr::new("created_at").desc())
            .limit(10)
            .offset(20)
            .build();

        assert!(sql.sql().contains("ORDER BY"));
        assert!(sql.sql().contains("LIMIT 10"));
        assert!(sql.sql().contains("OFFSET 20"));
    }

    #[test]
    fn test_select_distinct() {
        let qi = QualifiedIdentifier::unqualified("users");
        let sql = SelectBuilder::new()
            .distinct()
            .column("status")
            .from_table(&qi)
            .build();

        assert!(sql.sql().contains("SELECT DISTINCT"));
    }
}
