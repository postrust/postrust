//! Session locks.
//!
//! A session lock binds `(session id, model)` to one instance on one node.
//! While it lives:
//!
//! - every request carrying the session id goes to that instance, forwarded
//!   from whichever node the proxy happened to pick, so the server's prompt and
//!   KV caches stay warm across a conversation;
//! - the instance is not unloaded, except to make room for `critical` work.
//!
//! Locks expire `ttl_secs` after their last request, or are released
//! explicitly. If the instance goes away anyway -- a crash, a critical
//! eviction -- the next request binds the session to a new instance and the
//! response says so in `x-session-rebound: true`, so the client knows its
//! cache is cold.
//!
//! The table is the source of truth across nodes; the manager keeps a count
//! per instance only for the eviction rule, on the node that owns it.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::Serialize;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::AiResult;
use crate::manager::ModelManager;

/// Where a session is bound.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct SessionLock {
    /// Session id.
    pub session_id: String,
    /// Model.
    pub model_id: String,
    /// Owning node.
    pub node_id: String,
    /// Instance on that node.
    pub instance_id: Uuid,
}

/// Session lock storage for one node.
pub struct Sessions {
    pool: PgPool,
    node_id: String,
    ttl: Duration,
    manager: ModelManager,
    /// When each local lock was last extended, so a busy session costs one
    /// write per half-TTL instead of one per request.
    touched: Mutex<HashMap<(String, String), Instant>>,
}

impl Sessions {
    /// Session storage for this node.
    pub fn new(pool: PgPool, node_id: String, ttl: Duration, manager: ModelManager) -> Self {
        Self {
            pool,
            node_id,
            ttl,
            manager,
            touched: Mutex::new(HashMap::new()),
        }
    }

    /// This node's id.
    pub fn node_id(&self) -> &str {
        &self.node_id
    }

    /// Drop locks this node held before a restart: their instances are gone.
    pub async fn clear_own(&self) -> AiResult<u64> {
        let done = sqlx::query("DELETE FROM ai.sessions WHERE node_id = $1")
            .bind(&self.node_id)
            .execute(&self.pool)
            .await?;
        Ok(done.rows_affected())
    }

    /// The live lock for this session and model.
    pub async fn lookup(&self, session_id: &str, model_id: &str) -> AiResult<Option<SessionLock>> {
        let row = sqlx::query(
            "SELECT node_id, instance_id FROM ai.sessions
             WHERE session_id = $1 AND model_id = $2 AND expires_at > now()",
        )
        .bind(session_id)
        .bind(model_id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(|r| {
            Ok(SessionLock {
                session_id: session_id.into(),
                model_id: model_id.into(),
                node_id: r.try_get("node_id")?,
                instance_id: r.try_get("instance_id")?,
            })
        })
        .transpose()
    }

    /// Bind a session to a local instance. Takes over an expired lock, or one
    /// pointing at `replacing` (an instance known to be gone). If another node
    /// won the race, returns its lock instead, and the caller should use that.
    pub async fn bind(
        &self,
        session_id: &str,
        model_id: &str,
        instance_id: Uuid,
        replacing: Option<Uuid>,
    ) -> AiResult<SessionLock> {
        let row = sqlx::query(
            "INSERT INTO ai.sessions (session_id, model_id, node_id, instance_id, expires_at)
             VALUES ($1, $2, $3, $4, now() + make_interval(secs => $5))
             ON CONFLICT (session_id, model_id) DO UPDATE SET
                node_id = EXCLUDED.node_id,
                instance_id = EXCLUDED.instance_id,
                expires_at = EXCLUDED.expires_at,
                created_at = now()
             WHERE ai.sessions.expires_at <= now()
                OR ai.sessions.instance_id = $6
             RETURNING node_id, instance_id",
        )
        .bind(session_id)
        .bind(model_id)
        .bind(&self.node_id)
        .bind(instance_id)
        .bind(self.ttl.as_secs_f64())
        .bind(replacing)
        .fetch_optional(&self.pool)
        .await?;

        if row.is_some() {
            if let Some(old) = replacing {
                if self.manager.session_count(old).is_some() {
                    self.manager.unbind_session(old);
                }
            }
            self.manager.bind_session(instance_id);
            self.touched
                .lock()
                .insert((session_id.into(), model_id.into()), Instant::now());
            return Ok(SessionLock {
                session_id: session_id.into(),
                model_id: model_id.into(),
                node_id: self.node_id.clone(),
                instance_id,
            });
        }
        // Someone else holds a live lock.
        match self.lookup(session_id, model_id).await? {
            Some(lock) => Ok(lock),
            // It expired between the insert and the read; try once more.
            None => Box::pin(self.bind(session_id, model_id, instance_id, replacing)).await,
        }
    }

    /// Extend a lock after a request, at most once per half-TTL.
    pub async fn touch(&self, lock: &SessionLock) -> AiResult<()> {
        let key = (lock.session_id.clone(), lock.model_id.clone());
        {
            let touched = self.touched.lock();
            if touched
                .get(&key)
                .is_some_and(|t| t.elapsed() < self.ttl / 2)
            {
                return Ok(());
            }
        }
        sqlx::query(
            "UPDATE ai.sessions SET expires_at = now() + make_interval(secs => $4)
             WHERE session_id = $1 AND model_id = $2 AND node_id = $3",
        )
        .bind(&lock.session_id)
        .bind(&lock.model_id)
        .bind(&self.node_id)
        .bind(self.ttl.as_secs_f64())
        .execute(&self.pool)
        .await?;
        self.touched.lock().insert(key, Instant::now());
        Ok(())
    }

    /// Release a lock. Returns whether there was one.
    pub async fn release(&self, session_id: &str, model_id: &str) -> AiResult<bool> {
        let row = sqlx::query(
            "DELETE FROM ai.sessions WHERE session_id = $1 AND model_id = $2
             RETURNING node_id, instance_id",
        )
        .bind(session_id)
        .bind(model_id)
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else { return Ok(false) };
        // Only the owner counted it. A release that lands on another node
        // leaves the owner's count until its reaper notices the row is gone.
        if row.try_get::<String, _>("node_id")? == self.node_id {
            self.manager.unbind_session(row.try_get("instance_id")?);
        }
        self.touched
            .lock()
            .remove(&(session_id.to_string(), model_id.to_string()));
        Ok(true)
    }

    /// Expire this node's lapsed locks and resynchronise the manager's
    /// counts with the table (which a release on another node changes).
    pub async fn maintain(&self) -> AiResult<()> {
        sqlx::query("DELETE FROM ai.sessions WHERE node_id = $1 AND expires_at <= now()")
            .bind(&self.node_id)
            .execute(&self.pool)
            .await?;
        let rows = sqlx::query(
            "SELECT instance_id, count(*) AS n FROM ai.sessions
             WHERE node_id = $1 GROUP BY instance_id",
        )
        .bind(&self.node_id)
        .fetch_all(&self.pool)
        .await?;
        let mut counts: HashMap<Uuid, usize> = HashMap::new();
        for row in rows {
            counts.insert(
                row.try_get("instance_id")?,
                row.try_get::<i64, _>("n")? as usize,
            );
        }
        for instance in self.manager.instances() {
            let want = counts.get(&instance.id).copied().unwrap_or(0);
            let mut have = instance.sessions;
            while have > want {
                self.manager.unbind_session(instance.id);
                have -= 1;
            }
            while have < want {
                self.manager.bind_session(instance.id);
                have += 1;
            }
        }
        let ttl = self.ttl;
        self.touched.lock().retain(|_, t| t.elapsed() < ttl);
        Ok(())
    }

    /// This node's live locks.
    pub async fn list(&self) -> AiResult<Vec<SessionLock>> {
        let rows = sqlx::query(
            "SELECT session_id, model_id, node_id, instance_id FROM ai.sessions
             WHERE node_id = $1 AND expires_at > now() ORDER BY created_at",
        )
        .bind(&self.node_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|r| {
                Ok(SessionLock {
                    session_id: r.try_get("session_id")?,
                    model_id: r.try_get("model_id")?,
                    node_id: r.try_get("node_id")?,
                    instance_id: r.try_get("instance_id")?,
                })
            })
            .collect()
    }
}
