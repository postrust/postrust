//! JWT authentication for Postrust.
//!
//! Provides JWT token validation and role extraction for PostgreSQL RLS.

mod jwt;
mod claims;

pub use jwt::validate_token;
pub use claims::Claims;

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
}

/// JWT configuration.
#[derive(Clone, Debug)]
pub struct JwtConfig {
    /// Secret key for HS256/HS384/HS512
    pub secret: Option<String>,
    /// Whether secret is base64 encoded
    pub secret_is_base64: bool,
    /// Required audience claim
    pub audience: Option<String>,
    /// Claim key containing the role
    pub role_claim_key: String,
    /// Default role for anonymous requests
    pub anon_role: Option<String>,
}

impl Default for JwtConfig {
    fn default() -> Self {
        Self {
            secret: None,
            secret_is_base64: false,
            audience: None,
            role_claim_key: "role".to_string(),
            anon_role: None,
        }
    }
}

/// JWT validation error.
#[derive(Debug, thiserror::Error)]
pub enum JwtError {
    #[error("Missing authorization header")]
    MissingHeader,

    #[error("Invalid authorization header format")]
    InvalidHeaderFormat,

    #[error("Token expired")]
    Expired,

    #[error("Token not yet valid")]
    NotYetValid,

    #[error("Invalid signature")]
    InvalidSignature,

    #[error("Invalid token: {0}")]
    InvalidToken(String),

    #[error("Missing role claim")]
    MissingRole,

    #[error("Invalid audience")]
    InvalidAudience,
}

/// Extract and validate JWT from Authorization header.
pub fn authenticate(
    auth_header: Option<&str>,
    config: &JwtConfig,
) -> Result<AuthResult, JwtError> {
    // If no auth header, use anonymous role if configured
    let token = match auth_header {
        Some(header) => extract_bearer_token(header)?,
        None => {
            return match &config.anon_role {
                Some(role) => Ok(AuthResult::anonymous(role)),
                None => Err(JwtError::MissingHeader),
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
        assert_eq!(
            extract_bearer_token("Bearer abc123").unwrap(),
            "abc123"
        );
        assert_eq!(
            extract_bearer_token("bearer abc123").unwrap(),
            "abc123"
        );
        assert!(extract_bearer_token("Basic abc123").is_err());
    }

    #[test]
    fn test_auth_result_anonymous() {
        let result = AuthResult::anonymous("anon");
        assert_eq!(result.role, "anon");
        assert!(result.claims.is_empty());
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
