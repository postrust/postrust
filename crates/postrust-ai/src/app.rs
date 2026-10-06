//! Putting a node together: connect, migrate, build every component, start
//! the background loops, serve, and shut down cleanly.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use tokio_util::sync::CancellationToken;

use crate::api::{self, AppState};
use crate::cache::{FileCache, ResultCache};
use crate::config::AiConfig;
use crate::error::{AiError, AiResult};
use crate::executor::{Executor, JobHandler};
use crate::manager::ModelManager;
use crate::metrics::Metrics;
use crate::nodes::Nodes;
use crate::proxy::ProxyClient;
use crate::queue::{JobStore, PgJobStore};
use crate::resources::Probe;
use crate::scheduler::{self, Scheduler};
use crate::session::Sessions;
use crate::workflow::WorkflowEngine;

/// How often slower housekeeping runs: lease reaping, sessions, the node
/// heartbeat, workflow reconciliation.
const HOUSEKEEPING: Duration = Duration::from_secs(10);
/// How often finished jobs are archived and expired results purged.
const ARCHIVING: Duration = Duration::from_secs(3600);

/// A built node, ready to [`serve`](Node::serve).
pub struct Node {
    config: Arc<AiConfig>,
    state: AppState,
    executor: Arc<Executor>,
    shutdown: CancellationToken,
}

impl Node {
    /// Connect, migrate and build everything. `handlers` replaces the
    /// built-in HTTP handler for the named queues.
    pub async fn build(
        config: AiConfig,
        handlers: Vec<(String, Arc<dyn JobHandler>)>,
    ) -> AiResult<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(config.database.max_connections.max(3))
            .acquire_timeout(Duration::from_secs(10))
            .connect(&config.database.url)
            .await?;
        Self::build_with_pool(config, pool, handlers).await
    }

    /// Like [`build`](Self::build), on a pool the caller already holds: for
    /// embedding the node in a larger application, and for tests that run
    /// several nodes against one database.
    pub async fn build_with_pool(
        config: AiConfig,
        pool: PgPool,
        handlers: Vec<(String, Arc<dyn JobHandler>)>,
    ) -> AiResult<Self> {
        let config = Arc::new(config);

        let pg = PgJobStore::new(pool.clone());
        if config.database.migrate {
            pg.migrate().await?;
        }
        let store: Arc<dyn JobStore> = Arc::new(pg);
        for queue in &config.queues {
            store.upsert_queue(queue).await?;
        }

        // Model calls can be long; the manager and executor set per-request
        // timeouts where they need them.
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .build()?;
        let forward_http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .build()?;

        let files = Arc::new(FileCache::open(&config.cache, http.clone()).await?);
        let probe = Probe::read().await;
        let manager = ModelManager::new(
            config.node.id.clone(),
            &config.models,
            config.resources.clone(),
            probe,
            http.clone(),
            files.clone(),
            Some(pool.clone()),
        );
        let metrics = Arc::new(Metrics::default());
        let results = ResultCache::new(
            pool.clone(),
            Duration::from_secs(config.cache.result_ttl_secs),
        );

        let mut executor = Executor::new(
            config.node.id.clone(),
            store.clone(),
            manager.clone(),
            Some(results.clone()),
            http.clone(),
            Duration::from_secs(config.scheduler.lease_secs),
            Duration::from_secs(config.scheduler.heartbeat_secs),
            metrics.clone(),
        );
        for (queue, handler) in handlers {
            if config.queue(&queue).is_none() {
                return Err(AiError::Config(format!(
                    "handler for unknown queue '{queue}'"
                )));
            }
            executor = executor.with_handler(queue, handler);
        }
        let executor = Arc::new(executor);

        let known: HashSet<String> = config.models.iter().map(|m| m.id.clone()).collect();
        let workflows = WorkflowEngine::new(pool.clone(), store.clone(), known);
        let scheduler = Arc::new(Scheduler::new(
            config.node.id.clone(),
            store.clone(),
            manager.clone(),
            executor.clone(),
            Some(workflows.clone()),
            config.queues.clone(),
            config.scheduler.clone(),
        ));
        let sessions = Arc::new(Sessions::new(
            pool.clone(),
            config.node.id.clone(),
            Duration::from_secs(config.sessions.ttl_secs),
            manager.clone(),
        ));
        let cleared = sessions.clear_own().await?;
        if cleared > 0 {
            tracing::info!(
                cleared,
                "dropped session locks from this node's previous run"
            );
        }
        let nodes = Nodes::new(pool.clone(), config.node.id.clone());

        let state = AppState {
            config: config.clone(),
            store,
            manager,
            sessions,
            nodes,
            workflows,
            results,
            files,
            metrics,
            scheduler,
            forward_http,
            pool,
        };
        Ok(Self {
            config,
            state,
            executor,
            shutdown: CancellationToken::new(),
        })
    }

    /// The shared state, e.g. to mount the API inside another router.
    pub fn state(&self) -> &AppState {
        &self.state
    }

    /// A token that stops the node when cancelled.
    pub fn shutdown_token(&self) -> CancellationToken {
        self.shutdown.clone()
    }

    /// Run until the shutdown token is cancelled.
    pub async fn serve(self) -> AiResult<()> {
        let s = &self.state;
        let shutdown = self.shutdown.clone();

        let tasks = vec![
            tokio::spawn(s.scheduler.clone().run(shutdown.clone())),
            tokio::spawn(scheduler::listen(
                s.pool.clone(),
                s.scheduler.clone(),
                self.executor.clone(),
                shutdown.clone(),
            )),
            tokio::spawn(maintenance(s.clone(), shutdown.clone())),
        ];

        // Announce before registering with the proxy, so peers can find this
        // node the moment traffic arrives.
        let base_url = self.config.node.base_url();
        heartbeat(s, &base_url).await;
        let registration = match &self.config.proxy {
            Some(proxy) => {
                let client = ProxyClient::new(proxy.clone(), reqwest::Client::new());
                match client.register(&base_url).await {
                    Ok(r) => Some((client, r)),
                    Err(error) => {
                        tracing::error!(%error, "could not register with postrust-proxy; serving without it");
                        None
                    }
                }
            }
            None => None,
        };

        let listener = tokio::net::TcpListener::bind(self.config.node.listen).await?;
        tracing::info!(
            node = %self.config.node.id,
            listen = %self.config.node.listen,
            models = self.config.models.len(),
            queues = self.config.queues.len(),
            "postrust-ai serving"
        );
        let app = api::router(s.clone());
        let server_shutdown = shutdown.clone();
        axum::serve(listener, app)
            .with_graceful_shutdown(async move { server_shutdown.cancelled().await })
            .await?;

        tracing::info!("shutting down");
        if let Some((client, registration)) = registration {
            if let Err(error) = client.deregister(&registration).await {
                tracing::warn!(%error, "could not deregister from postrust-proxy");
            }
        }
        for task in tasks {
            let _ = task.await;
        }
        s.manager.shutdown().await;
        if let Err(error) = s.nodes.deregister().await {
            tracing::warn!(%error, "could not leave the node registry");
        }
        Ok(())
    }
}

async fn heartbeat(s: &AppState, base_url: &str) {
    let pool = s.manager.resources();
    let resources = json!({
        "memory_mb": { "budget": pool.mem_budget_mb, "reserved": pool.mem_reserved_mb, "free": pool.mem_free() },
        "gpus": pool.gpus.iter().map(|g| json!({
            "index": g.index, "budget_mb": g.budget_mb, "reserved_mb": g.reserved_mb, "free_mb": pool.gpu_free(g),
        })).collect::<Vec<_>>(),
    });
    if let Err(error) = s
        .nodes
        .heartbeat(base_url, &resources, &s.manager.ready_models())
        .await
    {
        tracing::warn!(%error, "node heartbeat failed");
    }
}

/// Every background chore, on its own cadence.
async fn maintenance(s: AppState, shutdown: CancellationToken) {
    let tick = Duration::from_secs(s.config.scheduler.maintenance_secs.max(1));
    let base_url = s.config.node.base_url();
    let mut last_housekeeping = Instant::now() - HOUSEKEEPING;
    let mut last_archive = Instant::now();
    loop {
        tokio::select! {
            _ = tokio::time::sleep(tick) => {}
            _ = shutdown.cancelled() => return,
        }

        match s.store.demand().await {
            Ok(demand) => s.manager.set_demand(demand),
            Err(error) => tracing::warn!(%error, "could not read queue demand"),
        }
        s.manager.maintain().await;

        if last_housekeeping.elapsed() >= HOUSEKEEPING {
            last_housekeeping = Instant::now();
            match s.store.reap_expired().await {
                Ok(reaped) if !reaped.is_empty() => {
                    tracing::warn!(jobs = ?reaped, "requeued jobs whose lease expired");
                    s.scheduler.wake();
                }
                Ok(_) => {}
                Err(error) => tracing::warn!(%error, "lease reaper failed"),
            }
            if let Err(error) = s.sessions.maintain().await {
                tracing::warn!(%error, "session maintenance failed");
            }
            heartbeat(&s, &base_url).await;
            if let Err(error) = s.workflows.reconcile().await {
                tracing::warn!(%error, "workflow reconcile failed");
            }
        }

        if last_archive.elapsed() >= ARCHIVING {
            last_archive = Instant::now();
            let age = Duration::from_secs(s.config.scheduler.archive_after_secs);
            match s.store.archive(age).await {
                Ok(n) if n > 0 => tracing::info!(jobs = n, "archived finished jobs"),
                Ok(_) => {}
                Err(error) => tracing::warn!(%error, "archiving failed"),
            }
            if let Err(error) = s.results.purge_expired().await {
                tracing::warn!(%error, "result cache purge failed");
            }
        }
    }
}
