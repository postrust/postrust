//! The priority job queue.
//!
//! [`JobStore`] is the seam: the scheduler, executor and API only speak to it,
//! and [`PgJobStore`] is the implementation. Postgres was chosen because AI
//! jobs are few and long -- seconds to hours each -- so its throughput ceiling
//! is never the constraint, while durability, transactional claims and being
//! queryable through Postrust itself all come free.

mod postgres;

pub use postgres::PgJobStore;

use std::fmt;
use std::str::FromStr;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::error::{AiError, AiResult};

// ============================================================================
// Priority
// ============================================================================

/// Priority class. Classes are bands: a job's numeric priority is its class
/// base plus an offset in `-49..=49`, so no offset moves a job into the next
/// band, but waiting can (see `SchedulerConfig::aging_secs`).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    /// Runs only on spare capacity, never ages, and is paused whenever
    /// anything else needs its slot or its resources.
    Superlow,
    /// Background work.
    Low,
    /// The default.
    #[default]
    Normal,
    /// Interactive work. Synchronous gateway requests count as this.
    High,
    /// May unload models that hold session locks.
    Critical,
}

impl Priority {
    /// All classes, lowest first.
    pub const ALL: [Priority; 5] = [
        Priority::Superlow,
        Priority::Low,
        Priority::Normal,
        Priority::High,
        Priority::Critical,
    ];

    /// The base of this class's numeric band.
    pub fn base(self) -> i32 {
        match self {
            Priority::Superlow => 0,
            Priority::Low => 100,
            Priority::Normal => 200,
            Priority::High => 300,
            Priority::Critical => 400,
        }
    }

    /// Numeric priority for this class and an offset within it.
    pub fn numeric(self, offset: i32) -> i32 {
        self.base() + offset.clamp(-49, 49)
    }

    /// The stored spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Priority::Superlow => "superlow",
            Priority::Low => "low",
            Priority::Normal => "normal",
            Priority::High => "high",
            Priority::Critical => "critical",
        }
    }

    /// Whether this is the pausable background class.
    pub fn is_superlow(self) -> bool {
        self == Priority::Superlow
    }
}

impl fmt::Display for Priority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Priority {
    type Err = AiError;
    fn from_str(s: &str) -> AiResult<Self> {
        Ok(match s {
            "superlow" => Priority::Superlow,
            "low" => Priority::Low,
            "normal" => Priority::Normal,
            "high" => Priority::High,
            "critical" => Priority::Critical,
            other => return Err(AiError::BadRequest(format!("unknown priority '{other}'"))),
        })
    }
}

// ============================================================================
// Queue settings
// ============================================================================

/// What pausing a running job does to its progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PauseMode {
    /// The job saves a checkpoint and resumes from it. If it has not stopped
    /// within the grace period it is cancelled and resumes from the last
    /// checkpoint it did save.
    #[default]
    Checkpoint,
    /// The job is cancelled at once and starts over when it next runs.
    Requeue,
}

impl PauseMode {
    /// The stored spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            PauseMode::Checkpoint => "checkpoint",
            PauseMode::Requeue => "requeue",
        }
    }
}

impl FromStr for PauseMode {
    type Err = AiError;
    fn from_str(s: &str) -> AiResult<Self> {
        match s {
            "checkpoint" => Ok(PauseMode::Checkpoint),
            "requeue" => Ok(PauseMode::Requeue),
            other => Err(AiError::BadRequest(format!("unknown pause_mode '{other}'"))),
        }
    }
}

/// A queue's settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct QueueSpec {
    /// Name jobs are submitted to.
    pub name: String,
    /// Worker slots on each node.
    #[serde(default = "default_concurrency")]
    pub concurrency: u32,
    /// What pausing does to a running job.
    #[serde(default)]
    pub pause_mode: PauseMode,
    /// Seconds a checkpointing job gets to stop by itself.
    #[serde(default = "default_pause_grace")]
    pub pause_grace_secs: u32,
    /// Attempts before a job is dead-lettered.
    #[serde(default = "default_max_attempts")]
    pub max_attempts: u32,
    /// Base retry delay; doubles each attempt, capped at an hour.
    #[serde(default = "default_backoff")]
    pub retry_backoff_secs: u32,
    /// Cache job results for every model on this queue.
    #[serde(default)]
    pub result_cache: bool,
}

fn default_concurrency() -> u32 {
    4
}
fn default_pause_grace() -> u32 {
    30
}
fn default_max_attempts() -> u32 {
    3
}
fn default_backoff() -> u32 {
    10
}

impl QueueSpec {
    /// A queue with default settings.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            concurrency: default_concurrency(),
            pause_mode: PauseMode::default(),
            pause_grace_secs: default_pause_grace(),
            max_attempts: default_max_attempts(),
            retry_backoff_secs: default_backoff(),
            result_cache: false,
        }
    }

    /// Reject settings the schema would refuse.
    pub fn validate(&self) -> AiResult<()> {
        if self.name.is_empty() {
            return Err(AiError::Config("queue name is empty".into()));
        }
        if self.concurrency == 0 || self.max_attempts == 0 {
            return Err(AiError::Config(format!(
                "queue '{}': concurrency and max_attempts must be at least 1",
                self.name
            )));
        }
        Ok(())
    }

    /// Grace period as a duration.
    pub fn pause_grace(&self) -> Duration {
        Duration::from_secs(self.pause_grace_secs as u64)
    }
}

// ============================================================================
// Jobs
// ============================================================================

/// Where a job is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    /// Waiting to be claimed.
    Queued,
    /// Claimed by a worker.
    Running,
    /// Stopped part-way; see `pause_reason`.
    Paused,
    /// Finished with output.
    Completed,
    /// Failed in a way retrying would not fix.
    Failed,
    /// Cancelled on request.
    Cancelled,
    /// Ran out of attempts.
    Dead,
}

impl JobStatus {
    /// The stored spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            JobStatus::Queued => "queued",
            JobStatus::Running => "running",
            JobStatus::Paused => "paused",
            JobStatus::Completed => "completed",
            JobStatus::Failed => "failed",
            JobStatus::Cancelled => "cancelled",
            JobStatus::Dead => "dead",
        }
    }

    /// Whether the job will never run again.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            JobStatus::Completed | JobStatus::Failed | JobStatus::Cancelled | JobStatus::Dead
        )
    }
}

impl FromStr for JobStatus {
    type Err = AiError;
    fn from_str(s: &str) -> AiResult<Self> {
        Ok(match s {
            "queued" => JobStatus::Queued,
            "running" => JobStatus::Running,
            "paused" => JobStatus::Paused,
            "completed" => JobStatus::Completed,
            "failed" => JobStatus::Failed,
            "cancelled" => JobStatus::Cancelled,
            "dead" => JobStatus::Dead,
            other => return Err(AiError::BadRequest(format!("unknown status '{other}'"))),
        })
    }
}

/// Why a job is paused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PauseReason {
    /// Made way for higher-priority work; resumes by itself.
    Preempted,
    /// Paused through the API; waits for a resume.
    Manual,
}

impl PauseReason {
    /// The stored spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            PauseReason::Preempted => "preempted",
            PauseReason::Manual => "manual",
        }
    }
}

/// A job to submit.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct NewJob {
    /// Queue to submit to.
    pub queue: String,
    /// Model the job runs on.
    pub model_id: String,
    /// Priority class.
    #[serde(default)]
    pub priority: Priority,
    /// Offset within the class, `-49..=49`.
    #[serde(default)]
    pub priority_offset: i32,
    /// What the job does. See `executor` for the shape the built-in handler
    /// takes.
    pub input: Value,
    /// Free-form parameters, part of the result-cache key.
    #[serde(default)]
    pub params: Value,
    /// Submitting the same key twice returns the first job.
    #[serde(default)]
    pub idempotency_key: Option<String>,
    /// Do not run before this time.
    #[serde(default)]
    pub run_at: Option<DateTime<Utc>>,
    /// Override the queue's attempt limit.
    #[serde(default)]
    pub max_attempts: Option<u32>,
    /// Set by the workflow engine.
    #[serde(skip_deserializing, default)]
    pub workflow_run_id: Option<Uuid>,
    /// Set by the workflow engine.
    #[serde(skip_deserializing, default)]
    pub workflow_step: Option<String>,
}

impl NewJob {
    /// A job with default priority and no options.
    pub fn new(queue: impl Into<String>, model_id: impl Into<String>, input: Value) -> Self {
        Self {
            queue: queue.into(),
            model_id: model_id.into(),
            priority: Priority::Normal,
            priority_offset: 0,
            input,
            params: Value::Object(Default::default()),
            idempotency_key: None,
            run_at: None,
            max_attempts: None,
            workflow_run_id: None,
            workflow_step: None,
        }
    }

    /// Set the priority class.
    pub fn with_priority(mut self, priority: Priority) -> Self {
        self.priority = priority;
        self
    }

    /// Set an idempotency key.
    pub fn with_idempotency_key(mut self, key: impl Into<String>) -> Self {
        self.idempotency_key = Some(key.into());
        self
    }
}

/// A job as stored.
#[derive(Debug, Clone, Serialize)]
#[non_exhaustive]
pub struct Job {
    /// Id.
    pub id: Uuid,
    /// Queue.
    pub queue: String,
    /// Model.
    pub model_id: String,
    /// Priority class.
    pub priority_class: Priority,
    /// Numeric priority, before aging.
    pub priority: i32,
    /// Status.
    pub status: JobStatus,
    /// Why it is paused, when it is.
    pub pause_reason: Option<PauseReason>,
    /// Input.
    pub input: Value,
    /// Parameters.
    pub params: Value,
    /// Output, once completed.
    pub output: Option<Value>,
    /// Last error.
    pub error: Option<String>,
    /// Saved progress.
    pub checkpoint: Option<Value>,
    /// Attempts so far, including the current one.
    pub attempts: i32,
    /// Attempt limit.
    pub max_attempts: i32,
    /// Times it was paused to make way.
    pub preemptions: i32,
    /// Idempotency key.
    pub idempotency_key: Option<String>,
    /// Whether the output came from the result cache.
    pub cache_hit: bool,
    /// Owning workflow run.
    pub workflow_run_id: Option<Uuid>,
    /// Step within the run.
    pub workflow_step: Option<String>,
    /// Node running it.
    pub worker_node: Option<String>,
    /// When the current lease ends.
    pub lease_expires_at: Option<DateTime<Utc>>,
    /// Earliest start.
    pub run_at: DateTime<Utc>,
    /// Submitted.
    pub created_at: DateTime<Utc>,
    /// First started.
    pub started_at: Option<DateTime<Utc>>,
    /// Finished.
    pub finished_at: Option<DateTime<Utc>>,
}

/// What to claim.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ClaimRequest {
    /// Claiming node.
    pub node: String,
    /// Queues with a free slot on this node.
    pub queues: Vec<String>,
    /// Which bands to consider.
    pub bands: ClaimBands,
    /// Only jobs for these models. `None`: any.
    pub models: Option<Vec<String>>,
    /// Lease length.
    pub lease: Duration,
    /// Aging: one point per this many seconds waiting.
    pub aging_secs: u64,
    /// Aging cap.
    pub aging_cap: i32,
}

impl ClaimRequest {
    /// A claim on `queues` with default aging and no model filter.
    pub fn new(
        node: impl Into<String>,
        queues: Vec<String>,
        bands: ClaimBands,
        lease: Duration,
    ) -> Self {
        Self {
            node: node.into(),
            queues,
            bands,
            models: None,
            lease,
            aging_secs: 60,
            aging_cap: 99,
        }
    }
}

/// Which priority bands a claim considers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimBands {
    /// Everything except superlow.
    Regular,
    /// Superlow only.
    SuperlowOnly,
}

/// What a heartbeat learned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Heartbeat {
    /// Carry on.
    Continue,
    /// Someone asked for a pause.
    PauseRequested,
    /// Someone asked for a cancel.
    CancelRequested,
    /// This node no longer holds the job: the lease expired and it was
    /// reaped, or it was reassigned. Stop without writing anything.
    Lost,
}

/// How a running job is being set aside. It always ends up `paused`: a
/// preempted job resumes by itself when there is capacity, a manual one waits
/// for a resume.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Yield {
    /// Why.
    pub reason: PauseReason,
    /// Progress to resume from. `None` keeps whatever was last saved.
    pub checkpoint: Option<Value>,
    /// Throw progress away: the queue's `requeue` pause mode.
    pub discard_progress: bool,
}

impl Yield {
    /// Pause keeping progress.
    pub fn keep(reason: PauseReason, checkpoint: Option<Value>) -> Self {
        Self {
            reason,
            checkpoint,
            discard_progress: false,
        }
    }

    /// Pause discarding progress.
    pub fn discard(reason: PauseReason) -> Self {
        Self {
            reason,
            checkpoint: None,
            discard_progress: true,
        }
    }
}

/// Waiting work for one model.
#[derive(Debug, Clone, Serialize)]
#[non_exhaustive]
pub struct ModelDemand {
    /// Model.
    pub model_id: String,
    /// Highest class waiting.
    pub top_priority: Priority,
    /// Jobs waiting.
    pub waiting: i64,
}

/// Depth of one queue.
#[derive(Debug, Clone, Serialize)]
#[non_exhaustive]
pub struct QueueStats {
    /// Queue.
    pub queue: String,
    /// Whether the queue is paused.
    pub paused: bool,
    /// Waiting jobs by priority class.
    pub waiting: std::collections::BTreeMap<String, i64>,
    /// Running jobs.
    pub running: i64,
    /// Paused jobs.
    pub paused_jobs: i64,
    /// Dead jobs still in the live table.
    pub dead: i64,
}

/// Job list filter.
#[derive(Debug, Clone, Default, Deserialize)]
#[non_exhaustive]
pub struct JobFilter {
    /// Queue.
    pub queue: Option<String>,
    /// Status.
    pub status: Option<String>,
    /// Model.
    pub model_id: Option<String>,
    /// Workflow run.
    pub workflow_run_id: Option<Uuid>,
    /// Page size, default 50, at most 500.
    pub limit: Option<i64>,
}

/// The queue's storage.
///
/// Every write that a worker makes is conditional on `worker_node` still being
/// the caller and the job still running, so a worker whose lease was reaped
/// cannot overwrite the work of the node that took the job over.
#[async_trait]
pub trait JobStore: Send + Sync + 'static {
    /// Create or update a queue's settings.
    async fn upsert_queue(&self, queue: &QueueSpec) -> AiResult<()>;
    /// Stop or start handing out a queue's jobs.
    async fn set_queue_paused(&self, queue: &str, paused: bool) -> AiResult<()>;
    /// Per-queue counts.
    async fn queue_stats(&self) -> AiResult<Vec<QueueStats>>;

    /// Submit a job. With an idempotency key already used, returns the
    /// existing job instead.
    async fn enqueue(&self, job: NewJob) -> AiResult<Job>;
    /// Claim the highest-priority claimable job.
    async fn claim(&self, request: &ClaimRequest) -> AiResult<Option<Job>>;
    /// Extend a lease, and learn whether a pause or cancel was asked for.
    async fn heartbeat(&self, id: Uuid, node: &str, lease: Duration) -> AiResult<Heartbeat>;
    /// Save progress mid-run.
    async fn save_checkpoint(&self, id: Uuid, node: &str, checkpoint: &Value) -> AiResult<bool>;
    /// Finish successfully.
    async fn complete(
        &self,
        id: Uuid,
        node: &str,
        output: &Value,
        cache_hit: bool,
    ) -> AiResult<bool>;
    /// Record a failure. Retryable failures go back to the queue until the
    /// attempt limit; the returned status says which happened.
    async fn fail(
        &self,
        id: Uuid,
        node: &str,
        error: &str,
        retryable: bool,
    ) -> AiResult<Option<JobStatus>>;
    /// Set a running job aside.
    async fn yield_job(&self, id: Uuid, node: &str, how: Yield) -> AiResult<bool>;
    /// Mark a running job cancelled after its worker stopped it.
    async fn finish_cancelled(&self, id: Uuid, node: &str) -> AiResult<bool>;

    /// Pause a job: at once if it is waiting, at its next heartbeat if it is
    /// running.
    async fn request_pause(&self, id: Uuid) -> AiResult<Job>;
    /// Resume a manually paused job.
    async fn resume(&self, id: Uuid) -> AiResult<Job>;
    /// Cancel a job: at once if it is waiting, at its next heartbeat if it is
    /// running.
    async fn request_cancel(&self, id: Uuid) -> AiResult<Job>;

    /// One job, live or archived.
    async fn get(&self, id: Uuid) -> AiResult<Option<Job>>;
    /// Jobs matching a filter, newest first.
    async fn list(&self, filter: &JobFilter) -> AiResult<Vec<Job>>;
    /// Claimable non-superlow work, by model.
    async fn demand(&self) -> AiResult<Vec<ModelDemand>>;
    /// Whether a queue has claimable non-superlow work.
    async fn has_regular_waiting(&self, queue: &str) -> AiResult<bool>;

    /// Return jobs with expired leases to the queue.
    async fn reap_expired(&self) -> AiResult<Vec<Uuid>>;
    /// Move finished jobs older than `age` to the archive.
    async fn archive(&self, age: Duration) -> AiResult<u64>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offsets_never_cross_a_band() {
        for pair in Priority::ALL.windows(2) {
            let (lower, upper) = (pair[0], pair[1]);
            assert!(
                lower.numeric(1000) < upper.numeric(-1000),
                "{lower} vs {upper}"
            );
        }
    }

    #[test]
    fn priorities_round_trip() {
        for p in Priority::ALL {
            assert_eq!(p.as_str().parse::<Priority>().unwrap(), p);
        }
    }
}
