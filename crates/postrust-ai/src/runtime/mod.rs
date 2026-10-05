//! Starting and stopping model servers.
//!
//! [`start`] turns a [`RuntimeSpec`] into a running [`ServerHandle`];
//! [`flavor`] knows how to talk to each kind of server once it is up.

pub mod flavor;
mod process;

use std::path::{Path, PathBuf};
use std::time::Duration;

use async_trait::async_trait;
use uuid::Uuid;

use crate::error::AiResult;
use crate::model::{ModelSpec, RuntimeSpec};

/// What a runtime needs to start one instance.
#[derive(Debug, Clone)]
pub struct StartContext<'a> {
    /// The model.
    pub spec: &'a ModelSpec,
    /// The instance being started.
    pub instance_id: Uuid,
    /// Port assigned to it.
    pub port: u16,
    /// GPU assigned to it, if any.
    pub gpu: Option<u32>,
    /// Local paths of the spec's files, in order.
    pub files: Vec<PathBuf>,
    /// The model cache directory.
    pub cache_dir: &'a Path,
    /// `HF_HOME` for servers that download their own weights.
    pub hf_home: PathBuf,
}

impl StartContext<'_> {
    /// Expand `{port}`, `{gpu}`, `{file:N}`, `{cache_dir}`, `{hf_home}`,
    /// `{model}` and `{instance}`.
    pub fn expand(&self, template: &str) -> String {
        let mut out = template
            .replace("{port}", &self.port.to_string())
            .replace(
                "{gpu}",
                &self.gpu.map(|g| g.to_string()).unwrap_or_default(),
            )
            .replace("{cache_dir}", &self.cache_dir.display().to_string())
            .replace("{hf_home}", &self.hf_home.display().to_string())
            .replace("{model}", self.spec.served_name())
            .replace("{instance}", &self.instance_id.to_string());
        for (i, path) in self.files.iter().enumerate() {
            out = out.replace(&format!("{{file:{i}}}"), &path.display().to_string());
        }
        out
    }
}

/// A running (or attached) model server.
#[async_trait]
pub trait ServerHandle: Send + Sync {
    /// Where to reach it.
    fn base_url(&self) -> String;
    /// `Some(why)` once the server has exited. Always `None` for attached
    /// servers, which this node does not own.
    fn exited(&mut self) -> Option<String>;
    /// Stop it: a polite signal, then force after `grace`.
    async fn stop(&mut self, grace: Duration);
}

/// Start an instance.
pub async fn start(ctx: &StartContext<'_>) -> AiResult<Box<dyn ServerHandle>> {
    match &ctx.spec.runtime {
        RuntimeSpec::Process {
            command,
            args,
            env,
            workdir,
        } => process::start_process(ctx, command, args, env, workdir.as_deref()).await,
        RuntimeSpec::Container {
            image,
            container_port,
            args,
            env,
            volumes,
            cli,
            extra_flags,
        } => {
            process::start_container(
                ctx,
                process::ContainerArgs {
                    image,
                    container_port: *container_port,
                    args,
                    env,
                    volumes,
                    cli,
                    extra_flags,
                },
            )
            .await
        }
        RuntimeSpec::Attached { base_url } => Ok(Box::new(Attached {
            base_url: base_url.trim_end_matches('/').to_string(),
        })),
    }
}

/// A server this node uses but does not own.
struct Attached {
    base_url: String,
}

#[async_trait]
impl ServerHandle for Attached {
    fn base_url(&self) -> String {
        self.base_url.clone()
    }
    fn exited(&mut self) -> Option<String> {
        None
    }
    async fn stop(&mut self, _grace: Duration) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Flavor, ModelKind};

    #[test]
    fn templates_expand() {
        let spec = ModelSpec::new(
            "w",
            ModelKind::Audio,
            Flavor::WhisperCpp,
            RuntimeSpec::Attached {
                base_url: "http://x".into(),
            },
        );
        let ctx = StartContext {
            spec: &spec,
            instance_id: Uuid::nil(),
            port: 18001,
            gpu: Some(1),
            files: vec![PathBuf::from("/c/blobs/a.bin")],
            cache_dir: Path::new("/c"),
            hf_home: PathBuf::from("/c/hf"),
        };
        assert_eq!(
            ctx.expand("-m {file:0} --port {port} --gpu {gpu} {model} {hf_home}"),
            "-m /c/blobs/a.bin --port 18001 --gpu 1 w /c/hf"
        );
    }
}
