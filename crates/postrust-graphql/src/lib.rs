//! GraphQL support for Postrust.
//!
//! This crate provides GraphQL API generation from PostgreSQL schema,
//! including queries, mutations, and subscriptions.

pub mod types;
pub mod scalar;
pub mod error;

pub mod schema;
pub mod input;
pub mod resolver;
pub mod subscription;

pub mod context;
pub mod handler;

// Re-exports
pub use error::GraphQLError;
pub use types::GraphQLType;
