//! Type-safe SQL builder for Postrust.
//!
//! Provides a safe way to construct SQL queries without string concatenation,
//! using parameterized queries to prevent SQL injection.

mod builder;
mod expr;
pub mod identifier;
mod param;
mod select;
mod insert;
mod update;
mod delete;

pub use builder::{SqlBuilder, SqlFragment};
pub use expr::{Expr, OrderExpr};
pub use identifier::{escape_ident, quote_literal, from_qi, QualifiedIdentifier};
pub use param::SqlParam;
pub use select::SelectBuilder;
pub use insert::InsertBuilder;
pub use update::UpdateBuilder;
pub use delete::DeleteBuilder;

/// Prelude for common imports.
pub mod prelude {
    pub use super::{
        SqlBuilder, SqlFragment, SqlParam,
        SelectBuilder, InsertBuilder, UpdateBuilder, DeleteBuilder,
        Expr, OrderExpr,
        escape_ident, quote_literal, from_qi,
    };
}
