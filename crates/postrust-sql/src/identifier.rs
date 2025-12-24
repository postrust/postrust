//! Safe SQL identifier handling.
//!
//! Provides functions for safely escaping and quoting SQL identifiers
//! and literals to prevent SQL injection.

/// Escape a SQL identifier (table name, column name, etc.).
///
/// This function wraps the identifier in double quotes and escapes
/// any embedded double quotes by doubling them.
///
/// # Examples
///
/// ```
/// use postrust_sql::escape_ident;
///
/// assert_eq!(escape_ident("users"), "\"users\"");
/// assert_eq!(escape_ident("user\"name"), "\"user\"\"name\"");
/// assert_eq!(escape_ident("My Table"), "\"My Table\"");
/// ```
pub fn escape_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// Quote a SQL literal string.
///
/// This function wraps the string in single quotes and escapes
/// any embedded single quotes by doubling them. This is only for
/// cases where parameterized queries can't be used (e.g., SET commands).
///
/// # Warning
///
/// Prefer using parameterized queries with `SqlParam` instead of this function
/// whenever possible. This function should only be used for SQL constructs
/// that don't support parameters (like some SET commands).
///
/// # Examples
///
/// ```
/// use postrust_sql::quote_literal;
///
/// assert_eq!(quote_literal("hello"), "'hello'");
/// assert_eq!(quote_literal("it's"), "'it''s'");
/// ```
pub fn quote_literal(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// Qualified identifier (schema.name).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct QualifiedIdentifier {
    pub schema: String,
    pub name: String,
}

impl QualifiedIdentifier {
    pub fn new(schema: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            schema: schema.into(),
            name: name.into(),
        }
    }

    pub fn unqualified(name: impl Into<String>) -> Self {
        Self {
            schema: String::new(),
            name: name.into(),
        }
    }
}

/// Convert a qualified identifier to a safe SQL string.
///
/// # Examples
///
/// ```
/// use postrust_sql::{from_qi, identifier::QualifiedIdentifier};
///
/// let qi = QualifiedIdentifier::new("public", "users");
/// assert_eq!(from_qi(&qi), "\"public\".\"users\"");
///
/// let qi = QualifiedIdentifier::unqualified("users");
/// assert_eq!(from_qi(&qi), "\"users\"");
/// ```
pub fn from_qi(qi: &QualifiedIdentifier) -> String {
    if qi.schema.is_empty() {
        escape_ident(&qi.name)
    } else {
        format!("{}.{}", escape_ident(&qi.schema), escape_ident(&qi.name))
    }
}

/// Check if a string is a valid unquoted identifier.
///
/// PostgreSQL unquoted identifiers must start with a letter or underscore,
/// and can contain letters, digits, underscores, and dollar signs.
pub fn is_valid_identifier(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }

    let mut chars = s.chars();
    let first = chars.next().unwrap();

    if !first.is_ascii_alphabetic() && first != '_' {
        return false;
    }

    chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
}

/// Check if a string is a SQL keyword that should be quoted.
pub fn is_keyword(s: &str) -> bool {
    const KEYWORDS: &[&str] = &[
        "all", "and", "any", "array", "as", "asc", "between", "by",
        "case", "cast", "check", "column", "constraint", "create",
        "cross", "current", "default", "delete", "desc", "distinct",
        "drop", "else", "end", "exists", "false", "for", "foreign",
        "from", "full", "group", "having", "in", "index", "inner",
        "insert", "into", "is", "join", "key", "left", "like", "limit",
        "not", "null", "offset", "on", "or", "order", "outer", "primary",
        "references", "right", "select", "set", "table", "then", "to",
        "true", "union", "unique", "update", "using", "values", "when",
        "where", "with",
    ];

    KEYWORDS.contains(&s.to_lowercase().as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escape_ident() {
        assert_eq!(escape_ident("users"), "\"users\"");
        assert_eq!(escape_ident("user_table"), "\"user_table\"");
        assert_eq!(escape_ident("user\"name"), "\"user\"\"name\"");
        assert_eq!(escape_ident("My Table"), "\"My Table\"");
        assert_eq!(escape_ident(""), "\"\"");
    }

    #[test]
    fn test_quote_literal() {
        assert_eq!(quote_literal("hello"), "'hello'");
        assert_eq!(quote_literal("it's"), "'it''s'");
        assert_eq!(quote_literal(""), "''");
    }

    #[test]
    fn test_from_qi() {
        let qi = QualifiedIdentifier::new("public", "users");
        assert_eq!(from_qi(&qi), "\"public\".\"users\"");

        let qi = QualifiedIdentifier::unqualified("users");
        assert_eq!(from_qi(&qi), "\"users\"");

        let qi = QualifiedIdentifier::new("my schema", "my\"table");
        assert_eq!(from_qi(&qi), "\"my schema\".\"my\"\"table\"");
    }

    #[test]
    fn test_is_valid_identifier() {
        assert!(is_valid_identifier("users"));
        assert!(is_valid_identifier("_private"));
        assert!(is_valid_identifier("user123"));
        assert!(is_valid_identifier("user$table"));

        assert!(!is_valid_identifier(""));
        assert!(!is_valid_identifier("123users"));
        assert!(!is_valid_identifier("my-table"));
        assert!(!is_valid_identifier("my table"));
    }

    #[test]
    fn test_is_keyword() {
        assert!(is_keyword("select"));
        assert!(is_keyword("SELECT"));
        assert!(is_keyword("from"));
        assert!(is_keyword("WHERE"));

        assert!(!is_keyword("users"));
        assert!(!is_keyword("my_column"));
    }
}
