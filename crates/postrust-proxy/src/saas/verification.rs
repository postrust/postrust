//! Domain verification service.
//!
//! Provides DNS TXT and HTTP challenge verification for domain ownership.

use crate::saas::types::VerificationResult;
use hickory_resolver::config::{ResolverConfig, ResolverOpts};
use hickory_resolver::TokioAsyncResolver;
use std::time::Duration;

/// Domain verification service for DNS and HTTP challenges.
pub struct DomainVerificationService {
    dns_resolver: TokioAsyncResolver,
    http_client: reqwest::Client,
}

impl DomainVerificationService {
    /// Create a new domain verification service.
    pub fn new() -> Self {
        // Create DNS resolver with system config
        let dns_resolver =
            TokioAsyncResolver::tokio(ResolverConfig::default(), ResolverOpts::default());

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
                // Check each TXT record
                for record in response.iter() {
                    let txt_data = record.to_string();
                    tracing::debug!(txt_value = %txt_data, "Found TXT record");

                    // TXT records may be quoted, so check both with and without quotes
                    let txt_clean = txt_data.trim_matches('"').trim();

                    if txt_clean == expected_value || txt_data == expected_value {
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
                let error_kind = e.kind();
                tracing::warn!(
                    domain = %domain,
                    error = %e,
                    kind = ?error_kind,
                    "DNS lookup failed"
                );

                // Provide helpful error messages based on error type
                let reason = match error_kind {
                    hickory_resolver::error::ResolveErrorKind::NoRecordsFound { .. } => {
                        format!(
                            "No TXT record found at {}. Please create a TXT record with value: {}",
                            record_name, expected_value
                        )
                    }
                    hickory_resolver::error::ResolveErrorKind::Timeout => {
                        "DNS lookup timed out. Please try again later.".into()
                    }
                    _ => format!("DNS lookup failed: {}", e),
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
