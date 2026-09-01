//! Checking a certificate before it is trusted with traffic.
//!
//! For certificates that arrive from outside — a tenant uploading its own —
//! rather than ones we obtained ourselves. Everything here is a check that, if
//! skipped, produces a listener that accepts the certificate happily and then
//! fails every handshake at runtime, when the person who could fix it is no
//! longer looking.

use chrono::{DateTime, Utc};

use crate::error::{ProxyError, ProxyResult};

/// What a certificate turned out to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertificateFacts {
    /// When the leaf expires.
    pub expires_at: DateTime<Utc>,
    /// Every name the leaf covers: its subject alternative names, plus the
    /// common name when there are no SANs at all.
    pub names: Vec<String>,
}

/// Check a PEM certificate chain and key, and report what they cover.
///
/// Four things, each of which has to hold before this certificate can serve a
/// domain:
///
/// 1. The chain and the key parse.
/// 2. **The key belongs to the certificate.** Delegated to rustls, which
///    compares the public keys — this is the check whose absence produces a
///    listener that fails every handshake with no clue why.
/// 3. The leaf has not expired.
/// 4. The leaf covers `domain`, wildcards included.
pub fn validate_for_domain(
    domain: &str,
    cert_pem: &[u8],
    key_pem: &[u8],
) -> ProxyResult<CertificateFacts> {
    // (1) and (2). `build_server_config` parses both and hands them to rustls,
    // which refuses a key that does not match the leaf.
    crate::tls::server::build_server_config(cert_pem, key_pem)?;

    let facts = facts(cert_pem)?;

    // (3)
    let now = Utc::now();
    if facts.expires_at <= now {
        return Err(ProxyError::Validation(format!(
            "the certificate expired at {} and cannot be used",
            facts.expires_at
        )));
    }

    // (4)
    if !facts.names.iter().any(|name| covers(name, domain)) {
        return Err(ProxyError::Validation(format!(
            "the certificate does not cover {domain}; it covers {}",
            facts.names.join(", ")
        )));
    }

    Ok(facts)
}

/// Read the expiry and the covered names out of a PEM chain.
///
/// The leaf is the first certificate: RFC 8446 section 4.4.2 requires the
/// sender's own certificate first, and every tool that writes a chain follows
/// it.
pub fn facts(cert_pem: &[u8]) -> ProxyResult<CertificateFacts> {
    let (_, pem) = x509_parser::pem::parse_x509_pem(cert_pem)
        .map_err(|e| ProxyError::Validation(format!("could not parse the certificate: {e}")))?;
    let (_, cert) = x509_parser::parse_x509_certificate(&pem.contents)
        .map_err(|e| ProxyError::Validation(format!("could not parse the certificate: {e}")))?;

    let expires_at = DateTime::from_timestamp(cert.validity().not_after.timestamp(), 0)
        .ok_or_else(|| ProxyError::Validation("the certificate's expiry is not a date".into()))?;

    let mut names: Vec<String> = cert
        .subject_alternative_name()
        .ok()
        .flatten()
        .map(|san| {
            san.value
                .general_names
                .iter()
                .filter_map(|name| match name {
                    x509_parser::extensions::GeneralName::DNSName(dns) => Some(dns.to_string()),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();

    // A certificate with no SAN at all: fall back to the common name. Modern
    // certificates always carry a SAN and browsers ignore CN entirely, but a
    // self-signed one made by hand may not, and refusing it for that alone
    // would be unhelpful.
    if names.is_empty() {
        names.extend(
            cert.subject()
                .iter_common_name()
                .filter_map(|cn| cn.as_str().ok())
                .map(str::to_owned),
        );
    }

    Ok(CertificateFacts { expires_at, names })
}

/// The expiry of a chain, or `None` if it cannot be read.
///
/// For a certificate we obtained ourselves and are storing regardless: a
/// missing expiry costs a renewal scan its scheduling hint, which is worth less
/// than discarding a certificate the CA has already issued and whose rate limit
/// we have already spent.
pub fn expiry_of(cert_pem: &[u8]) -> Option<DateTime<Utc>> {
    facts(cert_pem).ok().map(|f| f.expires_at)
}

/// Whether a certificate name covers a host.
///
/// Wildcards match exactly one label, per RFC 9110 section 4.3.4 and RFC 6125
/// section 6.4.3 — `*.example.com` covers `api.example.com` but not
/// `example.com` and not `a.b.example.com`. Getting this wrong in the generous
/// direction means accepting a certificate for a domain it does not actually
/// authorise.
fn covers(cert_name: &str, host: &str) -> bool {
    let cert_name = cert_name.trim().trim_end_matches('.').to_ascii_lowercase();
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();

    if cert_name.is_empty() || host.is_empty() {
        return false;
    }
    if cert_name == host {
        return true;
    }

    // Only a leading `*.` is a wildcard. `foo*.example.com` is not one.
    let Some(suffix) = cert_name.strip_prefix("*.") else {
        return false;
    };
    if suffix.is_empty() {
        return false;
    }
    let Some(label) = host.strip_suffix(&format!(".{suffix}")) else {
        return false;
    };
    // Exactly one label, and it must be a label rather than nothing.
    !label.is_empty() && !label.contains('.')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_exact_name_covers_its_host() {
        assert!(covers("example.com", "example.com"));
        assert!(covers("api.example.com", "api.example.com"));
    }

    #[test]
    fn matching_is_case_insensitive_and_ignores_a_trailing_dot() {
        assert!(covers("EXAMPLE.com", "example.COM"));
        assert!(covers("example.com.", "example.com"));
        assert!(covers("example.com", "example.com."));
    }

    #[test]
    fn a_wildcard_covers_exactly_one_label() {
        assert!(covers("*.example.com", "api.example.com"));
        assert!(covers("*.example.com", "www.example.com"));

        // Not the bare domain: RFC 6125 section 6.4.3.
        assert!(!covers("*.example.com", "example.com"));
        // Not two labels.
        assert!(!covers("*.example.com", "a.b.example.com"));
        // Not an empty label.
        assert!(!covers("*.example.com", ".example.com"));
    }

    #[test]
    fn a_wildcard_does_not_escape_its_suffix() {
        // The generous mistakes: a suffix match rather than a label match would
        // accept all of these.
        assert!(!covers("*.example.com", "api.example.com.evil.test"));
        assert!(!covers("*.example.com", "notexample.com"));
        assert!(!covers("*.example.com", "api.evil-example.com"));
    }

    #[test]
    fn a_partial_wildcard_is_not_a_wildcard() {
        // `foo*.example.com` is a literal name, not a pattern.
        assert!(!covers("foo*.example.com", "foobar.example.com"));
        assert!(!covers("*", "example.com"));
        assert!(!covers("*.", "example.com"));
    }

    #[test]
    fn empty_names_never_match() {
        assert!(!covers("", "example.com"));
        assert!(!covers("example.com", ""));
        assert!(!covers("", ""));
    }

    #[test]
    fn a_real_certificate_yields_its_expiry() {
        let pem = include_str!("testdata/expiry.pem");
        let facts = facts(pem.as_bytes()).expect("the fixture should parse");
        assert!(facts.expires_at > DateTime::from_timestamp(0, 0).unwrap());
        // No SAN in the fixture, so the common name is the fallback.
        assert_eq!(facts.names, vec!["expiry.test".to_string()]);
    }

    // Fixtures, generated once with openssl so these need no CA and no clock.
    // See testdata/README.md for the commands.
    const VALID: &[u8] = include_bytes!("testdata/valid.pem");
    const VALID_KEY: &[u8] = include_bytes!("testdata/valid.key.pem");
    const OTHER: &[u8] = include_bytes!("testdata/other.pem");
    const OTHER_KEY: &[u8] = include_bytes!("testdata/other.key.pem");
    const EXPIRED: &[u8] = include_bytes!("testdata/expired.pem");
    const EXPIRED_KEY: &[u8] = include_bytes!("testdata/expired.key.pem");
    const WILDCARD: &[u8] = include_bytes!("testdata/wildcard.pem");
    const WILDCARD_KEY: &[u8] = include_bytes!("testdata/wildcard.key.pem");

    #[test]
    fn a_matching_certificate_and_key_for_the_domain_is_accepted() {
        let facts = validate_for_domain("example.test", VALID, VALID_KEY)
            .expect("a valid pair for the domain should be accepted");
        assert!(facts.names.contains(&"example.test".to_string()));
        assert!(facts.expires_at > Utc::now());
    }

    #[test]
    fn a_san_other_than_the_first_still_counts() {
        // The fixture covers example.test and www.example.test.
        validate_for_domain("www.example.test", VALID, VALID_KEY)
            .expect("any SAN on the certificate should satisfy the check");
    }

    #[test]
    fn a_key_from_a_different_certificate_is_refused() {
        // The check that matters most: without it the listener accepts this and
        // then fails every TLS handshake, with nothing in the logs pointing
        // here.
        validate_for_domain("example.test", VALID, OTHER_KEY)
            .expect_err("a key that does not match the chain must be refused");
    }

    #[test]
    fn a_certificate_for_another_domain_is_refused() {
        let error = validate_for_domain("example.test", OTHER, OTHER_KEY)
            .expect_err("a certificate for another domain must be refused");
        assert!(
            error.to_string().contains("does not cover example.test"),
            "the rejection should name the domain: {error}"
        );
    }

    #[test]
    fn an_expired_certificate_is_refused() {
        let error = validate_for_domain("example.test", EXPIRED, EXPIRED_KEY)
            .expect_err("an expired certificate must be refused");
        assert!(
            error.to_string().contains("expired"),
            "the rejection should say it expired: {error}"
        );
    }

    #[test]
    fn a_wildcard_certificate_covers_a_subdomain_but_not_the_apex() {
        validate_for_domain("api.example.test", WILDCARD, WILDCARD_KEY)
            .expect("*.example.test should cover api.example.test");

        let error = validate_for_domain("example.test", WILDCARD, WILDCARD_KEY)
            .expect_err("*.example.test must not cover the apex");
        assert!(error.to_string().contains("does not cover"), "{error}");
    }

    #[test]
    fn rubbish_is_refused_rather_than_panicking() {
        assert!(facts(b"").is_err());
        assert!(facts(b"not a certificate").is_err());
        assert!(facts(b"-----BEGIN CERTIFICATE-----\nzz\n-----END CERTIFICATE-----").is_err());
        assert!(expiry_of(b"").is_none());

        // And through the whole check, which must not panic either.
        assert!(validate_for_domain("example.test", b"", b"").is_err());
        assert!(validate_for_domain("example.test", VALID, b"not a key").is_err());
    }
}
