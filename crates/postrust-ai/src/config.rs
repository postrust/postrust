//! Configuration: a TOML file, with a few settings overridable from the
//! environment so a container can be pointed at its database and address
//! without templating the file.
//!
//! Every struct here is `#[non_exhaustive]`, as in postrust 2.0: a setting
//! added later is a minor release, not a breaking one. Build them from TOML,
//! or from `Default` and field assignment.

use std::collections::HashSet;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{AiError, AiResult};
use crate::model::ModelSpec;
use crate::queue::QueueSpec;

/// The whole configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
#[non_exhaustive]
pub struct AiConfig {
    /// This node's identity and address.
    pub node: NodeConfig,
    /// Where the queue lives.
    pub database: DatabaseConfig,
    /// The memory and GPU budget models are loaded into.
    pub resources: ResourceConfig,
    /// The on-disk model file cache.
    pub cache: CacheConfig,
    /// Session locks.
    pub sessions: SessionConfig,
    /// The synchronous inference gateway.
    pub gateway: GatewayConfig,
    /// Worker and background-task timings.
    pub scheduler: SchedulerConfig,
    /// Registration with postrust-proxy. Absent: no registration.
    pub proxy: Option<ProxyRegistration>,
    /// API authentication.
    pub auth: AuthConfig,
    /// Job queues. A `default` queue is added if none are declared.
    #[serde(rename = "queue")]
    pub queues: Vec<QueueSpec>,
    /// Models this node can load.
    #[serde(rename = "model")]
    pub models: Vec<ModelSpec>,
}

/// This node.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
#[non_exhaustive]
pub struct NodeConfig {
    /// Unique across the cluster. Defaults to the hostname.
    pub id: String,
    /// Address the API listens on.
    pub listen: SocketAddr,
    /// URL other nodes and the proxy use to reach this one. Defaults to
    /// `http://{listen}`, which is only right when `listen` is not 0.0.0.0.
    pub advertise_url: Option<String>,
}

impl Default for NodeConfig {
    fn default() -> Self {
        Self {
            id: default_node_id(),
            listen: SocketAddr::from(([127, 0, 0, 1], 7700)),
            advertise_url: None,
        }
    }
}

impl NodeConfig {
    /// The URL to advertise, falling back to the listen address.
    pub fn base_url(&self) -> String {
        self.advertise_url
            .clone()
            .unwrap_or_else(|| format!("http://{}", self.listen))
            .trim_end_matches('/')
            .to_string()
    }
}

fn default_node_id() -> String {
    std::env::var("HOSTNAME")
        .ok()
        .filter(|h| !h.is_empty())
        .or_else(|| {
            std::fs::read_to_string("/etc/hostname")
                .ok()
                .map(|h| h.trim().to_string())
                .filter(|h| !h.is_empty())
        })
        .unwrap_or_else(|| "postrust-ai".to_string())
}

/// Database connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
#[non_exhaustive]
pub struct DatabaseConfig {
    /// Connection string. `DATABASE_URL` overrides it.
    pub url: String,
    /// Pool size. One more connection is held for LISTEN.
    pub max_connections: u32,
    /// Apply the bundled migration at startup.
    pub migrate: bool,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            url: "postgres://postgres:postgres@localhost:5432/postgres".into(),
            max_connections: 10,
            migrate: true,
        }
    }
}

/// The budget models are loaded into.
///
/// Accounting is by declaration: each model states what it needs, and the
/// manager keeps the sum of loaded models under the budget. The probe is a
/// second check, so memory taken by something else on the machine is noticed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
#[non_exhaustive]
pub struct ResourceConfig {
    /// Host memory budget in MiB. Absent: detected total minus the reserve.
    pub memory_mb: Option<u64>,
    /// Host memory kept back for the OS and this process.
    pub memory_reserve_mb: u64,
    /// GPUs to use. Absent: detected with `nvidia-smi`, if it is there.
    pub gpus: Option<Vec<GpuConfig>>,
    /// VRAM kept back on each GPU for the CUDA context and fragmentation.
    pub gpu_reserve_mb: u64,
    /// Also require the probed free memory to cover a load, not just the
    /// accounting. Catches memory used by processes this node does not manage.
    pub respect_probe: bool,
    /// Upper bound on concurrently loaded instances, whatever the budget says.
    pub max_instances: usize,
    /// Ports handed to model servers.
    pub port_range: (u16, u16),
}

impl Default for ResourceConfig {
    fn default() -> Self {
        Self {
            memory_mb: None,
            memory_reserve_mb: 2048,
            gpus: None,
            gpu_reserve_mb: 512,
            respect_probe: true,
            max_instances: 16,
            port_range: (18000, 18999),
        }
    }
}

/// One GPU's budget.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct GpuConfig {
    /// CUDA device index.
    pub index: u32,
    /// VRAM budget in MiB.
    pub vram_mb: u64,
}

impl GpuConfig {
    /// A GPU budget.
    pub fn new(index: u32, vram_mb: u64) -> Self {
        Self { index, vram_mb }
    }
}

/// The model file cache.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
#[non_exhaustive]
pub struct CacheConfig {
    /// Where model files are kept.
    pub dir: PathBuf,
    /// Disk budget in GiB. Least recently used files go first.
    pub max_disk_gb: u64,
    /// The directory is shared between nodes (NFS, EFS). Downloads take a lock
    /// file so two nodes do not fetch the same file at once.
    pub shared: bool,
    /// Default lifetime of a cached result, in seconds. 0: no expiry.
    pub result_ttl_secs: u64,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            dir: PathBuf::from("/var/cache/postrust-ai"),
            max_disk_gb: 100,
            shared: false,
            result_ttl_secs: 7 * 24 * 3600,
        }
    }
}

/// Session locks.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
#[non_exhaustive]
pub struct SessionConfig {
    /// Header carrying the session id.
    pub header: String,
    /// How long a lock lives after its last request, in seconds.
    pub ttl_secs: u64,
    /// Sessions one instance takes before new ones go to another replica.
    pub max_per_instance: usize,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            header: "x-session-id".into(),
            ttl_secs: 600,
            max_per_instance: 64,
        }
    }
}

/// The synchronous gateway.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
#[non_exhaustive]
pub struct GatewayConfig {
    /// How long a request waits for a model to load before getting a 503
    /// with `Retry-After`. The load carries on regardless.
    pub load_wait_secs: u64,
    /// Upper bound on one forwarded request, in seconds.
    pub request_timeout_secs: u64,
    /// Forward to another node that already has the model loaded, rather than
    /// loading it here.
    pub forward_to_ready_nodes: bool,
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            load_wait_secs: 30,
            request_timeout_secs: 600,
            forward_to_ready_nodes: true,
        }
    }
}

/// Worker and background timings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
#[non_exhaustive]
pub struct SchedulerConfig {
    /// How long a claimed job's lease lasts without a heartbeat.
    pub lease_secs: u64,
    /// Heartbeat interval. Every heartbeat is a row update, so not too often:
    /// this is the main source of table bloat.
    pub heartbeat_secs: u64,
    /// Backup poll when no NOTIFY arrives.
    pub poll_secs: u64,
    /// Waiting jobs gain one priority point per this many seconds, up to
    /// `aging_cap`, so low work is not starved. Superlow never ages.
    pub aging_secs: u64,
    /// Most priority a job can gain by waiting.
    pub aging_cap: i32,
    /// Move finished jobs to the archive after this many seconds.
    pub archive_after_secs: u64,
    /// How often the idle reaper and preloader run.
    pub maintenance_secs: u64,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            lease_secs: 60,
            heartbeat_secs: 15,
            poll_secs: 5,
            aging_secs: 60,
            aging_cap: 99,
            archive_after_secs: 24 * 3600,
            maintenance_secs: 5,
        }
    }
}

/// Registration with postrust-proxy's admin API.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
#[non_exhaustive]
pub struct ProxyRegistration {
    /// Base URL of the proxy admin API, e.g. `http://proxy:9000/admin/api`.
    pub admin_url: String,
    /// Upstream every node joins as a backend.
    pub upstream_name: String,
    /// Path prefix routed to that upstream.
    pub route_prefix: String,
    /// Sent as `Authorization: Bearer` if the admin API sits behind auth.
    pub admin_token: Option<String>,
}

impl Default for ProxyRegistration {
    fn default() -> Self {
        Self {
            admin_url: "http://127.0.0.1:9000/admin/api".into(),
            upstream_name: "postrust-ai".into(),
            route_prefix: "/ai/".into(),
            admin_token: None,
        }
    }
}

/// API authentication. With no secret the API is open: put it behind
/// something that authenticates.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
#[non_exhaustive]
pub struct AuthConfig {
    /// HS256 secret. `POSTRUST_AI_JWT_SECRET` overrides it.
    pub jwt_secret: Option<String>,
    /// Required `aud`.
    pub audience: Option<String>,
    /// Roles that may use the admin endpoints (load, unload, queue pause).
    /// Empty: any authenticated caller.
    pub admin_roles: Vec<String>,
}

impl AiConfig {
    /// Read a TOML file, apply environment overrides, and validate.
    pub fn load(path: &Path) -> AiResult<Self> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| AiError::Config(format!("cannot read {}: {e}", path.display())))?;
        Self::from_toml(&text)
    }

    /// Parse TOML text, apply environment overrides, and validate.
    pub fn from_toml(text: &str) -> AiResult<Self> {
        let mut config: AiConfig =
            toml::from_str(text).map_err(|e| AiError::Config(e.to_string()))?;
        config.apply_env();
        config.validate()?;
        Ok(config)
    }

    /// Environment overrides, for container deployments.
    pub fn apply_env(&mut self) {
        if let Ok(url) = std::env::var("DATABASE_URL") {
            self.database.url = url;
        }
        if let Ok(id) = std::env::var("POSTRUST_AI_NODE_ID") {
            self.node.id = id;
        }
        if let Ok(listen) = std::env::var("POSTRUST_AI_LISTEN") {
            if let Ok(addr) = listen.parse() {
                self.node.listen = addr;
            }
        }
        if let Ok(url) = std::env::var("POSTRUST_AI_ADVERTISE_URL") {
            self.node.advertise_url = Some(url);
        }
        if let Ok(secret) = std::env::var("POSTRUST_AI_JWT_SECRET") {
            self.auth.jwt_secret = Some(secret);
        }
        if let Ok(dir) = std::env::var("POSTRUST_AI_CACHE_DIR") {
            self.cache.dir = PathBuf::from(dir);
        }
    }

    /// Reject configurations that would misbehave rather than fail.
    pub fn validate(&mut self) -> AiResult<()> {
        if self.queues.is_empty() {
            self.queues.push(QueueSpec::new("default"));
        }

        let mut seen = HashSet::new();
        for queue in &self.queues {
            if !seen.insert(queue.name.as_str()) {
                return Err(AiError::Config(format!(
                    "queue '{}' declared twice",
                    queue.name
                )));
            }
            queue.validate()?;
        }

        let mut seen = HashSet::new();
        for model in &self.models {
            if !seen.insert(model.id.as_str()) {
                return Err(AiError::Config(format!(
                    "model '{}' declared twice",
                    model.id
                )));
            }
            model.validate()?;
        }

        let (low, high) = self.resources.port_range;
        if low == 0 || low > high {
            return Err(AiError::Config(format!(
                "resources.port_range ({low}, {high}) is empty"
            )));
        }
        if self.scheduler.heartbeat_secs == 0
            || self.scheduler.heartbeat_secs * 2 > self.scheduler.lease_secs
        {
            return Err(AiError::Config(format!(
                "scheduler.heartbeat_secs ({}) must be non-zero and at most half of lease_secs ({}), \
                 or one late heartbeat loses the job",
                self.scheduler.heartbeat_secs, self.scheduler.lease_secs
            )));
        }
        if self.node.id.is_empty() {
            return Err(AiError::Config("node.id is empty".into()));
        }
        Ok(())
    }

    /// The queue with this name.
    pub fn queue(&self, name: &str) -> Option<&QueueSpec> {
        self.queues.iter().find(|q| q.name == name)
    }

    /// The model with this id.
    pub fn model(&self, id: &str) -> Option<&ModelSpec> {
        self.models.iter().find(|m| m.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_config_gets_a_default_queue() {
        let config = AiConfig::from_toml("").unwrap();
        assert_eq!(config.queues.len(), 1);
        assert_eq!(config.queues[0].name, "default");
    }

    #[test]
    fn duplicate_models_are_rejected() {
        let text = r#"
            [[model]]
            id = "a"
            kind = "text"
            flavor = "generic"
            [model.runtime]
            type = "attached"
            base_url = "http://localhost:1"

            [[model]]
            id = "a"
            kind = "text"
            flavor = "generic"
            [model.runtime]
            type = "attached"
            base_url = "http://localhost:2"
        "#;
        let error = AiConfig::from_toml(text).unwrap_err().to_string();
        assert!(error.contains("declared twice"), "{error}");
    }

    #[test]
    fn heartbeat_must_fit_in_lease() {
        let text = "[scheduler]\nlease_secs = 20\nheartbeat_secs = 15\n";
        assert!(AiConfig::from_toml(text).is_err());
    }

    #[test]
    fn unknown_fields_are_rejected() {
        assert!(AiConfig::from_toml("[node]\nlisten_addr = \"x\"\n").is_err());
    }
}
