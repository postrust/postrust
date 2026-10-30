//! Measurements the pipeline takes for a host that exports metrics.
//!
//! The engine is shared with hosts that have no metrics pipeline -- a
//! Cloudflare Worker, a test -- and builds for wasm32, so it does not depend on
//! OpenTelemetry or any other metrics crate. Instead a host that does export
//! metrics installs an [`Observer`] once, at start-up, and the pipeline calls
//! it with what it measured.
//!
//! # What it costs when nothing is installed
//!
//! One load of an initialised-or-not flag per measurement point, and nothing
//! else: the clock is not even read unless an observer is there to be told.
//! That is the point of doing it this way rather than always timing and
//! throwing the result away -- a request that nobody is measuring pays for
//! nothing it does not use.
//!
//! # Why a process-wide hook
//!
//! Metrics are process-wide anyway: an OpenTelemetry meter provider and a
//! Prometheus registry are each one per process, and so is the endpoint a
//! scraper reads. Threading an observer through [`State`](crate::State) would
//! give every host a method to implement for something only one of them uses.

use std::sync::OnceLock;
use std::time::Duration;
// `std::time` natively; on wasm32, where `Instant::now()` panics, the
// JavaScript host's clock.
use web_time::Instant;

/// What a host wants to be told about. Every method has a default that
/// ignores the measurement, so an observer names only what it records, and a
/// measurement added here later breaks no host's build.
pub trait Observer: Send + Sync + 'static {
    /// How long a request waited for a transaction to begin: a connection
    /// from the pool, and the `BEGIN` sent on it. Mostly the pool -- `BEGIN`
    /// is one round trip -- so a wait that grows is a pool that is too small
    /// for the load, or a database that is slow to hand connections back.
    fn transaction_begun(&self, waited: Duration) {
        let _ = waited;
    }
}

static OBSERVER: OnceLock<Box<dyn Observer>> = OnceLock::new();

/// Install the observer, once per process.
///
/// Returns the observer back if one is already installed: the first one
/// stays, since measurements already sent to it would otherwise be split
/// between two pipelines.
pub fn set_observer(observer: impl Observer) -> Result<(), Box<dyn Observer>> {
    OBSERVER.set(Box::new(observer))
}

/// A timer for the wait for a transaction, started only if somebody will be
/// told what it measured.
pub(crate) fn begin_timer() -> Option<Instant> {
    OBSERVER.get().map(|_| Instant::now())
}

/// Report what [`begin_timer`] started.
pub(crate) fn transaction_begun(started: Option<Instant>) {
    if let (Some(observer), Some(started)) = (OBSERVER.get(), started) {
        observer.transaction_begun(started.elapsed());
    }
}
