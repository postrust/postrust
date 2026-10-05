//! Model declarations: what a model is, what it needs, and how to start it.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::error::{AiError, AiResult};

/// What a model takes in. Informational for routing and the API; the server
/// flavor decides how it is actually called.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelKind {
    /// Text generation and chat.
    Text,
    /// Speech-to-text, text-to-speech, audio classification.
    Audio,
    /// Video understanding (e.g. a vision-language model fed frames or clips).
    Video,
    /// Image models.
    Image,
    /// Embeddings.
    Embedding,
}

/// Which model server is behind the instance. Decides the health check,
/// whether the instance can be parked, and the default inference path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Flavor {
    /// vLLM's OpenAI-compatible server. Can be parked (sleep mode) when
    /// started with `--enable-sleep-mode` and `VLLM_SERVER_DEV_MODE=1`.
    Vllm,
    /// whisper.cpp's `whisper-server`. Takes multipart audio at `/inference`.
    WhisperCpp,
    /// Ollama. One server hosts many models; "loading" a model is asking the
    /// server to put it in memory, and "unloading" is `keep_alive: 0`.
    Ollama,
    /// Any HTTP server. Health check and paths come from the spec.
    Generic,
}

impl Flavor {
    /// Health check path when the spec does not give one.
    pub fn default_health_path(self) -> &'static str {
        match self {
            Flavor::Vllm => "/health",
            // whisper-server has no health endpoint; its index page answers
            // once the model is loaded, because it loads before listening.
            Flavor::WhisperCpp => "/",
            Flavor::Ollama => "/api/version",
            Flavor::Generic => "/health",
        }
    }

    /// Inference path when a job request does not name one.
    pub fn default_invoke_path(self, kind: ModelKind) -> &'static str {
        match (self, kind) {
            (Flavor::WhisperCpp, _) => "/inference",
            (Flavor::Vllm | Flavor::Ollama, ModelKind::Embedding) => "/v1/embeddings",
            (Flavor::Vllm | Flavor::Ollama, _) => "/v1/chat/completions",
            (Flavor::Generic, _) => "/",
        }
    }

    /// Whether this flavor can release its GPU memory while keeping the
    /// weights in host memory, for a fast wake.
    pub fn supports_park(self) -> bool {
        matches!(self, Flavor::Vllm)
    }
}

/// How a model's server is started.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum RuntimeSpec {
    /// A local process. `args` and `env` values may use `{port}`, `{gpu}`,
    /// `{file:N}` (the Nth cached file's path), `{cache_dir}` and `{model}`.
    Process {
        /// Executable.
        command: String,
        /// Arguments.
        #[serde(default)]
        args: Vec<String>,
        /// Extra environment.
        #[serde(default)]
        env: BTreeMap<String, String>,
        /// Working directory.
        #[serde(default)]
        workdir: Option<String>,
    },
    /// A container, run with the `docker` (or `podman`) CLI.
    Container {
        /// Image reference.
        image: String,
        /// Port the server listens on inside the container.
        container_port: u16,
        /// Arguments after the image.
        #[serde(default)]
        args: Vec<String>,
        /// Container environment.
        #[serde(default)]
        env: BTreeMap<String, String>,
        /// `host:container[:mode]` bind mounts. `{cache_dir}` is expanded.
        #[serde(default)]
        volumes: Vec<String>,
        /// `docker` or `podman`.
        #[serde(default = "default_container_cli")]
        cli: String,
        /// Extra `run` flags, e.g. `--ipc=host` for vLLM.
        #[serde(default)]
        extra_flags: Vec<String>,
    },
    /// A server someone else runs. Nothing is started or stopped; for Ollama,
    /// the model is loaded into and unloaded from that server.
    Attached {
        /// Base URL of the server.
        base_url: String,
    },
}

fn default_container_cli() -> String {
    "docker".into()
}

/// A file a model needs on local disk, fetched through the model cache.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ModelFile {
    /// `https://...`, `hf://org/repo[@revision]/path`, or `file:///path`
    /// (used in place, never copied or evicted).
    pub url: String,
    /// Expected sha256. Strongly recommended: without it a file is cached by
    /// URL and a changed upstream file is never noticed.
    #[serde(default)]
    pub sha256: Option<String>,
}

/// One model.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ModelSpec {
    /// Unique id; what jobs and requests name.
    pub id: String,
    /// What it takes in.
    pub kind: ModelKind,
    /// Which server runs it.
    pub flavor: Flavor,
    /// Part of the result-cache key, so bumping it invalidates cached
    /// results.
    #[serde(default = "default_version")]
    pub version: String,
    /// The model's name *inside* the server, where it differs from `id`:
    /// the Ollama tag (`llama3.1:8b`), or the vLLM served model name.
    #[serde(default)]
    pub served_name: Option<String>,
    /// How to start the server.
    pub runtime: RuntimeSpec,
    /// Files fetched into the cache before the server starts.
    #[serde(default)]
    pub files: Vec<ModelFile>,
    /// Host memory the instance takes, in MiB.
    #[serde(default)]
    pub memory_mb: u64,
    /// VRAM the instance takes on one GPU, in MiB. 0: CPU only.
    #[serde(default)]
    pub gpu_mb: u64,
    /// Health check path; the flavor's default if absent.
    #[serde(default)]
    pub health_path: Option<String>,
    /// How long to wait for the server to become healthy.
    #[serde(default = "default_load_timeout")]
    pub load_timeout_secs: u64,
    /// Unload (or park) after this long without a request. 0: never.
    #[serde(default = "default_idle_timeout")]
    pub idle_timeout_secs: u64,
    /// Stop a parked instance after this much longer.
    #[serde(default = "default_park_timeout")]
    pub park_timeout_secs: u64,
    /// Never unload.
    #[serde(default)]
    pub pinned: bool,
    /// Keep at least this many instances loaded, loading them at startup.
    #[serde(default)]
    pub min_warm: usize,
    /// At most this many instances on one node.
    #[serde(default = "default_one")]
    pub max_replicas: usize,
    /// Requests one instance serves at once.
    #[serde(default = "default_one")]
    pub max_concurrency: usize,
    /// Cache results of jobs on this model.
    #[serde(default)]
    pub result_cache: bool,
}

fn default_version() -> String {
    "1".into()
}
fn default_load_timeout() -> u64 {
    300
}
fn default_idle_timeout() -> u64 {
    600
}
fn default_park_timeout() -> u64 {
    1800
}
fn default_one() -> usize {
    1
}

impl ModelSpec {
    /// A spec with defaults, for tests and programmatic setup.
    pub fn new(
        id: impl Into<String>,
        kind: ModelKind,
        flavor: Flavor,
        runtime: RuntimeSpec,
    ) -> Self {
        Self {
            id: id.into(),
            kind,
            flavor,
            version: default_version(),
            served_name: None,
            runtime,
            files: Vec::new(),
            memory_mb: 0,
            gpu_mb: 0,
            health_path: None,
            load_timeout_secs: default_load_timeout(),
            idle_timeout_secs: default_idle_timeout(),
            park_timeout_secs: default_park_timeout(),
            pinned: false,
            min_warm: 0,
            max_replicas: 1,
            max_concurrency: 1,
            result_cache: false,
        }
    }

    /// The name the server knows the model by.
    pub fn served_name(&self) -> &str {
        self.served_name.as_deref().unwrap_or(&self.id)
    }

    /// Health check path.
    pub fn health_path(&self) -> &str {
        self.health_path
            .as_deref()
            .unwrap_or_else(|| self.flavor.default_health_path())
    }

    /// Whether an instance of this model can be parked rather than stopped.
    pub fn can_park(&self) -> bool {
        self.flavor.supports_park() && !matches!(self.runtime, RuntimeSpec::Attached { .. })
    }

    /// Reject specs that cannot work.
    pub fn validate(&self) -> AiResult<()> {
        let fail = |msg: String| Err(AiError::Config(format!("model '{}': {msg}", self.id)));
        if self.id.is_empty() || self.id.contains('/') {
            return fail("id must be non-empty and contain no '/'".into());
        }
        if self.max_replicas == 0 {
            return fail("max_replicas must be at least 1".into());
        }
        if self.max_concurrency == 0 {
            return fail("max_concurrency must be at least 1".into());
        }
        if self.min_warm > self.max_replicas {
            return fail(format!(
                "min_warm ({}) exceeds max_replicas ({})",
                self.min_warm, self.max_replicas
            ));
        }
        if let RuntimeSpec::Attached { .. } = self.runtime {
            // An attached server is one instance by definition: there is
            // nothing to start a second copy of.
            if self.max_replicas != 1 {
                return fail(
                    "attached runtimes have exactly one instance; set max_replicas = 1".into(),
                );
            }
        }
        for file in &self.files {
            let ok = file.url.starts_with("https://")
                || file.url.starts_with("http://")
                || file.url.starts_with("hf://")
                || file.url.starts_with("file://");
            if !ok {
                return fail(format!("unsupported file url '{}'", file.url));
            }
            if let Some(sha) = &file.sha256 {
                if sha.len() != 64 || !sha.chars().all(|c| c.is_ascii_hexdigit()) {
                    return fail(format!("sha256 for '{}' is not 64 hex digits", file.url));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_whisper_process_model() {
        let spec: ModelSpec = toml::from_str(
            r#"
            id = "whisper-large-v3"
            kind = "audio"
            flavor = "whisper_cpp"
            memory_mb = 1024
            gpu_mb = 3500
            result_cache = true
            files = [{ url = "hf://ggerganov/whisper.cpp/ggml-large-v3.bin" }]
            [runtime]
            type = "process"
            command = "whisper-server"
            args = ["-m", "{file:0}", "--port", "{port}"]
            "#,
        )
        .unwrap();
        spec.validate().unwrap();
        assert_eq!(spec.health_path(), "/");
        assert!(!spec.can_park());
    }

    #[test]
    fn vllm_can_park_unless_attached() {
        let mut spec = ModelSpec::new(
            "m",
            ModelKind::Text,
            Flavor::Vllm,
            RuntimeSpec::Process {
                command: "vllm".into(),
                args: vec![],
                env: Default::default(),
                workdir: None,
            },
        );
        assert!(spec.can_park());
        spec.runtime = RuntimeSpec::Attached {
            base_url: "http://x".into(),
        };
        assert!(!spec.can_park());
    }

    #[test]
    fn min_warm_cannot_exceed_replicas() {
        let mut spec = ModelSpec::new(
            "m",
            ModelKind::Text,
            Flavor::Generic,
            RuntimeSpec::Attached {
                base_url: "http://x".into(),
            },
        );
        spec.min_warm = 2;
        assert!(spec.validate().is_err());
    }
}
