//! SQL parameter types.

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

/// A SQL parameter value.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum SqlParam {
    /// NULL value
    Null,
    /// Boolean
    Bool(bool),
    /// 64-bit integer
    Int(i64),
    /// 64-bit float
    Float(f64),
    /// Text string
    Text(String),
    /// Binary data
    Bytes(Vec<u8>),
    /// JSON value
    Json(JsonValue),
    /// UUID
    Uuid(uuid::Uuid),
    /// Timestamp
    Timestamp(chrono::DateTime<chrono::Utc>),
    /// Array of parameters
    Array(Vec<SqlParam>),
}

impl SqlParam {
    /// Create a text parameter.
    pub fn text(s: impl Into<String>) -> Self {
        Self::Text(s.into())
    }

    /// Create an integer parameter.
    pub fn int(n: i64) -> Self {
        Self::Int(n)
    }

    /// Create a JSON parameter.
    pub fn json(v: JsonValue) -> Self {
        Self::Json(v)
    }

    /// Check if this is a NULL value.
    pub fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }

    /// Get the PostgreSQL type name for this parameter.
    pub fn pg_type(&self) -> &'static str {
        match self {
            Self::Null => "unknown",
            Self::Bool(_) => "boolean",
            Self::Int(_) => "bigint",
            Self::Float(_) => "double precision",
            Self::Text(_) => "text",
            Self::Bytes(_) => "bytea",
            Self::Json(_) => "jsonb",
            Self::Uuid(_) => "uuid",
            Self::Timestamp(_) => "timestamptz",
            Self::Array(arr) => {
                if let Some(first) = arr.first() {
                    match first {
                        Self::Text(_) => "text[]",
                        Self::Int(_) => "bigint[]",
                        Self::Bool(_) => "boolean[]",
                        _ => "unknown[]",
                    }
                } else {
                    "text[]"
                }
            }
        }
    }
}

impl From<String> for SqlParam {
    fn from(s: String) -> Self {
        Self::Text(s)
    }
}

impl From<&str> for SqlParam {
    fn from(s: &str) -> Self {
        Self::Text(s.to_string())
    }
}

impl From<i32> for SqlParam {
    fn from(n: i32) -> Self {
        Self::Int(n as i64)
    }
}

impl From<i64> for SqlParam {
    fn from(n: i64) -> Self {
        Self::Int(n)
    }
}

impl From<f64> for SqlParam {
    fn from(n: f64) -> Self {
        Self::Float(n)
    }
}

impl From<bool> for SqlParam {
    fn from(b: bool) -> Self {
        Self::Bool(b)
    }
}

impl From<JsonValue> for SqlParam {
    fn from(v: JsonValue) -> Self {
        Self::Json(v)
    }
}

impl From<Vec<String>> for SqlParam {
    fn from(v: Vec<String>) -> Self {
        Self::Array(v.into_iter().map(SqlParam::Text).collect())
    }
}

impl<T: Into<SqlParam>> From<Option<T>> for SqlParam {
    fn from(opt: Option<T>) -> Self {
        match opt {
            Some(v) => v.into(),
            None => Self::Null,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sql_param_types() {
        assert_eq!(SqlParam::text("hello").pg_type(), "text");
        assert_eq!(SqlParam::int(42).pg_type(), "bigint");
        assert_eq!(SqlParam::Bool(true).pg_type(), "boolean");
        assert_eq!(SqlParam::Null.pg_type(), "unknown");
    }

    #[test]
    fn test_sql_param_from() {
        let p: SqlParam = "hello".into();
        assert!(matches!(p, SqlParam::Text(s) if s == "hello"));

        let p: SqlParam = 42i64.into();
        assert!(matches!(p, SqlParam::Int(42)));

        let p: SqlParam = None::<String>.into();
        assert!(p.is_null());
    }
}
