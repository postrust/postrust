//! TLS server configuration for the HTTPS listener.
//!
//! Kept out of `acme.rs` deliberately: serving TLS from a certificate on disk
//! should not require the `acme` feature, which pulls in `rustls-acme` and is
//! compiled out of some builds.

use std::io::BufReader;
use std::sync::Arc;

use tokio_rustls::rustls::ServerConfig;

use crate::error::{ProxyError, ProxyResult};

/// ALPN protocols offered, in preference order.
///
/// This is what makes HTTP/2 reachable the way browsers actually use it. Without
/// it a TLS listener negotiates nothing and every client falls back to
/// HTTP/1.1, leaving h2 available only as cleartext h2c.
pub const ALPN_PROTOCOLS: [&[u8]; 2] = [b"h2", b"http/1.1"];

/// Select the rustls crypto provider for this process.
///
/// Both `aws-lc-rs` and `ring` end up in the dependency tree -- rustls is
/// configured for the former, reqwest pulls the latter -- so rustls refuses to
/// guess and panics on first use unless one is installed explicitly. Idempotent:
/// a second call returns Err and is ignored.
pub(crate) fn install_crypto_provider() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let _ = tokio_rustls::rustls::crypto::aws_lc_rs::default_provider().install_default();
    });
}

/// Build a rustls `ServerConfig` from PEM bytes, with ALPN configured.
pub fn build_server_config(cert_pem: &[u8], key_pem: &[u8]) -> ProxyResult<Arc<ServerConfig>> {
    install_crypto_provider();

    let certs: Vec<_> = rustls_pemfile::certs(&mut BufReader::new(cert_pem))
        .filter_map(|r| r.ok())
        .collect();

    if certs.is_empty() {
        return Err(ProxyError::Tls("no certificates found in PEM".into()));
    }

    let key = rustls_pemfile::private_key(&mut BufReader::new(key_pem))
        .map_err(|e| ProxyError::Tls(format!("could not parse private key: {}", e)))?
        .ok_or_else(|| ProxyError::Tls("no private key found in PEM".into()))?;

    let mut config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| ProxyError::Tls(format!("could not build TLS config: {}", e)))?;

    config.alpn_protocols = ALPN_PROTOCOLS.iter().map(|p| p.to_vec()).collect();

    Ok(Arc::new(config))
}

/// Build a `ServerConfig` that picks its certificate per handshake.
///
/// The single-certificate form above cannot serve a multi-tenant proxy: it
/// answers every handshake with the same chain whatever name was asked for.
/// See [`crate::tls::SniCertResolver`].
pub fn build_server_config_with_resolver(
    resolver: Arc<dyn tokio_rustls::rustls::server::ResolvesServerCert>,
) -> Arc<ServerConfig> {
    install_crypto_provider();

    let mut config = ServerConfig::builder()
        .with_no_client_auth()
        .with_cert_resolver(resolver);

    config.alpn_protocols = ALPN_PROTOCOLS.iter().map(|p| p.to_vec()).collect();

    Arc::new(config)
}

/// Load a certificate and key from disk and build a `ServerConfig`.
pub async fn load_server_config(cert_file: &str, key_file: &str) -> ProxyResult<Arc<ServerConfig>> {
    let cert_pem = tokio::fs::read(cert_file)
        .await
        .map_err(|e| ProxyError::Tls(format!("could not read {}: {}", cert_file, e)))?;
    let key_pem = tokio::fs::read(key_file)
        .await
        .map_err(|e| ProxyError::Tls(format!("could not read {}: {}", key_file, e)))?;
    build_server_config(&cert_pem, &key_pem)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_alpn_offers_h2_first() {
        // Order is preference order. If h2 is dropped or demoted, HTTP/2 stops
        // being reachable over TLS and every client silently falls back to
        // HTTP/1.1 -- which is exactly the state this module was added to fix.
        assert_eq!(ALPN_PROTOCOLS[0], b"h2");
        assert_eq!(ALPN_PROTOCOLS[1], b"http/1.1");
    }

    #[test]
    fn test_rejects_pem_without_certificate() {
        let err = build_server_config(b"not a certificate", b"not a key").unwrap_err();
        assert!(
            err.to_string().contains("no certificates"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn test_rejects_certificate_without_key() {
        // A well-formed but empty cert block, and no key at all.
        let cert = b"-----BEGIN CERTIFICATE-----\nMA==\n-----END CERTIFICATE-----\n";
        let err = build_server_config(cert, b"").unwrap_err();
        assert!(
            err.to_string().contains("no private key"),
            "unexpected error: {err}"
        );
    }
}
