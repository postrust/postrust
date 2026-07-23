//! Authentication middleware for SaaS domain management.
//!
//! Supports both JWT and API key authentication.

use crate::saas::api_keys::hash_api_key;
use crate::saas::db;
use axum::{
    extract::{Request, State},
    http::{header, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

/// Authentication context extracted from the request.
#[derive(Clone, Debug)]
pub struct AuthContext {
    /// The tenant ID
    pub tenant_id: Uuid,
    /// The authentication type used
    pub auth_type: AuthType,
    /// Scopes available to this authentication
    pub scopes: Vec<String>,
}

/// Type of authentication used.
#[derive(Clone, Debug)]
pub enum AuthType {
    /// JWT authentication
    Jwt {
        /// User ID from JWT claims
        user_id: String,
        /// Role from JWT claims
        role: Option<String>,
    },
    /// API key authentication
    ApiKey {
        /// API key ID
        key_id: Uuid,
    },
}

impl AuthContext {
    /// Check if the context has a specific scope.
    pub fn has_scope(&self, scope: &str) -> bool {
        self.scopes.iter().any(|s| s == scope || s == "*")
    }

    /// Check if the context can read a resource.
    pub fn can_read(&self, resource: &str) -> bool {
        self.has_scope(&format!("{}:read", resource))
            || self.has_scope(&format!("{}:write", resource))
            || self.has_scope("*")
    }

    /// Check if the context can write to a resource.
    pub fn can_write(&self, resource: &str) -> bool {
        self.has_scope(&format!("{}:write", resource)) || self.has_scope("*")
    }
}

/// SaaS authentication layer state.
#[derive(Clone)]
pub struct SaasAuthLayer {
    pool: PgPool,
    jwt_secret: Option<String>,
}

impl SaasAuthLayer {
    /// Create a new SaaS auth layer.
    pub fn new(pool: PgPool, jwt_secret: Option<String>) -> Self {
        Self { pool, jwt_secret }
    }
}

/// Authentication middleware function.
pub async fn auth_middleware(
    State(auth_layer): State<Arc<SaasAuthLayer>>,
    mut request: Request,
    next: Next,
) -> Result<Response, AuthError> {
    let auth_header = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok());

    let auth_context = match auth_header {
        Some(header) if header.starts_with("Bearer ") => {
            // JWT authentication
            let token = header.strip_prefix("Bearer ").unwrap();
            validate_jwt(token, &auth_layer).await?
        }
        Some(header) if header.starts_with("ApiKey ") => {
            // API key authentication
            let key = header.strip_prefix("ApiKey ").unwrap();
            validate_api_key(key, &auth_layer.pool).await?
        }
        Some(header) if header.starts_with("Basic ") => {
            // Could support basic auth in the future
            return Err(AuthError::UnsupportedAuthMethod);
        }
        Some(_) => {
            return Err(AuthError::InvalidFormat);
        }
        None => {
            return Err(AuthError::MissingHeader);
        }
    };

    // Check if tenant is active
    if !db::is_tenant_active(&auth_layer.pool, auth_context.tenant_id).await? {
        return Err(AuthError::TenantSuspended);
    }

    // Insert auth context into request extensions
    request.extensions_mut().insert(auth_context);

    Ok(next.run(request).await)
}

/// Validate a JWT token.
async fn validate_jwt(token: &str, auth_layer: &SaasAuthLayer) -> Result<AuthContext, AuthError> {
    let secret = auth_layer
        .jwt_secret
        .as_ref()
        .ok_or(AuthError::JwtNotConfigured)?;

    let claims = decode_jwt(token, secret)?;

    // Get tenant ID from claims
    let tenant_id = claims
        .tenant_id
        .ok_or_else(|| AuthError::MissingClaim("tenant_id".into()))?;

    Ok(AuthContext {
        tenant_id,
        auth_type: AuthType::Jwt {
            user_id: claims.sub,
            role: claims.role,
        },
        scopes: claims.scopes.unwrap_or_else(|| vec!["*".to_string()]),
    })
}

/// Validate an API key.
async fn validate_api_key(key: &str, pool: &PgPool) -> Result<AuthContext, AuthError> {
    let key_hash = hash_api_key(key);

    let validation = db::validate_api_key_by_hash(pool, &key_hash)
        .await?
        .ok_or(AuthError::InvalidApiKey)?;

    if !validation.enabled {
        return Err(AuthError::ApiKeyDisabled);
    }

    // Update last used timestamp asynchronously
    let pool = pool.clone();
    let key_id = validation.id;
    tokio::spawn(async move {
        let _ = db::update_last_used(&pool, key_id).await;
    });

    Ok(AuthContext {
        tenant_id: validation.tenant_id,
        auth_type: AuthType::ApiKey {
            key_id: validation.id,
        },
        scopes: validation.scopes,
    })
}

/// JWT claims structure.
#[derive(Debug, Deserialize)]
struct JwtClaims {
    /// Subject (user ID)
    sub: String,
    /// Tenant ID
    tenant_id: Option<Uuid>,
    /// User role
    role: Option<String>,
    /// Scopes
    scopes: Option<Vec<String>>,
    /// Expiration time
    exp: Option<i64>,
}

/// Decode and validate a JWT token (HMAC-SHA256 signature, then expiry).
fn decode_jwt(token: &str, secret: &str) -> Result<JwtClaims, AuthError> {
    use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};

    let mut validation = Validation::new(Algorithm::HS256);
    // `exp` is optional for these tokens: don't require it, and check it
    // manually below when present so absence isn't an error.
    validation.required_spec_claims.clear();
    validation.validate_exp = false;

    let token_data = decode::<JwtClaims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )
    .map_err(|e| match e.kind() {
        jsonwebtoken::errors::ErrorKind::ExpiredSignature => AuthError::TokenExpired,
        _ => AuthError::InvalidToken(e.to_string()),
    })?;

    let claims = token_data.claims;

    // Check expiration
    if let Some(exp) = claims.exp {
        let now = chrono::Utc::now().timestamp();
        if exp < now {
            return Err(AuthError::TokenExpired);
        }
    }

    Ok(claims)
}

/// Authentication errors.
#[derive(Debug)]
pub enum AuthError {
    MissingHeader,
    InvalidFormat,
    InvalidToken(String),
    TokenExpired,
    MissingClaim(String),
    InvalidApiKey,
    ApiKeyDisabled,
    ApiKeyExpired,
    TenantSuspended,
    JwtNotConfigured,
    UnsupportedAuthMethod,
    Database(sqlx::Error),
}

impl From<sqlx::Error> for AuthError {
    fn from(err: sqlx::Error) -> Self {
        AuthError::Database(err)
    }
}

impl From<crate::error::ProxyError> for AuthError {
    fn from(err: crate::error::ProxyError) -> Self {
        match err {
            crate::error::ProxyError::Database(e) => AuthError::Database(e),
            _ => AuthError::InvalidToken(err.to_string()),
        }
    }
}

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            AuthError::MissingHeader => (
                StatusCode::UNAUTHORIZED,
                "Missing Authorization header".to_string(),
            ),
            AuthError::InvalidFormat => (
                StatusCode::UNAUTHORIZED,
                "Invalid Authorization header format".to_string(),
            ),
            AuthError::InvalidToken(msg) => {
                (StatusCode::UNAUTHORIZED, format!("Invalid token: {}", msg))
            }
            AuthError::TokenExpired => (StatusCode::UNAUTHORIZED, "Token has expired".to_string()),
            AuthError::MissingClaim(claim) => (
                StatusCode::UNAUTHORIZED,
                format!("Missing required claim: {}", claim),
            ),
            AuthError::InvalidApiKey => (StatusCode::UNAUTHORIZED, "Invalid API key".to_string()),
            AuthError::ApiKeyDisabled => {
                (StatusCode::UNAUTHORIZED, "API key is disabled".to_string())
            }
            AuthError::ApiKeyExpired => {
                (StatusCode::UNAUTHORIZED, "API key has expired".to_string())
            }
            AuthError::TenantSuspended => (
                StatusCode::FORBIDDEN,
                "Tenant account is suspended".to_string(),
            ),
            AuthError::JwtNotConfigured => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "JWT authentication not configured".to_string(),
            ),
            AuthError::UnsupportedAuthMethod => (
                StatusCode::UNAUTHORIZED,
                "Unsupported authentication method".to_string(),
            ),
            AuthError::Database(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error".to_string(),
            ),
        };

        let body = serde_json::json!({
            "success": false,
            "error": message
        });

        (status, axum::Json(body)).into_response()
    }
}

/// Extractor for AuthContext from request extensions.
#[derive(Clone, Debug)]
pub struct Auth(pub AuthContext);

impl<S> axum::extract::FromRequestParts<S> for Auth
where
    S: Send + Sync,
{
    type Rejection = AuthError;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &S,
    ) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<AuthContext>()
            .cloned()
            .map(Auth)
            .ok_or(AuthError::MissingHeader)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use jsonwebtoken::{encode, EncodingKey, Header};

    const SECRET: &str = "test-secret";
    const TENANT_ID: &str = "6ba7b810-9dad-11d1-80b4-00c04fd430c8";

    fn sign(claims: &serde_json::Value, secret: &str) -> String {
        encode(
            &Header::default(),
            claims,
            &EncodingKey::from_secret(secret.as_bytes()),
        )
        .unwrap()
    }

    fn valid_claims() -> serde_json::Value {
        serde_json::json!({
            "sub": "user-1",
            "tenant_id": TENANT_ID,
            "role": "admin",
            "scopes": ["domains:read"],
            "exp": chrono::Utc::now().timestamp() + 3600,
        })
    }

    fn b64url(data: &[u8]) -> String {
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(data)
    }

    #[test]
    fn valid_token_decodes() {
        let token = sign(&valid_claims(), SECRET);
        let claims = decode_jwt(&token, SECRET).unwrap();
        assert_eq!(claims.sub, "user-1");
        assert_eq!(claims.tenant_id, Some(TENANT_ID.parse().unwrap()));
        assert_eq!(claims.role.as_deref(), Some("admin"));
        assert_eq!(claims.scopes, Some(vec!["domains:read".to_string()]));
    }

    #[test]
    fn token_without_exp_is_accepted() {
        let mut claims = valid_claims();
        claims.as_object_mut().unwrap().remove("exp");
        let token = sign(&claims, SECRET);
        assert!(decode_jwt(&token, SECRET).is_ok());
    }

    #[test]
    fn token_signed_with_wrong_secret_is_rejected() {
        let token = sign(&valid_claims(), "some-other-secret");
        assert!(matches!(
            decode_jwt(&token, SECRET),
            Err(AuthError::InvalidToken(_))
        ));
    }

    #[test]
    fn tampered_payload_is_rejected() {
        // Re-encode the payload with an escalated tenant_id while keeping the
        // original (now mismatched) signature.
        let token = sign(&valid_claims(), SECRET);
        let parts: Vec<&str> = token.split('.').collect();

        let mut claims = valid_claims();
        claims["tenant_id"] = serde_json::json!("00000000-0000-0000-0000-000000000001");
        let forged_payload = b64url(serde_json::to_vec(&claims).unwrap().as_slice());

        let forged = format!("{}.{}.{}", parts[0], forged_payload, parts[2]);
        assert!(matches!(
            decode_jwt(&forged, SECRET),
            Err(AuthError::InvalidToken(_))
        ));
    }

    #[test]
    fn unsigned_alg_none_token_is_rejected() {
        // A classic "alg": "none" forgery must not pass HS256 validation,
        // with or without a trailing signature segment.
        let header = b64url(br#"{"alg":"none","typ":"JWT"}"#);
        let payload = b64url(serde_json::to_vec(&valid_claims()).unwrap().as_slice());

        for token in [
            format!("{header}.{payload}."),
            format!("{header}.{payload}"),
        ] {
            assert!(
                matches!(decode_jwt(&token, SECRET), Err(AuthError::InvalidToken(_))),
                "token {token:?} should be rejected"
            );
        }
    }

    #[test]
    fn expired_token_is_rejected() {
        let mut claims = valid_claims();
        claims["exp"] = serde_json::json!(chrono::Utc::now().timestamp() - 60);
        let token = sign(&claims, SECRET);
        assert!(matches!(
            decode_jwt(&token, SECRET),
            Err(AuthError::TokenExpired)
        ));
    }

    #[test]
    fn malformed_token_is_rejected() {
        assert!(matches!(
            decode_jwt("not-a-jwt", SECRET),
            Err(AuthError::InvalidToken(_))
        ));
    }
}
