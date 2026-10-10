//! # postrust-ai
//!
//! On-demand AI model serving for Postrust.
//!
//! - **Lazy loading.** Models (video, audio, text) start the first time a job
//!   or request needs them, and are parked or unloaded when idle or when
//!   something more important needs the memory. See [`manager`].
//! - **Priority queues** in PostgreSQL, from `critical` down to `superlow`.
//!   Superlow jobs run only on spare capacity and are paused -- keeping a
//!   checkpoint, or requeued from scratch, per queue -- whenever anything else
//!   needs their slot or their GPU. See [`queue`], [`scheduler`],
//!   [`executor`].
//! - **Workflows**: steps chained into a graph, with fan-out and fan-in. See
//!   [`workflow`].
//! - **Session locks**: a conversation sticks to one model instance, which
//!   stays loaded while the session lives. See [`session`].
//! - **Caching** at five layers, from model files on disk to whole job
//!   results. See [`cache`].
//! - **Routing** through postrust-proxy, with nodes forwarding to each other.
//!   See [`proxy`] and [`api`].
//!
//! Model servers run as separate processes or containers -- vLLM,
//! whisper.cpp, Ollama, or anything that speaks HTTP -- and this crate starts,
//! stops, parks and routes to them. It never runs a model in-process.
//!
//! ```text
//!                  postrust-proxy  (/ai/* -> any node)
//!                         |
//!      +------------------+------------------+
//!      |               node                  |
//!      |  api/gateway --- sessions --- nodes |----> peer nodes
//!      |       |                             |
//!      |  scheduler -> executor -> manager --+--> model servers
//!      |       |                    |        |     (vLLM, whisper.cpp,
//!      |  queue (Postgres)    resources,     |      Ollama, ...)
//!      |  workflows           file cache     |
//!      +-------------------------------------+
//! ```

#![warn(missing_docs)]

pub mod api;
pub mod app;
pub mod cache;
pub mod config;
pub mod executor;
pub mod manager;
pub mod measure;
pub mod metrics;
pub mod model;
pub mod nodes;
pub mod proxy;
pub mod queue;
pub mod resources;
pub mod runtime;
pub mod scheduler;
pub mod session;
pub mod workflow;

mod error;

pub use config::AiConfig;
pub use error::{AiError, AiResult};
pub use model::{Flavor, ModelKind, ModelSpec, RuntimeSpec};
pub use queue::{JobStore, NewJob, PauseMode, PgJobStore, Priority, QueueSpec};
