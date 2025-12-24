//! GraphQL-specific error types.

use thiserror::Error;

/// Errors that can occur during GraphQL operations.
#[derive(Debug, Error)]
pub enum GraphQLError {
    #[error("Schema generation failed: {0}")]
    SchemaGeneration(String),

    #[error("Schema error: {0}")]
    SchemaError(String),

    #[error("Query execution failed: {0}")]
    QueryExecution(String),

    #[error("Invalid filter: {0}")]
    InvalidFilter(String),

    #[error("Type mapping error: {0}")]
    TypeMapping(String),

    #[error("Authentication required")]
    AuthenticationRequired,

    #[error("Database error: {0}")]
    Database(String),
}

pub type Result<T> = std::result::Result<T, GraphQLError>;
