//! SQL expression building.

use crate::{builder::SqlFragment, identifier::escape_ident, param::SqlParam};

/// A SQL expression (for WHERE, HAVING, etc.).
#[derive(Clone, Debug)]
pub struct Expr {
    fragment: SqlFragment,
}

impl Expr {
    /// Create an expression from a SQL fragment.
    pub fn from_fragment(fragment: SqlFragment) -> Self {
        Self { fragment }
    }

    /// Create a column reference expression.
    pub fn column(name: &str) -> Self {
        Self {
            fragment: SqlFragment::raw(escape_ident(name)),
        }
    }

    /// Create a qualified column reference (table.column).
    pub fn qualified_column(table: &str, column: &str) -> Self {
        Self {
            fragment: SqlFragment::raw(format!(
                "{}.{}",
                escape_ident(table),
                escape_ident(column)
            )),
        }
    }

    /// Create an equality expression: column = $1
    pub fn eq(column: &str, value: impl Into<SqlParam>) -> Self {
        let mut frag = SqlFragment::new();
        frag.push(&escape_ident(column));
        frag.push(" = ");
        frag.push_param(value);
        Self { fragment: frag }
    }

    /// Create a not-equal expression: column <> $1
    pub fn neq(column: &str, value: impl Into<SqlParam>) -> Self {
        let mut frag = SqlFragment::new();
        frag.push(&escape_ident(column));
        frag.push(" <> ");
        frag.push_param(value);
        Self { fragment: frag }
    }

    /// Create a greater-than expression: column > $1
    pub fn gt(column: &str, value: impl Into<SqlParam>) -> Self {
        let mut frag = SqlFragment::new();
        frag.push(&escape_ident(column));
        frag.push(" > ");
        frag.push_param(value);
        Self { fragment: frag }
    }

    /// Create a greater-than-or-equal expression: column >= $1
    pub fn gte(column: &str, value: impl Into<SqlParam>) -> Self {
        let mut frag = SqlFragment::new();
        frag.push(&escape_ident(column));
        frag.push(" >= ");
        frag.push_param(value);
        Self { fragment: frag }
    }

    /// Create a less-than expression: column < $1
    pub fn lt(column: &str, value: impl Into<SqlParam>) -> Self {
        let mut frag = SqlFragment::new();
        frag.push(&escape_ident(column));
        frag.push(" < ");
        frag.push_param(value);
        Self { fragment: frag }
    }

    /// Create a less-than-or-equal expression: column <= $1
    pub fn lte(column: &str, value: impl Into<SqlParam>) -> Self {
        let mut frag = SqlFragment::new();
        frag.push(&escape_ident(column));
        frag.push(" <= ");
        frag.push_param(value);
        Self { fragment: frag }
    }

    /// Create a LIKE expression: column LIKE $1
    pub fn like(column: &str, pattern: impl Into<SqlParam>) -> Self {
        let mut frag = SqlFragment::new();
        frag.push(&escape_ident(column));
        frag.push(" LIKE ");
        frag.push_param(pattern);
        Self { fragment: frag }
    }

    /// Create an ILIKE expression: column ILIKE $1
    pub fn ilike(column: &str, pattern: impl Into<SqlParam>) -> Self {
        let mut frag = SqlFragment::new();
        frag.push(&escape_ident(column));
        frag.push(" ILIKE ");
        frag.push_param(pattern);
        Self { fragment: frag }
    }

    /// Create an IS NULL expression: column IS NULL
    pub fn is_null(column: &str) -> Self {
        Self {
            fragment: SqlFragment::raw(format!("{} IS NULL", escape_ident(column))),
        }
    }

    /// Create an IS NOT NULL expression: column IS NOT NULL
    pub fn is_not_null(column: &str) -> Self {
        Self {
            fragment: SqlFragment::raw(format!("{} IS NOT NULL", escape_ident(column))),
        }
    }

    /// Create an IN expression: column IN ($1, $2, ...)
    pub fn in_list(column: &str, values: Vec<SqlParam>) -> Self {
        if values.is_empty() {
            return Self {
                fragment: SqlFragment::raw("FALSE"),
            };
        }

        let mut frag = SqlFragment::new();
        frag.push(&escape_ident(column));
        frag.push(" IN (");

        for (i, value) in values.into_iter().enumerate() {
            if i > 0 {
                frag.push(", ");
            }
            frag.push_param(value);
        }

        frag.push(")");
        Self { fragment: frag }
    }

    /// Create a contains expression: column @> $1
    pub fn contains(column: &str, value: impl Into<SqlParam>) -> Self {
        let mut frag = SqlFragment::new();
        frag.push(&escape_ident(column));
        frag.push(" @> ");
        frag.push_param(value);
        Self { fragment: frag }
    }

    /// Create a contained-by expression: column <@ $1
    pub fn contained_by(column: &str, value: impl Into<SqlParam>) -> Self {
        let mut frag = SqlFragment::new();
        frag.push(&escape_ident(column));
        frag.push(" <@ ");
        frag.push_param(value);
        Self { fragment: frag }
    }

    /// Create an overlap expression: column && $1
    pub fn overlaps(column: &str, value: impl Into<SqlParam>) -> Self {
        let mut frag = SqlFragment::new();
        frag.push(&escape_ident(column));
        frag.push(" && ");
        frag.push_param(value);
        Self { fragment: frag }
    }

    /// Create a full-text search expression: column @@ to_tsquery($1)
    pub fn fts(column: &str, query: impl Into<SqlParam>, language: Option<&str>) -> Self {
        let mut frag = SqlFragment::new();
        frag.push(&escape_ident(column));
        frag.push(" @@ ");

        if let Some(lang) = language {
            frag.push("to_tsquery(");
            frag.push_param(lang);
            frag.push(", ");
            frag.push_param(query);
            frag.push(")");
        } else {
            frag.push("to_tsquery(");
            frag.push_param(query);
            frag.push(")");
        }

        Self { fragment: frag }
    }

    /// Negate this expression: NOT (expr)
    pub fn not(self) -> Self {
        let mut frag = SqlFragment::raw("NOT ");
        frag.append(self.fragment.parens());
        Self { fragment: frag }
    }

    /// Combine with AND: self AND other
    pub fn and(self, other: Expr) -> Self {
        let mut frag = self.fragment.parens();
        frag.push(" AND ");
        frag.append(other.fragment.parens());
        Self { fragment: frag }
    }

    /// Combine with OR: self OR other
    pub fn or(self, other: Expr) -> Self {
        let mut frag = self.fragment.parens();
        frag.push(" OR ");
        frag.append(other.fragment.parens());
        Self { fragment: frag }
    }

    /// Combine multiple expressions with AND.
    pub fn and_all(exprs: impl IntoIterator<Item = Expr>) -> Self {
        let frags: Vec<_> = exprs.into_iter().map(|e| e.fragment.parens()).collect();
        if frags.is_empty() {
            return Self {
                fragment: SqlFragment::raw("TRUE"),
            };
        }
        Self {
            fragment: SqlFragment::join(" AND ", frags),
        }
    }

    /// Combine multiple expressions with OR.
    pub fn or_all(exprs: impl IntoIterator<Item = Expr>) -> Self {
        let frags: Vec<_> = exprs.into_iter().map(|e| e.fragment.parens()).collect();
        if frags.is_empty() {
            return Self {
                fragment: SqlFragment::raw("FALSE"),
            };
        }
        Self {
            fragment: SqlFragment::join(" OR ", frags),
        }
    }

    /// Convert to a SQL fragment.
    pub fn into_fragment(self) -> SqlFragment {
        self.fragment
    }

    /// Get the SQL string.
    pub fn sql(&self) -> &str {
        self.fragment.sql()
    }

    /// Get the parameters.
    pub fn params(&self) -> &[SqlParam] {
        self.fragment.params()
    }
}

/// ORDER BY expression.
#[derive(Clone, Debug)]
pub struct OrderExpr {
    column: String,
    direction: Option<OrderDirection>,
    nulls: Option<NullsOrder>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum OrderDirection {
    Asc,
    Desc,
}

#[derive(Clone, Debug, PartialEq)]
pub enum NullsOrder {
    First,
    Last,
}

impl OrderExpr {
    /// Create a new ORDER BY expression.
    pub fn new(column: impl Into<String>) -> Self {
        Self {
            column: column.into(),
            direction: None,
            nulls: None,
        }
    }

    /// Set ascending order.
    pub fn asc(mut self) -> Self {
        self.direction = Some(OrderDirection::Asc);
        self
    }

    /// Set descending order.
    pub fn desc(mut self) -> Self {
        self.direction = Some(OrderDirection::Desc);
        self
    }

    /// Set NULLS FIRST.
    pub fn nulls_first(mut self) -> Self {
        self.nulls = Some(NullsOrder::First);
        self
    }

    /// Set NULLS LAST.
    pub fn nulls_last(mut self) -> Self {
        self.nulls = Some(NullsOrder::Last);
        self
    }

    /// Convert to SQL fragment.
    pub fn into_fragment(self) -> SqlFragment {
        let mut frag = SqlFragment::raw(escape_ident(&self.column));

        if let Some(dir) = self.direction {
            match dir {
                OrderDirection::Asc => frag.push(" ASC"),
                OrderDirection::Desc => frag.push(" DESC"),
            };
        }

        if let Some(nulls) = self.nulls {
            match nulls {
                NullsOrder::First => frag.push(" NULLS FIRST"),
                NullsOrder::Last => frag.push(" NULLS LAST"),
            };
        }

        frag
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_expr_eq() {
        let expr = Expr::eq("name", "John");
        assert_eq!(expr.sql(), "\"name\" = $1");
        assert_eq!(expr.params().len(), 1);
    }

    #[test]
    fn test_expr_in_list() {
        let expr = Expr::in_list(
            "id",
            vec![SqlParam::Int(1), SqlParam::Int(2), SqlParam::Int(3)],
        );
        assert_eq!(expr.sql(), "\"id\" IN ($1, $2, $3)");
        assert_eq!(expr.params().len(), 3);
    }

    #[test]
    fn test_expr_is_null() {
        let expr = Expr::is_null("deleted_at");
        assert_eq!(expr.sql(), "\"deleted_at\" IS NULL");
    }

    #[test]
    fn test_expr_and() {
        let expr1 = Expr::eq("a", 1i64);
        let expr2 = Expr::eq("b", 2i64);
        let combined = expr1.and(expr2);

        assert!(combined.sql().contains(" AND "));
        assert_eq!(combined.params().len(), 2);
    }

    #[test]
    fn test_expr_or() {
        let expr1 = Expr::eq("a", 1i64);
        let expr2 = Expr::eq("b", 2i64);
        let combined = expr1.or(expr2);

        assert!(combined.sql().contains(" OR "));
    }

    #[test]
    fn test_expr_not() {
        let expr = Expr::eq("active", true).not();
        assert!(expr.sql().starts_with("NOT"));
    }

    #[test]
    fn test_order_expr() {
        let order = OrderExpr::new("created_at").desc().nulls_last();
        let frag = order.into_fragment();
        assert_eq!(frag.sql(), "\"created_at\" DESC NULLS LAST");
    }
}
