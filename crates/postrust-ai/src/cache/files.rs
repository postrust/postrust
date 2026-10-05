//! Layer 1: model files on local disk.
//!
//! Files are fetched once and kept under the cache directory, named by their
//! sha256 when the spec declares one and by a hash of the URL when it does
//! not. Each blob has a small `.meta` sidecar recording its size, source and
//! last use. Sidecars rather than one index file, so nodes sharing the
//! directory over NFS never contend on a single file.
//!
//! Over budget, the least recently used blobs go first, never one that a
//! loaded or loading instance is using.
//!
//! Layout:
//!
//! ```text
//! {dir}/blobs/{key}          the file
//! {dir}/blobs/{key}.meta     JSON: url, size, last_used
//! {dir}/locks/{key}.lock     held while downloading (shared directories)
//! {dir}/tmp/                 partial downloads
//! {dir}/hf/                  HF_HOME for servers that fetch their own weights
//! ```

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

use crate::config::CacheConfig;
use crate::error::{AiError, AiResult};
use crate::model::ModelFile;

/// A lock file older than this is from a node that died mid-download.
const STALE_LOCK: Duration = Duration::from_secs(3600);

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Meta {
    url: String,
    size: u64,
    last_used: u64,
}

/// One cached blob, for the API.
#[derive(Debug, Clone, Serialize)]
pub struct CachedFile {
    /// Cache key.
    pub key: String,
    /// Source.
    pub url: String,
    /// Bytes.
    pub size: u64,
    /// Seconds since the epoch.
    pub last_used: u64,
    /// In use by an instance.
    pub pinned: bool,
}

/// The model file cache.
pub struct FileCache {
    dir: PathBuf,
    budget_bytes: u64,
    shared: bool,
    http: reqwest::Client,
    /// One download per key in this process; the lock file covers the rest.
    inflight: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    /// Keys in use, with a count per user.
    pins: parking_lot::Mutex<HashMap<String, usize>>,
}

impl FileCache {
    /// Open (and create) the cache directory.
    pub async fn open(config: &CacheConfig, http: reqwest::Client) -> AiResult<Self> {
        for sub in ["blobs", "locks", "tmp", "hf"] {
            tokio::fs::create_dir_all(config.dir.join(sub)).await?;
        }
        Ok(Self {
            dir: config.dir.clone(),
            budget_bytes: config.max_disk_gb.saturating_mul(1024 * 1024 * 1024),
            shared: config.shared,
            http,
            inflight: Mutex::new(HashMap::new()),
            pins: parking_lot::Mutex::new(HashMap::new()),
        })
    }

    /// The cache directory.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Where servers that download their own weights (vLLM, via the Hugging
    /// Face hub) should put them, so they land under this cache's disk.
    pub fn hf_home(&self) -> PathBuf {
        self.dir.join("hf")
    }

    /// Make sure every file is on disk and pin them. Returns local paths in
    /// the same order. Unpin with [`unpin`](Self::unpin) when the instance
    /// stops.
    pub async fn ensure_all(&self, files: &[ModelFile]) -> AiResult<Vec<PathBuf>> {
        let mut paths = Vec::with_capacity(files.len());
        let mut pinned = Vec::new();
        for file in files {
            match self.ensure(file).await {
                Ok((path, key)) => {
                    if let Some(key) = key {
                        pinned.push(key);
                    }
                    paths.push(path);
                }
                Err(error) => {
                    self.unpin_keys(&pinned);
                    return Err(error);
                }
            }
        }
        Ok(paths)
    }

    /// Release the pins [`ensure_all`](Self::ensure_all) took.
    pub fn unpin(&self, files: &[ModelFile]) {
        let keys: Vec<String> = files.iter().filter_map(cache_key).collect();
        self.unpin_keys(&keys);
    }

    fn unpin_keys(&self, keys: &[String]) {
        let mut pins = self.pins.lock();
        for key in keys {
            if let Some(n) = pins.get_mut(key) {
                *n -= 1;
                if *n == 0 {
                    pins.remove(key);
                }
            }
        }
    }

    fn pin(&self, key: &str) {
        *self.pins.lock().entry(key.to_string()).or_default() += 1;
    }

    /// One file. Returns its path and, for cached (not `file://`) files, its
    /// key, already pinned.
    async fn ensure(&self, file: &ModelFile) -> AiResult<(PathBuf, Option<String>)> {
        if let Some(path) = file.url.strip_prefix("file://") {
            let path = PathBuf::from(path);
            if !tokio::fs::try_exists(&path).await? {
                return Err(AiError::NotFound(format!("model file {}", path.display())));
            }
            return Ok((path, None));
        }
        let key = cache_key(file).expect("non-file urls always have a key");
        // Pin before checking, so an eviction running concurrently cannot
        // delete the blob between the check and the server opening it.
        self.pin(&key);
        let result = self.ensure_key(file, &key).await;
        if result.is_err() {
            self.unpin_keys(std::slice::from_ref(&key));
        }
        result.map(|path| (path, Some(key)))
    }

    async fn ensure_key(&self, file: &ModelFile, key: &str) -> AiResult<PathBuf> {
        let blob = self.blob_path(key);
        let gate = {
            let mut inflight = self.inflight.lock().await;
            inflight.entry(key.to_string()).or_default().clone()
        };
        let _held = gate.lock().await;

        if tokio::fs::try_exists(&blob).await? {
            self.touch(key, &file.url).await;
            return Ok(blob);
        }

        let lock = if self.shared {
            Some(self.acquire_lock(key, &blob).await?)
        } else {
            None
        };
        // Another node may have finished it while we waited for the lock.
        let result = if tokio::fs::try_exists(&blob).await? {
            Ok(())
        } else {
            self.download(file, key, &blob).await
        };
        if let Some(lock) = lock {
            let _ = tokio::fs::remove_file(lock).await;
        }
        result?;
        self.touch(key, &file.url).await;
        Ok(blob)
    }

    async fn acquire_lock(&self, key: &str, blob: &Path) -> AiResult<PathBuf> {
        let lock = self.dir.join("locks").join(format!("{key}.lock"));
        loop {
            match tokio::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&lock)
                .await
            {
                Ok(_) => return Ok(lock),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    if tokio::fs::try_exists(blob).await? {
                        // The holder finished; we will see the blob.
                        return Ok(lock);
                    }
                    let stale = tokio::fs::metadata(&lock)
                        .await
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|t| t.elapsed().ok())
                        .is_some_and(|age| age > STALE_LOCK);
                    if stale {
                        tracing::warn!(lock = %lock.display(), "removing stale download lock");
                        let _ = tokio::fs::remove_file(&lock).await;
                        continue;
                    }
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
                Err(e) => return Err(e.into()),
            }
        }
    }

    async fn download(&self, file: &ModelFile, key: &str, blob: &Path) -> AiResult<()> {
        let url = resolve_url(&file.url)?;
        tracing::info!(%url, key, "downloading model file");
        let mut request = self.http.get(&url);
        if url.starts_with("https://huggingface.co/") {
            if let Ok(token) = std::env::var("HF_TOKEN") {
                request = request.bearer_auth(token);
            }
        }
        let response = request.send().await?.error_for_status()?;
        if let Some(len) = response.content_length() {
            self.make_room(len).await?;
        }

        let tmp = self.dir.join("tmp").join(uuid::Uuid::new_v4().to_string());
        let mut out = tokio::fs::File::create(&tmp).await?;
        let mut hasher = Sha256::new();
        let mut size = 0u64;
        let mut stream = response.bytes_stream();
        let written: AiResult<()> = async {
            while let Some(chunk) = stream.next().await {
                let chunk = chunk?;
                hasher.update(&chunk);
                size += chunk.len() as u64;
                out.write_all(&chunk).await?;
            }
            out.flush().await?;
            out.sync_all().await?;
            Ok(())
        }
        .await;
        drop(out);
        if let Err(error) = written {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(error);
        }

        let actual = hex::encode(hasher.finalize());
        if let Some(expected) = &file.sha256 {
            if !expected.eq_ignore_ascii_case(&actual) {
                let _ = tokio::fs::remove_file(&tmp).await;
                return Err(AiError::Checksum {
                    url: file.url.clone(),
                    expected: expected.clone(),
                    actual,
                });
            }
        }
        // Rename is atomic within a filesystem, so a reader sees either no
        // blob or a whole one.
        tokio::fs::rename(&tmp, blob).await?;
        tracing::info!(key, size, "model file cached");
        Ok(())
    }

    async fn touch(&self, key: &str, url: &str) {
        let size = tokio::fs::metadata(self.blob_path(key))
            .await
            .map(|m| m.len())
            .unwrap_or(0);
        let meta = Meta {
            url: url.to_string(),
            size,
            last_used: now_secs(),
        };
        if let Ok(json) = serde_json::to_vec(&meta) {
            let _ = tokio::fs::write(self.meta_path(key), json).await;
        }
    }

    /// Evict least recently used, unpinned blobs until `incoming` more bytes
    /// fit the budget. Fails if pinned blobs alone exceed it.
    pub async fn make_room(&self, incoming: u64) -> AiResult<()> {
        let mut entries = self.list().await?;
        let mut used: u64 = entries.iter().map(|e| e.size).sum();
        if used + incoming <= self.budget_bytes {
            return Ok(());
        }
        entries.sort_by_key(|e| e.last_used);
        for entry in entries.iter().filter(|e| !e.pinned) {
            if used + incoming <= self.budget_bytes {
                break;
            }
            tracing::info!(key = %entry.key, size = entry.size, "evicting cached model file");
            let _ = tokio::fs::remove_file(self.blob_path(&entry.key)).await;
            let _ = tokio::fs::remove_file(self.meta_path(&entry.key)).await;
            used = used.saturating_sub(entry.size);
        }
        if used + incoming > self.budget_bytes {
            return Err(AiError::InsufficientResources {
                model: "model file cache".into(),
                detail: format!(
                    "{} bytes needed, {} of {} in use by loaded models",
                    incoming, used, self.budget_bytes
                ),
            });
        }
        Ok(())
    }

    /// Everything cached.
    pub async fn list(&self) -> AiResult<Vec<CachedFile>> {
        let pins: HashSet<String> = self.pins.lock().keys().cloned().collect();
        let mut out = Vec::new();
        let mut dir = tokio::fs::read_dir(self.dir.join("blobs")).await?;
        while let Some(entry) = dir.next_entry().await? {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.ends_with(".meta") {
                continue;
            }
            let size = entry.metadata().await.map(|m| m.len()).unwrap_or(0);
            let meta: Option<Meta> = tokio::fs::read(self.meta_path(&name))
                .await
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok());
            out.push(CachedFile {
                pinned: pins.contains(&name),
                url: meta.as_ref().map(|m| m.url.clone()).unwrap_or_default(),
                last_used: meta.map(|m| m.last_used).unwrap_or(0),
                key: name,
                size,
            });
        }
        Ok(out)
    }

    fn blob_path(&self, key: &str) -> PathBuf {
        self.dir.join("blobs").join(key)
    }

    fn meta_path(&self, key: &str) -> PathBuf {
        self.dir.join("blobs").join(format!("{key}.meta"))
    }
}

/// The cache key for a file, or `None` for `file://`, which is never cached.
fn cache_key(file: &ModelFile) -> Option<String> {
    if file.url.starts_with("file://") {
        return None;
    }
    // Keep the extension: some servers sniff the format from the file name.
    let ext = Path::new(file.url.split(['?', '#']).next().unwrap_or(""))
        .extension()
        .and_then(|e| e.to_str())
        .filter(|e| e.len() <= 12 && e.chars().all(|c| c.is_ascii_alphanumeric()))
        .map(|e| format!(".{e}"))
        .unwrap_or_default();
    Some(match &file.sha256 {
        Some(sha) => format!("sha256-{}{ext}", sha.to_ascii_lowercase()),
        None => format!(
            "url-{}{ext}",
            hex::encode(Sha256::digest(file.url.as_bytes()))
        ),
    })
}

/// Turn `hf://org/repo[@rev]/path` into its download URL.
pub fn resolve_url(url: &str) -> AiResult<String> {
    let Some(rest) = url.strip_prefix("hf://") else {
        return Ok(url.to_string());
    };
    let mut parts = rest.splitn(3, '/');
    let (Some(org), Some(repo), Some(path)) = (parts.next(), parts.next(), parts.next()) else {
        return Err(AiError::Config(format!(
            "'{url}' is not hf://org/repo[@revision]/path"
        )));
    };
    let (repo, revision) = repo.split_once('@').unwrap_or((repo, "main"));
    Ok(format!(
        "https://huggingface.co/{org}/{repo}/resolve/{revision}/{path}"
    ))
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(url: &str, sha: Option<&str>) -> ModelFile {
        ModelFile {
            url: url.into(),
            sha256: sha.map(Into::into),
        }
    }

    #[test]
    fn hf_urls_resolve() {
        assert_eq!(
            resolve_url("hf://ggerganov/whisper.cpp/ggml-base.en.bin").unwrap(),
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin"
        );
        assert_eq!(
            resolve_url("hf://org/repo@v2/sub/dir/w.gguf").unwrap(),
            "https://huggingface.co/org/repo/resolve/v2/sub/dir/w.gguf"
        );
        assert!(resolve_url("hf://org").is_err());
    }

    #[test]
    fn keys_prefer_the_checksum_and_keep_the_extension() {
        let sha = "AB".repeat(32);
        assert_eq!(
            cache_key(&file("https://x/m.gguf", Some(&sha))).unwrap(),
            format!("sha256-{}.gguf", "ab".repeat(32))
        );
        let by_url = cache_key(&file("https://x/m.bin?download=1", None)).unwrap();
        assert!(
            by_url.starts_with("url-") && by_url.ends_with(".bin"),
            "{by_url}"
        );
        assert!(cache_key(&file("file:///models/m.bin", None)).is_none());
    }

    #[tokio::test]
    async fn eviction_skips_pinned_blobs() {
        let dir = std::env::temp_dir().join(format!("postrust-ai-cache-{}", uuid::Uuid::new_v4()));
        let config = CacheConfig {
            dir: dir.clone(),
            max_disk_gb: 0,
            ..Default::default()
        };
        let cache = FileCache::open(&config, reqwest::Client::new())
            .await
            .unwrap();

        for key in ["old", "pinned"] {
            tokio::fs::write(cache.blob_path(key), vec![0u8; 10])
                .await
                .unwrap();
            cache.touch(key, "https://x").await;
        }
        cache.pin("pinned");

        // A zero budget cannot be met while a blob is pinned...
        assert!(cache.make_room(0).await.is_err());
        // ...but everything unpinned was evicted trying.
        let left: Vec<String> = cache
            .list()
            .await
            .unwrap()
            .into_iter()
            .map(|f| f.key)
            .collect();
        assert_eq!(left, vec!["pinned".to_string()]);

        tokio::fs::remove_dir_all(dir).await.unwrap();
    }
}
