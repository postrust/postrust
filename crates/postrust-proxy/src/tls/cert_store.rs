//! Certificate storage with database metadata and file caching.

use crate::error::{ProxyError, ProxyResult};
use sqlx::PgPool;
use std::path::{Path, PathBuf};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

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
    ///
    /// The cache directory holds private keys, so it is created owner-only
    /// (0700) and every key in it is kept at 0600. A directory that already
    /// exists is left as it is -- `cert_dir` might be a shared path, and
    /// tightening it could break whatever else uses it -- but a warning says
    /// so if others can read it, and key files written by earlier versions,
    /// which were left at the umask's default (typically world-readable), are
    /// tightened.
    pub async fn new(pool: PgPool, cache_dir: impl AsRef<Path>) -> ProxyResult<Self> {
        let cache_dir = cache_dir.as_ref().to_path_buf();

        create_private_dir(&cache_dir).await?;
        tighten_existing_keys(&cache_dir).await;

        Ok(Self {
            pool,
            cache_dir,
            cache: RwLock::new(std::collections::HashMap::new()),
        })
    }

    /// Get a certificate for a domain.
    ///
    /// `None` for a name that is not a valid lowercase DNS name: see
    /// [`check_domain`].
    pub async fn get(&self, domain: &str) -> Option<Certificate> {
        if let Err(e) = check_domain(domain) {
            debug!("Not looking up a certificate: {}", e);
            return None;
        }

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
    ///
    /// Refused unless `cert.domain` is a valid lowercase DNS name: see
    /// [`check_domain`].
    pub async fn save(&self, cert: Certificate) -> ProxyResult<()> {
        check_domain(&cert.domain)?;

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
    ///
    /// Refused unless `domain` is a valid lowercase DNS name: see
    /// [`check_domain`].
    pub async fn remove(&self, domain: &str) -> ProxyResult<()> {
        check_domain(domain)?;

        // Remove from database
        sqlx::query("DELETE FROM proxy_certificates WHERE domain = $1")
            .bind(domain)
            .execute(&self.pool)
            .await?;

        // Remove from file cache
        let (cert_path, key_path) = self.paths(domain)?;
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

    /// The cache files for `domain`: certificate, then key.
    ///
    /// The only place a path is built from a domain, and it refuses any name
    /// [`check_domain`] does: a domain is a tenant-supplied string, and
    /// `../../somewhere` joined onto the cache directory is somewhere else.
    fn paths(&self, domain: &str) -> ProxyResult<(PathBuf, PathBuf)> {
        check_domain(domain)?;
        Ok((
            self.cache_dir.join(format!("{domain}.crt")),
            self.cache_dir.join(format!("{domain}.key")),
        ))
    }

    async fn load_from_file(&self, domain: &str) -> ProxyResult<Certificate> {
        let (cert_path, key_path) = self.paths(domain)?;

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
        let (cert_path, key_path) = self.paths(&cert.domain)?;

        // The chain is public; the key is not. `tokio::fs::write` created both
        // with the umask's default, which on most systems left every tenant's
        // private key readable by every local user.
        write_replacing(&cert_path, &cert.cert_pem, 0o644).await?;
        write_replacing(&key_path, &cert.key_pem, 0o600).await?;

        debug!("Saved certificate to file: {}", cert.domain);
        Ok(())
    }

    /// Read every certificate from the database, ignoring both caches.
    ///
    /// [`get`](Self::get) answers from memory once a domain has been read
    /// once, which is right for a handshake and wrong for a refresh: a
    /// certificate renewed by *another* instance would never be noticed, and
    /// this proxy would go on serving the old one until it was restarted. The
    /// database is the authority, so a refresh asks it.
    ///
    /// The in-memory cache is replaced with what was read, so the two do not
    /// then disagree.
    pub async fn load_all(&self) -> ProxyResult<Vec<Certificate>> {
        let rows: Vec<(
            String,
            Vec<u8>,
            Vec<u8>,
            Option<chrono::DateTime<chrono::Utc>>,
        )> = sqlx::query_as("SELECT domain, cert_pem, key_pem, expires_at FROM proxy_certificates")
            .fetch_all(&self.pool)
            .await?;

        let certificates: Vec<Certificate> = rows
            .into_iter()
            .map(|(domain, cert_pem, key_pem, expires_at)| Certificate {
                domain,
                cert_pem,
                key_pem,
                expires_at,
            })
            .collect();

        let mut cache = self.cache.write().await;
        *cache = certificates
            .iter()
            .map(|c| (c.domain.clone(), c.clone()))
            .collect();

        debug!(
            "Loaded {} certificate(s) from the database",
            certificates.len()
        );
        Ok(certificates)
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

/// Refuse anything but a lowercase DNS name, optionally a wildcard.
///
/// Labels of `a-z`, `0-9` and `-`, not starting or ending with `-`, at most 63
/// bytes each and 253 in all, with at most a single leading `*.`. This is what
/// stands between a tenant-supplied domain and a file path: no `/`, no `..`,
/// no NUL, nothing a filesystem treats specially.
///
/// Lowercase only, rather than lowered here: the store is keyed by the exact
/// string, and quietly accepting `Example.com` would make it a different
/// entry from `example.com` in the database while naming the same file.
pub(crate) fn check_domain(domain: &str) -> ProxyResult<()> {
    let bare = domain.strip_prefix("*.").unwrap_or(domain);
    let label_ok = |label: &str| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    };
    if domain.len() <= 253 && !bare.is_empty() && bare.split('.').all(label_ok) {
        Ok(())
    } else {
        Err(ProxyError::Validation(format!(
            "{domain:?} is not a valid lowercase DNS name"
        )))
    }
}

/// Create the cache directory, owner-only if it is new.
async fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    let mut builder = tokio::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    builder.mode(0o700);
    builder.create(dir).await?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = tokio::fs::metadata(dir).await?.permissions().mode();
        if mode & 0o077 != 0 {
            warn!(
                "Certificate directory {} is accessible to other users (mode {:o}); \
                 the keys in it are 0600, but consider making it 0700",
                dir.display(),
                mode & 0o777
            );
        }
    }
    Ok(())
}

/// Make any key file an earlier version left readable owner-only.
///
/// Only files named like ours (`<domain>.key`), so pointing `cert_dir` at a
/// directory that holds other things does not change their permissions.
async fn tighten_existing_keys(dir: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let Ok(mut entries) = tokio::fs::read_dir(dir).await else {
            return;
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name();
            let Some(domain) = name.to_str().and_then(|n| n.strip_suffix(".key")) else {
                continue;
            };
            if check_domain(domain).is_err() {
                continue;
            }
            let Ok(meta) = entry.metadata().await else {
                continue;
            };
            if !meta.is_file() || meta.permissions().mode() & 0o077 == 0 {
                continue;
            }
            let path = entry.path();
            match tokio::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).await {
                Ok(()) => info!("Restricted {} to 0600", path.display()),
                Err(e) => warn!(
                    "{} is readable by other users and could not be restricted: {}",
                    path.display(),
                    e
                ),
            }
        }
    }
    #[cfg(not(unix))]
    let _ = dir;
}

/// Write `contents` to `path` with `mode`, replacing whatever was there.
///
/// Written to a fresh file beside it and renamed into place. The mode applies
/// at creation, so the key is never readable by others even for a moment, and
/// a file that already existed with looser permissions is replaced rather
/// than rewritten with them. The rename also means a reader never sees a
/// half-written key.
async fn write_replacing(path: &Path, contents: &[u8], mode: u32) -> std::io::Result<()> {
    use tokio::io::AsyncWriteExt;

    let mut tmp_name = path.as_os_str().to_owned();
    tmp_name.push(format!(".tmp-{}", uuid::Uuid::new_v4().simple()));
    let tmp = PathBuf::from(tmp_name);

    let mut options = tokio::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(mode);
    #[cfg(not(unix))]
    let _ = mode;

    let result = async {
        let mut file = options.open(&tmp).await?;
        file.write_all(contents).await?;
        file.sync_all().await?;
        drop(file);
        tokio::fs::rename(&tmp, path).await
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&tmp).await;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_dir() -> PathBuf {
        std::env::temp_dir().join(format!("postrust-cert-store-{}", uuid::Uuid::new_v4()))
    }

    async fn store(dir: &Path) -> CertificateStore {
        // Lazy, and never connected: these exercise the file cache only.
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://nobody@127.0.0.1:1/none")
            .unwrap();
        CertificateStore::new(pool, dir).await.unwrap()
    }

    fn cert(domain: &str) -> Certificate {
        Certificate {
            domain: domain.into(),
            cert_pem: b"CERT".to_vec(),
            key_pem: b"KEY".to_vec(),
            expires_at: None,
        }
    }

    #[test]
    fn only_lowercase_dns_names_are_accepted() {
        for ok in [
            "example.com",
            "a.b.example.com",
            "xn--bcher-kva.example",
            "*.example.com",
            "a-b.c0.example",
            "localhost",
        ] {
            assert!(check_domain(ok).is_ok(), "{ok} should be accepted");
        }
        let long_label = format!("{}.com", "a".repeat(64));
        let long_name = format!("{}.com", vec!["a".repeat(60); 5].join("."));
        for bad in [
            "",
            ".",
            "..",
            "../etc/passwd",
            "a/b",
            "a\\b",
            "example.com/../../x",
            "Example.com",
            "example..com",
            ".example.com",
            "example.com.",
            "*.*.example.com",
            "a.*.example.com",
            "*",
            "*.",
            "-a.example.com",
            "a-.example.com",
            "exa mple.com",
            "example.com\0",
            "ex_ample.com",
            long_label.as_str(),
            long_name.as_str(),
        ] {
            assert!(check_domain(bad).is_err(), "{bad:?} should be refused");
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn keys_are_written_owner_only_in_an_owner_only_directory() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch_dir().join("nested");
        let store = store(&dir).await;
        store.save_to_file(&cert("a.example.com")).await.unwrap();

        let mode = |p: PathBuf| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(dir.clone()), 0o700);
        assert_eq!(mode(dir.join("a.example.com.key")), 0o600);
        // The chain is public, but never more than 0644.
        assert_eq!(mode(dir.join("a.example.com.crt")) & !0o644, 0);
        assert_eq!(
            std::fs::read(dir.join("a.example.com.key")).unwrap(),
            b"KEY"
        );

        // No temporary files left behind.
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_world_readable_key_from_an_earlier_version_is_tightened() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let key = dir.join("old.example.com.key");
        let other = dir.join("unrelated.txt");
        for path in [&key, &other] {
            std::fs::write(path, b"x").unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o644)).unwrap();
        }

        let store = store(&dir).await;
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&key), 0o600);
        // Not ours, not touched.
        assert_eq!(mode(&other), 0o644);

        // And overwriting replaces the file rather than inheriting its mode.
        std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o644)).unwrap();
        store.save_to_file(&cert("old.example.com")).await.unwrap();
        assert_eq!(mode(&key), 0o600);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_domain_cannot_name_a_path_outside_the_cache() {
        let root = scratch_dir();
        let dir = root.join("certs");
        let store = store(&dir).await;
        // A certificate sitting just outside the cache directory.
        std::fs::write(root.join("evil.crt"), b"CERT").unwrap();
        std::fs::write(root.join("evil.key"), b"KEY").unwrap();

        assert!(store.get("../evil").await.is_none());
        assert!(store.load_from_file("../evil").await.is_err());
        assert!(store.save_to_file(&cert("../evil")).await.is_err());
        assert!(store.save(cert("../evil")).await.is_err());
        assert!(store.remove("../evil").await.is_err());
        assert!(root.join("evil.key").exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
