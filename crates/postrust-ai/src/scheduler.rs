//! The worker loop: claim jobs into free slots, highest priority first.
//!
//! Each queue has `concurrency` slots on this node. A pass fills free slots
//! with regular work first. Only when no regular job is claimable does it
//! consider superlow, and then only for models with spare capacity right now
//! -- a free slot on a ready instance, or room to load one without unloading
//! anything. That is what "runs on spare capacity" means.
//!
//! When a queue's slots are full and some hold superlow jobs while regular
//! work waits in that queue, one superlow job is preempted per pass. The
//! manager does the same for resources, see [`crate::manager`].
//!
//! Workers wake on `LISTEN postrust_ai_jobs` (sent by a trigger whenever a job
//! becomes claimable), on a slot freeing up, and on a slow backup poll for
//! notifications lost to a dropped connection.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use sqlx::postgres::PgListener;
use sqlx::PgPool;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::config::SchedulerConfig;
use crate::executor::Executor;
use crate::manager::ModelManager;
use crate::queue::{ClaimBands, ClaimRequest, Job, JobStore, Priority, QueueSpec};
use crate::workflow::WorkflowEngine;

#[derive(Debug, Clone)]
struct Running {
    id: Uuid,
    priority: Priority,
    started: Instant,
    preempt_sent: bool,
}

/// The worker loop for one node.
pub struct Scheduler {
    node: String,
    store: Arc<dyn JobStore>,
    manager: ModelManager,
    executor: Arc<Executor>,
    workflows: Option<WorkflowEngine>,
    queues: Vec<QueueSpec>,
    config: SchedulerConfig,
    running: Mutex<HashMap<String, Vec<Running>>>,
    wake: Notify,
}

impl Scheduler {
    /// A scheduler.
    pub fn new(
        node: String,
        store: Arc<dyn JobStore>,
        manager: ModelManager,
        executor: Arc<Executor>,
        workflows: Option<WorkflowEngine>,
        queues: Vec<QueueSpec>,
        config: SchedulerConfig,
    ) -> Self {
        Self {
            node,
            store,
            manager,
            executor,
            workflows,
            queues,
            config,
            running: Mutex::new(HashMap::new()),
            wake: Notify::new(),
        }
    }

    /// Ask for a pass now.
    pub fn wake(&self) {
        self.wake.notify_one();
    }

    /// Jobs running on this node, by queue.
    pub fn running_counts(&self) -> HashMap<String, usize> {
        self.running
            .lock()
            .iter()
            .map(|(q, jobs)| (q.clone(), jobs.len()))
            .collect()
    }

    /// Run until `shutdown`, then wait for running jobs to finish (they keep
    /// their leases; anything not finished by the deadline is reaped by
    /// another node once the lease runs out).
    pub async fn run(self: Arc<Self>, shutdown: CancellationToken) {
        let poll = Duration::from_secs(self.config.poll_secs.max(1));
        loop {
            if shutdown.is_cancelled() {
                break;
            }
            self.fill_slots().await;
            self.preempt_for_slots().await;
            tokio::select! {
                _ = self.wake.notified() => {}
                _ = tokio::time::sleep(poll) => {}
                _ = shutdown.cancelled() => break,
            }
        }
        let deadline = Instant::now() + Duration::from_secs(self.config.lease_secs);
        while Instant::now() < deadline && self.running.lock().values().any(|v| !v.is_empty()) {
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    fn free_queues(&self) -> Vec<String> {
        let running = self.running.lock();
        self.queues
            .iter()
            .filter(|q| running.get(&q.name).map_or(0, Vec::len) < q.concurrency as usize)
            .map(|q| q.name.clone())
            .collect()
    }

    fn request(
        &self,
        queues: Vec<String>,
        bands: ClaimBands,
        models: Option<Vec<String>>,
    ) -> ClaimRequest {
        ClaimRequest {
            node: self.node.clone(),
            queues,
            bands,
            models,
            lease: Duration::from_secs(self.config.lease_secs),
            aging_secs: self.config.aging_secs,
            aging_cap: self.config.aging_cap,
        }
    }

    async fn fill_slots(self: &Arc<Self>) {
        // Only models this node can serve; a job for a model declared on
        // other nodes is theirs to take.
        let local_models: Vec<String> = self.manager.specs().iter().map(|s| s.id.clone()).collect();
        if local_models.is_empty() {
            return;
        }
        loop {
            let free = self.free_queues();
            if free.is_empty() {
                return;
            }
            let claimed = match self
                .store
                .claim(&self.request(
                    free.clone(),
                    ClaimBands::Regular,
                    Some(local_models.clone()),
                ))
                .await
            {
                Ok(Some(job)) => Some(job),
                Ok(None) => {
                    let spare = self.manager.spare_models();
                    if spare.is_empty() {
                        None
                    } else {
                        match self
                            .store
                            .claim(&self.request(free, ClaimBands::SuperlowOnly, Some(spare)))
                            .await
                        {
                            Ok(job) => job,
                            Err(error) => {
                                tracing::warn!(%error, "claim failed");
                                None
                            }
                        }
                    }
                }
                Err(error) => {
                    tracing::warn!(%error, "claim failed");
                    None
                }
            };
            let Some(job) = claimed else { return };
            self.spawn(job);
        }
    }

    fn spawn(self: &Arc<Self>, job: Job) {
        let Some(queue) = self.queues.iter().find(|q| q.name == job.queue).cloned() else {
            // Claimed from a queue this node does not run; cannot happen,
            // since claims name only our queues.
            return;
        };
        self.running
            .lock()
            .entry(queue.name.clone())
            .or_default()
            .push(Running {
                id: job.id,
                priority: job.priority_class,
                started: Instant::now(),
                preempt_sent: false,
            });
        tracing::debug!(job = %job.id, queue = %queue.name, model = %job.model_id, priority = %job.priority_class, "claimed job");

        let this = self.clone();
        tokio::spawn(async move {
            let id = job.id;
            let run_id = job.workflow_run_id;
            this.executor.run(job, &queue).await;
            if let Some(list) = this.running.lock().get_mut(&queue.name) {
                list.retain(|r| r.id != id);
            }
            if let (Some(engine), Some(run_id)) = (&this.workflows, run_id) {
                if let Err(error) = engine.advance(run_id).await {
                    tracing::warn!(run = %run_id, %error, "could not advance workflow");
                }
            }
            this.wake();
        });
    }

    /// A full queue holding superlow jobs while regular work waits gives up
    /// one superlow slot.
    async fn preempt_for_slots(&self) {
        let candidates: Vec<(String, Uuid)> = {
            let running = self.running.lock();
            self.queues
                .iter()
                .filter_map(|q| {
                    let jobs = running.get(&q.name)?;
                    if jobs.len() < q.concurrency as usize {
                        return None;
                    }
                    if jobs.iter().any(|r| r.preempt_sent) {
                        return None; // one at a time
                    }
                    // The most recently started has the least to lose.
                    jobs.iter()
                        .filter(|r| r.priority.is_superlow())
                        .max_by_key(|r| r.started)
                        .map(|r| (q.name.clone(), r.id))
                })
                .collect()
        };
        for (queue, id) in candidates {
            match self.store.has_regular_waiting(&queue).await {
                Ok(true) => {
                    if self.executor.preempt(id) {
                        tracing::info!(job = %id, %queue, "preempting superlow job for a queue slot");
                        if let Some(r) = self
                            .running
                            .lock()
                            .get_mut(&queue)
                            .and_then(|l| l.iter_mut().find(|r| r.id == id))
                        {
                            r.preempt_sent = true;
                        }
                    }
                }
                Ok(false) => {}
                Err(error) => tracing::warn!(%error, "could not check queue for waiting work"),
            }
        }
    }
}

/// Listen for job and control notifications until `shutdown`, reconnecting
/// on failure.
pub async fn listen(
    pool: PgPool,
    scheduler: Arc<Scheduler>,
    executor: Arc<Executor>,
    shutdown: CancellationToken,
) {
    let mut backoff = Duration::from_millis(500);
    loop {
        let mut listener = match PgListener::connect_with(&pool).await {
            Ok(l) => l,
            Err(error) => {
                tracing::warn!(%error, "LISTEN connection failed; retrying");
                tokio::select! {
                    _ = tokio::time::sleep(backoff) => {}
                    _ = shutdown.cancelled() => return,
                }
                backoff = (backoff * 2).min(Duration::from_secs(30));
                continue;
            }
        };
        if let Err(error) = listener
            .listen_all(["postrust_ai_jobs", "postrust_ai_control"])
            .await
        {
            tracing::warn!(%error, "LISTEN failed; retrying");
            continue;
        }
        backoff = Duration::from_millis(500);
        // Anything sent while disconnected is gone: do a pass now.
        scheduler.wake();
        loop {
            tokio::select! {
                message = listener.recv() => match message {
                    Ok(n) if n.channel() == "postrust_ai_control" => {
                        if let Ok(id) = n.payload().parse::<Uuid>() {
                            executor.poke(id);
                        }
                    }
                    Ok(_) => scheduler.wake(),
                    Err(error) => {
                        tracing::warn!(%error, "LISTEN connection lost; reconnecting");
                        break;
                    }
                },
                _ = shutdown.cancelled() => return,
            }
        }
    }
}
