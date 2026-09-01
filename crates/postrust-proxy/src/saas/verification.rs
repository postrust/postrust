//! Domain verification service.
//!
//! Provides DNS TXT and HTTP challenge verification for domain ownership.

use crate::saas::types::VerificationResult;
use hickory_resolver::config::ResolverConfig;
use hickory_resolver::net::runtime::TokioRuntimeProvider;
use hickory_resolver::TokioResolver;
use std::time::Duration;

/// Whether a TXT record's rendered rdata carries the expected challenge value.
///
/// Worth its own function because the rendering is not the raw value. A TXT
/// record's rdata renders quoted (`"postrust-verify=abc"`), and a record with
/// several character strings renders them space-separated and each quoted, so
/// comparing the rendered form directly against a bare token fails on a record
/// that is in fact correct.
fn txt_matches(rendered: &str, expected: &str) -> bool {
    if rendered == expected {
        return true;
    }
    // A single quoted string, which is the ordinary case.
    if rendered.trim().trim_matches('"').trim() == expected {
        return true;
    }
    // A long value is split into several 255-byte character strings, each
    // rendered quoted. DNS defines the value as their concatenation.
    let joined: String = rendered
        .split_whitespace()
        .map(|part| part.trim_matches('"'))
        .collect();
    joined == expected
}

/// Domain verification service for DNS and HTTP challenges.
pub struct DomainVerificationService {
    dns_resolver: TokioResolver,
    http_client: reqwest::Client,
}

impl DomainVerificationService {
    /// Create a new domain verification service.
    pub fn new() -> Self {
        // Create DNS resolver with the default config.
        //
        // `builder_with_config` rather than `builder`: the latter reads
        // /etc/resolv.conf, and a verification service should not refuse to
        // construct because the host's resolver file is unreadable. The
        // default config is what this used before hickory 0.26.
        let dns_resolver = TokioResolver::builder_with_config(
            ResolverConfig::default(),
            TokioRuntimeProvider::default(),
        )
        .build()
        .expect("Failed to create DNS resolver");

        // Create HTTP client with reasonable timeouts
        let http_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .connect_timeout(Duration::from_secs(5))
            .redirect(reqwest::redirect::Policy::limited(3))
            .user_agent("PostrustProxy/1.0 DomainVerification")
            .build()
            .expect("Failed to create HTTP client");

        Self {
            dns_resolver,
            http_client,
        }
    }

    /// Verify domain ownership via DNS TXT record.
    ///
    /// The user must create a TXT record at `_postrust-verification.{domain}`
    /// with the value `postrust-verify={token}`.
    pub async fn verify_dns(&self, domain: &str, token: &str) -> VerificationResult {
        let record_name = format!("_postrust-verification.{}", domain);
        let expected_value = format!("postrust-verify={}", token);

        tracing::debug!(
            record_name = %record_name,
            "Performing DNS TXT verification"
        );

        // Perform DNS TXT lookup
        match self.dns_resolver.txt_lookup(&record_name).await {
            Ok(response) => {
                // `answers()`, and `record.data()` rather than `record` --
                // hickory 0.26 replaced the rdata iterator with a record
                // slice, and `Record`'s Display renders the whole line
                // (`name ttl class type rdata`), which would never match a
                // bare token. Getting this wrong fails every verification.
                for record in response.answers() {
                    let txt_data = record.data.to_string();
                    tracing::debug!(txt_value = %txt_data, "Found TXT record");

                    if txt_matches(&txt_data, &expected_value) {
                        tracing::info!(domain = %domain, "DNS verification successful");
                        return VerificationResult::Verified;
                    }
                }

                tracing::warn!(
                    domain = %domain,
                    expected = %expected_value,
                    "DNS TXT record found but value mismatch"
                );
                VerificationResult::Failed {
                    reason: "DNS TXT record found but value does not match".into(),
                }
            }
            Err(e) => {
                tracing::warn!(
                    domain = %domain,
                    error = %e,
                    "DNS lookup failed"
                );

                // Provide helpful error messages based on error type.
                //
                // hickory 0.26 moved the taxonomy: what was
                // `ResolveErrorKind::NoRecordsFound` is now reached through
                // `NetError::is_no_records_found`, and `Timeout` is a variant
                // of `NetError` rather than of the resolver's own error.
                let reason = if e.is_no_records_found() {
                    format!(
                        "No TXT record found at {}. Please create a TXT record with value: {}",
                        record_name, expected_value
                    )
                } else if matches!(e, hickory_resolver::net::NetError::Timeout) {
                    "DNS lookup timed out. Please try again later.".to_string()
                } else {
                    format!("DNS lookup failed: {}", e)
                };

                VerificationResult::Failed { reason }
            }
        }
    }

    /// Verify domain ownership via HTTP challenge.
    ///
    /// The user must serve a file at `https://{domain}/.well-known/postrust-verification/{token}`
    /// with the content `postrust-verify={token}`.
    pub async fn verify_http(&self, domain: &str, token: &str) -> VerificationResult {
        let url = format!(
            "https://{}/.well-known/postrust-verification/{}",
            domain, token
        );
        let expected_content = format!("postrust-verify={}", token);

        tracing::debug!(url = %url, "Performing HTTP verification");

        // Try HTTPS first
        match self.fetch_verification_content(&url).await {
            Ok(content) => {
                let content_trimmed = content.trim();
                if content_trimmed == expected_content {
                    tracing::info!(domain = %domain, "HTTP verification successful");
                    VerificationResult::Verified
                } else {
                    tracing::warn!(
                        domain = %domain,
                        expected = %expected_content,
                        actual = %content_trimmed,
                        "HTTP content mismatch"
                    );
                    VerificationResult::Failed {
                        reason: format!(
                            "Content mismatch. Expected '{}' but got '{}'",
                            expected_content, content_trimmed
                        ),
                    }
                }
            }
            Err(e) => {
                // Try HTTP as fallback (for domains not yet having SSL)
                let http_url = format!(
                    "http://{}/.well-known/postrust-verification/{}",
                    domain, token
                );

                tracing::debug!(
                    url = %http_url,
                    "HTTPS failed, trying HTTP fallback"
                );

                match self.fetch_verification_content(&http_url).await {
                    Ok(content) => {
                        let content_trimmed = content.trim();
                        if content_trimmed == expected_content {
                            tracing::info!(
                                domain = %domain,
                                "HTTP verification successful (via HTTP fallback)"
                            );
                            VerificationResult::Verified
                        } else {
                            VerificationResult::Failed {
                                reason: format!(
                                    "Content mismatch. Expected '{}' but got '{}'",
                                    expected_content, content_trimmed
                                ),
                            }
                        }
                    }
                    Err(http_err) => {
                        tracing::warn!(
                            domain = %domain,
                            https_error = %e,
                            http_error = %http_err,
                            "Both HTTPS and HTTP verification failed"
                        );

                        VerificationResult::Failed {
                            reason: format!(
                                "Failed to fetch verification file. HTTPS error: {}. HTTP error: {}. \
                                 Please ensure the file is accessible at {}",
                                e, http_err, url
                            ),
                        }
                    }
                }
            }
        }
    }

    /// Fetch content from a URL for verification.
    async fn fetch_verification_content(&self, url: &str) -> Result<String, String> {
        let response = self
            .http_client
            .get(url)
            .send()
            .await
            .map_err(|e| format!("Request failed: {}", e))?;

        let status = response.status();
        if !status.is_success() {
            return Err(format!("HTTP {} response", status));
        }

        let content_length = response.content_length().unwrap_or(0);
        if content_length > 1024 {
            return Err("Response too large (max 1KB)".into());
        }

        response
            .text()
            .await
            .map_err(|e| format!("Failed to read response: {}", e))
    }
}

impl Default for DomainVerificationService {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // These cover the comparison that the hickory 0.26 migration moved. The
    // old code iterated rdata and stringified it; the new code takes
    // `record.data` off a `Record`, because `Record`'s own Display renders
    // `name ttl class type rdata` and would never match a bare token. A
    // regression here fails every DNS verification silently.

    #[test]
    fn a_quoted_txt_value_matches() {
        assert!(txt_matches(
            "\"postrust-verify=abc123\"",
            "postrust-verify=abc123"
        ));
    }

    #[test]
    fn an_unquoted_txt_value_matches() {
        assert!(txt_matches(
            "postrust-verify=abc123",
            "postrust-verify=abc123"
        ));
    }

    #[test]
    fn surrounding_whitespace_does_not_break_it() {
        assert!(txt_matches(
            "  \"postrust-verify=abc123\"  ",
            "postrust-verify=abc123"
        ));
    }

    #[test]
    fn a_value_split_across_character_strings_matches() {
        // A TXT value over 255 bytes is stored as several character strings
        // and renders as several quoted parts; DNS defines the value as their
        // concatenation.
        assert!(txt_matches(
            "\"postrust-verify=\" \"abc123\"",
            "postrust-verify=abc123"
        ));
    }

    #[test]
    fn a_different_value_does_not_match() {
        assert!(!txt_matches(
            "\"postrust-verify=wrong\"",
            "postrust-verify=abc123"
        ));
        assert!(!txt_matches("\"\"", "postrust-verify=abc123"));
        assert!(!txt_matches("", "postrust-verify=abc123"));
    }

    #[test]
    fn a_whole_record_line_does_not_match() {
        // Exactly what `record.to_string()` would have produced. If this ever
        // starts passing, the comparison has been loosened too far.
        assert!(!txt_matches(
            "_postrust-verification.example.com. 300 IN TXT \"postrust-verify=abc123\"",
            "postrust-verify=abc123"
        ));
    }

    #[tokio::test]
    async fn test_dns_verification_not_found() {
        let service = DomainVerificationService::new();

        // Use a domain that definitely won't have our verification record
        let result = service
            .verify_dns("definitely-not-a-real-domain-12345.invalid", "testtoken123")
            .await;

        match result {
            VerificationResult::Failed { reason } => {
                assert!(reason.contains("No TXT record") || reason.contains("DNS lookup failed"));
            }
            _ => panic!("Expected verification to fail for non-existent domain"),
        }
    }

    #[tokio::test]
    async fn test_http_verification_not_found() {
        let service = DomainVerificationService::new();

        // Use a domain that won't have our verification file
        let result = service.verify_http("example.com", "testtoken123").await;

        match result {
            VerificationResult::Failed { .. } => {
                // Expected to fail
            }
            _ => panic!("Expected verification to fail"),
        }
    }
}
