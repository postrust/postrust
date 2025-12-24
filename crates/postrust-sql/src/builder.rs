//! Core SQL builder types.

use crate::param::SqlParam;
use std::fmt::Write;

/// A SQL fragment with its associated parameters.
///
/// This is the core type for building SQL queries safely. It maintains
/// a SQL string with parameter placeholders ($1, $2, etc.) and a vector
/// of parameter values.
#[derive(Clone, Debug, Default)]
pub struct SqlFragment {
    sql: String,
    params: Vec<SqlParam>,
}

impl SqlFragment {
    /// Create a new empty SQL fragment.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a SQL fragment from raw SQL (no parameters).
    ///
    /// # Warning
    ///
    /// Only use this for known-safe SQL strings (e.g., keywords, operators).
    /// Never use this with user input.
    pub fn raw(sql: impl Into<String>) -> Self {
        Self {
            sql: sql.into(),
            params: Vec::new(),
        }
    }

    /// Create a SQL fragment with a single parameter.
    pub fn param(value: impl Into<SqlParam>) -> Self {
        let mut frag = Self::new();
        frag.push_param(value);
        frag
    }

    /// Get the SQL string.
    pub fn sql(&self) -> &str {
        &self.sql
    }

    /// Get the parameters.
    pub fn params(&self) -> &[SqlParam] {
        &self.params
    }

    /// Get the current parameter count.
    pub fn param_count(&self) -> usize {
        self.params.len()
    }

    /// Check if the fragment is empty.
    pub fn is_empty(&self) -> bool {
        self.sql.is_empty()
    }

    /// Push raw SQL (no parameters).
    pub fn push(&mut self, sql: &str) -> &mut Self {
        self.sql.push_str(sql);
        self
    }

    /// Push a character.
    pub fn push_char(&mut self, c: char) -> &mut Self {
        self.sql.push(c);
        self
    }

    /// Push a parameter and its placeholder.
    pub fn push_param(&mut self, value: impl Into<SqlParam>) -> &mut Self {
        let param_num = self.params.len() + 1;
        write!(self.sql, "${}", param_num).unwrap();
        self.params.push(value.into());
        self
    }

    /// Push a typed parameter with explicit cast.
    pub fn push_typed_param(&mut self, value: impl Into<SqlParam>, pg_type: &str) -> &mut Self {
        let param_num = self.params.len() + 1;
        write!(self.sql, "${}::{}", param_num, pg_type).unwrap();
        self.params.push(value.into());
        self
    }

    /// Append another SQL fragment.
    ///
    /// This renumbers the parameters in the appended fragment to continue
    /// from the current count.
    pub fn append(&mut self, other: SqlFragment) -> &mut Self {
        let offset = self.params.len();

        // Renumber parameters in the other fragment
        let renumbered_sql = renumber_params(&other.sql, offset);
        self.sql.push_str(&renumbered_sql);
        self.params.extend(other.params);
        self
    }

    /// Append with a separator if not empty.
    pub fn append_sep(&mut self, sep: &str, other: SqlFragment) -> &mut Self {
        if !self.is_empty() && !other.is_empty() {
            self.push(sep);
        }
        self.append(other)
    }

    /// Join multiple fragments with a separator.
    pub fn join(sep: &str, fragments: impl IntoIterator<Item = SqlFragment>) -> Self {
        let mut result = Self::new();
        let mut first = true;

        for frag in fragments {
            if frag.is_empty() {
                continue;
            }
            if !first {
                result.push(sep);
            }
            result.append(frag);
            first = false;
        }

        result
    }

    /// Wrap in parentheses.
    pub fn parens(mut self) -> Self {
        self.sql = format!("({})", self.sql);
        self
    }

    /// Build the final SQL and parameters.
    pub fn build(self) -> (String, Vec<SqlParam>) {
        (self.sql, self.params)
    }
}

/// Renumber parameter placeholders in a SQL string.
fn renumber_params(sql: &str, offset: usize) -> String {
    let mut result = String::with_capacity(sql.len());
    let mut chars = sql.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '$' {
            // Parse the parameter number
            let mut num_str = String::new();
            while let Some(&next) = chars.peek() {
                if next.is_ascii_digit() {
                    num_str.push(chars.next().unwrap());
                } else {
                    break;
                }
            }

            if let Ok(num) = num_str.parse::<usize>() {
                write!(result, "${}", num + offset).unwrap();
            } else {
                result.push('$');
                result.push_str(&num_str);
            }
        } else {
            result.push(c);
        }
    }

    result
}

/// Trait for types that can be converted to SQL fragments.
pub trait SqlBuilder {
    /// Build the SQL fragment for this type.
    fn build_sql(&self) -> SqlFragment;
}

impl SqlBuilder for SqlFragment {
    fn build_sql(&self) -> SqlFragment {
        self.clone()
    }
}

impl SqlBuilder for &str {
    fn build_sql(&self) -> SqlFragment {
        SqlFragment::raw(*self)
    }
}

impl SqlBuilder for String {
    fn build_sql(&self) -> SqlFragment {
        SqlFragment::raw(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sql_fragment_raw() {
        let frag = SqlFragment::raw("SELECT * FROM users");
        assert_eq!(frag.sql(), "SELECT * FROM users");
        assert!(frag.params().is_empty());
    }

    #[test]
    fn test_sql_fragment_param() {
        let mut frag = SqlFragment::new();
        frag.push("SELECT * FROM users WHERE id = ");
        frag.push_param(42i64);

        assert_eq!(frag.sql(), "SELECT * FROM users WHERE id = $1");
        assert_eq!(frag.params().len(), 1);
    }

    #[test]
    fn test_sql_fragment_append() {
        let mut frag1 = SqlFragment::new();
        frag1.push("SELECT * FROM users WHERE id = ");
        frag1.push_param(42i64);

        let mut frag2 = SqlFragment::new();
        frag2.push(" AND name = ");
        frag2.push_param("John");

        frag1.append(frag2);

        assert_eq!(
            frag1.sql(),
            "SELECT * FROM users WHERE id = $1 AND name = $2"
        );
        assert_eq!(frag1.params().len(), 2);
    }

    #[test]
    fn test_sql_fragment_join() {
        let frags = vec![
            SqlFragment::raw("a = $1").push_param(1i64).clone(),
            SqlFragment::raw("b = $1").push_param(2i64).clone(),
            SqlFragment::raw("c = $1").push_param(3i64).clone(),
        ];

        let joined = SqlFragment::join(" AND ", frags.into_iter().map(|mut f| {
            f.params.clear();
            f.push_param(1i64);
            f
        }));

        // Note: This test shows the renumbering behavior
    }

    #[test]
    fn test_renumber_params() {
        assert_eq!(renumber_params("$1", 2), "$3");
        assert_eq!(renumber_params("$1 AND $2", 5), "$6 AND $7");
        assert_eq!(renumber_params("no params", 5), "no params");
    }

    #[test]
    fn test_sql_fragment_parens() {
        let frag = SqlFragment::raw("a OR b").parens();
        assert_eq!(frag.sql(), "(a OR b)");
    }
}
