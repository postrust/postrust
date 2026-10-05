//! Error type for postrust-ai.

use std::time::Duration;

/// Everything that can go wrong in postrust-ai.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum AiError {
    /// The configuration is unusable. Raised at startup, never mid-flight.
    #[error("configuration: {0}")]
    Config(String),

    /// The database refused or could not be reached.
    #[error("database: {0}")]
    Database(#[from] sqlx::Error),

    /// No model with this id is registered.
    #[error("unknown model '{0}'")]
    UnknownModel(String),

    /// No queue with this name is configured.
    #[error("unknown queue '{0}'")]
    UnknownQueue(String),

    /// The model could not fit within the node's resource budget, even after
    /// unloading everything this request was allowed to unload.
    #[error("not enough resources to load '{model}': {detail}")]
    InsufficientResources {
        /// The model that did not fit.
        model: String,
        /// What was short.
        detail: String,
    },

    /// The model did not become ready in time. The load may still finish; a
    /// retry after this will find it.
    #[error("model '{model}' was not ready within {waited:?}")]
    LoadTimeout {
        /// The model being waited for.
        model: String,
        /// How long the caller waited.
        waited: Duration,
    },

    /// The model server failed to start or failed its health check.
    #[error("model '{model}' failed to load: {detail}")]
    LoadFailed {
        /// The model that failed.
        model: String,
        /// Why.
        detail: String,
    },

    /// A model server answered, but not with success.
    #[error("model server: {0}")]
    Upstream(String),

    /// An HTTP request to a model server or a download failed.
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),

    /// A file operation in the model cache failed.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    /// A download did not match its declared checksum.
    #[error("checksum mismatch for {url}: expected {expected}, got {actual}")]
    Checksum {
        /// Where it came from.
        url: String,
        /// The declared sha256.
        expected: String,
        /// What was downloaded.
        actual: String,
    },

    /// The request is malformed.
    #[error("{0}")]
    BadRequest(String),

    /// The thing asked for does not exist.
    #[error("{0} not found")]
    NotFound(String),

    /// The request conflicts with the current state, e.g. resuming a job that
    /// is not paused.
    #[error("{0}")]
    Conflict(String),

    /// A job or request was cancelled.
    #[error("cancelled")]
    Cancelled,

    /// Anything else.
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// Result alias.
pub type AiResult<T> = Result<T, AiError>;

impl From<serde_json::Error> for AiError {
    fn from(error: serde_json::Error) -> Self {
        AiError::BadRequest(format!("invalid JSON: {error}"))
    }
}
