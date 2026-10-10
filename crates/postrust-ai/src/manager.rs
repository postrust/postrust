//! The model manager: which instances are loaded, and what to load, park or
//! stop next.
//!
//! Everything goes through [`ModelManager::acquire`], which returns a
//! [`Lease`] on a ready instance -- loading, waking or making room for one
//! first if it has to. Releasing the lease (dropping it) is what makes an
//! instance idle again.
//!
//! # Making room
//!
//! When a model does not fit, the manager works down a ladder, cheapest first:
//!
//! 1. **Idle instances** of other models are parked (VRAM only short, and the
//!    server can sleep) or stopped, lowest value first. Value is how recently
//!    the instance was used, what it costs to reload, and whether queued jobs
//!    are waiting for it. Instances with queued work of a higher class than the
//!    requester, `pinned` models, the last `min_warm` instances of a model, and
//!    instances holding session locks (unless the requester is `critical`) are
//!    never candidates.
//! 2. **Superlow jobs** holding instances are asked to pause. Their instances
//!    then become idle and step 1 applies.
//! 3. Otherwise the requester waits, until its deadline.
//!
//! Superlow requesters skip both: they only ever use what is already free.
//!
//! # Locking
//!
//! State sits behind a synchronous mutex, never held across an await. Slow
//! work -- downloads, starting servers, health checks, stopping -- runs in
//! spawned tasks that take the lock only to record their outcome, and every
//! outcome is announced on one `Notify` that waiters re-check against.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::Serialize;
use sqlx::PgPool;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::cache::FileCache;
use crate::config::ResourceConfig;
use crate::error::{AiError, AiResult};
use crate::measure;
use crate::model::ModelSpec;
use crate::queue::{ModelDemand, Priority};
use crate::resources::{Need, Probe, Reservation, ResourcePool};
use crate::runtime::{self, flavor, ServerHandle, StartContext};

type SharedHandle = Arc<tokio::sync::Mutex<Box<dyn ServerHandle>>>;

/// How long a server gets to exit after SIGTERM.
const STOP_GRACE: Duration = Duration::from_secs(30);

/// Where an instance is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InstanceState {
    /// Fetching files, starting, or waiting for the health check.
    Loading,
    /// Taking requests.
    Ready,
    /// Being parked.
    Parking,
    /// Weights in host memory, GPU released.
    Parked,
    /// Being woken from parked.
    Waking,
    /// Being stopped; gone once it exits.
    Stopping,
}

#[derive(Debug)]
struct LeaseInfo {
    priority: Priority,
    job_id: Option<Uuid>,
    preempt: CancellationToken,
}

struct Instance {
    id: Uuid,
    spec: Arc<ModelSpec>,
    state: InstanceState,
    reservation: Reservation,
    /// The GPU this instance was started on, which a parked instance has to
    /// wake on: the server only ever saw that device.
    home_gpu: Option<(u32, u64)>,
    port: Option<u16>,
    base_url: String,
    handle: Option<SharedHandle>,
    leases: HashMap<u64, LeaseInfo>,
    sessions: usize,
    last_used: Instant,
    load_duration: Duration,
    /// Was asked to stop because of pressure rather than idleness.
    evicted: bool,
    /// Last measured host memory and VRAM, MiB. Reported, never acted on.
    measured_mem_mb: Option<u64>,
    measured_gpu_mb: Option<u64>,
    /// The over-budget warning has been given, so it is not repeated.
    over_budget_reported: bool,
}

impl Instance {
    fn is_live(&self) -> bool {
        self.state != InstanceState::Stopping
    }
    fn has_capacity(&self) -> bool {
        self.state == InstanceState::Ready && self.leases.len() < self.spec.max_concurrency
    }
    fn idle(&self) -> bool {
        self.leases.is_empty()
    }
}

/// One instance, for the API and node heartbeats.
#[derive(Debug, Clone, Serialize)]
pub struct InstanceInfo {
    /// Id.
    pub id: Uuid,
    /// Model.
    pub model_id: String,
    /// State.
    pub state: InstanceState,
    /// Where it listens.
    pub base_url: String,
    /// Requests in flight.
    pub in_flight: usize,
    /// Of which superlow.
    pub in_flight_superlow: usize,
    /// Session locks held on it.
    pub sessions: usize,
    /// Seconds since last use.
    pub idle_secs: u64,
    /// What it holds.
    pub reservation: Reservation,
    /// How long it took to load, in seconds.
    pub load_secs: f64,
    /// Host memory the server's process group holds, MiB, as of the last
    /// `maintain` pass. `None`: not measured (container, attached, loading).
    pub measured_mem_mb: Option<u64>,
    /// VRAM held by that group, MiB. `None` without `nvidia-smi`.
    pub measured_gpu_mb: Option<u64>,
}

struct State {
    pool: ResourcePool,
    instances: HashMap<Uuid, Instance>,
    ports: HashSet<u16>,
    /// Last load failure per model, so waiters fail fast instead of timing
    /// out on a model that cannot start.
    failures: HashMap<String, (Instant, String)>,
    demand: HashMap<String, ModelDemand>,
}

struct Inner {
    node_id: String,
    specs: HashMap<String, Arc<ModelSpec>>,
    config: ResourceConfig,
    state: Mutex<State>,
    changed: Notify,
    http: reqwest::Client,
    files: Arc<FileCache>,
    events: Option<PgPool>,
    next_lease: AtomicU64,
    shutdown: CancellationToken,
}

/// The model manager. Cheap to clone.
#[derive(Clone)]
pub struct ModelManager {
    inner: Arc<Inner>,
}

/// What [`ModelManager::acquire`] may do to get an instance.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct AcquireOptions {
    /// Requester's class.
    pub priority: Priority,
    /// The job this is for, so a preemption can name it.
    pub job_id: Option<Uuid>,
    /// Require this instance (a session lock).
    pub instance: Option<Uuid>,
    /// Give up after this long.
    pub wait: Duration,
    /// Whether idle instances may be unloaded to make room. Always false
    /// for superlow.
    pub may_evict: bool,
    /// Whether superlow jobs may be paused to make room. Always false for
    /// superlow.
    pub may_preempt: bool,
    /// Prefer the instance with fewest sessions, for binding a new one.
    pub for_new_session: bool,
    /// For a new session: instances already holding this many sessions are
    /// passed over while another replica can still be loaded.
    pub max_sessions: Option<usize>,
}

impl AcquireOptions {
    /// Defaults for a requester of this class.
    pub fn new(priority: Priority, wait: Duration) -> Self {
        let regular = !priority.is_superlow();
        Self {
            priority,
            job_id: None,
            instance: None,
            wait,
            may_evict: regular,
            may_preempt: regular,
            for_new_session: false,
            max_sessions: None,
        }
    }
}

/// A claim on one ready instance. Dropping it releases the claim.
pub struct Lease {
    inner: Arc<Inner>,
    instance_id: Uuid,
    lease_id: u64,
    base_url: String,
    spec: Arc<ModelSpec>,
    preempt: CancellationToken,
}

impl Lease {
    /// Where to send requests.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }
    /// The instance.
    pub fn instance_id(&self) -> Uuid {
        self.instance_id
    }
    /// The model.
    pub fn spec(&self) -> &ModelSpec {
        &self.spec
    }
    /// Cancelled when the manager wants this lease's job to pause and give the
    /// instance up.
    pub fn preempted(&self) -> &CancellationToken {
        &self.preempt
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        {
            let mut state = self.inner.state.lock();
            if let Some(instance) = state.instances.get_mut(&self.instance_id) {
                instance.leases.remove(&self.lease_id);
                instance.last_used = Instant::now();
            }
        }
        self.inner.changed.notify_waiters();
    }
}

impl std::fmt::Debug for Lease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Lease")
            .field("model", &self.spec.id)
            .field("instance_id", &self.instance_id)
            .field("base_url", &self.base_url)
            .finish()
    }
}

/// What one pass of `acquire` decided.
enum Step {
    Leased(Lease),
    Wait,
    Fail(AiError),
}

impl ModelManager {
    /// A manager for these models within this budget.
    pub fn new(
        node_id: String,
        models: &[ModelSpec],
        config: ResourceConfig,
        probe: Probe,
        http: reqwest::Client,
        files: Arc<FileCache>,
        events: Option<PgPool>,
    ) -> Self {
        let pool = ResourcePool::new(&config, probe);
        tracing::info!(
            memory_mb = pool.mem_budget_mb,
            gpus = ?pool.gpus.iter().map(|g| (g.index, g.budget_mb)).collect::<Vec<_>>(),
            "resource budget"
        );
        Self {
            inner: Arc::new(Inner {
                node_id,
                specs: models
                    .iter()
                    .map(|m| (m.id.clone(), Arc::new(m.clone())))
                    .collect(),
                config,
                state: Mutex::new(State {
                    pool,
                    instances: HashMap::new(),
                    ports: HashSet::new(),
                    failures: HashMap::new(),
                    demand: HashMap::new(),
                }),
                changed: Notify::new(),
                http,
                files,
                events,
                next_lease: AtomicU64::new(1),
                shutdown: CancellationToken::new(),
            }),
        }
    }

    /// The spec for a model.
    pub fn spec(&self, model_id: &str) -> AiResult<Arc<ModelSpec>> {
        self.inner
            .specs
            .get(model_id)
            .cloned()
            .ok_or_else(|| AiError::UnknownModel(model_id.into()))
    }

    /// Every registered model.
    pub fn specs(&self) -> Vec<Arc<ModelSpec>> {
        let mut specs: Vec<_> = self.inner.specs.values().cloned().collect();
        specs.sort_by(|a, b| a.id.cmp(&b.id));
        specs
    }

    /// Get a lease on a ready instance of `model_id`, loading one if needed.
    pub async fn acquire(&self, model_id: &str, opts: AcquireOptions) -> AiResult<Lease> {
        let spec = self.spec(model_id)?;
        let started = Instant::now();
        let deadline = started + opts.wait;
        let mut opts = opts;
        if opts.priority.is_superlow() {
            opts.may_evict = false;
            opts.may_preempt = false;
        }

        loop {
            let notified = self.inner.changed.notified();
            tokio::pin!(notified);
            // Registered before the state is read, so a change between the
            // read and the await is not missed.
            notified.as_mut().enable();

            match self.step(&spec, &opts, started) {
                Step::Leased(lease) => return Ok(lease),
                Step::Fail(error) => return Err(error),
                Step::Wait => {}
            }

            let now = Instant::now();
            if now >= deadline {
                return Err(AiError::LoadTimeout {
                    model: model_id.into(),
                    waited: opts.wait,
                });
            }
            tokio::select! {
                _ = &mut notified => {}
                // Belt and braces: re-check periodically even without a
                // notification, e.g. for the probe freeing up.
                _ = tokio::time::sleep((deadline - now).min(Duration::from_secs(2))) => {}
                _ = self.inner.shutdown.cancelled() => return Err(AiError::Cancelled),
            }
        }
    }

    /// One decision pass. Never awaits; spawns whatever slow work it starts.
    fn step(&self, spec: &Arc<ModelSpec>, opts: &AcquireOptions, started: Instant) -> Step {
        let mut state = self.inner.state.lock();

        if let Some((at, why)) = state.failures.get(&spec.id) {
            if *at > started {
                return Step::Fail(AiError::LoadFailed {
                    model: spec.id.clone(),
                    detail: why.clone(),
                });
            }
        }

        // A session lock names its instance and nothing else will do.
        if let Some(wanted) = opts.instance {
            let Some(instance) = state.instances.get(&wanted).filter(|i| i.is_live()) else {
                return Step::Fail(AiError::NotFound(format!("instance {wanted}")));
            };
            match instance.state {
                InstanceState::Ready if instance.has_capacity() => {
                    return Step::Leased(self.lease(&mut state, wanted, opts));
                }
                InstanceState::Parked => {
                    self.try_wake(&mut state, wanted, opts);
                }
                _ => {}
            }
            return Step::Wait;
        }

        let live: Vec<(Uuid, InstanceState)> = state
            .instances
            .values()
            .filter(|i| i.spec.id == spec.id && i.is_live())
            .map(|i| (i.id, i.state))
            .collect();
        let replicas = live
            .iter()
            .filter(|(_, s)| *s != InstanceState::Parking)
            .count();

        // 1. A ready instance with a free slot: the least busy, or for a new
        // session the one holding fewest sessions. A new session skips
        // instances at their session limit while another replica could still
        // be loaded; at the replica limit, it takes the least-locked anyway.
        let session_cap = opts
            .max_sessions
            .filter(|_| opts.for_new_session && replicas < spec.max_replicas);
        let best = state
            .instances
            .values()
            .filter(|i| i.spec.id == spec.id && i.has_capacity())
            .filter(|i| session_cap.is_none_or(|cap| i.sessions < cap))
            .min_by_key(|i| {
                if opts.for_new_session {
                    (i.sessions, i.leases.len())
                } else {
                    (i.leases.len(), i.sessions)
                }
            })
            .map(|i| i.id);
        if let Some(id) = best {
            return Step::Leased(self.lease(&mut state, id, opts));
        }

        // 2. One on its way up: wait for it.
        if live
            .iter()
            .any(|(_, s)| matches!(s, InstanceState::Loading | InstanceState::Waking))
        {
            return Step::Wait;
        }

        // 3. A parked one: wake it.
        if let Some((id, _)) = live.iter().find(|(_, s)| *s == InstanceState::Parked) {
            if self.try_wake(&mut state, *id, opts) {
                return Step::Wait;
            }
        }

        // 4. Room for another replica: load one.
        let total_live = state.instances.values().filter(|i| i.is_live()).count();
        if replicas < spec.max_replicas && total_live < self.inner.config.max_instances {
            let need = Need {
                mem_mb: spec.memory_mb,
                gpu_mb: spec.gpu_mb,
            };
            if !state.pool.could_ever_fit(need) {
                return Step::Fail(AiError::InsufficientResources {
                    model: spec.id.clone(),
                    detail: format!(
                        "needs {} MiB memory and {} MiB VRAM, more than this node's whole budget",
                        need.mem_mb, need.gpu_mb
                    ),
                });
            }
            if let Some(reservation) = state.pool.try_reserve(need) {
                self.start_load(&mut state, spec.clone(), reservation);
                return Step::Wait;
            }
            // A new session passed over full-of-sessions instances hoping
            // for a fresh replica. There is no room for one: share instead.
            if session_cap.is_some() {
                let fallback = state
                    .instances
                    .values()
                    .filter(|i| i.spec.id == spec.id && i.has_capacity())
                    .min_by_key(|i| (i.sessions, i.leases.len()))
                    .map(|i| i.id);
                if let Some(id) = fallback {
                    return Step::Leased(self.lease(&mut state, id, opts));
                }
            }
            self.make_room(&mut state, spec, need, opts);
            return Step::Wait;
        }

        // 5. At the replica limit and every slot busy. A regular requester can
        // take a slot from a superlow job.
        if opts.may_preempt {
            // One at a time: a single freed slot is all this requester needs.
            // Skip if a preemption is already on its way out.
            let leases = state
                .instances
                .values()
                .filter(|i| i.spec.id == spec.id && i.state == InstanceState::Ready)
                .flat_map(|i| i.leases.values())
                .filter(|l| l.priority.is_superlow());
            let mut pending = false;
            let mut victim = None;
            for lease in leases {
                if lease.preempt.is_cancelled() {
                    pending = true;
                } else if victim.is_none() {
                    victim = Some(lease);
                }
            }
            if let (false, Some(lease)) = (pending, victim) {
                lease.preempt.cancel();
                self.event(
                    &spec.id,
                    None,
                    "preempted_jobs",
                    serde_json::json!({ "jobs": [lease.job_id], "for": "slot" }),
                );
            }
        }
        Step::Wait
    }

    fn lease(&self, state: &mut State, instance_id: Uuid, opts: &AcquireOptions) -> Lease {
        let lease_id = self.inner.next_lease.fetch_add(1, Ordering::Relaxed);
        let instance = state
            .instances
            .get_mut(&instance_id)
            .expect("caller checked the instance exists");
        let preempt = CancellationToken::new();
        instance.leases.insert(
            lease_id,
            LeaseInfo {
                priority: opts.priority,
                job_id: opts.job_id,
                preempt: preempt.clone(),
            },
        );
        instance.last_used = Instant::now();
        Lease {
            inner: self.inner.clone(),
            instance_id,
            lease_id,
            base_url: instance.base_url.clone(),
            spec: instance.spec.clone(),
            preempt,
        }
    }

    /// Free resources for `need` by parking or stopping idle instances, or
    /// failing that by preempting superlow jobs.
    fn make_room(
        &self,
        state: &mut State,
        spec: &Arc<ModelSpec>,
        need: Need,
        opts: &AcquireOptions,
    ) {
        if !opts.may_evict {
            return;
        }
        // Plan against what the pool will look like once transitions already
        // under way finish, or every pass would pick fresh victims while the
        // last ones are still exiting.
        let mut projected = self.projected_pool(state);
        if projected.shortfall(need).is_zero() {
            return;
        }

        let mut ready_counts: HashMap<&str, usize> = HashMap::new();
        for i in state.instances.values() {
            if matches!(i.state, InstanceState::Ready | InstanceState::Loading) {
                *ready_counts.entry(i.spec.id.as_str()).or_default() += 1;
            }
        }

        let mut candidates: Vec<(f64, Uuid)> = state
            .instances
            .values()
            .filter(|i| {
                matches!(i.state, InstanceState::Ready | InstanceState::Parked)
                    && i.idle()
                    && !i.spec.pinned
                    && (i.sessions == 0 || opts.priority == Priority::Critical)
            })
            .filter(|i| {
                state
                    .demand
                    .get(&i.spec.id)
                    .is_none_or(|d| d.top_priority <= opts.priority)
            })
            .map(|i| (self.value(state, i), i.id))
            .collect();
        candidates.sort_by(|a, b| a.0.total_cmp(&b.0));

        // Choose victims until the projection fits. Park when only VRAM is
        // short and host memory can take the weights: stopping throws away a
        // reload's worth of work.
        let mut chosen: Vec<(Uuid, bool)> = Vec::new();
        for (_, id) in candidates {
            if projected.shortfall(need).is_zero() {
                break;
            }
            let instance = &state.instances[&id];
            // Stopping something that frees nothing this request is short of
            // is pure loss. Waking a parked instance asks only for VRAM, so
            // without this the parked instance itself (its host memory is the
            // one thing it holds) was the first victim, and was stopped to
            // make room for waking it.
            //
            // Judged per dimension, not by whether the shortfall drops: the
            // VRAM shortfall is that of the GPU closest to fitting, so freeing
            // part of what another GPU lacks leaves it unchanged, though
            // several such victims together may make room there.
            let short = projected.shortfall(need);
            let frees_mem = short.mem_mb > 0 && instance.reservation.mem_mb > 0;
            let frees_gpu =
                short.gpu_mb > 0 && instance.reservation.gpu.is_some_and(|(_, mb)| mb > 0);
            if !frees_mem && !frees_gpu {
                continue;
            }
            // Never below min_warm for a ready instance, counting the victims
            // already chosen in this pass: the counts were taken before any
            // were, so without this two idle replicas of a `min_warm = 1`
            // model could both be picked. A parked instance is not counted
            // as warm and may go.
            if instance.state == InstanceState::Ready {
                let left = ready_counts.entry(instance.spec.id.as_str()).or_default();
                if *left <= instance.spec.min_warm {
                    continue;
                }
                *left -= 1;
            }
            if instance.state == InstanceState::Ready
                && instance.spec.can_park()
                && projected.shortfall(need).mem_mb == 0
            {
                let mut trial = projected.clone();
                let mut reservation = instance.reservation;
                if trial.park(&mut reservation) && trial.shortfall(need).mem_mb == 0 {
                    projected = trial;
                    chosen.push((id, true));
                    continue;
                }
            }
            projected.release(&instance.reservation);
            chosen.push((id, false));
        }

        if projected.shortfall(need).is_zero() {
            for (id, park) in chosen {
                if park {
                    self.start_park(state, id);
                } else {
                    self.start_stop(state, id, true);
                }
            }
            return;
        }

        // Not enough idle capacity. Ask superlow jobs to make way: find
        // instances held only by superlow leases and preempt them all. Once
        // they yield, those instances are idle and the next pass evicts them.
        if !opts.may_preempt {
            return;
        }
        let mut preempted = Vec::new();
        for instance in state.instances.values() {
            if instance.state != InstanceState::Ready
                || instance.idle()
                || instance.spec.id == spec.id
                || instance.spec.pinned
            {
                continue;
            }
            if instance.leases.values().all(|l| l.priority.is_superlow()) {
                for lease in instance.leases.values() {
                    if !lease.preempt.is_cancelled() {
                        lease.preempt.cancel();
                        if let Some(job) = lease.job_id {
                            preempted.push(job);
                        }
                    }
                }
                // Stop as soon as the freed instances would be enough.
                projected.release(&instance.reservation);
                if projected.shortfall(need).is_zero() {
                    break;
                }
            }
        }
        if !preempted.is_empty() {
            tracing::info!(model = %spec.id, jobs = ?preempted, "preempting superlow jobs for resources");
            self.event(
                &spec.id,
                None,
                "preempted_jobs",
                serde_json::json!({ "jobs": preempted, "for": "resources" }),
            );
        }
    }

    /// Keep-worthiness: higher stays loaded longer.
    fn value(&self, state: &State, instance: &Instance) -> f64 {
        let idle_minutes = instance.last_used.elapsed().as_secs_f64() / 60.0;
        let reload_secs = instance.load_duration.as_secs_f64();
        let demand = state
            .demand
            .get(&instance.spec.id)
            .map(|d| 1000.0 + d.top_priority.base() as f64 + d.waiting as f64)
            .unwrap_or(0.0);
        // A parked instance is already cheap to keep and cheap to restore;
        // prefer evicting it over a ready one only when both are cold.
        let parked = if instance.state == InstanceState::Parked {
            -5.0
        } else {
            0.0
        };
        demand + reload_secs + parked - idle_minutes + instance.sessions as f64 * 10.0
    }

    /// The pool as it will be once in-flight stops and parks finish.
    fn projected_pool(&self, state: &State) -> ResourcePool {
        let mut pool = state.pool.clone();
        for instance in state.instances.values() {
            match instance.state {
                InstanceState::Stopping => pool.release(&instance.reservation),
                InstanceState::Parking => {
                    let mut r = instance.reservation;
                    if !pool.park(&mut r) {
                        pool.release(&instance.reservation);
                    }
                }
                _ => {}
            }
        }
        pool
    }

    fn allocate_port(&self, state: &mut State) -> Option<u16> {
        let (low, high) = self.inner.config.port_range;
        (low..=high).find(|port| {
            !state.ports.contains(port)
                && std::net::TcpListener::bind(("127.0.0.1", *port)).is_ok()
                && state.ports.insert(*port)
        })
    }

    fn start_load(&self, state: &mut State, spec: Arc<ModelSpec>, reservation: Reservation) {
        let id = Uuid::new_v4();
        let attached = matches!(spec.runtime, crate::model::RuntimeSpec::Attached { .. });
        let port = if attached {
            None
        } else {
            self.allocate_port(state)
        };
        if !attached && port.is_none() {
            state.pool.release(&reservation);
            state.failures.insert(
                spec.id.clone(),
                (
                    Instant::now(),
                    "no free port in resources.port_range".into(),
                ),
            );
            self.inner.changed.notify_waiters();
            return;
        }
        state.instances.insert(
            id,
            Instance {
                id,
                spec: spec.clone(),
                state: InstanceState::Loading,
                reservation,
                home_gpu: reservation.gpu,
                port,
                base_url: String::new(),
                handle: None,
                leases: HashMap::new(),
                sessions: 0,
                last_used: Instant::now(),
                load_duration: Duration::ZERO,
                evicted: false,
                measured_mem_mb: None,
                measured_gpu_mb: None,
                over_budget_reported: false,
            },
        );
        tracing::info!(model = %spec.id, instance = %id, ?reservation, "loading model");
        self.event(
            &spec.id,
            Some(id),
            "load_started",
            serde_json::json!({ "reservation": reservation }),
        );

        let this = self.clone();
        tokio::spawn(async move {
            let started = Instant::now();
            let result = this
                .load(&spec, id, port, reservation.gpu.map(|g| g.0))
                .await;
            let base_url = match &result {
                Ok(handle) => handle.lock().await.base_url(),
                Err(_) => String::new(),
            };
            let mut state = this.inner.state.lock();
            match result {
                Ok(handle) => {
                    if let Some(instance) = state.instances.get_mut(&id) {
                        instance.base_url = base_url;
                        instance.handle = Some(handle);
                        instance.load_duration = started.elapsed();
                        instance.last_used = Instant::now();
                        let mut r = instance.reservation;
                        // Stopping already (shutdown raced the load): leave it.
                        if instance.state == InstanceState::Loading {
                            instance.state = InstanceState::Ready;
                        }
                        state.pool.materialize(&mut r);
                        state.instances.get_mut(&id).expect("present").reservation = r;
                        state.failures.remove(&spec.id);
                        tracing::info!(model = %spec.id, instance = %id, secs = started.elapsed().as_secs_f64(), "model ready");
                        drop(state);
                        this.event(
                            &spec.id,
                            Some(id),
                            "ready",
                            serde_json::json!({ "load_secs": started.elapsed().as_secs_f64() }),
                        );
                    }
                }
                Err(error) => {
                    tracing::error!(model = %spec.id, instance = %id, %error, "model failed to load");
                    if let Some(instance) = state.instances.remove(&id) {
                        state.pool.release(&instance.reservation);
                        if let Some(port) = instance.port {
                            state.ports.remove(&port);
                        }
                    }
                    state
                        .failures
                        .insert(spec.id.clone(), (Instant::now(), error.to_string()));
                    drop(state);
                    this.inner.files.unpin(&spec.files);
                    this.event(
                        &spec.id,
                        Some(id),
                        "load_failed",
                        serde_json::json!({ "error": error.to_string() }),
                    );
                }
            }
            this.inner.changed.notify_waiters();
        });
    }

    async fn load(
        &self,
        spec: &ModelSpec,
        id: Uuid,
        port: Option<u16>,
        gpu: Option<u32>,
    ) -> AiResult<SharedHandle> {
        let files = self.inner.files.ensure_all(&spec.files).await?;
        let ctx = StartContext {
            spec,
            instance_id: id,
            port: port.unwrap_or(0),
            gpu,
            files,
            cache_dir: self.inner.files.dir(),
            hf_home: self.inner.files.hf_home(),
        };
        let mut handle = runtime::start(&ctx).await?;
        let timeout = Duration::from_secs(spec.load_timeout_secs);
        if let Err(error) =
            flavor::wait_ready(&self.inner.http, spec, handle.as_mut(), timeout).await
        {
            handle.stop(Duration::from_secs(5)).await;
            return Err(error);
        }
        Ok(Arc::new(tokio::sync::Mutex::new(handle)))
    }

    /// Wake a parked instance if its GPU has room. Returns whether it started.
    fn try_wake(&self, state: &mut State, id: Uuid, opts: &AcquireOptions) -> bool {
        let Some((index, mb)) = state.instances.get(&id).and_then(|i| i.home_gpu) else {
            return false;
        };
        let mut reservation = state.instances[&id].reservation;
        if !state.pool.unpark(&mut reservation, index, mb) {
            let spec = state.instances[&id].spec.clone();
            self.make_room(
                state,
                &spec,
                Need {
                    mem_mb: 0,
                    gpu_mb: mb,
                },
                opts,
            );
            return false;
        }
        let instance = state.instances.get_mut(&id).expect("checked above");
        instance.reservation = reservation;
        instance.state = InstanceState::Waking;
        let spec = instance.spec.clone();
        let base = instance.base_url.clone();
        tracing::info!(model = %spec.id, instance = %id, "waking parked model");

        let this = self.clone();
        tokio::spawn(async move {
            let result = flavor::wake(&this.inner.http, &spec, &base).await;
            {
                let mut state = this.inner.state.lock();
                if let Some(instance) = state.instances.get_mut(&id) {
                    if result.is_ok() && instance.state == InstanceState::Waking {
                        instance.state = InstanceState::Ready;
                        instance.last_used = Instant::now();
                    }
                }
            }
            match result {
                Ok(()) => this.event(&spec.id, Some(id), "woken", serde_json::Value::Null),
                Err(error) => {
                    tracing::error!(model = %spec.id, instance = %id, %error, "wake failed; stopping");
                    let mut state = this.inner.state.lock();
                    this.start_stop(&mut state, id, false);
                }
            }
            this.inner.changed.notify_waiters();
        });
        true
    }

    fn start_park(&self, state: &mut State, id: Uuid) {
        let Some(instance) = state.instances.get_mut(&id) else {
            return;
        };
        instance.state = InstanceState::Parking;
        let spec = instance.spec.clone();
        let base = instance.base_url.clone();
        tracing::info!(model = %spec.id, instance = %id, "parking model");

        let this = self.clone();
        tokio::spawn(async move {
            let parked = flavor::park(&this.inner.http, &spec, &base).await;
            {
                let mut guard = this.inner.state.lock();
                let state = &mut *guard;
                match parked {
                    Ok(true) => {
                        if let Some(instance) = state.instances.get_mut(&id) {
                            let mut r = instance.reservation;
                            if state.pool.park(&mut r) {
                                instance.reservation = r;
                                instance.state = InstanceState::Parked;
                                instance.last_used = Instant::now();
                                this.event(&spec.id, Some(id), "parked", serde_json::Value::Null);
                            } else {
                                // Host memory filled up meanwhile. The server
                                // is asleep anyway; stop it rather than lie
                                // about where its weights are.
                                this.start_stop(state, id, true);
                            }
                        }
                    }
                    Ok(false) | Err(_) => {
                        if let Err(error) = &parked {
                            tracing::warn!(model = %spec.id, %error, "park failed; stopping instead");
                        }
                        this.start_stop(state, id, true);
                    }
                }
            }
            this.inner.changed.notify_waiters();
        });
    }

    fn start_stop(&self, state: &mut State, id: Uuid, evicted: bool) {
        let Some(instance) = state.instances.get_mut(&id) else {
            return;
        };
        if instance.state == InstanceState::Stopping {
            return;
        }
        instance.state = InstanceState::Stopping;
        instance.evicted = evicted;
        let spec = instance.spec.clone();
        let base = instance.base_url.clone();
        let handle = instance.handle.clone();
        tracing::info!(model = %spec.id, instance = %id, evicted, "stopping model");

        let this = self.clone();
        tokio::spawn(async move {
            if !base.is_empty() {
                flavor::before_stop(&this.inner.http, &spec, &base).await;
            }
            if let Some(handle) = handle {
                handle.lock().await.stop(STOP_GRACE).await;
            }
            this.forget(id);
            this.inner.files.unpin(&spec.files);
            this.event(
                &spec.id,
                Some(id),
                if evicted { "evicted" } else { "stopped" },
                serde_json::Value::Null,
            );
            this.inner.changed.notify_waiters();
        });
    }

    /// Drop an instance from the books and give back what it held.
    fn forget(&self, id: Uuid) {
        let mut state = self.inner.state.lock();
        if let Some(instance) = state.instances.remove(&id) {
            state.pool.release(&instance.reservation);
            if let Some(port) = instance.port {
                state.ports.remove(&port);
            }
        }
    }

    /// Periodic housekeeping: crashed servers, idle unloads, parked
    /// timeouts, the probe, and `min_warm`.
    pub async fn maintain(&self) {
        let probe = Probe::read().await;

        // Crash detection needs the handle lock, so it is gathered first and
        // checked outside the state lock.
        let handles: Vec<(Uuid, String, SharedHandle)> = {
            let state = self.inner.state.lock();
            state
                .instances
                .values()
                .filter(|i| matches!(i.state, InstanceState::Ready | InstanceState::Parked))
                .filter_map(|i| i.handle.clone().map(|h| (i.id, i.spec.id.clone(), h)))
                .collect()
        };
        let mut crashed = Vec::new();
        let mut pids: Vec<(Uuid, u32)> = Vec::new();
        for (id, model, handle) in handles {
            if let Ok(mut h) = handle.try_lock() {
                if let Some(exit) = h.exited() {
                    crashed.push((id, model, exit));
                } else if let Some(pid) = h.pid() {
                    pids.push((id, pid));
                }
            }
        }
        // Once per pass, outside the state lock: it reads /proc and runs
        // nvidia-smi.
        let usage = measure::measure(&pids.iter().map(|(_, pid)| *pid).collect::<Vec<_>>()).await;

        let mut warm_up = Vec::new();
        let mut over_budget = Vec::new();
        {
            let mut state = self.inner.state.lock();
            state.pool.update_probe(probe);

            for (id, pid) in &pids {
                let (Some(instance), Some(used)) = (state.instances.get_mut(id), usage.get(pid))
                else {
                    continue;
                };
                // An instance that moved on (stopping) keeps its last reading.
                if !matches!(instance.state, InstanceState::Ready | InstanceState::Parked) {
                    continue;
                }
                instance.measured_mem_mb = used.mem_mb;
                instance.measured_gpu_mb = used.gpu_mb;
                // Against what the instance holds now: a parked one holds its
                // weights in host memory, and that is what is reserved.
                let held_mem = instance.reservation.mem_mb;
                let held_gpu = instance.reservation.gpu.map_or(0, |(_, mb)| mb);
                let mem_over = used
                    .mem_mb
                    .is_some_and(|m| measure::is_over_budget(held_mem, m));
                let gpu_over = used
                    .gpu_mb
                    .is_some_and(|m| measure::is_over_budget(held_gpu, m));
                if (mem_over || gpu_over) && !instance.over_budget_reported {
                    instance.over_budget_reported = true;
                    let detail = serde_json::json!({
                        "declared_mem_mb": held_mem,
                        "measured_mem_mb": used.mem_mb,
                        "declared_gpu_mb": held_gpu,
                        "measured_gpu_mb": used.gpu_mb,
                    });
                    tracing::warn!(
                        model = %instance.spec.id,
                        instance = %id,
                        %detail,
                        "model server uses more than its spec declares; reservations are unchanged"
                    );
                    over_budget.push((instance.spec.id.clone(), *id, detail));
                }
            }

            for (id, model, exit) in &crashed {
                tracing::error!(%model, instance = %id, %exit, "model server exited unexpectedly");
                state
                    .failures
                    .insert(model.clone(), (Instant::now(), format!("server {exit}")));
            }
            for (id, _, _) in &crashed {
                if let Some(instance) = state.instances.remove(id) {
                    state.pool.release(&instance.reservation);
                    if let Some(port) = instance.port {
                        state.ports.remove(&port);
                    }
                    // Leases are left alone: their requests fail on their
                    // own, and a failure counts as an attempt where a
                    // preemption would not, so a model that always crashes
                    // cannot loop a job forever.
                    self.inner.files.unpin(&instance.spec.files);
                }
            }

            let mut ready_counts: HashMap<String, usize> = HashMap::new();
            for i in state.instances.values() {
                if matches!(
                    i.state,
                    InstanceState::Ready | InstanceState::Loading | InstanceState::Waking
                ) {
                    *ready_counts.entry(i.spec.id.clone()).or_default() += 1;
                }
            }

            let mut to_park = Vec::new();
            let mut to_stop = Vec::new();
            for i in state.instances.values() {
                if i.spec.pinned || !i.idle() || i.sessions > 0 {
                    continue;
                }
                let idle = i.last_used.elapsed();
                match i.state {
                    InstanceState::Ready
                        if i.spec.idle_timeout_secs > 0
                            && idle >= Duration::from_secs(i.spec.idle_timeout_secs)
                            && ready_counts.get(&i.spec.id).copied().unwrap_or(0)
                                > i.spec.min_warm
                            && !state.demand.contains_key(&i.spec.id) =>
                    {
                        *ready_counts.get_mut(&i.spec.id).expect("counted") -= 1;
                        if i.spec.can_park() {
                            to_park.push(i.id);
                        } else {
                            to_stop.push(i.id);
                        }
                    }
                    InstanceState::Parked
                        if idle >= Duration::from_secs(i.spec.park_timeout_secs) =>
                    {
                        to_stop.push(i.id);
                    }
                    _ => {}
                }
            }
            for id in to_park {
                self.start_park(&mut state, id);
            }
            for id in to_stop {
                self.start_stop(&mut state, id, false);
            }

            for spec in self.inner.specs.values() {
                let have = ready_counts.get(&spec.id).copied().unwrap_or(0);
                if spec.min_warm > have || (spec.pinned && have == 0) {
                    warm_up.push(spec.id.clone());
                }
            }
            // Preload for queued work: start loading before a worker claims
            // the job, using only free capacity.
            for model in state.demand.keys() {
                if ready_counts.get(model).copied().unwrap_or(0) == 0
                    && self.inner.specs.contains_key(model)
                {
                    warm_up.push(model.clone());
                }
            }
        }
        for (model, id, detail) in over_budget {
            self.event(&model, Some(id), "over_budget", detail);
        }
        if !crashed.is_empty() {
            for (id, model, exit) in crashed {
                self.event(
                    &model,
                    Some(id),
                    "crashed",
                    serde_json::json!({ "exit": exit }),
                );
            }
            self.inner.changed.notify_waiters();
        }

        warm_up.sort();
        warm_up.dedup();
        for model in warm_up {
            self.preload_in_background(&model, Priority::Low, false);
        }
    }

    /// Load a model without holding it, in the background.
    pub fn preload_in_background(&self, model_id: &str, priority: Priority, may_evict: bool) {
        let this = self.clone();
        let model = model_id.to_string();
        tokio::spawn(async move {
            let Ok(spec) = this.spec(&model) else { return };
            let mut opts =
                AcquireOptions::new(priority, Duration::from_secs(spec.load_timeout_secs + 60));
            opts.may_evict = may_evict;
            opts.may_preempt = false;
            match this.acquire(&model, opts).await {
                Ok(lease) => drop(lease),
                Err(AiError::Cancelled) => {}
                Err(error) => tracing::debug!(%model, %error, "preload did not complete"),
            }
        });
    }

    /// Unload every idle instance of a model, pinned or not. Returns how many
    /// were stopped and how many are busy.
    pub fn unload(&self, model_id: &str) -> AiResult<(usize, usize)> {
        self.spec(model_id)?;
        let mut state = self.inner.state.lock();
        let (idle, busy): (Vec<_>, Vec<_>) = state
            .instances
            .values()
            .filter(|i| i.spec.id == model_id && i.is_live())
            .partition(|i| i.idle() && i.sessions == 0 && i.state != InstanceState::Loading);
        let idle: Vec<Uuid> = idle.into_iter().map(|i| i.id).collect();
        let busy = busy.len();
        for id in &idle {
            self.start_stop(&mut state, *id, false);
        }
        Ok((idle.len(), busy))
    }

    /// The latest queue demand, which steers eviction and preloading.
    pub fn set_demand(&self, demand: Vec<ModelDemand>) {
        let mut state = self.inner.state.lock();
        state.demand = demand
            .into_iter()
            .map(|d| (d.model_id.clone(), d))
            .collect();
    }

    /// Whether a superlow job for this model could run now without
    /// disturbing anything: a ready instance has a free slot, or there is room
    /// to load one.
    pub fn has_spare_capacity(&self, model_id: &str) -> bool {
        let Some(spec) = self.inner.specs.get(model_id) else {
            return false;
        };
        let state = self.inner.state.lock();
        let instances: Vec<&Instance> = state
            .instances
            .values()
            .filter(|i| i.spec.id == model_id && i.is_live())
            .collect();
        if instances.iter().any(|i| i.has_capacity()) {
            return true;
        }
        if instances
            .iter()
            .any(|i| matches!(i.state, InstanceState::Loading | InstanceState::Waking))
        {
            return false;
        }
        let total_live = state.instances.values().filter(|i| i.is_live()).count();
        instances.len() < spec.max_replicas
            && total_live < self.inner.config.max_instances
            && state
                .pool
                .shortfall(Need {
                    mem_mb: spec.memory_mb,
                    gpu_mb: spec.gpu_mb,
                })
                .is_zero()
    }

    /// Models a superlow job could run on now.
    pub fn spare_models(&self) -> Vec<String> {
        self.inner
            .specs
            .keys()
            .filter(|m| self.has_spare_capacity(m))
            .cloned()
            .collect()
    }

    /// Count a session lock against an instance, protecting it from eviction.
    pub fn bind_session(&self, instance_id: Uuid) -> bool {
        let mut state = self.inner.state.lock();
        match state
            .instances
            .get_mut(&instance_id)
            .filter(|i| i.is_live())
        {
            Some(instance) => {
                instance.sessions += 1;
                true
            }
            None => false,
        }
    }

    /// Release a session lock.
    pub fn unbind_session(&self, instance_id: Uuid) {
        let mut state = self.inner.state.lock();
        if let Some(instance) = state.instances.get_mut(&instance_id) {
            instance.sessions = instance.sessions.saturating_sub(1);
            instance.last_used = Instant::now();
        }
    }

    /// Sessions an instance holds, or `None` if it is gone.
    pub fn session_count(&self, instance_id: Uuid) -> Option<usize> {
        let state = self.inner.state.lock();
        state
            .instances
            .get(&instance_id)
            .filter(|i| i.is_live())
            .map(|i| i.sessions)
    }

    /// All instances.
    pub fn instances(&self) -> Vec<InstanceInfo> {
        let state = self.inner.state.lock();
        let mut out: Vec<InstanceInfo> = state
            .instances
            .values()
            .map(|i| InstanceInfo {
                id: i.id,
                model_id: i.spec.id.clone(),
                state: i.state,
                base_url: i.base_url.clone(),
                in_flight: i.leases.len(),
                in_flight_superlow: i
                    .leases
                    .values()
                    .filter(|l| l.priority.is_superlow())
                    .count(),
                sessions: i.sessions,
                idle_secs: i.last_used.elapsed().as_secs(),
                reservation: i.reservation,
                load_secs: i.load_duration.as_secs_f64(),
                measured_mem_mb: i.measured_mem_mb,
                measured_gpu_mb: i.measured_gpu_mb,
            })
            .collect();
        out.sort_by(|a, b| a.model_id.cmp(&b.model_id).then(a.id.cmp(&b.id)));
        out
    }

    /// Model ids with a ready instance.
    pub fn ready_models(&self) -> Vec<String> {
        let state = self.inner.state.lock();
        let mut models: Vec<String> = state
            .instances
            .values()
            .filter(|i| i.state == InstanceState::Ready)
            .map(|i| i.spec.id.clone())
            .collect();
        models.sort();
        models.dedup();
        models
    }

    /// The pool, for the API.
    pub fn resources(&self) -> ResourcePool {
        self.inner.state.lock().pool.clone()
    }

    /// Stop everything this node started.
    pub async fn shutdown(&self) {
        self.inner.shutdown.cancel();
        let ids: Vec<Uuid> = self.inner.state.lock().instances.keys().copied().collect();
        {
            let mut state = self.inner.state.lock();
            for id in &ids {
                self.start_stop(&mut state, *id, false);
            }
        }
        let deadline = Instant::now() + STOP_GRACE + Duration::from_secs(10);
        while Instant::now() < deadline && !self.inner.state.lock().instances.is_empty() {
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    fn event(
        &self,
        model_id: &str,
        instance_id: Option<Uuid>,
        event: &str,
        detail: serde_json::Value,
    ) {
        let Some(pool) = self.inner.events.clone() else {
            return;
        };
        let node = self.inner.node_id.clone();
        let model = model_id.to_string();
        let event = event.to_string();
        tokio::spawn(async move {
            let result = sqlx::query(
                "INSERT INTO ai.model_events (node_id, model_id, instance_id, event, detail)
                 VALUES ($1, $2, $3, $4, $5)",
            )
            .bind(node)
            .bind(model)
            .bind(instance_id)
            .bind(&event)
            .bind(if detail.is_null() {
                serde_json::json!({})
            } else {
                detail
            })
            .execute(&pool)
            .await;
            if let Err(error) = result {
                tracing::debug!(%error, %event, "could not record model event");
            }
        });
    }
}

#[cfg(test)]
mod tests;
