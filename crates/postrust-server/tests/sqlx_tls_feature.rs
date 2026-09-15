use std::{fs, path::Path};

#[test]
fn workspace_sqlx_dependency_enables_tls() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace_manifest_path = manifest_dir
        .ancestors()
        .nth(2)
        .expect("postrust-server should live under crates/postrust-server")
        .join("Cargo.toml");

    let workspace_manifest =
        fs::read_to_string(&workspace_manifest_path).expect("failed to read workspace Cargo.toml");

    let sqlx_dependency = workspace_manifest
        .lines()
        .find(|line| line.trim_start().starts_with("sqlx ="))
        .expect("workspace Cargo.toml should declare the sqlx dependency");

    assert!(
        sqlx_dependency.contains("\"tls-rustls-ring-native-roots\""),
        "workspace sqlx dependency must enable TLS support for sslmode=require database URLs"
    );
}
