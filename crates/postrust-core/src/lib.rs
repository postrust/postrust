//! Postrust Core - PostgREST-compatible REST API for PostgreSQL in Rust.
//!
//! This crate provides the core functionality for Postrust, a serverless
//! alternative to PostgREST written in Rust.
//!
//! # Architecture
//!
//! The request processing pipeline:
//!
//! 1. **API Request Parsing** (`api_request`) - Parse HTTP request into domain types
//! 2. **Schema Cache** (`schema_cache`) - PostgreSQL metadata for validation
//! 3. **Query Planning** (`plan`) - Convert request to execution plan
//! 4. **SQL Generation** (`query`) - Generate parameterized SQL
//! 5. **Response Formatting** - Format results for HTTP response
//!
//! # Example
//!
//! ```text
//! use postrust_core::{ApiRequest, SchemaCache, create_action_plan};
//!
//! // Parse HTTP request
//! let request = parse_request(&http_request, "public", &schemas)?;
//!
//! // Create execution plan
//! let plan = create_action_plan(&request, &schema_cache)?;
//!
//! // Generate SQL
//! let (sql, params) = build_query(&plan)?;
//! ```

// The `Error` enum carries a rich `DatabaseError` payload (PostgREST-style error
// bodies), which makes it larger than clippy's `Result`/enum size thresholds.
// Boxing it would ripple through every construction site for little benefit, so
// the size lints are allowed crate-wide instead.
#![allow(clippy::result_large_err)]
#![allow(clippy::large_enum_variant)]

pub mod api_request;
pub mod config;
pub mod embed;
pub mod error;
pub mod plan;
pub mod query;
pub mod row_json;
pub mod schema_cache;

// Re-export main types
pub use api_request::{
    parse_request, Action, ApiRequest, DbAction, Filter, LogicTree, MediaType, Mutation, Operation,
    Payload, PreferRepresentation, Preferences, QualifiedIdentifier, QueryParams, Range, Resource,
    SelectItem,
};
pub use config::{AppConfig, IsolationLevel, LogLevel};
pub use embed::EmbedRendering;
pub use error::{Error, Result};
pub use plan::{create_action_plan, ActionPlan, CallPlan, DbActionPlan, MutatePlan, ReadPlan};
pub use schema_cache::{Column, Relationship, Routine, SchemaCache, SchemaCacheRef, Table};

/// Prelude for common imports.
pub mod prelude {
    pub use super::api_request::{
        parse_request, Action, ApiRequest, Filter, MediaType, Preferences, QualifiedIdentifier,
        QueryParams, Range, SelectItem,
    };
    pub use super::config::AppConfig;
    pub use super::error::{Error, Result};
    pub use super::plan::{create_action_plan, ActionPlan};
    pub use super::schema_cache::{SchemaCache, SchemaCacheRef, Table};
}
