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
}

impl Routine {
    /// Get the qualified identifier for this routine.
    pub fn qualified_identifier(&self) -> QualifiedIdentifier {
        QualifiedIdentifier::new(&self.schema, &self.name)
    }

    /// Check if this function is safe for GET requests.
    pub fn is_safe_for_get(&self) -> bool {
        matches!(self.volatility, FuncVolatility::Immutable | FuncVolatility::Stable)
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
            schema: "public".into(),
            name: "get_users".into(),
            description: None,
            params: vec![],
            return_type: RetType::SetOf("users".into()),
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
