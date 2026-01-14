//! File-based configuration loading.

use crate::error::ProxyResult;
use crate::config::ProxyConfig;
use std::path::Path;

/// Load proxy configuration from a TOML file.
pub async fn load_from_file(path: impl AsRef<Path>) -> ProxyResult<ProxyConfig> {
    let content = tokio::fs::read_to_string(path).await?;
    let config: ProxyConfig = toml::from_str(&content)?;
    Ok(config)
}
