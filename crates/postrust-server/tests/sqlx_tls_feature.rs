//! The workspace `sqlx` dependency must keep a TLS feature enabled.
//!
//! This is not a style preference. Built without one, sqlx rejects
//! `sslmode=require` outright -- "SQLx was built without TLS support enabled"
//! -- and the default `sslmode=prefer` quietly falls back to a cleartext
//! connection instead. `docs/configuration.md` documents `sslmode=require`
//! against hosted databases, so dropping the feature would break a documented
//! configuration and silently downgrade the rest.
//!
//! Asserted against the parsed manifest rather than the compiled crate because
//! Cargo does not expose a dependency's feature selection to a dependent's
//! `cfg`. The choice of *which* TLS feature is explained where it is made, in
//! the workspace `Cargo.toml`.

use std::{fs, path::Path};

/// Any of these keeps `sslmode=require` working. The workspace picks one of
/// them for reasons recorded in `Cargo.toml`; this test only cares that it
/// picks one at all, so re-evaluating that choice does not fail the build.
const TLS_FEATURES: &[&str] = &[
    "tls-rustls",
    "tls-rustls-ring",
    "tls-rustls-ring-webpki",
    "tls-rustls-ring-native-roots",
    "tls-rustls-aws-lc-rs",
    "tls-native-tls",
    "runtime-tokio-rustls",
    "runtime-tokio-native-tls",
];

#[test]
fn workspace_sqlx_dependency_enables_tls() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("postrust-server should live under crates/postrust-server");

    let manifest: toml::Value = fs::read_to_string(workspace_root.join("Cargo.toml"))
        .expect("failed to read workspace Cargo.toml")
        .parse()
        .expect("workspace Cargo.toml is not valid TOML");

    let features = manifest
        .get("workspace")
        .and_then(|w| w.get("dependencies"))
        .and_then(|d| d.get("sqlx"))
        .expect("workspace Cargo.toml should declare the sqlx dependency")
        .get("features")
        .and_then(toml::Value::as_array)
        .expect("the sqlx dependency should declare a features list");

    let enabled: Vec<&str> = features.iter().filter_map(toml::Value::as_str).collect();

    assert!(
        enabled.iter().any(|f| TLS_FEATURES.contains(f)),
        "the workspace sqlx dependency must enable a TLS feature, or `sslmode=require` \
         database URLs stop working and `sslmode=prefer` silently connects in cleartext.\n\
         enabled: {enabled:?}\n\
         expected one of: {TLS_FEATURES:?}"
    );
}
