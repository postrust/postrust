//! The model file cache downloading from a local HTTP server. No database.

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use common::{serve, unique};
use postrust_ai::cache::FileCache;
use postrust_ai::config::CacheConfig;
use postrust_ai::model::ModelFile;
use postrust_ai::AiError;
use serde_json::json;
use sha2::{Digest, Sha256};

const BODY: &[u8] = b"pretend these are model weights";

/// Serves `BODY` at `/m.bin`, counting requests.
async fn weights_server() -> (String, Arc<AtomicUsize>) {
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = hits.clone();
    let app = axum::Router::new().route(
        "/m.bin",
        axum::routing::get(move || {
            let counter = counter.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                BODY
            }
        }),
    );
    (serve(app).await, hits)
}

fn sha(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// `ModelFile` is `#[non_exhaustive]`, so build it the way config does.
fn file(url: &str, sha256: Option<&str>) -> ModelFile {
    serde_json::from_value(json!({ "url": url, "sha256": sha256 })).unwrap()
}

async fn cache(shared: bool) -> (FileCache, std::path::PathBuf) {
    let mut config = CacheConfig::default();
    config.dir = std::env::temp_dir().join(unique("postrust-ai-files"));
    config.shared = shared;
    let dir = config.dir.clone();
    let cache = FileCache::open(&config, reqwest::Client::new())
        .await
        .unwrap();
    (cache, dir)
}

fn entries(dir: &std::path::Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .collect()
}

#[tokio::test]
async fn a_matching_checksum_is_cached_pinned_and_not_fetched_again() {
    let (url, hits) = weights_server().await;
    let (cache, _dir) = cache(false).await;
    let wanted = file(&format!("{url}/m.bin"), Some(&sha(BODY)));

    let paths = cache
        .ensure_all(std::slice::from_ref(&wanted))
        .await
        .unwrap();
    assert_eq!(std::fs::read(&paths[0]).unwrap(), BODY);
    assert_eq!(
        paths[0].file_name().unwrap().to_string_lossy(),
        format!("sha256-{}.bin", sha(BODY)),
        "named by checksum, keeping the extension"
    );
    let listed = cache.list().await.unwrap();
    assert_eq!(listed.len(), 1);
    assert!(listed[0].pinned, "a file in use is pinned");
    assert_eq!(listed[0].size, BODY.len() as u64);

    cache.unpin(std::slice::from_ref(&wanted));
    assert!(!cache.list().await.unwrap()[0].pinned);

    let again = cache
        .ensure_all(std::slice::from_ref(&wanted))
        .await
        .unwrap();
    assert_eq!(again, paths);
    assert_eq!(hits.load(Ordering::SeqCst), 1, "the second ensure is a hit");
}

#[tokio::test]
async fn a_checksum_mismatch_is_refused_and_leaves_nothing_behind() {
    let (url, _) = weights_server().await;
    let (cache, dir) = cache(false).await;
    let wrong = sha(b"something else");
    let bad = file(&format!("{url}/m.bin"), Some(&wrong));

    let error = cache
        .ensure_all(std::slice::from_ref(&bad))
        .await
        .unwrap_err();
    match error {
        AiError::Checksum {
            expected, actual, ..
        } => {
            assert_eq!(expected, wrong);
            assert_eq!(actual, sha(BODY));
        }
        other => panic!("wanted a checksum error, got {other}"),
    }
    assert!(entries(&dir.join("blobs")).is_empty(), "no blob");
    assert!(entries(&dir.join("tmp")).is_empty(), "no partial download");
    assert!(cache.list().await.unwrap().is_empty());

    // The failed ensure released its pin, so the same file can be retried
    // (and, with the right checksum, succeeds).
    let good = file(&format!("{url}/m.bin"), Some(&sha(BODY)));
    cache.ensure_all(&[good]).await.unwrap();
}

#[tokio::test]
async fn shared_directories_download_under_a_lock_file_and_release_it() {
    let (url, hits) = weights_server().await;
    let (cache, dir) = cache(true).await;
    let wanted = file(&format!("{url}/m.bin"), Some(&sha(BODY)));

    cache.ensure_all(&[wanted]).await.unwrap();
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    assert!(
        entries(&dir.join("locks")).is_empty(),
        "the lock is removed once the blob is in place"
    );
}

#[tokio::test]
async fn a_shared_download_waits_for_another_node_and_uses_its_blob() {
    let (url, hits) = weights_server().await;
    let (cache, dir) = cache(true).await;
    let digest = sha(BODY);
    let wanted = file(&format!("{url}/m.bin"), Some(&digest));

    // Another node holds the lock and is mid-download.
    let key = format!("sha256-{digest}.bin");
    std::fs::write(dir.join("locks").join(format!("{key}.lock")), b"").unwrap();
    let blob = dir.join("blobs").join(&key);
    tokio::spawn({
        let blob = blob.clone();
        async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            std::fs::write(blob, BODY).unwrap();
        }
    });

    let paths = cache.ensure_all(&[wanted]).await.unwrap();
    assert_eq!(paths, vec![blob]);
    assert_eq!(
        hits.load(Ordering::SeqCst),
        0,
        "this node did not download what the other one finished"
    );
}
