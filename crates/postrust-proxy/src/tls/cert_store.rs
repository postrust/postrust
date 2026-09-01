//! Certificate storage with database metadata and file caching.

use crate::error::ProxyResult;
use sqlx::PgPool;
use std::path::{Path, PathBuf};
use tokio::sync::RwLock;
use tracing::{debug, info};

/// Certificate and key pair.
///
/// `Debug` prints the domain and expiry but never the key or the chain: a
/// private key that reaches a log is a private key that has to be rotated.
#[derive(Clone)]
pub struct Certificate {
    /// Domain name
    pub domain: String,
    /// Certificate chain in PEM format
    pub cert_pem: Vec<u8>,
    /// Private key in PEM format
    pub key_pem: Vec<u8>,
    /// Expiry timestamp
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl std::fmt::Debug for Certificate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Certificate")
            .field("domain", &self.domain)
            .field("expires_at", &self.expires_at)
            .field("cert_pem", &format_args!("<{} bytes>", self.cert_pem.len()))
            .field("key_pem", &format_args!("<redacted>"))
            .finish()
    }
}

/// Certificate store with database metadata and file caching.
pub struct CertificateStore {
    /// Database pool for metadata
    pool: PgPool,
    /// Cache directory for certificate files
    cache_dir: PathBuf,
    /// In-memory certificate cache
    cache: RwLock<std::collections::HashMap<String, Certificate>>,
}

impl CertificateStore {
    /// Create a new certificate store.
    pub async fn new(pool: PgPool, cache_dir: impl AsRef<Path>) -> ProxyResult<Self> {
        let cache_dir = cache_dir.as_ref().to_path_buf();

        // Ensure cache directory exists
        tokio::fs::create_dir_all(&cache_dir).await?;

        Ok(Self {
            pool,
            cache_dir,
            cache: RwLock::new(std::collections::HashMap::new()),
        })
    }

    /// Get a certificate for a domain.
    pub async fn get(&self, domain: &str) -> Option<Certificate> {
        // Check in-memory cache first
        {
            let cache = self.cache.read().await;
            if let Some(cert) = cache.get(domain) {
                return Some(cert.clone());
            }
        }

        // Try to load from file cache
        if let Ok(cert) = self.load_from_file(domain).await {
            let mut cache = self.cache.write().await;
            cache.insert(domain.to_string(), cert.clone());
            return Some(cert);
        }

        // Try to load from database
        if let Ok(Some(cert)) = self.load_from_database(domain).await {
            // Cache to file and memory
            let _ = self.save_to_file(&cert).await;
            let mut cache = self.cache.write().await;
            cache.insert(domain.to_string(), cert.clone());
            return Some(cert);
        }

        None
    }

    /// Save a certificate.
    pub async fn save(&self, cert: Certificate) -> ProxyResult<()> {
        // Save to database
        self.save_to_database(&cert).await?;

        // Save to file cache
        self.save_to_file(&cert).await?;

        // Update in-memory cache
        let mut cache = self.cache.write().await;
        cache.insert(cert.domain.clone(), cert);

        Ok(())
    }

    /// Remove a certificate.
    pub async fn remove(&self, domain: &str) -> ProxyResult<()> {
        // Remove from database
        sqlx::query("DELETE FROM proxy_certificates WHERE domain = $1")
            .bind(domain)
            .execute(&self.pool)
            .await?;

        // Remove from file cache
        let cert_path = self.cache_dir.join(format!("{}.crt", domain));
        let key_path = self.cache_dir.join(format!("{}.key", domain));
        let _ = tokio::fs::remove_file(cert_path).await;
        let _ = tokio::fs::remove_file(key_path).await;

        // Remove from memory cache
        let mut cache = self.cache.write().await;
        cache.remove(domain);

        info!("Removed certificate for domain: {}", domain);
        Ok(())
    }

    /// List all stored domains.
    pub async fn list_domains(&self) -> ProxyResult<Vec<String>> {
        let rows: Vec<(String,)> = sqlx::query_as("SELECT domain FROM proxy_certificates")
            .fetch_all(&self.pool)
            .await?;

        Ok(rows.into_iter().map(|(d,)| d).collect())
    }

    async fn load_from_file(&self, domain: &str) -> ProxyResult<Certificate> {
        let cert_path = self.cache_dir.join(format!("{}.crt", domain));
        let key_path = self.cache_dir.join(format!("{}.key", domain));

        let cert_pem = tokio::fs::read(&cert_path).await?;
        let key_pem = tokio::fs::read(&key_path).await?;

        debug!("Loaded certificate from file: {}", domain);

        Ok(Certificate {
            domain: domain.to_string(),
            cert_pem,
            key_pem,
            expires_at: None, // Would need to parse cert to get expiry
        })
    }

    async fn save_to_file(&self, cert: &Certificate) -> ProxyResult<()> {
        let cert_path = self.cache_dir.join(format!("{}.crt", cert.domain));
        let key_path = self.cache_dir.join(format!("{}.key", cert.domain));

        tokio::fs::write(&cert_path, &cert.cert_pem).await?;
        tokio::fs::write(&key_path, &cert.key_pem).await?;

        debug!("Saved certificate to file: {}", cert.domain);
        Ok(())
    }

    async fn load_from_database(&self, domain: &str) -> ProxyResult<Option<Certificate>> {
        let row: Option<(String, Vec<u8>, Vec<u8>, Option<chrono::DateTime<chrono::Utc>>)> =
            sqlx::query_as(
                "SELECT domain, cert_pem, key_pem, expires_at FROM proxy_certificates WHERE domain = $1",
            )
            .bind(domain)
            .fetch_optional(&self.pool)
            .await?;

        Ok(
            row.map(|(domain, cert_pem, key_pem, expires_at)| Certificate {
                domain,
                cert_pem,
                key_pem,
                expires_at,
            }),
        )
    }

    async fn save_to_database(&self, cert: &Certificate) -> ProxyResult<()> {
        sqlx::query(
            r#"
            INSERT INTO proxy_certificates (domain, cert_pem, key_pem, expires_at, updated_at)
            VALUES ($1, $2, $3, $4, NOW())
            ON CONFLICT (domain) DO UPDATE SET
                cert_pem = EXCLUDED.cert_pem,
                key_pem = EXCLUDED.key_pem,
                expires_at = EXCLUDED.expires_at,
                updated_at = NOW()
            "#,
        )
        .bind(&cert.domain)
        .bind(&cert.cert_pem)
        .bind(&cert.key_pem)
        .bind(cert.expires_at)
        .execute(&self.pool)
        .await?;

        info!("Saved certificate to database: {}", cert.domain);
        Ok(())
    }
}
