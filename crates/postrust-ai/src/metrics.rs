//! Prometheus metrics, in the text exposition format.
//!
//! Counters are atomics updated in place; gauges (queue depth, instances,
//! budget) are read at scrape time, so they are never stale.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use parking_lot::Mutex;

use crate::manager::InstanceInfo;
use crate::queue::QueueStats;
use crate::resources::ResourcePool;

/// Counters.
#[derive(Default)]
pub struct Metrics {
    result_cache_hits: AtomicU64,
    result_cache_misses: AtomicU64,
    preemptions: AtomicU64,
    gateway_requests: AtomicU64,
    gateway_load_waits: AtomicU64,
    gateway_unavailable: AtomicU64,
    gateway_forwarded: AtomicU64,
    session_rebinds: AtomicU64,
    /// Per queue: (jobs finished, total seconds).
    job_durations: Mutex<BTreeMap<String, (u64, f64)>>,
}

impl Metrics {
    /// A job finished, in whatever way.
    pub fn observe_job(&self, queue: &str, took: Duration) {
        let mut map = self.job_durations.lock();
        let entry = map.entry(queue.to_string()).or_default();
        entry.0 += 1;
        entry.1 += took.as_secs_f64();
    }
    /// A result came from the cache.
    pub fn result_cache_hit(&self) {
        self.result_cache_hits.fetch_add(1, Ordering::Relaxed);
    }
    /// A cacheable job missed the cache.
    pub fn result_cache_miss(&self) {
        self.result_cache_misses.fetch_add(1, Ordering::Relaxed);
    }
    /// A job was paused to make way.
    pub fn preemption(&self) {
        self.preemptions.fetch_add(1, Ordering::Relaxed);
    }
    /// A gateway request arrived.
    pub fn gateway_request(&self) {
        self.gateway_requests.fetch_add(1, Ordering::Relaxed);
    }
    /// A gateway request had to wait for a load.
    pub fn gateway_load_wait(&self) {
        self.gateway_load_waits.fetch_add(1, Ordering::Relaxed);
    }
    /// A gateway request got a 503.
    pub fn gateway_unavailable(&self) {
        self.gateway_unavailable.fetch_add(1, Ordering::Relaxed);
    }
    /// A gateway request went to another node.
    pub fn gateway_forwarded(&self) {
        self.gateway_forwarded.fetch_add(1, Ordering::Relaxed);
    }
    /// A session's instance was gone and it was bound to a new one.
    pub fn session_rebind(&self) {
        self.session_rebinds.fetch_add(1, Ordering::Relaxed);
    }

    /// Render everything.
    pub fn render(
        &self,
        queues: &[QueueStats],
        instances: &[InstanceInfo],
        pool: &ResourcePool,
    ) -> String {
        let mut out = String::new();
        let counter = |out: &mut String, name: &str, help: &str, value: u64| {
            let _ = writeln!(
                out,
                "# HELP {name} {help}\n# TYPE {name} counter\n{name} {value}"
            );
        };
        counter(
            &mut out,
            "postrust_ai_result_cache_hits_total",
            "Jobs answered from the result cache.",
            self.result_cache_hits.load(Ordering::Relaxed),
        );
        counter(
            &mut out,
            "postrust_ai_result_cache_misses_total",
            "Cacheable jobs that ran the model.",
            self.result_cache_misses.load(Ordering::Relaxed),
        );
        counter(
            &mut out,
            "postrust_ai_preemptions_total",
            "Jobs paused to make way for higher priority work.",
            self.preemptions.load(Ordering::Relaxed),
        );
        counter(
            &mut out,
            "postrust_ai_gateway_requests_total",
            "Synchronous inference requests.",
            self.gateway_requests.load(Ordering::Relaxed),
        );
        counter(
            &mut out,
            "postrust_ai_gateway_load_waits_total",
            "Gateway requests that waited for a model to load.",
            self.gateway_load_waits.load(Ordering::Relaxed),
        );
        counter(
            &mut out,
            "postrust_ai_gateway_unavailable_total",
            "Gateway requests answered 503 while a model loaded.",
            self.gateway_unavailable.load(Ordering::Relaxed),
        );
        counter(
            &mut out,
            "postrust_ai_gateway_forwarded_total",
            "Gateway requests forwarded to another node.",
            self.gateway_forwarded.load(Ordering::Relaxed),
        );
        counter(
            &mut out,
            "postrust_ai_session_rebinds_total",
            "Sessions whose instance was gone and were bound anew.",
            self.session_rebinds.load(Ordering::Relaxed),
        );

        let _ = writeln!(out, "# HELP postrust_ai_jobs_finished_total Jobs this node finished running.\n# TYPE postrust_ai_jobs_finished_total counter");
        let durations = self.job_durations.lock().clone();
        for (queue, (n, _)) in &durations {
            let _ = writeln!(
                out,
                "postrust_ai_jobs_finished_total{{queue=\"{}\"}} {n}",
                escape(queue)
            );
        }
        let _ = writeln!(out, "# HELP postrust_ai_job_seconds_total Time spent running jobs.\n# TYPE postrust_ai_job_seconds_total counter");
        for (queue, (_, secs)) in &durations {
            let _ = writeln!(
                out,
                "postrust_ai_job_seconds_total{{queue=\"{}\"}} {secs}",
                escape(queue)
            );
        }

        let _ = writeln!(out, "# HELP postrust_ai_queue_waiting Claimable jobs by queue and class.\n# TYPE postrust_ai_queue_waiting gauge");
        for q in queues {
            for (class, n) in &q.waiting {
                let _ = writeln!(
                    out,
                    "postrust_ai_queue_waiting{{queue=\"{}\",class=\"{class}\"}} {n}",
                    escape(&q.queue)
                );
            }
        }
        let _ = writeln!(out, "# HELP postrust_ai_queue_running Running jobs by queue, cluster-wide.\n# TYPE postrust_ai_queue_running gauge");
        for q in queues {
            let _ = writeln!(
                out,
                "postrust_ai_queue_running{{queue=\"{}\"}} {}",
                escape(&q.queue),
                q.running
            );
        }
        let _ = writeln!(out, "# HELP postrust_ai_queue_paused_jobs Paused jobs by queue.\n# TYPE postrust_ai_queue_paused_jobs gauge");
        for q in queues {
            let _ = writeln!(
                out,
                "postrust_ai_queue_paused_jobs{{queue=\"{}\"}} {}",
                escape(&q.queue),
                q.paused_jobs
            );
        }

        let _ = writeln!(out, "# HELP postrust_ai_instances Model instances on this node by state.\n# TYPE postrust_ai_instances gauge");
        let mut by_state: BTreeMap<(String, String), usize> = BTreeMap::new();
        for i in instances {
            let state = serde_json::to_value(i.state)
                .ok()
                .and_then(|v| v.as_str().map(String::from))
                .unwrap_or_default();
            *by_state.entry((i.model_id.clone(), state)).or_default() += 1;
        }
        for ((model, state), n) in by_state {
            let _ = writeln!(
                out,
                "postrust_ai_instances{{model=\"{}\",state=\"{state}\"}} {n}",
                escape(&model)
            );
        }

        let _ = writeln!(out, "# HELP postrust_ai_memory_mb Host memory budget and reservations.\n# TYPE postrust_ai_memory_mb gauge");
        let _ = writeln!(
            out,
            "postrust_ai_memory_mb{{kind=\"budget\"}} {}",
            pool.mem_budget_mb
        );
        let _ = writeln!(
            out,
            "postrust_ai_memory_mb{{kind=\"reserved\"}} {}",
            pool.mem_reserved_mb
        );
        let _ = writeln!(out, "# HELP postrust_ai_gpu_mb VRAM budget and reservations.\n# TYPE postrust_ai_gpu_mb gauge");
        for g in &pool.gpus {
            let _ = writeln!(
                out,
                "postrust_ai_gpu_mb{{gpu=\"{}\",kind=\"budget\"}} {}",
                g.index, g.budget_mb
            );
            let _ = writeln!(
                out,
                "postrust_ai_gpu_mb{{gpu=\"{}\",kind=\"reserved\"}} {}",
                g.index, g.reserved_mb
            );
        }
        out
    }
}

fn escape(label: &str) -> String {
    label
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}
