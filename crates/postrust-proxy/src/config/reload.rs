//! Configuration hot-reload via file watching and database LISTEN/NOTIFY.

use crate::config::ProxyConfig;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Configuration change event.
#[derive(Debug, Clone)]
pub enum ConfigChangeEvent {
    /// File configuration changed
    FileChanged,
    /// Database route changed
    RouteChanged { id: uuid::Uuid },
    /// Database upstream changed
    UpstreamChanged { id: uuid::Uuid },
    /// Database backend changed
    BackendChanged { id: uuid::Uuid },
    /// Full reload requested
    FullReload,
}

/// Manages configuration reloading.
pub struct ConfigReloader {
    /// Current configuration
    config: Arc<RwLock<ProxyConfig>>,
    /// Change event sender
    change_tx: tokio::sync::mpsc::Sender<ConfigChangeEvent>,
    /// Change event receiver
    change_rx: tokio::sync::Mutex<tokio::sync::mpsc::Receiver<ConfigChangeEvent>>,
}

impl ConfigReloader {
    /// Create a new config reloader.
    pub fn new(config: Arc<RwLock<ProxyConfig>>) -> Self {
        let (change_tx, change_rx) = tokio::sync::mpsc::channel(100);

        Self {
            config,
            change_tx,
            change_rx: tokio::sync::Mutex::new(change_rx),
        }
    }

    /// Request a full configuration reload.
    pub async fn request_reload(&self) {
        let _ = self.change_tx.send(ConfigChangeEvent::FullReload).await;
    }

    /// Get the change event sender for external triggers.
    pub fn change_sender(&self) -> tokio::sync::mpsc::Sender<ConfigChangeEvent> {
        self.change_tx.clone()
    }
}
