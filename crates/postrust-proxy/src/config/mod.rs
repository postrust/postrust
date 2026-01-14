//! Configuration types and loaders for the proxy.
//!
//! Supports both file-based (TOML) and database-backed configuration,
//! with hot-reload via file watching and PostgreSQL LISTEN/NOTIFY.

mod types;
mod file;
mod database;
mod reload;

pub use types::*;
pub use file::load_from_file;
pub use database::load_from_database;
pub use reload::ConfigReloader;
