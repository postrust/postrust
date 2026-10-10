//! Caches. The layers, from slowest to fastest to restore:
//!
//! 1. [`FileCache`]: model files on local disk, instead of the network.
//! 2. Loaded instances kept warm: the manager's idle timeouts, `pinned`,
//!    `min_warm` and queue-driven preloading.
//! 3. Parked instances: weights moved from VRAM to host memory (vLLM sleep
//!    mode), woken in seconds instead of reloaded.
//! 4. Server-side prompt/KV caches, made useful by session locks routing a
//!    conversation to the same instance.
//! 5. [`ResultCache`]: whole job results.
//!
//! Layers 2-4 live in [`crate::manager`] and [`crate::session`].

mod files;
mod results;

pub use files::{resolve_url, CachedFile, FileCache};
pub use results::{canonical, ResultCache};
