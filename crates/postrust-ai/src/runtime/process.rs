//! Local processes and containers.

use std::collections::BTreeMap;
use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

use super::{ServerHandle, StartContext};
use crate::error::{AiError, AiResult};
use crate::model::Flavor;

/// A child process, possibly the `docker run` that fronts a container.
struct ProcessHandle {
    model: String,
    child: Child,
    base_url: String,
    /// For containers: how to stop the container, which killing the CLI
    /// process does not do.
    container: Option<(String, String)>,
    exit: Option<String>,
}

#[async_trait]
impl ServerHandle for ProcessHandle {
    fn base_url(&self) -> String {
        self.base_url.clone()
    }

    fn exited(&mut self) -> Option<String> {
        if self.exit.is_none() {
            if let Ok(Some(status)) = self.child.try_wait() {
                self.exit = Some(format!("exited with {status}"));
            }
        }
        self.exit.clone()
    }

    async fn stop(&mut self, grace: Duration) {
        if self.exited().is_some() {
            return;
        }
        if let Some((cli, name)) = &self.container {
            // `docker stop` does the SIGTERM-then-SIGKILL itself.
            let _ = Command::new(cli)
                .args(["stop", "-t", &grace.as_secs().to_string(), name])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .await;
        } else {
            terminate(&self.child);
        }
        match tokio::time::timeout(grace + Duration::from_secs(5), self.child.wait()).await {
            Ok(_) => {}
            Err(_) => {
                tracing::warn!(model = %self.model, "model server ignored SIGTERM; killing");
                kill_group(&self.child);
                let _ = self.child.kill().await;
            }
        }
        self.exit = Some("stopped".into());
    }
}

/// SIGTERM to the whole process group: vLLM and others fork engine workers
/// that must go with the parent, or they keep the GPU memory.
#[cfg(unix)]
fn terminate(child: &Child) {
    if let Some(pid) = child.id() {
        // SAFETY: kill(2) with a negative pid signals a process group; it has
        // no memory-safety preconditions.
        unsafe {
            libc::kill(-(pid as i32), libc::SIGTERM);
        }
    }
}

#[cfg(unix)]
fn kill_group(child: &Child) {
    if let Some(pid) = child.id() {
        // SAFETY: as above.
        unsafe {
            libc::kill(-(pid as i32), libc::SIGKILL);
        }
    }
}

#[cfg(not(unix))]
fn terminate(_child: &Child) {}

#[cfg(not(unix))]
fn kill_group(_child: &Child) {}

/// Forward a child's output to tracing, one event per line.
fn forward_output(child: &mut Child, model: &str) {
    if let Some(stdout) = child.stdout.take() {
        let model = model.to_string();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::info!(target: "postrust_ai::model_server", model = %model, "{line}");
            }
        });
    }
    if let Some(stderr) = child.stderr.take() {
        let model = model.to_string();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::info!(target: "postrust_ai::model_server", model = %model, "{line}");
            }
        });
    }
}

/// Environment every managed server gets, before the spec's own.
fn base_env(ctx: &StartContext<'_>) -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    // Pin the server to its GPU, or hide GPUs from a CPU-only model so it
    // cannot take VRAM that was never reserved for it.
    env.insert(
        "CUDA_VISIBLE_DEVICES".into(),
        ctx.gpu.map(|g| g.to_string()).unwrap_or_default(),
    );
    env.insert("HF_HOME".into(), ctx.hf_home.display().to_string());
    match ctx.spec.flavor {
        // Exposes /sleep and /wake_up, which parking needs. They are only
        // reachable on the address the server binds; bind it to 127.0.0.1.
        Flavor::Vllm => {
            env.insert("VLLM_SERVER_DEV_MODE".into(), "1".into());
        }
        Flavor::Ollama => {
            env.insert("OLLAMA_HOST".into(), format!("127.0.0.1:{}", ctx.port));
            env.insert(
                "OLLAMA_MODELS".into(),
                ctx.cache_dir.join("ollama").display().to_string(),
            );
            // This node decides when models leave memory, not Ollama's
            // five-minute default.
            env.insert("OLLAMA_KEEP_ALIVE".into(), "-1".into());
        }
        Flavor::WhisperCpp | Flavor::Generic => {}
    }
    env
}

pub(super) async fn start_process(
    ctx: &StartContext<'_>,
    command: &str,
    args: &[String],
    env: &BTreeMap<String, String>,
    workdir: Option<&str>,
) -> AiResult<Box<dyn ServerHandle>> {
    let mut cmd = Command::new(ctx.expand(command));
    cmd.args(args.iter().map(|a| ctx.expand(a)))
        .envs(base_env(ctx))
        .envs(env.iter().map(|(k, v)| (k.clone(), ctx.expand(v))))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if let Some(dir) = workdir {
        cmd.current_dir(ctx.expand(dir));
    }
    #[cfg(unix)]
    cmd.process_group(0);

    let mut child = cmd.spawn().map_err(|e| AiError::LoadFailed {
        model: ctx.spec.id.clone(),
        detail: format!("could not start '{command}': {e}"),
    })?;
    forward_output(&mut child, &ctx.spec.id);
    Ok(Box::new(ProcessHandle {
        model: ctx.spec.id.clone(),
        child,
        base_url: format!("http://127.0.0.1:{}", ctx.port),
        container: None,
        exit: None,
    }))
}

pub(super) struct ContainerArgs<'a> {
    pub image: &'a str,
    pub container_port: u16,
    pub args: &'a [String],
    pub env: &'a BTreeMap<String, String>,
    pub volumes: &'a [String],
    pub cli: &'a str,
    pub extra_flags: &'a [String],
}

pub(super) async fn start_container(
    ctx: &StartContext<'_>,
    c: ContainerArgs<'_>,
) -> AiResult<Box<dyn ServerHandle>> {
    let name = format!("postrust-ai-{}", ctx.instance_id);
    let mut cmd = Command::new(c.cli);
    cmd.args(["run", "--rm", "--name", &name]).args([
        "-p",
        &format!("127.0.0.1:{}:{}", ctx.port, c.container_port),
    ]);
    if let Some(gpu) = ctx.gpu {
        cmd.args(["--gpus", &format!("device={gpu}")]);
    }
    let mut env = base_env(ctx);
    // Inside the container the GPU it was given is device 0, and the server
    // listens on its own port, not the host one.
    env.remove("CUDA_VISIBLE_DEVICES");
    if ctx.spec.flavor == Flavor::Ollama {
        env.insert(
            "OLLAMA_HOST".into(),
            format!("0.0.0.0:{}", c.container_port),
        );
        env.remove("OLLAMA_MODELS");
    }
    env.remove("HF_HOME");
    for (k, v) in env
        .into_iter()
        .chain(c.env.iter().map(|(k, v)| (k.clone(), ctx.expand(v))))
    {
        cmd.args(["-e", &format!("{k}={v}")]);
    }
    for volume in c.volumes {
        cmd.args(["-v", &ctx.expand(volume)]);
    }
    cmd.args(c.extra_flags.iter().map(|f| ctx.expand(f)))
        .arg(ctx.expand(c.image))
        .args(c.args.iter().map(|a| ctx.expand(a)))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let mut child = cmd.spawn().map_err(|e| AiError::LoadFailed {
        model: ctx.spec.id.clone(),
        detail: format!("could not run '{}': {e}", c.cli),
    })?;
    forward_output(&mut child, &ctx.spec.id);
    Ok(Box::new(ProcessHandle {
        model: ctx.spec.id.clone(),
        child,
        base_url: format!("http://127.0.0.1:{}", ctx.port),
        container: Some((c.cli.to_string(), name)),
        exit: None,
    }))
}
