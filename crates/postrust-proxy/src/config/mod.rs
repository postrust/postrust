//! Configuration types and loaders for the proxy.
//!
//! Supports both file-based (TOML) and database-backed configuration.
//!
//! There is no hot reload. A `ConfigReloader` used to sit here with a channel
//! nobody read, and `POST /config/reload` answered "Configuration reload
//! requested" without reloading anything. Both are gone rather than left to be
//! believed; changing the configuration needs a restart.

mod database;
mod file;
mod types;

pub use database::{
    delete_route, delete_upstream, load_from_database, load_routes, load_upstreams, save_route,
    save_upstream,
};
pub use file::load_from_file;
pub use types::*;
