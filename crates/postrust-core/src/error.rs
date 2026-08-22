//! Error types for Postrust.
//!
//! Provides comprehensive error handling with HTTP status code mapping.

use http::StatusCode;
use thiserror::Error;

/// Result type for Postrust operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Main error type for Postrust.
#[derive(Error, Debug)]
pub enum Error {
    // ========================================================================
    // Request Parsing Errors (4xx)
    // ========================================================================
    #[error("Invalid path: {0}")]
    InvalidPath(String),

    /// An aggregate was requested while `db-aggregates-enabled` is off.
    ///
    /// Off is the default, matching PostgREST: an unrestricted aggregate over
    /// a large table is an easy way to make a server do far more work than the
    /// request looks like it asks for.
    #[error("Use of aggregate functions is not allowed")]
    AggregatesNotAllowed,

    #[error("Invalid query parameter: {0}")]
    InvalidQueryParam(String),

    #[error("Invalid header: {0}")]
    InvalidHeader(&'static str),

    #[error("Invalid request body: {0}")]
    InvalidBody(String),

    #[error("Unsupported HTTP method: {0}")]
    UnsupportedMethod(String),

    /// A profile header named a schema the server does not expose.
    ///
    /// Carries the schemas that *are* exposed, because the client needs them
    /// to correct the request and they are part of the API's own contract.
    #[error("Invalid schema: {requested}")]
    UnacceptableSchema {
        requested: String,
        exposed: Vec<String>,
    },

    #[error("Unknown column: {0}")]
    UnknownColumn(String),

    #[error("Invalid range: {0}")]
    InvalidRange(String),

    #[error("Invalid media type: {0}")]
    InvalidMediaType(String),

    #[error("Missing required parameter: {0}")]
    MissingParameter(String),

    #[error("Ambiguous request: {0}")]
    AmbiguousRequest(String),

    /// Several signatures of a function fit the arguments equally well.
    #[error("Could not choose the best candidate function between: {}", .candidates.join(", "))]
    AmbiguousFunction { candidates: Vec<String> },

    // ========================================================================
    // Authentication/Authorization Errors (401/403)
    // ========================================================================
    #[error("Invalid JWT: {0}")]
    InvalidJwt(String),

    #[error("JWT expired")]
    JwtExpired,

    #[error("Missing authentication")]
    MissingAuth,

    #[error("Insufficient permissions: {0}")]
    InsufficientPermissions(String),

    // ========================================================================
    // Resource Errors (404)
    // ========================================================================
    #[error("Resource not found: {0}")]
    NotFound(String),

    #[error("Table not found: {0}")]
    TableNotFound(String),

    /// No function of that name accepts the arguments supplied.
    ///
    /// Carries what was looked for so the client can see why nothing matched:
    /// the name, the argument names it was called with, and the signature of
    /// an overload that does exist, where one does.
    #[error("Could not find the function {name}")]
    FunctionNotFound {
        /// The function's qualified name, without arguments.
        name: String,
        /// The argument names it was called with.
        params: Vec<String>,
        /// A signature that does exist, where one does.
        candidate: Option<String>,
    },

    #[error("Column not found: {0}")]
    ColumnNotFound(String),

    #[error("Relationship not found: {0}")]
    RelationshipNotFound(String),

    // ========================================================================
    // Schema Cache Errors
    // ========================================================================
    #[error("Schema cache not loaded")]
    SchemaCacheNotLoaded,

    #[error("Schema cache load failed: {0}")]
    SchemaCacheLoadFailed(String),

    // ========================================================================
    // Database Errors (500/4xx depending on type)
    // ========================================================================
    #[error("Database error: {0}")]
    Database(#[from] DatabaseError),

    #[error("Connection pool error: {0}")]
    ConnectionPool(String),

    // ========================================================================
    // Internal Errors (500)
    // ========================================================================
    #[error("Internal error: {0}")]
    Internal(String),

    #[error("Configuration error: {0}")]
    Config(String),

    // ========================================================================
    // Plan Errors
    // ========================================================================
    #[error("Invalid plan: {0}")]
    InvalidPlan(String),

    #[error("Embedding error: {0}")]
    EmbeddingError(String),
}

impl Error {
    /// Get the HTTP status code for this error.
    pub fn status_code(&self) -> StatusCode {
        match self {
            // 400 Bad Request
            Self::InvalidPath(_)
            | Self::InvalidQueryParam(_)
            | Self::InvalidHeader(_)
            | Self::InvalidBody(_)
            | Self::InvalidRange(_)
            | Self::InvalidMediaType(_)
            | Self::MissingParameter(_)
            | Self::AmbiguousRequest(_)
            | Self::UnknownColumn(_)
            | Self::InvalidPlan(_)
            | Self::EmbeddingError(_)
            | Self::AggregatesNotAllowed => StatusCode::BAD_REQUEST,

            // The request names something real, but not which one of it.
            Self::AmbiguousFunction { .. } => StatusCode::MULTIPLE_CHOICES,

            // 401 Unauthorized
            Self::InvalidJwt(_) | Self::JwtExpired | Self::MissingAuth => StatusCode::UNAUTHORIZED,

            // 403 Forbidden
            Self::InsufficientPermissions(_) => StatusCode::FORBIDDEN,

            // 404 Not Found
            Self::NotFound(_)
            | Self::TableNotFound(_)
            | Self::FunctionNotFound { .. }
            | Self::ColumnNotFound(_)
            | Self::RelationshipNotFound(_) => StatusCode::NOT_FOUND,

            // 405 Method Not Allowed
            Self::UnsupportedMethod(_) => StatusCode::METHOD_NOT_ALLOWED,

            // 406 Not Acceptable
            Self::UnacceptableSchema { .. } => StatusCode::NOT_ACCEPTABLE,

            // 500 Internal Server Error
            Self::SchemaCacheNotLoaded
            | Self::SchemaCacheLoadFailed(_)
            | Self::ConnectionPool(_)
            | Self::Internal(_)
            | Self::Config(_) => StatusCode::INTERNAL_SERVER_ERROR,

            // Database errors map based on type
            Self::Database(db_err) => db_err.status_code(),
        }
    }

    /// Get the error code for API responses.
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidPath(_) => "PGRST100",
            Self::AggregatesNotAllowed => "PGRST123",
            // PGRST100 is the parse failure in PostgREST's taxonomy, covering
            // both the path and the query string; PGRST101 is a function
            // invoked with a method its volatility does not allow.
            Self::InvalidQueryParam(_) => "PGRST100",
            Self::InvalidHeader(_) => "PGRST102",
            Self::InvalidBody(_) => "PGRST103",
            Self::UnsupportedMethod(_) => "PGRST104",
            Self::UnacceptableSchema { .. } => "PGRST106",
            Self::UnknownColumn(_) => "PGRST204",
            Self::InvalidRange(_) => "PGRST107",
            Self::InvalidMediaType(_) => "PGRST108",
            Self::MissingParameter(_) => "PGRST109",
            Self::AmbiguousRequest(_) => "PGRST110",
            Self::AmbiguousFunction { .. } => "PGRST203",

            Self::InvalidJwt(_) => "PGRST303",
            Self::JwtExpired => "PGRST301",
            Self::MissingAuth => "PGRST302",
            Self::InsufficientPermissions(_) => "PGRST203",

            Self::NotFound(_) => "PGRST205",
            Self::TableNotFound(_) => "PGRST205",
            Self::FunctionNotFound { .. } => "PGRST202",
            Self::ColumnNotFound(_) => "PGRST204",
            Self::RelationshipNotFound(_) => "PGRST200",

            Self::SchemaCacheNotLoaded => "PGRST400",
            Self::SchemaCacheLoadFailed(_) => "PGRST401",

            Self::Database(e) => e.code(),
            Self::ConnectionPool(_) => "PGRST500",

            Self::Internal(_) => "PGRST900",
            Self::Config(_) => "PGRST901",

            Self::InvalidPlan(_) => "PGRST600",
            Self::EmbeddingError(_) => "PGRST601",
        }
    }

    /// Convert to JSON error response.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "code": self.code(),
            "message": self.to_string(),
            "details": self.details(),
            "hint": self.hint(),
        })
    }

    /// Get additional details for the error.
    pub fn details(&self) -> Option<String> {
        match self {
            Self::Database(db_err) => db_err.details.clone(),
            Self::FunctionNotFound { name, params, .. } => Some(format!(
                "Searched for the function {} {}, but no matches were found in the schema cache.",
                name,
                describe_params(params)
            )),
            _ => None,
        }
    }

    /// Get a hint for resolving the error.
    pub fn hint(&self) -> Option<String> {
        match self {
            Self::InvalidJwt(_) => {
                Some("Check that the JWT is properly signed and not expired".into())
            }
            Self::MissingAuth => Some("Provide a valid JWT in the Authorization header".into()),
            Self::AmbiguousFunction { .. } => Some(
                "Try renaming the parameters or the function itself in the database so function \
                 overloading can be resolved"
                    .into(),
            ),
            Self::FunctionNotFound { candidate, .. } => candidate
                .as_ref()
                .map(|c| format!("Perhaps you meant to call the function {}", c)),
            // The schemas a server exposes are part of its contract, not an
            // internal detail, so naming them is how the client is told what
            // to ask for instead.
            Self::UnacceptableSchema { exposed, .. } => Some(format!(
                "Only the following schemas are exposed: {}",
                exposed.join(", ")
            )),
            Self::Database(db_err) => db_err.hint.clone(),
            _ => None,
        }
    }
}

/// How a function's argument list reads in an error message.
///
/// PostgREST words the no-argument case differently rather than printing an
/// empty list, and the messages are matched verbatim by clients.
pub fn function_signature(name: &str, params: &[String]) -> String {
    match params.is_empty() {
        true => format!("{} without parameters", name),
        false => format!("{}({})", name, params.join(", ")),
    }
}

fn describe_params(params: &[String]) -> String {
    match params.len() {
        0 => "without parameters".to_string(),
        1 => format!("with parameter {}", params[0]),
        _ => format!("with parameters {}", params.join(", ")),
    }
}

/// Database-specific error type.
#[derive(Error, Debug)]
#[error("Database error [{code}]: {message}")]
pub struct DatabaseError {
    pub code: String,
    pub message: String,
    pub details: Option<String>,
    pub hint: Option<String>,
    pub constraint: Option<String>,
    pub table: Option<String>,
    pub column: Option<String>,
}

impl DatabaseError {
    /// Get HTTP status code based on PostgreSQL error code.
    pub fn status_code(&self) -> StatusCode {
        // PostgreSQL error codes: https://www.postgresql.org/docs/current/errcodes-appendix.html
        match self.code.as_str() {
            // A write attempted inside the read-only transaction a read
            // request runs in. The request was not wrong, the method was.
            "25006" => StatusCode::METHOD_NOT_ALLOWED,
            // Class 23 - Integrity Constraint Violation
            c if c.starts_with("23") => StatusCode::CONFLICT,
            // Class 42 - Syntax Error or Access Rule Violation
            c if c.starts_with("42") => StatusCode::BAD_REQUEST,
            // Class 28 - Invalid Authorization Specification
            c if c.starts_with("28") => StatusCode::FORBIDDEN,
            // Class 40 - Transaction Rollback
            c if c.starts_with("40") => StatusCode::CONFLICT,
            // Class 53 - Insufficient Resources
            c if c.starts_with("53") => StatusCode::SERVICE_UNAVAILABLE,
            // Class 54 - Program Limit Exceeded
            c if c.starts_with("54") => StatusCode::PAYLOAD_TOO_LARGE,
            // Class P0 - PL/pgSQL Errors (custom raised errors)
            "P0001" => StatusCode::BAD_REQUEST, // RAISE EXCEPTION
            // Default to internal server error
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// Get error code for API response.
    pub fn code(&self) -> &'static str {
        match self.code.as_str() {
            c if c.starts_with("23") => "PGRST503", // Constraint violation
            c if c.starts_with("42") => "PGRST504", // SQL error
            c if c.starts_with("28") => "PGRST505", // Auth error
            _ => "PGRST500",                        // Generic database error
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_status_codes() {
        assert_eq!(
            Error::InvalidQueryParam("test".into()).status_code(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(Error::MissingAuth.status_code(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            Error::TableNotFound("users".into()).status_code(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            Error::UnsupportedMethod("TRACE".into()).status_code(),
            StatusCode::METHOD_NOT_ALLOWED
        );
    }

    #[test]
    fn test_error_codes() {
        // A malformed query string is a parse failure, PGRST100 -- the same
        // code as a malformed path. PGRST101 means something else entirely.
        assert_eq!(Error::InvalidQueryParam("test".into()).code(), "PGRST100");
        assert_eq!(Error::MissingAuth.code(), "PGRST302");
        assert_eq!(Error::TableNotFound("users".into()).code(), "PGRST205");
    }

    #[test]
    fn test_database_error_status() {
        let constraint_error = DatabaseError {
            code: "23505".into(), // unique_violation
            message: "Duplicate key".into(),
            details: None,
            hint: None,
            constraint: Some("users_pkey".into()),
            table: Some("users".into()),
            column: None,
        };
        assert_eq!(constraint_error.status_code(), StatusCode::CONFLICT);
    }

    #[test]
    fn test_error_to_json() {
        let error = Error::InvalidQueryParam("bad filter".into());
        let json = error.to_json();
        assert_eq!(json["code"], "PGRST100");
        assert!(json["message"].as_str().unwrap().contains("bad filter"));
    }
}
