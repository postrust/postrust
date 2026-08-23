//! Stored function/procedure types.

use crate::api_request::QualifiedIdentifier;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A stored function or procedure.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Routine {
    /// Schema name
    pub schema: String,
    /// Function name
    pub name: String,
    /// Description from comment
    pub description: Option<String>,
    /// Function parameters
    pub params: Vec<RoutineParam>,
    /// Return type
    pub return_type: RetType,
    /// Whether the return type is composite (a row type or `record`), meaning
    /// `SELECT * FROM fn()` expands to the type's own columns rather than a
    /// single column named after the function.
    #[serde(default)]
    pub returns_composite: bool,
    /// Function volatility
    pub volatility: FuncVolatility,
    /// Whether the function has VARIADIC parameters
    pub has_variadic: bool,
    /// Isolation level (if set by function)
    pub isolation_level: Option<String>,
    /// Function-level GUC settings
    pub settings: Vec<(String, String)>,
    /// Whether this is a procedure (vs function)
    pub is_procedure: bool,
    /// The media type this function's value already is.
    ///
    /// A domain named `image/png` is a declaration about the value, not a type
    /// the API layer serialises: `returns "text/plain"` returns text, and
    /// wrapping it in JSON would be answering a question nobody asked. `*/*`
    /// is spelled that way in the catalogue and reaches a client as
    /// `application/octet-stream`, which is what "some bytes" is called on the
    /// wire.
    #[serde(default)]
    pub media_type: Option<String>,
    /// The type that media-type domain is a domain over.
    ///
    /// A `bytea` renders as `\x` followed by hex when this side cannot name
    /// its type, and that rendering is a description of the bytes rather than
    /// the bytes. See [`Self::media_type`].
    #[serde(default)]
    pub media_base_type: Option<String>,
    /// The columns a `RETURNS TABLE` function's rows have, as `(name, type)`.
    ///
    /// A function returning rows of a table is shaped by that table, whose
    /// columns are known. One declaring its own columns is not, and this is
    /// the only account of them -- without it the result can only be selected
    /// with `*`, and a column of a type this process cannot decode comes back
    /// null with nothing to say it should have been rendered by the database.
    #[serde(default)]
    pub output_columns: Vec<(String, String)>,
}

impl Routine {
    /// The media type this function produces, as a client would name it.
    pub fn produced_media_type(&self) -> Option<&str> {
        match self.media_type.as_deref() {
            Some("*/*") => Some("application/octet-stream"),
            other => other,
        }
    }
}

impl Routine {
    /// Get the qualified identifier for this routine.
    pub fn qualified_identifier(&self) -> QualifiedIdentifier {
        QualifiedIdentifier::new(&self.schema, &self.name)
    }

    /// Check if this function is safe for GET requests.
    pub fn is_safe_for_get(&self) -> bool {
        matches!(
            self.volatility,
            FuncVolatility::Immutable | FuncVolatility::Stable
        )
    }

    /// Get required parameters (no default).
    pub fn required_params(&self) -> impl Iterator<Item = &RoutineParam> {
        self.params.iter().filter(|p| p.required)
    }

    /// Find a parameter by name.
    pub fn find_param(&self, name: &str) -> Option<&RoutineParam> {
        self.params.iter().find(|p| p.name == name)
    }
}

/// A function parameter.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RoutineParam {
    /// Parameter name
    pub name: String,
    /// PostgreSQL type
    pub param_type: String,
    /// Type with max length info
    pub type_max_length: String,
    /// Whether this parameter is required
    pub required: bool,
    /// Whether this is a VARIADIC parameter
    pub variadic: bool,
}

/// Function return type.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum RetType {
    /// Returns a single value
    Single(String),
    /// Returns a set of values (SETOF)
    SetOf(String),
    /// Returns a table (RETURNS TABLE)
    Table(Vec<(String, String)>),
    /// Returns void
    Void,
}

impl RetType {
    /// Check if this returns multiple rows.
    pub fn is_set_returning(&self) -> bool {
        matches!(self, Self::SetOf(_) | Self::Table(_))
    }

    /// Get the base type name.
    pub fn type_name(&self) -> Option<&str> {
        match self {
            Self::Single(t) => Some(t),
            Self::SetOf(t) => Some(t),
            Self::Table(_) => None,
            Self::Void => None,
        }
    }
}

/// Function volatility category.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FuncVolatility {
    /// Function cannot modify database and always returns same result for same inputs
    Immutable,
    /// Function cannot modify database but result may change across queries
    Stable,
    /// Function can modify database
    Volatile,
}

impl FuncVolatility {
    pub fn from_char(c: char) -> Self {
        match c {
            'i' => Self::Immutable,
            's' => Self::Stable,
            _ => Self::Volatile,
        }
    }
}

/// Map of qualified identifier to routines (overloaded functions share name).
pub type RoutineMap = HashMap<QualifiedIdentifier, Vec<Routine>>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_routine_is_safe_for_get() {
        let mut routine = Routine {
            output_columns: Vec::new(),
            media_type: None,
            media_base_type: None,
            schema: "public".into(),
            name: "get_users".into(),
            description: None,
            params: vec![],
            return_type: RetType::SetOf("users".into()),
            returns_composite: true,
            volatility: FuncVolatility::Stable,
            has_variadic: false,
            isolation_level: None,
            settings: vec![],
            is_procedure: false,
        };

        assert!(routine.is_safe_for_get());

        routine.volatility = FuncVolatility::Volatile;
        assert!(!routine.is_safe_for_get());
    }

    #[test]
    fn test_ret_type_is_set_returning() {
        assert!(!RetType::Single("text".into()).is_set_returning());
        assert!(RetType::SetOf("users".into()).is_set_returning());
        assert!(RetType::Table(vec![("id".into(), "int".into())]).is_set_returning());
        assert!(!RetType::Void.is_set_returning());
    }

    #[test]
    fn test_func_volatility_from_char() {
        assert_eq!(FuncVolatility::from_char('i'), FuncVolatility::Immutable);
        assert_eq!(FuncVolatility::from_char('s'), FuncVolatility::Stable);
        assert_eq!(FuncVolatility::from_char('v'), FuncVolatility::Volatile);
        assert_eq!(FuncVolatility::from_char('x'), FuncVolatility::Volatile);
    }
}
