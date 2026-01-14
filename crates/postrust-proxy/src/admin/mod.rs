//! Admin REST API and UI for proxy management.

pub mod api;
mod ui;

pub use api::{admin_router, ApiResponse};
pub use ui::admin_ui_router;
