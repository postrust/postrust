//! [`JobStore`] on PostgreSQL.
//!
//! Claims use `FOR UPDATE SKIP LOCKED`, so any number of workers on any number
//! of nodes can claim concurrently without handing one job to two of them, and
//! without waiting on each other's locks.
//!
//! Queries use runtime sqlx rather than the compile-time macros, like the
//! rest of the workspace, so the crate builds without a live database.

use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;
use sqlx::postgres::PgRow;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use super::{
    ClaimBands, ClaimRequest, Heartbeat, Job, JobFilter, JobStatus, JobStore, ModelDemand, NewJob,
    PauseReason, Priority, QueueSpec, QueueStats, Yield,
};
use crate::error::{AiError, AiResult};

/// The bundled migration.
pub const MIGRATION: &str = include_str!("../../migrations/20261005000001_ai_core.sql");

/// The version the bundled migration brings the schema to. A later
/// migration gets its own file, constant and check.
const SCHEMA_VERSION: i32 = 1;

/// Advisory lock id serialising the migration across nodes starting at once.
const MIGRATION_LOCK: i64 = 0x706f_7374_6169_0001;

/// Columns every job query returns, in one place so they cannot drift.
const JOB_COLUMNS: &str = "id, queue, model_id, priority_class, priority, status, pause_reason, \
    input, params, output, error, checkpoint, attempts, max_attempts, preemptions, \
    idempotency_key, cache_hit, workflow_run_id, workflow_step, worker_node, lease_expires_at, \
    run_at, created_at, started_at, finished_at";

/// Claimable, as SQL. Matches the `jobs_claimable` partial index.
const CLAIMABLE: &str =
    "(j.status = 'queued' OR (j.status = 'paused' AND j.pause_reason = 'preempted'))";

/// The Postgres job store.
#[derive(Clone)]
pub struct PgJobStore {
    pool: PgPool,
}

impl PgJobStore {
    /// Wrap a pool.
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// The pool, for the modules that share the schema.
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Apply the bundled migration if it has not been. Safe to run on every
    /// start and from several nodes at once.
    ///
    /// The applied version is recorded and checked first, so a node starting
    /// while others work does not re-run the DDL. That matters for more than
    /// speed: `ALTER TABLE` and trigger replacement take ACCESS EXCLUSIVE on
    /// `ai.jobs`, and taking that against live claims deadlocks.
    pub async fn migrate(&self) -> AiResult<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(MIGRATION_LOCK)
            .execute(&mut *tx)
            .await?;
        sqlx::raw_sql(
            "CREATE SCHEMA IF NOT EXISTS ai;
             CREATE TABLE IF NOT EXISTS ai.schema_version (
                version    integer PRIMARY KEY,
                applied_at timestamptz NOT NULL DEFAULT now())",
        )
        .execute(&mut *tx)
        .await?;
        let applied: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM ai.schema_version WHERE version = $1)",
        )
        .bind(SCHEMA_VERSION)
        .fetch_one(&mut *tx)
        .await?;
        if !applied {
            sqlx::raw_sql(MIGRATION).execute(&mut *tx).await?;
            sqlx::query("INSERT INTO ai.schema_version (version) VALUES ($1)")
                .bind(SCHEMA_VERSION)
                .execute(&mut *tx)
                .await?;
            tracing::info!(version = SCHEMA_VERSION, "applied postrust-ai schema");
        }
        tx.commit().await?;
        Ok(())
    }

    async fn job_by_id(&self, id: Uuid) -> AiResult<Option<Job>> {
        let sql = format!("SELECT {JOB_COLUMNS} FROM ai.jobs WHERE id = $1");
        let row = sqlx::query(&sql)
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        row.map(|r| decode_job(&r)).transpose()
    }

    async fn require(&self, id: Uuid) -> AiResult<Job> {
        self.job_by_id(id)
            .await?
            .ok_or_else(|| AiError::NotFound(format!("job {id}")))
    }

    /// Tell every node to look at this job now rather than at its next
    /// heartbeat. Best effort: the heartbeat is the guarantee.
    async fn notify_control(&self, id: Uuid) {
        if let Err(error) = sqlx::query("SELECT pg_notify('postrust_ai_control', $1)")
            .bind(id.to_string())
            .execute(&self.pool)
            .await
        {
            tracing::warn!(%error, %id, "could not send control notification");
        }
    }
}

fn decode_job(row: &PgRow) -> AiResult<Job> {
    let priority_class: String = row.try_get("priority_class")?;
    let status: String = row.try_get("status")?;
    let pause_reason: Option<String> = row.try_get("pause_reason")?;
    Ok(Job {
        id: row.try_get("id")?,
        queue: row.try_get("queue")?,
        model_id: row.try_get("model_id")?,
        priority_class: priority_class.parse()?,
        priority: row.try_get("priority")?,
        status: status.parse()?,
        pause_reason: match pause_reason.as_deref() {
            None => None,
            Some("preempted") => Some(PauseReason::Preempted),
            Some("manual") => Some(PauseReason::Manual),
            Some(other) => {
                return Err(AiError::Conflict(format!("unknown pause_reason '{other}'")))
            }
        },
        input: row.try_get("input")?,
        params: row.try_get("params")?,
        output: row.try_get("output")?,
        error: row.try_get("error")?,
        checkpoint: row.try_get("checkpoint")?,
        attempts: row.try_get("attempts")?,
        max_attempts: row.try_get("max_attempts")?,
        preemptions: row.try_get("preemptions")?,
        idempotency_key: row.try_get("idempotency_key")?,
        cache_hit: row.try_get("cache_hit")?,
        workflow_run_id: row.try_get("workflow_run_id")?,
        workflow_step: row.try_get("workflow_step")?,
        worker_node: row.try_get("worker_node")?,
        lease_expires_at: row.try_get("lease_expires_at")?,
        run_at: row.try_get("run_at")?,
        created_at: row.try_get("created_at")?,
        started_at: row.try_get("started_at")?,
        finished_at: row.try_get("finished_at")?,
    })
}

fn secs(d: Duration) -> f64 {
    d.as_secs_f64()
}

#[async_trait]
impl JobStore for PgJobStore {
    async fn upsert_queue(&self, queue: &QueueSpec) -> AiResult<()> {
        // `paused` is deliberately left alone on conflict: it is runtime
        // state set through the API, and a restart must not undo it.
        sqlx::query(
            "INSERT INTO ai.queues (name, concurrency, pause_mode, pause_grace_secs, max_attempts,
                                    retry_backoff_secs, result_cache)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             ON CONFLICT (name) DO UPDATE SET
                concurrency = EXCLUDED.concurrency,
                pause_mode = EXCLUDED.pause_mode,
                pause_grace_secs = EXCLUDED.pause_grace_secs,
                max_attempts = EXCLUDED.max_attempts,
                retry_backoff_secs = EXCLUDED.retry_backoff_secs,
                result_cache = EXCLUDED.result_cache,
                updated_at = now()",
        )
        .bind(&queue.name)
        .bind(queue.concurrency as i32)
        .bind(queue.pause_mode.as_str())
        .bind(queue.pause_grace_secs as i32)
        .bind(queue.max_attempts as i32)
        .bind(queue.retry_backoff_secs as i32)
        .bind(queue.result_cache)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn set_queue_paused(&self, queue: &str, paused: bool) -> AiResult<()> {
        let done =
            sqlx::query("UPDATE ai.queues SET paused = $2, updated_at = now() WHERE name = $1")
                .bind(queue)
                .bind(paused)
                .execute(&self.pool)
                .await?;
        if done.rows_affected() == 0 {
            return Err(AiError::UnknownQueue(queue.into()));
        }
        if !paused {
            // Resuming a queue makes its jobs claimable without any row
            // changing status, so the trigger does not fire. Wake workers.
            sqlx::query("SELECT pg_notify('postrust_ai_jobs', $1)")
                .bind(queue)
                .execute(&self.pool)
                .await?;
        }
        Ok(())
    }

    async fn queue_stats(&self) -> AiResult<Vec<QueueStats>> {
        let queues = sqlx::query("SELECT name, paused FROM ai.queues ORDER BY name")
            .fetch_all(&self.pool)
            .await?;
        let counts = sqlx::query(
            "SELECT queue, status, priority_class, count(*) AS n
             FROM ai.jobs
             WHERE status IN ('queued', 'running', 'paused', 'dead')
             GROUP BY queue, status, priority_class",
        )
        .fetch_all(&self.pool)
        .await?;

        let mut out = Vec::with_capacity(queues.len());
        for q in queues {
            let name: String = q.try_get("name")?;
            let mut stats = QueueStats {
                queue: name.clone(),
                paused: q.try_get("paused")?,
                waiting: Priority::ALL
                    .iter()
                    .map(|p| (p.as_str().to_string(), 0))
                    .collect(),
                running: 0,
                paused_jobs: 0,
                dead: 0,
            };
            for c in counts
                .iter()
                .filter(|c| c.get::<String, _>("queue") == name)
            {
                let n: i64 = c.try_get("n")?;
                let class: String = c.try_get("priority_class")?;
                match c.try_get::<String, _>("status")?.as_str() {
                    "queued" => *stats.waiting.entry(class).or_default() += n,
                    "running" => stats.running += n,
                    "paused" => stats.paused_jobs += n,
                    "dead" => stats.dead += n,
                    _ => {}
                }
            }
            out.push(stats);
        }
        Ok(out)
    }

    async fn enqueue(&self, job: NewJob) -> AiResult<Job> {
        if !job.input.is_object() {
            return Err(AiError::BadRequest(
                "job input must be a JSON object".into(),
            ));
        }
        let id = Uuid::new_v4();
        let params = if job.params.is_null() {
            Value::Object(Default::default())
        } else {
            job.params
        };
        // `ON CONFLICT DO NOTHING` then a re-read, rather than `DO UPDATE ...
        // RETURNING`, because a no-op update would still bump the row and
        // fire the trigger for a job that may already be finished.
        let sql = format!(
            "INSERT INTO ai.jobs (id, queue, model_id, priority_class, priority, input, params,
                                  idempotency_key, run_at, max_attempts, workflow_run_id, workflow_step)
             SELECT $1, q.name, $3, $4, $5, $6, $7, $8, coalesce($9, now()),
                    coalesce($10, q.max_attempts), $11, $12
             FROM ai.queues q WHERE q.name = $2
             ON CONFLICT (idempotency_key) DO NOTHING
             RETURNING {JOB_COLUMNS}"
        );
        let row = sqlx::query(&sql)
            .bind(id)
            .bind(&job.queue)
            .bind(&job.model_id)
            .bind(job.priority.as_str())
            .bind(job.priority.numeric(job.priority_offset))
            .bind(&job.input)
            .bind(&params)
            .bind(&job.idempotency_key)
            .bind(job.run_at)
            .bind(job.max_attempts.map(|n| n as i32))
            .bind(job.workflow_run_id)
            .bind(&job.workflow_step)
            .fetch_optional(&self.pool)
            .await?;

        if let Some(row) = row {
            return decode_job(&row);
        }
        // Nothing inserted: either the key was taken or the queue is unknown.
        if let Some(key) = &job.idempotency_key {
            let sql = format!("SELECT {JOB_COLUMNS} FROM ai.jobs WHERE idempotency_key = $1");
            if let Some(row) = sqlx::query(&sql)
                .bind(key)
                .fetch_optional(&self.pool)
                .await?
            {
                return decode_job(&row);
            }
        }
        Err(AiError::UnknownQueue(job.queue))
    }

    async fn claim(&self, request: &ClaimRequest) -> AiResult<Option<Job>> {
        if request.queues.is_empty() {
            return Ok(None);
        }
        let band = match request.bands {
            ClaimBands::Regular => "j.priority_class <> 'superlow'",
            ClaimBands::SuperlowOnly => "j.priority_class = 'superlow'",
        };
        // Effective priority: superlow never ages, so it can never overtake
        // anything; everything else gains a point per `aging_secs` waited,
        // which is what stops a steady stream of normal work starving low.
        let sql = format!(
            "WITH candidate AS (
                SELECT j.id FROM ai.jobs j
                JOIN ai.queues q ON q.name = j.queue AND NOT q.paused
                WHERE j.queue = ANY($1)
                  AND {CLAIMABLE}
                  AND j.run_at <= now()
                  AND {band}
                  AND ($2::text[] IS NULL OR j.model_id = ANY($2))
                ORDER BY
                  CASE WHEN j.priority_class = 'superlow' THEN j.priority
                       ELSE j.priority + LEAST($3::int,
                            floor(extract(epoch FROM now() - j.run_at) / $4::float8)::int)
                  END DESC,
                  j.run_at, j.created_at
                LIMIT 1
                FOR UPDATE OF j SKIP LOCKED
             )
             UPDATE ai.jobs j SET
                status = 'running',
                pause_reason = NULL,
                pause_requested = false,
                attempts = j.attempts + 1,
                worker_node = $5,
                lease_expires_at = now() + make_interval(secs => $6),
                started_at = coalesce(j.started_at, now()),
                updated_at = now()
             FROM candidate WHERE j.id = candidate.id
             RETURNING {}",
            JOB_COLUMNS
                .split(", ")
                .map(|c| format!("j.{c}"))
                .collect::<Vec<_>>()
                .join(", ")
        );
        let row = sqlx::query(&sql)
            .bind(&request.queues)
            .bind(&request.models)
            .bind(request.aging_cap)
            .bind(request.aging_secs.max(1) as f64)
            .bind(&request.node)
            .bind(secs(request.lease))
            .fetch_optional(&self.pool)
            .await?;
        row.map(|r| decode_job(&r)).transpose()
    }

    async fn heartbeat(&self, id: Uuid, node: &str, lease: Duration) -> AiResult<Heartbeat> {
        let row = sqlx::query(
            "UPDATE ai.jobs SET lease_expires_at = now() + make_interval(secs => $3),
                                updated_at = now()
             WHERE id = $1 AND worker_node = $2 AND status = 'running'
             RETURNING pause_requested, cancel_requested",
        )
        .bind(id)
        .bind(node)
        .bind(secs(lease))
        .fetch_optional(&self.pool)
        .await?;
        Ok(match row {
            None => Heartbeat::Lost,
            Some(r) if r.try_get::<bool, _>("cancel_requested")? => Heartbeat::CancelRequested,
            Some(r) if r.try_get::<bool, _>("pause_requested")? => Heartbeat::PauseRequested,
            Some(_) => Heartbeat::Continue,
        })
    }

    async fn save_checkpoint(&self, id: Uuid, node: &str, checkpoint: &Value) -> AiResult<bool> {
        let done = sqlx::query(
            "UPDATE ai.jobs SET checkpoint = $3, updated_at = now()
             WHERE id = $1 AND worker_node = $2 AND status = 'running'",
        )
        .bind(id)
        .bind(node)
        .bind(checkpoint)
        .execute(&self.pool)
        .await?;
        Ok(done.rows_affected() == 1)
    }

    async fn complete(
        &self,
        id: Uuid,
        node: &str,
        output: &Value,
        cache_hit: bool,
    ) -> AiResult<bool> {
        let done = sqlx::query(
            "UPDATE ai.jobs SET status = 'completed', output = $3, cache_hit = $4,
                                checkpoint = NULL, error = NULL, lease_expires_at = NULL,
                                finished_at = now(), updated_at = now()
             WHERE id = $1 AND worker_node = $2 AND status = 'running'",
        )
        .bind(id)
        .bind(node)
        .bind(output)
        .bind(cache_hit)
        .execute(&self.pool)
        .await?;
        Ok(done.rows_affected() == 1)
    }

    async fn fail(
        &self,
        id: Uuid,
        node: &str,
        error: &str,
        retryable: bool,
    ) -> AiResult<Option<JobStatus>> {
        // Backoff doubles per attempt from the queue's base, capped at an
        // hour. The checkpoint is kept: a retry resumes rather than restarts.
        let row = sqlx::query(
            "UPDATE ai.jobs j SET
                status = CASE
                    WHEN NOT $4 THEN 'failed'
                    WHEN j.attempts >= j.max_attempts THEN 'dead'
                    ELSE 'queued' END,
                run_at = CASE
                    WHEN $4 AND j.attempts < j.max_attempts
                    THEN now() + make_interval(secs => LEAST(3600,
                         q.retry_backoff_secs * power(2, GREATEST(j.attempts - 1, 0))))
                    ELSE j.run_at END,
                finished_at = CASE
                    WHEN NOT $4 OR j.attempts >= j.max_attempts THEN now() END,
                error = $3,
                worker_node = CASE
                    WHEN $4 AND j.attempts < j.max_attempts THEN NULL ELSE j.worker_node END,
                lease_expires_at = NULL,
                updated_at = now()
             FROM ai.queues q
             WHERE q.name = j.queue AND j.id = $1 AND j.worker_node = $2 AND j.status = 'running'
             RETURNING j.status",
        )
        .bind(id)
        .bind(node)
        .bind(error)
        .bind(retryable)
        .fetch_optional(&self.pool)
        .await?;
        row.map(|r| r.try_get::<String, _>("status")?.parse())
            .transpose()
    }

    async fn yield_job(&self, id: Uuid, node: &str, how: Yield) -> AiResult<bool> {
        // Setting a job aside is not a failed attempt, so the claim's
        // increment is undone.
        let done = sqlx::query(
            "UPDATE ai.jobs SET
                status = 'paused',
                pause_reason = $3,
                pause_requested = false,
                checkpoint = CASE WHEN $5 THEN NULL ELSE coalesce($4, checkpoint) END,
                attempts = GREATEST(attempts - 1, 0),
                preemptions = preemptions + CASE WHEN $3 = 'preempted' THEN 1 ELSE 0 END,
                worker_node = NULL,
                lease_expires_at = NULL,
                updated_at = now()
             WHERE id = $1 AND worker_node = $2 AND status = 'running'",
        )
        .bind(id)
        .bind(node)
        .bind(how.reason.as_str())
        .bind(how.checkpoint)
        .bind(how.discard_progress)
        .execute(&self.pool)
        .await?;
        Ok(done.rows_affected() == 1)
    }

    async fn finish_cancelled(&self, id: Uuid, node: &str) -> AiResult<bool> {
        let done = sqlx::query(
            "UPDATE ai.jobs SET status = 'cancelled', lease_expires_at = NULL,
                                finished_at = now(), updated_at = now()
             WHERE id = $1 AND worker_node = $2 AND status = 'running'",
        )
        .bind(id)
        .bind(node)
        .execute(&self.pool)
        .await?;
        Ok(done.rows_affected() == 1)
    }

    async fn request_pause(&self, id: Uuid) -> AiResult<Job> {
        // Waiting jobs pause on the spot; a preempted job becomes a manual
        // pause, so it stops resuming by itself.
        let row = sqlx::query(
            "UPDATE ai.jobs SET
                status = CASE WHEN status = 'running' THEN status ELSE 'paused' END,
                pause_reason = CASE WHEN status = 'running' THEN pause_reason ELSE 'manual' END,
                pause_requested = (status = 'running'),
                updated_at = now()
             WHERE id = $1 AND status IN ('queued', 'running', 'paused')
             RETURNING status",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else {
            let job = self.require(id).await?;
            return Err(AiError::Conflict(format!(
                "job {id} is {} and cannot be paused",
                job.status.as_str()
            )));
        };
        if row.try_get::<String, _>("status")? == "running" {
            self.notify_control(id).await;
        }
        self.require(id).await
    }

    async fn resume(&self, id: Uuid) -> AiResult<Job> {
        let done = sqlx::query(
            "UPDATE ai.jobs SET status = 'queued', pause_reason = NULL, run_at = now(),
                                updated_at = now()
             WHERE id = $1 AND status = 'paused'",
        )
        .bind(id)
        .execute(&self.pool)
        .await?;
        if done.rows_affected() == 0 {
            // A running job with a pause still pending can be un-asked.
            let undone = sqlx::query(
                "UPDATE ai.jobs SET pause_requested = false, updated_at = now()
                 WHERE id = $1 AND status = 'running' AND pause_requested",
            )
            .bind(id)
            .execute(&self.pool)
            .await?;
            if undone.rows_affected() == 0 {
                let job = self.require(id).await?;
                return Err(AiError::Conflict(format!(
                    "job {id} is {} and not paused",
                    job.status.as_str()
                )));
            }
        }
        self.require(id).await
    }

    async fn request_cancel(&self, id: Uuid) -> AiResult<Job> {
        let row = sqlx::query(
            "UPDATE ai.jobs SET
                status = CASE WHEN status = 'running' THEN status ELSE 'cancelled' END,
                finished_at = CASE WHEN status = 'running' THEN finished_at ELSE now() END,
                cancel_requested = true,
                updated_at = now()
             WHERE id = $1 AND status IN ('queued', 'running', 'paused')
             RETURNING status",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else {
            let job = self.require(id).await?;
            return Err(AiError::Conflict(format!(
                "job {id} is {} and cannot be cancelled",
                job.status.as_str()
            )));
        };
        if row.try_get::<String, _>("status")? == "running" {
            self.notify_control(id).await;
        }
        self.require(id).await
    }

    async fn get(&self, id: Uuid) -> AiResult<Option<Job>> {
        if let Some(job) = self.job_by_id(id).await? {
            return Ok(Some(job));
        }
        let sql = format!("SELECT {JOB_COLUMNS} FROM ai.jobs_archive WHERE id = $1");
        let row = sqlx::query(&sql)
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        row.map(|r| decode_job(&r)).transpose()
    }

    async fn list(&self, filter: &JobFilter) -> AiResult<Vec<Job>> {
        let sql = format!(
            "SELECT {JOB_COLUMNS} FROM ai.jobs
             WHERE ($1::text IS NULL OR queue = $1)
               AND ($2::text IS NULL OR status = $2)
               AND ($3::text IS NULL OR model_id = $3)
               AND ($4::uuid IS NULL OR workflow_run_id = $4)
             ORDER BY created_at DESC
             LIMIT $5"
        );
        let rows = sqlx::query(&sql)
            .bind(&filter.queue)
            .bind(&filter.status)
            .bind(&filter.model_id)
            .bind(filter.workflow_run_id)
            .bind(filter.limit.unwrap_or(50).clamp(1, 500))
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(decode_job).collect()
    }

    async fn demand(&self) -> AiResult<Vec<ModelDemand>> {
        let sql = format!(
            "SELECT j.model_id, max(j.priority) AS top, count(*) AS waiting
             FROM ai.jobs j JOIN ai.queues q ON q.name = j.queue AND NOT q.paused
             WHERE {CLAIMABLE} AND j.run_at <= now() AND j.priority_class <> 'superlow'
             GROUP BY j.model_id"
        );
        let rows = sqlx::query(&sql).fetch_all(&self.pool).await?;
        rows.iter()
            .map(|r| {
                let top: i32 = r.try_get("top")?;
                Ok(ModelDemand {
                    model_id: r.try_get("model_id")?,
                    top_priority: class_of(top),
                    waiting: r.try_get("waiting")?,
                })
            })
            .collect()
    }

    async fn has_regular_waiting(&self, queue: &str) -> AiResult<bool> {
        let sql = format!(
            "SELECT EXISTS (
                SELECT 1 FROM ai.jobs j JOIN ai.queues q ON q.name = j.queue AND NOT q.paused
                WHERE j.queue = $1 AND {CLAIMABLE} AND j.run_at <= now()
                  AND j.priority_class <> 'superlow')"
        );
        Ok(sqlx::query_scalar(&sql)
            .bind(queue)
            .fetch_one(&self.pool)
            .await?)
    }

    async fn reap_expired(&self) -> AiResult<Vec<Uuid>> {
        // A lost lease counts as the attempt it was: a job that kills its
        // worker every time must not be retried forever.
        let rows = sqlx::query(
            "UPDATE ai.jobs SET
                status = CASE WHEN cancel_requested THEN 'cancelled'
                              WHEN attempts >= max_attempts THEN 'dead'
                              ELSE 'queued' END,
                finished_at = CASE WHEN cancel_requested OR attempts >= max_attempts
                                   THEN now() END,
                error = 'lease expired on node ' || coalesce(worker_node, '?'),
                worker_node = NULL,
                lease_expires_at = NULL,
                pause_requested = false,
                updated_at = now()
             WHERE status = 'running' AND lease_expires_at < now()
             RETURNING id",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(|r| Ok(r.try_get("id")?)).collect()
    }

    async fn archive(&self, age: Duration) -> AiResult<u64> {
        // Jobs of a running workflow stay: the engine reads step outputs from
        // the live table.
        let done = sqlx::query(
            "WITH moved AS (
                DELETE FROM ai.jobs j
                WHERE j.status IN ('completed', 'failed', 'cancelled', 'dead')
                  AND j.finished_at < now() - make_interval(secs => $1)
                  AND (j.workflow_run_id IS NULL OR NOT EXISTS (
                        SELECT 1 FROM ai.workflow_runs r
                        WHERE r.id = j.workflow_run_id AND r.status = 'running'))
                RETURNING j.*)
             INSERT INTO ai.jobs_archive SELECT * FROM moved",
        )
        .bind(secs(age))
        .execute(&self.pool)
        .await?;
        Ok(done.rows_affected())
    }
}

/// The class a numeric priority falls in.
fn class_of(priority: i32) -> Priority {
    Priority::ALL
        .iter()
        .rev()
        .copied()
        .find(|p| priority >= p.base() - 49)
        .unwrap_or(Priority::Superlow)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn class_of_inverts_numeric() {
        for p in Priority::ALL {
            for offset in [-49, 0, 49] {
                assert_eq!(class_of(p.numeric(offset)), p, "{p} {offset}");
            }
        }
    }
}
