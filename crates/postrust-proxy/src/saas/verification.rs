//! Domain verification service.
//!
//! Provides DNS TXT and HTTP challenge verification for domain ownership.

use crate::saas::types::VerificationResult;
use crate::saas::validation::is_forbidden_address;
use hickory_resolver::config::ResolverConfig;
use hickory_resolver::net::runtime::TokioRuntimeProvider;
use hickory_resolver::TokioResolver;
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

/// The most of a verification response that is read. The expected content is
/// about fifty bytes.
const MAX_BODY: usize = 1024;

/// What a tenant is told when the HTTP check could not get an answer.
///
/// Fixed, and deliberately unspecific. The underlying error -- a TLS failure,
/// a refused connection, a timeout, a status code -- is logged for the
/// operator. Returned to the tenant, those were a way to probe what answers
/// where from the proxy's vantage point.
const COULD_NOT_FETCH: &str = "Could not fetch the verification file from the domain over \
     HTTPS or HTTP. Check that it is served at the URL in the instructions, with status 200 \
     and no redirect.";

/// Why a fetch did not produce a body. For the log only.
#[derive(Debug)]
enum FetchError {
    /// The request could not be made or did not complete.
    Request(reqwest::Error),
    /// Anything but 200, redirects included: they are not followed.
    Status(reqwest::StatusCode),
    /// More than [`MAX_BODY`].
    TooLarge,
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FetchError::Request(e) => write!(f, "request failed: {e}"),
            FetchError::Status(status) => write!(f, "status {status}"),
            FetchError::TooLarge => write!(f, "response larger than {MAX_BODY} bytes"),
        }
    }
}

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

        Self { dns_resolver }
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
                    // The resolver's own error text is logged above, not
                    // returned: it describes this proxy's resolver, not the
                    // tenant's record.
                    "DNS lookup failed. Please try again later.".to_string()
                };

                VerificationResult::Failed { reason }
            }
        }
    }

    /// Verify domain ownership via HTTP challenge.
    ///
    /// The user must serve a file at `https://{domain}/.well-known/postrust-verification/{token}`
    /// (or the `http://` equivalent) with the content `postrust-verify={token}`.
    ///
    /// This makes the proxy fetch a URL on a name the tenant chose, so it is
    /// careful about where it connects:
    ///
    /// - the name is resolved here, every address in a [forbidden
    ///   range](is_forbidden_address) is discarded, and the request is pinned
    ///   to an address that survived. Pinned, because letting the HTTP client
    ///   resolve again would let a short-TTL record answer differently the
    ///   second time -- public for the check, `169.254.169.254` for the
    ///   connection.
    /// - redirects are not followed. A redirect is a second URL, on a host
    ///   nobody checked.
    /// - at most [`MAX_BODY`] bytes are read, whatever `Content-Length` says
    ///   or does not say.
    /// - what the tenant is told is a fixed sentence. Neither the fetched body
    ///   nor the client's error text is returned: either would turn this into
    ///   a way to read responses from wherever the proxy can reach.
    pub async fn verify_http(&self, domain: &str, token: &str) -> VerificationResult {
        let address = match self.resolve_public(domain).await {
            Ok(address) => address,
            Err(reason) => return VerificationResult::Failed { reason },
        };
        check_http_at(domain, token, SocketAddr::new(address, 0)).await
    }

    /// The first public address `domain` resolves to.
    async fn resolve_public(&self, domain: &str) -> Result<IpAddr, String> {
        let addresses: Vec<IpAddr> = match domain.parse::<IpAddr>() {
            Ok(ip) => vec![ip],
            Err(_) => match self.dns_resolver.lookup_ip(domain).await {
                Ok(lookup) => lookup.iter().collect(),
                Err(error) => {
                    tracing::warn!(%domain, %error, "could not resolve a domain for HTTP verification");
                    return Err("Could not resolve the domain. Check its A/AAAA records.".into());
                }
            },
        };

        match addresses
            .iter()
            .copied()
            .find(|ip| !is_forbidden_address(*ip))
        {
            Some(ip) => Ok(ip),
            None => {
                tracing::warn!(
                    %domain,
                    ?addresses,
                    "refused HTTP verification: the domain resolves only to non-public addresses"
                );
                Err("The domain does not resolve to a public address.".into())
            }
        }
    }
}

/// Fetch the challenge from `target`, as `domain`, and compare it.
///
/// `target` has already been checked; a port of 0 means the scheme's own. A
/// separate function so tests can point it at a local server, which the
/// address check would otherwise -- correctly -- refuse.
async fn check_http_at(domain: &str, token: &str, target: SocketAddr) -> VerificationResult {
    let expected_content = format!("postrust-verify={}", token);

    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .connect_timeout(Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::none())
        // A configured HTTP proxy would make its own connection, to wherever
        // it resolved the name, and the pinning would be for nothing.
        .no_proxy()
        .resolve(domain, target)
        .user_agent("PostrustProxy/1.0 DomainVerification")
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            tracing::error!(%error, "could not build the verification HTTP client");
            return VerificationResult::Failed {
                reason: "Verification could not be attempted. Please try again later.".into(),
            };
        }
    };

    // HTTPS first; plain HTTP for a domain that has no certificate yet. A
    // response that arrives but does not match is final -- only a failure to
    // get one falls through to the next scheme.
    let mut errors = Vec::new();
    for scheme in ["https", "http"] {
        let url = format!("{scheme}://{domain}/.well-known/postrust-verification/{token}");
        match fetch_capped(&client, &url).await {
            Ok(body) => {
                let content = String::from_utf8_lossy(&body);
                if content.trim() == expected_content {
                    tracing::info!(%domain, %scheme, "HTTP verification successful");
                    return VerificationResult::Verified;
                }
                tracing::warn!(%domain, %scheme, "HTTP verification content mismatch");
                return VerificationResult::Failed {
                    reason: "The verification file was fetched, but its content did not match."
                        .into(),
                };
            }
            Err(error) => errors.push(format!("{scheme}: {error}")),
        }
    }

    tracing::warn!(%domain, errors = ?errors, "HTTP verification could not fetch the challenge");
    VerificationResult::Failed {
        reason: COULD_NOT_FETCH.into(),
    }
}

/// GET `url` and read at most [`MAX_BODY`] bytes of a 200 response.
///
/// Streamed, with the cap applied to what arrives: `Content-Length` is
/// checked first as a shortcut, but a chunked response has none, and one that
/// lies would otherwise be read to the end.
async fn fetch_capped(client: &reqwest::Client, url: &str) -> Result<Vec<u8>, FetchError> {
    let mut response = client.get(url).send().await.map_err(FetchError::Request)?;

    if response.status() != reqwest::StatusCode::OK {
        return Err(FetchError::Status(response.status()));
    }
    if response
        .content_length()
        .is_some_and(|len| len > MAX_BODY as u64)
    {
        return Err(FetchError::TooLarge);
    }

    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(FetchError::Request)? {
        if body.len() + chunk.len() > MAX_BODY {
            return Err(FetchError::TooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
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

    fn reason(result: VerificationResult) -> String {
        match result {
            VerificationResult::Failed { reason } => reason,
            other => panic!("expected a failure, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn loopback_and_metadata_addresses_are_never_fetched() {
        let service = DomainVerificationService::new();
        // IP literals, and `localhost`, which the resolver answers itself
        // without asking the network (RFC 6761).
        for target in [
            "127.0.0.1",
            "169.254.169.254",
            "::1",
            "10.0.0.1",
            "localhost",
        ] {
            let reason = reason(service.verify_http(target, "tok").await);
            assert!(
                reason.contains("public address"),
                "{target}: unexpected reason {reason:?}"
            );
        }
    }

    /// A local server for the HTTP checks, with whatever routes the test
    /// gives it, and a counter of requests to `/landed`.
    async fn serve(
        app: axum::Router,
    ) -> (SocketAddr, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let landed = std::sync::Arc::new(AtomicUsize::new(0));
        let counter = landed.clone();
        let app = app.route(
            "/landed",
            axum::routing::get(move || {
                counter.fetch_add(1, Ordering::SeqCst);
                async { "postrust-verify=tok" }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.ok() });
        (addr, landed)
    }

    const PATH: &str = "/.well-known/postrust-verification/tok";

    #[tokio::test]
    async fn the_right_content_verifies() {
        let app = axum::Router::new().route(
            PATH,
            axum::routing::get(|| async { "postrust-verify=tok\n" }),
        );
        let (addr, _) = serve(app).await;
        assert!(matches!(
            check_http_at("victim.test", "tok", addr).await,
            VerificationResult::Verified
        ));
    }

    #[tokio::test]
    async fn redirects_are_not_followed() {
        use std::sync::atomic::Ordering;
        let app = axum::Router::new().route(
            PATH,
            axum::routing::get(|| async { axum::response::Redirect::temporary("/landed") }),
        );
        let (addr, landed) = serve(app).await;

        let reason = reason(check_http_at("victim.test", "tok", addr).await);
        assert_eq!(reason, COULD_NOT_FETCH);
        assert_eq!(
            landed.load(Ordering::SeqCst),
            0,
            "the redirect target was fetched"
        );
    }

    #[tokio::test]
    async fn the_fetched_body_is_never_echoed() {
        let app = axum::Router::new().route(
            PATH,
            axum::routing::get(|| async { "SECRET-INTERNAL-RESPONSE" }),
        );
        let (addr, _) = serve(app).await;

        let reason = reason(check_http_at("victim.test", "tok", addr).await);
        assert!(!reason.contains("SECRET"), "{reason}");
        assert!(reason.contains("did not match"), "{reason}");
    }

    #[tokio::test]
    async fn an_oversized_body_is_refused_without_a_content_length() {
        // Chunked, so there is no Content-Length to check up front: the cap
        // has to be applied to what is actually read.
        let app = axum::Router::new().route(
            PATH,
            axum::routing::get(|| async {
                let chunks = futures::stream::iter(
                    (0..64).map(|_| Ok::<_, std::io::Error>(bytes::Bytes::from(vec![b'a'; 1024]))),
                );
                axum::body::Body::from_stream(chunks)
            }),
        );
        let (addr, _) = serve(app).await;

        let client = reqwest::Client::builder()
            .no_proxy()
            .resolve("victim.test", addr)
            .build()
            .unwrap();
        let fetched = fetch_capped(&client, &format!("http://victim.test{PATH}")).await;
        assert!(matches!(fetched, Err(FetchError::TooLarge)), "{fetched:?}");
    }

    #[tokio::test]
    async fn connection_errors_are_not_echoed() {
        // Nothing listens here: bind, note the port, close.
        let addr = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.local_addr().unwrap()
        };
        let reason = reason(check_http_at("victim.test", "tok", addr).await);
        assert_eq!(reason, COULD_NOT_FETCH);
    }
}
