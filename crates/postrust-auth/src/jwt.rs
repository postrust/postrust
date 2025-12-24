//! JWT token validation.

use super::{AuthResult, JwtConfig, JwtError};
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Validate a JWT token and extract claims.
pub fn validate_token(token: &str, config: &JwtConfig) -> Result<AuthResult, JwtError> {
    let secret = config.secret.as_ref().ok_or_else(|| {
        JwtError::InvalidToken("No JWT secret configured".into())
    })?;

    // Decode secret
    let key_bytes = if config.secret_is_base64 {
        base64_decode(secret)?
    } else {
        secret.as_bytes().to_vec()
    };

    let key = DecodingKey::from_secret(&key_bytes);

    // Set up validation
    let mut validation = Validation::new(Algorithm::HS256);
    validation.validate_exp = true;
    validation.validate_nbf = true;

    if let Some(aud) = &config.audience {
        validation.set_audience(&[aud]);
    } else {
        validation.validate_aud = false;
    }

    // Decode and validate
    let token_data = decode::<Claims>(token, &key, &validation)
        .map_err(|e| map_jwt_error(e))?;

    let claims = token_data.claims;

    // Extract role
    let role = claims
        .extra
        .get(&config.role_claim_key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| config.anon_role.clone())
        .ok_or(JwtError::MissingRole)?;

    // Build claims map
    let mut claims_map = claims.extra;
    if let Some(sub) = claims.sub {
        claims_map.insert("sub".into(), serde_json::Value::String(sub));
    }
    if let Some(iss) = claims.iss {
        claims_map.insert("iss".into(), serde_json::Value::String(iss));
    }
    if let Some(exp) = claims.exp {
        claims_map.insert("exp".into(), serde_json::Value::Number(exp.into()));
    }

    Ok(AuthResult {
        role,
        claims: claims_map,
    })
}

/// Standard and custom JWT claims.
#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    /// Subject
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sub: Option<String>,
    /// Issuer
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iss: Option<String>,
    /// Expiration time
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exp: Option<i64>,
    /// Not before
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nbf: Option<i64>,
    /// Issued at
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iat: Option<i64>,
    /// Audience
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aud: Option<String>,
    /// Custom claims
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

/// Decode base64 secret.
fn base64_decode(s: &str) -> Result<Vec<u8>, JwtError> {
    use base64::{engine::general_purpose::STANDARD, Engine};
    STANDARD
        .decode(s)
        .map_err(|e| JwtError::InvalidToken(format!("Invalid base64 secret: {}", e)))
}

/// Map jsonwebtoken error to JwtError.
fn map_jwt_error(e: jsonwebtoken::errors::Error) -> JwtError {
    use jsonwebtoken::errors::ErrorKind;

    match e.kind() {
        ErrorKind::ExpiredSignature => JwtError::Expired,
        ErrorKind::ImmatureSignature => JwtError::NotYetValid,
        ErrorKind::InvalidSignature => JwtError::InvalidSignature,
        ErrorKind::InvalidAudience => JwtError::InvalidAudience,
        _ => JwtError::InvalidToken(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{encode, EncodingKey, Header};

    fn make_token(claims: &Claims, secret: &str) -> String {
        let key = EncodingKey::from_secret(secret.as_bytes());
        encode(&Header::default(), claims, &key).unwrap()
    }

    #[test]
    fn test_validate_valid_token() {
        let secret = "test_secret_key_at_least_32_bytes!";

        let claims = Claims {
            sub: Some("user123".into()),
            iss: None,
            exp: Some(chrono::Utc::now().timestamp() + 3600),
            nbf: None,
            iat: None,
            aud: None,
            extra: {
                let mut m = HashMap::new();
                m.insert("role".into(), serde_json::Value::String("web_user".into()));
                m
            },
        };

        let token = make_token(&claims, secret);

        let config = JwtConfig {
            secret: Some(secret.into()),
            ..Default::default()
        };

        let result = validate_token(&token, &config).unwrap();
        assert_eq!(result.role, "web_user");
        assert_eq!(result.get_claim("sub").unwrap(), "user123");
    }

    #[test]
    fn test_validate_expired_token() {
        let secret = "test_secret_key_at_least_32_bytes!";

        let claims = Claims {
            sub: None,
            iss: None,
            exp: Some(chrono::Utc::now().timestamp() - 3600), // Expired
            nbf: None,
            iat: None,
            aud: None,
            extra: HashMap::new(),
        };

        let token = make_token(&claims, secret);

        let config = JwtConfig {
            secret: Some(secret.into()),
            ..Default::default()
        };

        let result = validate_token(&token, &config);
        assert!(matches!(result, Err(JwtError::Expired)));
    }
}
