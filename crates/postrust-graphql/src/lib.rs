//! GraphQL support for Postrust.
//!
//! This crate provides GraphQL API generation from PostgreSQL schema,
//! including queries, mutations, and subscriptions.

pub mod error;
pub mod scalar;
pub mod types;

pub mod input;
pub mod resolver;
pub mod schema;
pub mod subscription;

pub mod context;
pub mod handler;
pub mod hasura;
pub mod names;

// Re-exports
pub use error::GraphQLError;
pub use types::GraphQLType;
