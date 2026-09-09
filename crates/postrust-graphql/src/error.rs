//! GraphQL-specific error types.

use thiserror::Error;

/// Errors that can occur during GraphQL operations.
#[derive(Debug, Error)]
pub enum GraphQLError {
    /// The schema could not be built from the database's metadata. A fault in
    /// the translation, not in the request: no query has been seen yet.
    #[error("Schema generation failed: {0}")]
    SchemaGeneration(String),

    /// The built schema was rejected, or a lookup into it failed.
    #[error("Schema error: {0}")]
    SchemaError(String),

    /// The query parsed and resolving it failed.
    #[error("Query execution failed: {0}")]
    QueryExecution(String),

    /// A `where` argument that does not describe a filter this server can
    /// express -- an unknown operator, or an operand of the wrong shape.
    #[error("Invalid filter: {0}")]
    InvalidFilter(String),

    /// A PostgreSQL type with no GraphQL equivalent to map it onto.
    #[error("Type mapping error: {0}")]
    TypeMapping(String),

    /// The field needs an identity and the request carried none.
    #[error("Authentication required")]
    AuthenticationRequired,

    /// PostgreSQL refused the query. Its message is carried verbatim.
    #[error("Database error: {0}")]
    Database(String),
}

/// `Result` with this crate's error type as the default failure.
pub type Result<T> = std::result::Result<T, GraphQLError>;
