//! Vendored proxy logic from rust-rpxy.
//!
//! This module contains adapted code from rpxy-lib for the core proxy functionality:
//! - HTTP/1.1 and HTTP/2 request handling
//! - Upstream forwarding
//! - Load balancing (round-robin, random, sticky)
//! - Header manipulation (X-Forwarded-*, Host rewriting)
//! - Hyper body types and utilities
//!
//! The original code is from: https://github.com/junkurihara/rust-rpxy
//! Licensed under MIT License.

mod backend;
mod forwarder;
mod handler;
mod hyper_ext;
mod proxy;
mod types;

pub use backend::{BackendAppManager, LoadBalance, LoadBalanceRandom, LoadBalanceRoundRobin};
pub use forwarder::ForwarderClient;
pub use handler::MessageHandler;
pub use proxy::ProxyService;
pub use types::{PathName, ProxyError as VendoredError, ServerName};
