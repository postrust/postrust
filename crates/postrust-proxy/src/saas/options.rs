//! Settings for the SaaS control plane.
//!
//! Gathered into one place so the defaults are visible together and an
//! embedder changes them by naming them, rather than by finding which
//! constructor took which positional argument.

use crate::saas::validation::ReservedDomains;
use std::sync::Arc;
use std::time::Duration;

/// The audience a proxy JWT must name when none is configured.
///
/// Required, rather than optional, because the main Postrust server may be
/// configured with the same signing secret: without an audience, a token
/// minted for it would be accepted here too.
pub const DEFAULT_JWT_AUDIENCE: &str = "postrust-proxy";

/// Everything [`SaasState`](crate::saas::handlers::SaasState) is configured
/// with.
///
/// `#[non_exhaustive]`, so build it from [`Default`] and set fields:
///
/// ```
/// # use postrust_proxy::saas::SaasOptions;
/// let mut options = SaasOptions::default();
/// options.jwt_secret = Some("secret".into());
/// ```
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct SaasOptions {
    /// HMAC secret for proxy JWTs. `None` refuses every JWT.
    pub jwt_secret: Option<String>,
    /// The `aud` a JWT must carry. Defaults to [`DEFAULT_JWT_AUDIENCE`].
    pub jwt_audience: String,
    /// The `iss` a JWT must carry, if set. Unchecked by default.
    pub jwt_issuer: Option<String>,
    /// Rules for domains: what may be registered and how verification runs.
    pub domains: DomainPolicy,
}

impl Default for SaasOptions {
    fn default() -> Self {
        Self {
            jwt_secret: None,
            jwt_audience: DEFAULT_JWT_AUDIENCE.to_string(),
            jwt_issuer: None,
            domains: DomainPolicy::default(),
        }
    }
}

/// Rules for registering, verifying and certifying domains.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct DomainPolicy {
    /// Names no tenant may register. Put the platform's own hostnames here.
    pub reserved: ReservedDomains,
    /// How long an unverified registration holds its name.
    ///
    /// After this a pending (or failed) row stops blocking the name: the next
    /// registration of it by anyone removes the stale row, and the stale row
    /// can no longer be verified. Otherwise a tenant could register a name it
    /// does not control and hold it against its real owner indefinitely.
    /// The verification challenge expires at the same moment. Seven days.
    pub pending_ttl: Duration,
    /// The shortest gap allowed between two verification attempts for one
    /// domain. Each attempt makes outbound DNS or HTTP requests. Ten seconds.
    pub verification_min_interval: Duration,
    /// How many verification attempts one registration gets. After that, the
    /// tenant has to delete and re-add the domain. Fifty.
    pub verification_max_attempts: i32,
    /// What an uploaded certificate chain must chain to.
    pub upload_trust: UploadTrust,
}

impl Default for DomainPolicy {
    fn default() -> Self {
        Self {
            reserved: ReservedDomains::default(),
            pending_ttl: Duration::from_secs(7 * 24 * 60 * 60),
            verification_min_interval: Duration::from_secs(10),
            verification_max_attempts: 50,
            upload_trust: UploadTrust::PublicRoots,
        }
    }
}

/// What an uploaded certificate chain has to be trusted by.
#[derive(Clone, Debug)]
pub enum UploadTrust {
    /// The Mozilla root programme, as bundled by `webpki-roots`. The default:
    /// the certificate is served to browsers, which will trust nothing else.
    PublicRoots,
    /// These roots, for a platform whose tenants use an internal CA.
    Roots(Arc<rustls::RootCertStore>),
    /// No chain check. The key must still match, the leaf must still be in
    /// date and cover the domain, and the domain must still be verified.
    Unchecked,
}
