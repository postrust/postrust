//! Configuration types and loaders for the proxy.
//!
//! Supports both file-based (TOML) and database-backed configuration,
//! with hot-reload via file watching and PostgreSQL LISTEN/NOTIFY.

mod database;
mod file;
mod reload;
mod types;

pub use database::load_from_database;
pub use file::load_from_file;
pub use reload::ConfigReloader;
pub use types::*;
