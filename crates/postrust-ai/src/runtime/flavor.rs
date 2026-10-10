//! Per-server behaviour: readiness, loading a model into a server that hosts
//! several, parking, and unloading.

use std::time::{Duration, Instant};

use serde_json::json;

use super::ServerHandle;
use crate::error::{AiError, AiResult};
use crate::model::{Flavor, ModelSpec};

/// Wait until the server answers its health check, the timeout passes, or
/// the process exits -- whichever is first. Then do whatever the flavor needs
/// before the instance can take requests.
pub async fn wait_ready(
    http: &reqwest::Client,
    spec: &ModelSpec,
    handle: &mut dyn ServerHandle,
    timeout: Duration,
) -> AiResult<()> {
    let base = handle.base_url();
    let url = format!("{base}{}", spec.health_path());
    let deadline = Instant::now() + timeout;
    let mut delay = Duration::from_millis(250);
    loop {
        if let Some(exit) = handle.exited() {
            return Err(AiError::LoadFailed {
                model: spec.id.clone(),
                detail: format!("server {exit} before becoming ready"),
            });
        }
        let healthy = http
            .get(&url)
            .timeout(Duration::from_secs(5))
            .send()
            .await
            .is_ok_and(|r| r.status().is_success());
        if healthy {
            break;
        }
        if Instant::now() >= deadline {
            return Err(AiError::LoadTimeout {
                model: spec.id.clone(),
                waited: timeout,
            });
        }
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(Duration::from_secs(3));
    }

    if spec.flavor == Flavor::Ollama {
        ollama_load(
            http,
            spec,
            &base,
            deadline.saturating_duration_since(Instant::now()),
        )
        .await?;
    }
    Ok(())
}

/// Put an Ollama model in memory, pulling it first if the server lacks it.
/// Ollama's own model store is this flavor's file cache.
async fn ollama_load(
    http: &reqwest::Client,
    spec: &ModelSpec,
    base: &str,
    budget: Duration,
) -> AiResult<()> {
    let name = spec.served_name();
    let present = http
        .post(format!("{base}/api/show"))
        .json(&json!({ "model": name }))
        .send()
        .await
        .is_ok_and(|r| r.status().is_success());
    if !present {
        tracing::info!(model = %spec.id, name, "pulling Ollama model");
        let response = http
            .post(format!("{base}/api/pull"))
            .json(&json!({ "model": name, "stream": false }))
            .timeout(budget.max(Duration::from_secs(60)))
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(AiError::LoadFailed {
                model: spec.id.clone(),
                detail: format!("ollama pull: {}", response.text().await.unwrap_or_default()),
            });
        }
    }
    // A generate with no prompt loads the model and returns; keep_alive -1
    // keeps it until this node says otherwise.
    let response = http
        .post(format!("{base}/api/generate"))
        .json(&json!({ "model": name, "keep_alive": -1 }))
        .timeout(budget.max(Duration::from_secs(60)))
        .send()
        .await?;
    if !response.status().is_success() {
        return Err(AiError::LoadFailed {
            model: spec.id.clone(),
            detail: format!("ollama load: {}", response.text().await.unwrap_or_default()),
        });
    }
    Ok(())
}

/// Release the GPU but keep the weights in host memory. `Ok(false)` if this
/// server cannot, and the caller should stop it instead.
pub async fn park(http: &reqwest::Client, spec: &ModelSpec, base: &str) -> AiResult<bool> {
    if !spec.can_park() {
        return Ok(false);
    }
    // Level 1 offloads weights to CPU memory and drops the KV cache; level 2
    // would discard the weights too, which is no faster to undo than a stop.
    let response = http
        .post(format!("{base}/sleep?level=1"))
        .timeout(Duration::from_secs(120))
        .send()
        .await?;
    match response.status() {
        s if s.is_success() => Ok(true),
        // Not started with --enable-sleep-mode, or dev mode off.
        reqwest::StatusCode::NOT_FOUND | reqwest::StatusCode::METHOD_NOT_ALLOWED => {
            tracing::warn!(
                model = %spec.id,
                "vLLM has no /sleep endpoint; start it with --enable-sleep-mode to allow parking"
            );
            Ok(false)
        }
        s => Err(AiError::Upstream(format!("sleep returned {s}"))),
    }
}

/// Undo [`park`].
pub async fn wake(http: &reqwest::Client, spec: &ModelSpec, base: &str) -> AiResult<()> {
    let response = http
        .post(format!("{base}/wake_up"))
        .timeout(Duration::from_secs(spec.load_timeout_secs.max(30)))
        .send()
        .await?;
    if !response.status().is_success() {
        return Err(AiError::Upstream(format!(
            "wake_up returned {}",
            response.status()
        )));
    }
    Ok(())
}

/// What to do before stopping. For an attached Ollama there is no process to
/// stop, so this is the whole of unloading.
pub async fn before_stop(http: &reqwest::Client, spec: &ModelSpec, base: &str) {
    if spec.flavor == Flavor::Ollama {
        let result = http
            .post(format!("{base}/api/generate"))
            .json(&json!({ "model": spec.served_name(), "keep_alive": 0 }))
            .timeout(Duration::from_secs(30))
            .send()
            .await;
        if let Err(error) = result {
            tracing::warn!(model = %spec.id, %error, "could not unload Ollama model");
        }
    }
}
