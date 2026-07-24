//! Type-safe SQL builder for Postrust.
//!
//! Provides a safe way to construct SQL queries without string concatenation,
//! using parameterized queries to prevent SQL injection.

mod builder;
mod delete;
mod expr;
pub mod identifier;
mod insert;
mod param;
mod select;
mod update;

pub use builder::{SqlBuilder, SqlFragment};
pub use delete::DeleteBuilder;
pub use expr::{Expr, OrderExpr};
pub use identifier::{escape_ident, from_qi, quote_literal, QualifiedIdentifier};
pub use insert::InsertBuilder;
pub use param::SqlParam;
pub use select::SelectBuilder;
pub use update::UpdateBuilder;

/// Prelude for common imports.
pub mod prelude {
    pub use super::{
        escape_ident, from_qi, quote_literal, DeleteBuilder, Expr, InsertBuilder, OrderExpr,
        SelectBuilder, SqlBuilder, SqlFragment, SqlParam, UpdateBuilder,
    };
}
