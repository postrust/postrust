//! ACME certificate management using rustls-acme.

use crate::config::AcmeConfig;
use crate::error::{ProxyError, ProxyResult};
use crate::tls::CertificateStore;
use rustls_acme::caches::DirCache;
use rustls_acme::AcmeConfig as RustlsAcmeConfig;
use std::path::Path;
use std::sync::Arc;
use tokio_rustls::rustls::ServerConfig;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

/// ACME certificate manager.
pub struct AcmeManager {
    /// ACME configuration
    config: AcmeConfig,
    /// Certificate store for persistence
    cert_store: Arc<CertificateStore>,
    /// Cache directory for ACME state
    cache_dir: std::path::PathBuf,
}

impl AcmeManager {
    /// Create a new ACME manager.
    pub fn new(
        config: AcmeConfig,
        cert_store: Arc<CertificateStore>,
        cache_dir: impl AsRef<Path>,
    ) -> Self {
        Self {
            config,
            cert_store,
            cache_dir: cache_dir.as_ref().to_path_buf(),
        }
    }

    /// Create an ACME resolver for TLS.
    ///
    /// This returns an Arc<ServerConfig> that automatically handles ACME challenges
    /// and certificate renewal.
    pub fn create_resolver(&self, domains: Vec<String>) -> ProxyResult<Arc<ServerConfig>> {
        if !self.config.enabled {
            return Err(ProxyError::Tls("ACME is not enabled".into()));
        }

        let directory = if self.config.staging {
            "https://acme-staging-v02.api.letsencrypt.org/directory"
        } else {
            "https://acme-v02.api.letsencrypt.org/directory"
        };

        let contacts: Vec<String> = self
            .config
            .email
            .as_ref()
            .map(|email| vec![format!("mailto:{}", email)])
            .unwrap_or_default();

        info!("Setting up ACME for domains: {:?}", domains);

        // Create ACME state with directory cache
        let cache_dir = self.cache_dir.clone();
        let cache = DirCache::new(cache_dir);

        let state = RustlsAcmeConfig::new(domains)
            .contact(contacts)
            .cache(cache)
            .directory(directory)
            .state();

        // Get the resolver which handles ACME challenges
        let resolver = state.resolver();

        // Build server config with ACME resolver
        let server_config = ServerConfig::builder()
            .with_no_client_auth()
            .with_cert_resolver(resolver);

        Ok(Arc::new(server_config))
    }

    /// Build a ServerConfig from PEM-encoded certificate and key.
    pub fn build_server_config_from_pem(
        cert_pem: &[u8],
        key_pem: &[u8],
    ) -> ProxyResult<Arc<ServerConfig>> {
        use rustls_pemfile::{certs, private_key};
        use std::io::BufReader;

        // Parse certificates
        let certs: Vec<_> = certs(&mut BufReader::new(cert_pem))
            .filter_map(|r| r.ok())
            .collect();

        if certs.is_empty() {
            return Err(ProxyError::Tls("No certificates found in PEM".into()));
        }

        // Parse private key
        let key = private_key(&mut BufReader::new(key_pem))
            .map_err(|e| ProxyError::Tls(format!("Failed to parse private key: {}", e)))?
            .ok_or_else(|| ProxyError::Tls("No private key found in PEM".into()))?;

        // Build server config
        let config = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .map_err(|e| ProxyError::Tls(format!("Failed to build TLS config: {}", e)))?;

        Ok(Arc::new(config))
    }

    /// Start background certificate renewal task.
    pub async fn start_renewal_task(self: Arc<Self>, cancel_token: CancellationToken) {
        let check_interval = std::time::Duration::from_secs(86400); // Daily

        info!("ACME certificate renewal task started");

        loop {
            tokio::select! {
                _ = cancel_token.cancelled() => {
                    info!("ACME renewal task stopped");
                    break;
                }
                _ = tokio::time::sleep(check_interval) => {
                    if let Err(e) = self.check_renewals().await {
                        error!("Certificate renewal check failed: {}", e);
                    }
                }
            }
        }
    }

    /// Check for certificates that need renewal.
    async fn check_renewals(&self) -> ProxyResult<()> {
        let domains = self.cert_store.list_domains().await?;
        let now = chrono::Utc::now();
        let renewal_threshold = chrono::Duration::days(30);

        for domain in domains {
            if let Some(cert) = self.cert_store.get(&domain).await {
                if let Some(expires_at) = cert.expires_at {
                    if expires_at < now + renewal_threshold {
                        info!(
                            "Certificate for {} needs renewal (expires {})",
                            domain, expires_at
                        );
                        // Note: With rustls-acme, renewal happens automatically via the resolver
                        // This is just for monitoring/alerting purposes
                    }
                }
            }
        }

        Ok(())
    }

    /// Check if ACME is enabled.
    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    /// Get configured domains.
    pub fn domains(&self) -> &[String] {
        &self.config.domains
    }
}
