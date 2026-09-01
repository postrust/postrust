//! Configuration types and loaders for the proxy.
//!
//! Supports both file-based (TOML) and database-backed configuration,
//! with hot-reload via file watching and PostgreSQL LISTEN/NOTIFY.

mod database;
mod file;
mod reload;
mod types;

pub use database::{
    delete_route, delete_upstream, load_from_database, load_routes, load_upstreams, save_route,
    save_upstream,
};
pub use file::load_from_file;
pub use reload::ConfigReloader;
pub use types::*;
