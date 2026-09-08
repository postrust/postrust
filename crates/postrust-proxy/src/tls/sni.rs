//! Per-domain certificate selection at handshake time.
//!
//! The HTTPS listener used to be built with `with_single_cert`, which answers
//! every handshake with the same certificate whatever name the client asked
//! for. That is fine for one domain and wrong for a multi-tenant proxy: ACME
//! issued a certificate per tenant domain, stored it, renewed it, and nothing
//! ever served it. This is the piece that reads them back.
//!
//! # Why there is a second cache here
//!
//! [`rustls::server::ResolvesServerCert::resolve`] is synchronous and runs
//! inside the handshake. [`CertificateStore`] is asynchronous -- it reads a
//! database and the filesystem -- so it cannot be consulted from `resolve`
//! without blocking a runtime thread on every connection.
//!
//! So certificates are parsed into handshake-ready [`CertifiedKey`]s ahead of
//! time and held in a `std` lock that `resolve` can read directly. The cost is
//! that a newly issued certificate is served once the next refresh has run, or
//! immediately if whoever issued it calls [`SniCertResolver::refresh`].

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use tokio_rustls::rustls::crypto::aws_lc_rs::sign::any_supported_type;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio_rustls::rustls::server::{ClientHello, ResolvesServerCert};
use tokio_rustls::rustls::sign::CertifiedKey;

use crate::error::{ProxyError, ProxyResult};
use crate::tls::cert_store::CertificateStore;

/// Resolves a certificate from the store by the SNI name of the handshake.
pub struct SniCertResolver {
    /// Parsed and ready to serve, by the domain they were issued for.
    keys: RwLock<HashMap<String, Arc<CertifiedKey>>>,
    /// Served when SNI names nothing we hold, and when the client sends no
    /// SNI at all -- an IP-address client, or an old one.
    ///
    /// Without a fallback such a handshake is refused, which is the correct
    /// answer but a confusing one if the operator also configured a static
    /// certificate and expected it to work.
    fallback: Option<Arc<CertifiedKey>>,
}

impl SniCertResolver {
    /// Build a resolver holding nothing but the fallback.
    ///
    /// Call [`refresh`](Self::refresh) to populate it from the store.
    pub fn new(fallback: Option<Arc<CertifiedKey>>) -> Self {
        Self {
            keys: RwLock::new(HashMap::new()),
            fallback,
        }
    }

    /// Build the fallback from the statically configured PEM pair.
    pub fn certified_key(cert_pem: &[u8], key_pem: &[u8]) -> ProxyResult<Arc<CertifiedKey>> {
        crate::tls::server::install_crypto_provider();

        let certs: Vec<CertificateDer<'static>> =
            rustls_pemfile::certs(&mut std::io::BufReader::new(cert_pem))
                .filter_map(|r| r.ok())
                .collect();
        if certs.is_empty() {
            return Err(ProxyError::Tls("no certificates found in PEM".into()));
        }

        let key: PrivateKeyDer<'static> =
            rustls_pemfile::private_key(&mut std::io::BufReader::new(key_pem))
                .map_err(|e| ProxyError::Tls(format!("could not parse private key: {e}")))?
                .ok_or_else(|| ProxyError::Tls("no private key found in PEM".into()))?;

        let signing_key = any_supported_type(&key)
            .map_err(|e| ProxyError::Tls(format!("unsupported private key: {e}")))?;

        Ok(Arc::new(CertifiedKey::new(certs, signing_key)))
    }

    /// Reload every certificate the store holds.
    ///
    /// Returns how many are now being served. A certificate that will not
    /// parse is logged and skipped rather than failing the refresh: one bad
    /// row must not take every other tenant's TLS down with it.
    pub async fn refresh(&self, store: &CertificateStore) -> ProxyResult<usize> {
        // `load_all`, not `get` per domain: `get` answers from the store's own
        // memory cache, so a certificate another instance renewed would never
        // be seen here.
        let certificates = store.load_all().await?;
        let mut fresh = HashMap::with_capacity(certificates.len());

        for cert in certificates {
            match Self::certified_key(&cert.cert_pem, &cert.key_pem) {
                Ok(key) => {
                    fresh.insert(cert.domain, key);
                }
                Err(e) => tracing::warn!("Certificate for {} will not load: {}", cert.domain, e),
            }
        }

        let count = fresh.len();
        *self.keys.write().unwrap_or_else(|e| e.into_inner()) = fresh;
        tracing::info!("Serving {} certificate(s) by SNI", count);
        Ok(count)
    }

    /// Look up a name, exactly and then as the wildcard that would cover it.
    fn lookup(&self, name: &str) -> Option<Arc<CertifiedKey>> {
        let keys = self.keys.read().unwrap_or_else(|e| e.into_inner());
        // SNI is case-insensitive; a certificate may have been stored either
        // way, so both sides are lowered before comparing.
        let name = name.to_ascii_lowercase();
        if let Some(key) = keys.get(&name) {
            return Some(key.clone());
        }
        // `*.example.com` covers `a.example.com` and, per RFC 6125, exactly one
        // label -- not `a.b.example.com`, and not the bare `example.com`.
        let (_, parent) = name.split_once('.')?;
        if parent.contains('.') {
            if let Some(key) = keys.get(&format!("*.{parent}")) {
                return Some(key.clone());
            }
        }
        None
    }
}

impl ResolvesServerCert for SniCertResolver {
    fn resolve(&self, client_hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        match client_hello.server_name() {
            Some(name) => self.lookup(name).or_else(|| self.fallback.clone()),
            // No SNI. There is no name to match, so the fallback is the only
            // honest answer.
            None => self.fallback.clone(),
        }
    }
}

/// Names the domains held, never the keys. A private key that reaches a log is
/// a private key that has to be rotated.
impl std::fmt::Debug for SniCertResolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let keys = self.keys.read().unwrap_or_else(|e| e.into_inner());
        f.debug_struct("SniCertResolver")
            .field("domains", &keys.keys().collect::<Vec<_>>())
            .field("has_fallback", &self.fallback.is_some())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The fixtures `tls::validate` already uses; see testdata/README.md.
    // Which name a fixture covers does not matter here -- this resolver is
    // keyed by the domain a certificate was *stored* under, and checking that
    // the chain matches that name is `tls::validate`'s job, done at upload.
    const CERT: &[u8] = include_bytes!("testdata/valid.pem");
    const KEY: &[u8] = include_bytes!("testdata/valid.key.pem");

    fn resolver_with(domains: &[&str]) -> SniCertResolver {
        let resolver = SniCertResolver::new(None);
        let key = SniCertResolver::certified_key(CERT, KEY).unwrap();
        let keys = domains
            .iter()
            .map(|d| (d.to_string(), key.clone()))
            .collect();
        *resolver.keys.write().unwrap() = keys;
        resolver
    }

    #[test]
    fn an_exact_name_is_found() {
        let resolver = resolver_with(&["a.example.com"]);
        assert!(resolver.lookup("a.example.com").is_some());
        assert!(resolver.lookup("b.example.com").is_none());
    }

    #[test]
    fn sni_is_matched_case_insensitively() {
        let resolver = resolver_with(&["a.example.com"]);
        assert!(resolver.lookup("A.Example.COM").is_some());
    }

    #[test]
    fn a_wildcard_covers_exactly_one_label() {
        let resolver = resolver_with(&["*.example.com"]);
        assert!(resolver.lookup("a.example.com").is_some());
        // Two labels down is not covered by a single wildcard.
        assert!(resolver.lookup("a.b.example.com").is_none());
        // Nor is the bare parent domain.
        assert!(resolver.lookup("example.com").is_none());
    }

    #[test]
    fn a_wildcard_does_not_swallow_a_public_suffix() {
        // `*.com` must not be reachable by asking for `example.com`, or one
        // stored certificate would answer for every domain under it.
        let resolver = resolver_with(&["*.com"]);
        assert!(resolver.lookup("example.com").is_none());
    }

    #[test]
    fn the_fallback_answers_when_nothing_matches() {
        let fallback = SniCertResolver::certified_key(CERT, KEY).unwrap();
        let resolver = SniCertResolver::new(Some(fallback));
        // Nothing is loaded, so every name falls back.
        assert!(resolver.lookup("anything.example.com").is_none());
        assert!(resolver.fallback.is_some());
    }

    #[test]
    fn a_pem_that_will_not_parse_is_an_error_not_a_panic() {
        assert!(SniCertResolver::certified_key(b"nonsense", b"nonsense").is_err());
    }
}
