//! JWT authentication for Postrust.
//!
//! Provides JWT token validation and role extraction for PostgreSQL RLS.

#![warn(missing_docs)]

// jsonwebtoken 10 on wasm32 (and natively under the test-only
// `jwt-rust-crypto` feature), under another name so it cannot be mistaken for
// the native one; see Cargo.toml. Everything below says `jsonwebtoken`.
#[cfg(any(target_arch = "wasm32", feature = "jwt-rust-crypto"))]
extern crate jsonwebtoken10 as jsonwebtoken;

mod cache;
mod claims;
pub mod hasura;
pub mod jwks;
mod jwt;
mod keys;

pub use cache::{CacheOutcome, CacheStats, JwtCache};
pub use claims::Claims;
pub use hasura::{HasuraAuthConfig, HasuraIdentity, SecretOutcome, TokenRole};
pub use jwks::Jwks;
pub use jwt::validate_token;
pub use keys::{JwtKeyError, JwtKeys, KeySet};

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Authentication result containing role and claims.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuthResult {
    /// PostgreSQL role to use
    pub role: String,
    /// All JWT claims
    pub claims: HashMap<String, serde_json::Value>,
}

impl AuthResult {
    /// Create an anonymous auth result.
    pub fn anonymous(anon_role: &str) -> Self {
        Self {
            role: anon_role.to_string(),
            claims: HashMap::new(),
        }
    }

    /// Get a claim value.
    pub fn get_claim(&self, key: &str) -> Option<&serde_json::Value> {
        self.claims.get(key)
    }

    /// Get claims as JSON for GUC.
    pub fn claims_json(&self) -> String {
        serde_json::to_string(&self.claims).unwrap_or_else(|_| "{}".to_string())
    }

    /// Build transaction-local PostgreSQL settings for the verified claims.
    ///
    /// The complete JSON document preserves every claim. Claims with names
    /// valid for custom PostgreSQL settings are also exposed individually for
    /// policies using `request.jwt.claims.<name>`.
    pub fn claim_settings(&self) -> Vec<(String, String)> {
        let mut claims = Claims::from(self.claims.clone());
        claims.set("role", serde_json::Value::String(self.role.clone()));
        let mut settings = vec![("request.jwt.claims".to_string(), claims.to_json())];
        settings.extend(claims.prefixed_entries("request.jwt.claims."));
        settings
    }
}

/// JWT configuration.
///
/// `#[non_exhaustive]` from 3.0.0, so that verifying keys and claims of new
/// kinds is a minor release: start from `JwtConfig::default()` and assign, or
/// take `postrust_engine::jwt_config` for the one an `AppConfig` describes.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct JwtConfig {
    /// The keys tokens are verified with, parsed from `PGRST_JWT_SECRET` by
    /// [`JwtKeys::from_config`] when the configuration is read. `None` when
    /// no secret is configured, and every token is then refused with
    /// [`JwtError::SecretMissing`].
    pub keys: Option<JwtKeys>,
    /// Required audience claim
    pub audience: Option<String>,
    /// The issuer a token's `iss` must name (`PGRST_JWT_ISS`). When set, a
    /// token with no `iss` is refused too.
    pub issuer: Option<String>,
    /// Claim key containing the role
    pub role_claim_key: String,
    /// Default role for anonymous requests
    pub anon_role: Option<String>,
    /// Where `keys` are fetched from and kept current, when
    /// `PGRST_JWT_JWKS_URL` is set. Set it with [`JwtConfig::with_jwks`],
    /// which points `keys` at the same set; a host then awaits
    /// [`ensure_keys`] before verifying.
    pub jwks: Option<Jwks>,
}

impl Default for JwtConfig {
    fn default() -> Self {
        Self {
            keys: None,
            audience: None,
            issuer: None,
            role_claim_key: "role".to_string(),
            anon_role: None,
            jwks: None,
        }
    }
}

impl JwtConfig {
    /// Verify with the keys `jwks` keeps, which already include whatever
    /// `keys` held when it was built ([`jwks::JwksBuilder::fixed`]).
    pub fn with_jwks(mut self, jwks: Jwks) -> Self {
        self.keys = Some(jwks.keys().clone());
        self.jwks = Some(jwks);
        self
    }
}

/// Make sure the keys the request's token needs are loaded, before the
/// synchronous [`authenticate`] (or [`JwtCache::authenticate`]) runs.
///
/// The one asynchronous step in verifying a token, and a no-op unless keys
/// come from a JWKS URL ([`JwtConfig::jwks`]). Even then it awaits nothing
/// for a token whose key is already held, which is every request but the few
/// after the identity provider rotates a key. See [`jwks`] for when it
/// fetches. Never fails: a token it could not find a key for is refused by
/// the check that follows, as it would have been without it.
///
/// Every place a token is verified awaits this first: the REST pipeline, and
/// the server's GraphQL handlers.
pub async fn ensure_keys(auth_header: Option<&str>, config: &JwtConfig) {
    let Some(jwks) = &config.jwks else {
        return;
    };
    let Some(Ok(token)) = auth_header.map(extract_bearer_token) else {
        return;
    };
    jwks.ensure_for_token(token).await;
}

/// What was wrong with a token's claims.
///
/// Separate from the failures above because a token whose claims are wrong was
/// read successfully: the signature verified, the server has the right key,
/// and what remains is a disagreement about the claims themselves. A client
/// that cannot tell those apart cannot tell "rotate your key" from "your clock
/// is wrong".
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ClaimFault {
    /// `exp` is in the past.
    #[error("JWT expired")]
    Expired,

    /// `nbf` is in the future: the token is valid, but not yet.
    #[error("JWT not yet valid")]
    NotYetValid,

    /// `iat` is in the future, which usually means the two clocks disagree
    /// rather than that the token is forged.
    #[error("JWT issued at future")]
    IssuedInFuture,

    /// `aud` is present and does not list the audience this server accepts.
    #[error("JWT not in audience")]
    NotInAudience,

    /// An issuer is configured and `iss` does not name it, or is absent.
    #[error("JWT issuer is not the expected one")]
    UnexpectedIssuer,

    /// The payload decoded but is not a JSON object of claims.
    #[error("Parsing claims failed")]
    Unparsable,

    /// A timestamp claim that is present and is not a timestamp.
    #[error("The JWT '{0}' claim must be a number")]
    NotANumber(&'static str),

    /// `aud` is present and is neither a string nor an array of strings.
    #[error("The JWT 'aud' claim must be a string or an array of strings")]
    AudienceNotStrings,
}

/// JWT validation error.
#[derive(Debug, thiserror::Error)]
pub enum JwtError {
    /// Nothing said who is asking, and there is no anonymous role to be.
    #[error("Anonymous access is disabled")]
    NoIdentity,

    /// The `Authorization` header is not a bearer token this server can read.
    #[error("Unsupported token type")]
    InvalidHeaderFormat,

    /// A token was presented and no secret is configured to verify it with.
    #[error("Server lacks JWT secret")]
    SecretMissing,

    /// The signature did not verify under the key that was selected.
    #[error("JWT cryptographic operation failed")]
    InvalidSignature,

    /// No key this server holds could decode the token.
    #[error("No suitable key or wrong key type")]
    NoSuitableKey,

    /// The token was read, and its claims were not acceptable.
    #[error("{0}")]
    Claim(#[from] ClaimFault),
}

/// Extract and validate JWT from Authorization header.
pub fn authenticate(auth_header: Option<&str>, config: &JwtConfig) -> Result<AuthResult, JwtError> {
    // If no auth header, use anonymous role if configured
    let token = match auth_header {
        Some(header) => extract_bearer_token(header)?,
        None => {
            return match &config.anon_role {
                Some(role) => Ok(AuthResult::anonymous(role)),
                None => Err(JwtError::NoIdentity),
            };
        }
    };

    // Validate token
    validate_token(token, config)
}

/// Extract Bearer token from Authorization header.
fn extract_bearer_token(header: &str) -> Result<&str, JwtError> {
    let header = header.trim();

    if let Some(token) = header.strip_prefix("Bearer ") {
        Ok(token.trim())
    } else if let Some(token) = header.strip_prefix("bearer ") {
        Ok(token.trim())
    } else {
        Err(JwtError::InvalidHeaderFormat)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_bearer_token() {
        assert_eq!(extract_bearer_token("Bearer abc123").unwrap(), "abc123");
        assert_eq!(extract_bearer_token("bearer abc123").unwrap(), "abc123");
        assert!(extract_bearer_token("Basic abc123").is_err());
    }

    #[test]
    fn test_auth_result_anonymous() {
        let result = AuthResult::anonymous("anon");
        assert_eq!(result.role, "anon");
        assert!(result.claims.is_empty());
    }

    #[test]
    fn auth_result_claim_settings_include_json_and_individual_claims() {
        let result = AuthResult {
            role: "authenticated_user".to_string(),
            claims: HashMap::from([
                ("role".to_string(), serde_json::json!("untrusted_role")),
                ("user_id".to_string(), serde_json::json!("user-1")),
                ("org_class".to_string(), serde_json::json!(["defense"])),
            ]),
        };

        let settings = result.claim_settings();
        let setting = |name: &str| {
            settings
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.as_str())
        };
        let claims: serde_json::Value =
            serde_json::from_str(setting("request.jwt.claims").unwrap()).unwrap();

        assert_eq!(claims["role"], serde_json::json!("authenticated_user"));
        assert_eq!(
            setting("request.jwt.claims.role"),
            Some("authenticated_user")
        );
        assert_eq!(setting("request.jwt.claims.user_id"), Some("user-1"));
        assert_eq!(
            setting("request.jwt.claims.org_class"),
            Some(r#"["defense"]"#)
        );
    }

    #[test]
    fn test_authenticate_no_header_with_anon() {
        let config = JwtConfig {
            anon_role: Some("web_anon".to_string()),
            ..Default::default()
        };

        let result = authenticate(None, &config).unwrap();
        assert_eq!(result.role, "web_anon");
    }

    #[test]
    fn test_authenticate_no_header_no_anon() {
        let config = JwtConfig::default();
        assert!(authenticate(None, &config).is_err());
    }
}
