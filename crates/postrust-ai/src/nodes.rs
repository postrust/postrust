//! Node registry: who is up, where to reach them, and what they have loaded.
//! Used to forward session-locked requests to their owner and to send
//! requests to a node that already has the model ready.

use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use sqlx::{PgPool, Row};

use crate::error::AiResult;

/// A node is alive if it heartbeated within this long.
pub const ALIVE_WITHIN: Duration = Duration::from_secs(30);

/// A peer.
#[derive(Debug, Clone, Serialize)]
#[non_exhaustive]
pub struct NodeInfo {
    /// Id.
    pub node_id: String,
    /// URL.
    pub base_url: String,
    /// Models with a ready instance.
    pub ready_models: Vec<String>,
    /// Budget and use.
    pub resources: Value,
    /// Seconds since its last heartbeat.
    pub last_seen_secs: f64,
}

/// The registry.
#[derive(Clone)]
pub struct Nodes {
    pool: PgPool,
    node_id: String,
}

impl Nodes {
    /// The registry, as seen from this node.
    pub fn new(pool: PgPool, node_id: String) -> Self {
        Self { pool, node_id }
    }

    /// Announce this node.
    pub async fn heartbeat(
        &self,
        base_url: &str,
        resources: &Value,
        ready_models: &[String],
    ) -> AiResult<()> {
        sqlx::query(
            "INSERT INTO ai.nodes (node_id, base_url, resources, ready_models, last_seen)
             VALUES ($1, $2, $3, $4, now())
             ON CONFLICT (node_id) DO UPDATE SET
                base_url = EXCLUDED.base_url,
                resources = EXCLUDED.resources,
                ready_models = EXCLUDED.ready_models,
                last_seen = now()",
        )
        .bind(&self.node_id)
        .bind(base_url)
        .bind(resources)
        .bind(ready_models)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Leave the registry.
    pub async fn deregister(&self) -> AiResult<()> {
        sqlx::query("DELETE FROM ai.nodes WHERE node_id = $1")
            .bind(&self.node_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// A live peer's URL.
    pub async fn url_of(&self, node_id: &str) -> AiResult<Option<String>> {
        Ok(sqlx::query_scalar(
            "SELECT base_url FROM ai.nodes
             WHERE node_id = $1 AND last_seen > now() - make_interval(secs => $2)",
        )
        .bind(node_id)
        .bind(ALIVE_WITHIN.as_secs_f64())
        .fetch_optional(&self.pool)
        .await?)
    }

    /// Live peers with the model ready, most recently seen first.
    pub async fn peers_with(&self, model_id: &str) -> AiResult<Vec<(String, String)>> {
        let rows = sqlx::query(
            "SELECT node_id, base_url FROM ai.nodes
             WHERE node_id <> $1 AND $2 = ANY(ready_models)
               AND last_seen > now() - make_interval(secs => $3)
             ORDER BY last_seen DESC",
        )
        .bind(&self.node_id)
        .bind(model_id)
        .bind(ALIVE_WITHIN.as_secs_f64())
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|r| Ok((r.try_get("node_id")?, r.try_get("base_url")?)))
            .collect()
    }

    /// Every node, live or not.
    pub async fn list(&self) -> AiResult<Vec<NodeInfo>> {
        let rows = sqlx::query(
            "SELECT node_id, base_url, ready_models, resources,
                    extract(epoch FROM now() - last_seen)::float8 AS age
             FROM ai.nodes ORDER BY node_id",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|r| {
                Ok(NodeInfo {
                    node_id: r.try_get("node_id")?,
                    base_url: r.try_get("base_url")?,
                    ready_models: r.try_get("ready_models")?,
                    resources: r.try_get("resources")?,
                    last_seen_secs: r.try_get("age")?,
                })
            })
            .collect()
    }
}
