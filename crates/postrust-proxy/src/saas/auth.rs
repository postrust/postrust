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

/// Every scope an API key may be given.
///
/// `resource:read` and `resource:write`, where write implies read, plus `*` for
/// everything. A key's scopes are checked against this list when it is
/// created: a misspelt scope would otherwise be stored, grant nothing, and look
/// to whoever reads the key's metadata as though it granted something.
pub const KNOWN_SCOPES: &[&str] = &[
    "*",
    "domains:read",
    "domains:write",
    "routes:read",
    "routes:write",
    "upstreams:read",
    "upstreams:write",
    "api_keys:read",
    "api_keys:write",
    "tenant:read",
];

/// Refuse any scope not in [`KNOWN_SCOPES`].
pub fn validate_scopes(scopes: &[String]) -> crate::error::ProxyResult<()> {
    match scopes.iter().find(|s| !KNOWN_SCOPES.contains(&s.as_str())) {
        Some(unknown) => Err(crate::error::ProxyError::Validation(format!(
            "unknown scope {unknown:?}; known scopes are {}",
            KNOWN_SCOPES.join(", ")
        ))),
        None => Ok(()),
    }
}

impl AuthContext {
    /// Check if the context has a specific scope.
    pub fn has_scope(&self, scope: &str) -> bool {
        self.scopes.iter().any(|s| s == scope || s == "*")
    }

    /// Whether this context holds at least what `scope` would grant.
    ///
    /// What decides whether a caller may hand a scope to a new key, or revoke
    /// a key holding it: nobody may give or take away more than they have.
    /// `*` is held only by `*`; `x:read` by anything that can read `x`;
    /// `x:write` by anything that can write it. An unrecognised scope is held
    /// by nobody but `*`.
    pub fn grants(&self, scope: &str) -> bool {
        if self.has_scope("*") {
            return true;
        }
        match scope.rsplit_once(':') {
            Some((resource, "read")) => self.can_read(resource),
            Some((resource, "write")) => self.can_write(resource),
            _ => false,
        }
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
    jwt_audience: String,
    jwt_issuer: Option<String>,
}

impl SaasAuthLayer {
    /// Create a new SaaS auth layer.
    ///
    /// JWTs must carry `aud` = [`DEFAULT_JWT_AUDIENCE`] unless
    /// [`with_jwt_audience`](Self::with_jwt_audience) says otherwise.
    ///
    /// [`DEFAULT_JWT_AUDIENCE`]: crate::saas::options::DEFAULT_JWT_AUDIENCE
    pub fn new(pool: PgPool, jwt_secret: Option<String>) -> Self {
        Self {
            pool,
            jwt_secret,
            jwt_audience: crate::saas::options::DEFAULT_JWT_AUDIENCE.to_string(),
            jwt_issuer: None,
        }
    }

    /// Require JWTs to name this audience instead of the default.
    pub fn with_jwt_audience(mut self, audience: impl Into<String>) -> Self {
        self.jwt_audience = audience.into();
        self
    }

    /// Require JWTs to name this issuer.
    pub fn with_jwt_issuer(mut self, issuer: Option<String>) -> Self {
        self.jwt_issuer = issuer;
        self
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

    let claims = decode_jwt(
        token,
        secret,
        &auth_layer.jwt_audience,
        auth_layer.jwt_issuer.as_deref(),
    )?;

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
        // No `scopes` claim means no scopes. Defaulting to `*` made every token
        // minted without thinking about scopes an all-powerful one.
        scopes: claims.scopes.unwrap_or_default(),
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
    /// Scopes. Absent means none.
    scopes: Option<Vec<String>>,
}

/// Decode and validate a JWT: HMAC-SHA256 signature, then `exp`, `aud` and,
/// when configured, `iss`.
///
/// `exp` and `aud` are required, and `iss` when an issuer is configured. A
/// token without `exp` never expires, so a leaked one is good for ever. A token without the right `aud` may have been
/// minted for something else that shares the secret -- the main Postrust
/// server, typically -- and must not open this API.
fn decode_jwt(
    token: &str,
    secret: &str,
    audience: &str,
    issuer: Option<&str>,
) -> Result<JwtClaims, AuthError> {
    use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};

    let mut validation = Validation::new(Algorithm::HS256);
    validation.set_required_spec_claims(&["exp", "aud"]);
    validation.validate_exp = true;
    validation.set_audience(&[audience]);
    if let Some(issuer) = issuer {
        validation.set_issuer(&[issuer]);
        validation.required_spec_claims.insert("iss".to_string());
    }

    let token_data = decode::<JwtClaims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )
    .map_err(|e| match e.kind() {
        jsonwebtoken::errors::ErrorKind::ExpiredSignature => AuthError::TokenExpired,
        _ => AuthError::InvalidToken(e.to_string()),
    })?;

    Ok(token_data.claims)
}

/// Authentication errors.
#[derive(Debug)]
pub enum AuthError {
    /// No `Authorization` header at all.
    MissingHeader,
    /// The header is present but not `Bearer <jwt>` or `ApiKey <key>`.
    InvalidFormat,
    /// A JWT that does not verify, with the reason.
    InvalidToken(String),
    /// A JWT whose `exp` has passed.
    TokenExpired,
    /// A JWT missing a claim this proxy needs, named.
    MissingClaim(String),
    /// An API key that matches no stored hash.
    InvalidApiKey,
    /// A key that exists but has been revoked.
    ApiKeyDisabled,
    /// A key past its `expires_at`.
    ApiKeyExpired,
    /// The credential is good but its tenant may not act.
    TenantSuspended,
    /// A JWT was presented and no signing secret is configured, so it cannot
    /// be verified. Refused rather than trusted.
    JwtNotConfigured,
    /// An `Authorization` scheme this proxy does not implement.
    UnsupportedAuthMethod,
    /// The lookup itself failed. Ours, not the caller's -- reported as a 500
    /// rather than a 401, so an outage does not read as a bad credential.
    Database(sqlx::Error),
    /// Some other failure of ours. Logged; the caller sees a generic 500.
    Internal(String),
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
            // Anything else reaching here is a failure of ours, from a lookup.
            // It used to be reported as an invalid token, with its text.
            other => AuthError::Internal(other.to_string()),
        }
    }
}

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        match &self {
            AuthError::Database(error) => {
                tracing::error!(%error, "authentication lookup failed");
            }
            AuthError::Internal(error) => {
                tracing::error!(%error, "authentication failed internally");
            }
            AuthError::InvalidToken(reason) => {
                tracing::debug!(%reason, "refused a JWT");
            }
            _ => {}
        }

        let (status, message) = match self {
            AuthError::MissingHeader => (
                StatusCode::UNAUTHORIZED,
                "Missing Authorization header".to_string(),
            ),
            AuthError::InvalidFormat => (
                StatusCode::UNAUTHORIZED,
                "Invalid Authorization header format".to_string(),
            ),
            // The reason is logged, not returned: it is the library's text,
            // and says which check a forger has yet to get past.
            AuthError::InvalidToken(_) => (StatusCode::UNAUTHORIZED, "Invalid token".to_string()),
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
            AuthError::Database(_) | AuthError::Internal(_) => (
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
pub struct Auth(
    /// The context the middleware put in the request's extensions.
    pub AuthContext,
);

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
            "aud": AUDIENCE,
            "exp": chrono::Utc::now().timestamp() + 3600,
        })
    }

    const AUDIENCE: &str = "postrust-proxy";

    fn decode(token: &str, secret: &str) -> Result<JwtClaims, AuthError> {
        decode_jwt(token, secret, AUDIENCE, None)
    }

    fn b64url(data: &[u8]) -> String {
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(data)
    }

    #[test]
    fn valid_token_decodes() {
        let token = sign(&valid_claims(), SECRET);
        let claims = decode(&token, SECRET).unwrap();
        assert_eq!(claims.sub, "user-1");
        assert_eq!(claims.tenant_id, Some(TENANT_ID.parse().unwrap()));
        assert_eq!(claims.role.as_deref(), Some("admin"));
        assert_eq!(claims.scopes, Some(vec!["domains:read".to_string()]));
    }

    #[test]
    fn token_without_exp_is_rejected() {
        // It would never expire, so a leaked one would be good for ever.
        let mut claims = valid_claims();
        claims.as_object_mut().unwrap().remove("exp");
        let token = sign(&claims, SECRET);
        assert!(matches!(
            decode(&token, SECRET),
            Err(AuthError::InvalidToken(_))
        ));
    }

    #[test]
    fn token_without_an_audience_is_rejected() {
        let mut claims = valid_claims();
        claims.as_object_mut().unwrap().remove("aud");
        let token = sign(&claims, SECRET);
        assert!(matches!(
            decode(&token, SECRET),
            Err(AuthError::InvalidToken(_))
        ));
    }

    #[test]
    fn token_for_another_audience_is_rejected() {
        // What the main Postrust server would mint with the same secret.
        let mut claims = valid_claims();
        claims["aud"] = serde_json::json!("postrust");
        let token = sign(&claims, SECRET);
        assert!(matches!(
            decode(&token, SECRET),
            Err(AuthError::InvalidToken(_))
        ));

        // A configured audience replaces the default rather than adding to it.
        let token = sign(&valid_claims(), SECRET);
        assert!(decode_jwt(&token, SECRET, "elsewhere", None).is_err());
    }

    #[test]
    fn a_configured_issuer_is_required() {
        let token = sign(&valid_claims(), SECRET);
        assert!(
            decode_jwt(&token, SECRET, AUDIENCE, Some("https://id.example")).is_err(),
            "a token with no iss passed an issuer check"
        );

        let mut claims = valid_claims();
        claims["iss"] = serde_json::json!("https://id.example");
        let token = sign(&claims, SECRET);
        assert!(decode_jwt(&token, SECRET, AUDIENCE, Some("https://id.example")).is_ok());
        assert!(decode_jwt(&token, SECRET, AUDIENCE, Some("https://other.example")).is_err());
    }

    #[tokio::test]
    async fn a_token_without_scopes_has_none() {
        let mut claims = valid_claims();
        claims.as_object_mut().unwrap().remove("scopes");
        let token = sign(&claims, SECRET);

        // Never connects: validate_jwt does not touch the database.
        let pool = PgPool::connect_lazy("postgres://localhost/unused").unwrap();
        let layer = SaasAuthLayer::new(pool, Some(SECRET.to_string()));
        let context = validate_jwt(&token, &layer).await.expect("valid token");
        assert!(context.scopes.is_empty(), "got {:?}", context.scopes);
        assert!(!context.can_read("domains"));
    }

    fn context(scopes: &[&str]) -> AuthContext {
        AuthContext {
            tenant_id: Uuid::nil(),
            auth_type: AuthType::ApiKey {
                key_id: Uuid::nil(),
            },
            scopes: scopes.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn only_a_wildcard_grants_a_wildcard() {
        assert!(context(&["*"]).grants("*"));
        assert!(!context(&["domains:write", "api_keys:write"]).grants("*"));
        assert!(!context(&[]).grants("domains:read"));
    }

    #[test]
    fn write_grants_read_but_not_the_reverse() {
        let writer = context(&["domains:write"]);
        assert!(writer.grants("domains:read"));
        assert!(writer.grants("domains:write"));
        assert!(!writer.grants("routes:read"));

        let reader = context(&["domains:read"]);
        assert!(reader.grants("domains:read"));
        assert!(!reader.grants("domains:write"));
        assert!(!reader.grants("bogus"));
    }

    #[test]
    fn unknown_scopes_are_refused() {
        assert!(validate_scopes(&["domains:read".into(), "tenant:read".into()]).is_ok());
        assert!(validate_scopes(&["domain:read".into()]).is_err());
        assert!(validate_scopes(&["domains:admin".into()]).is_err());
        assert!(validate_scopes(&["".into()]).is_err());
    }

    #[test]
    fn token_signed_with_wrong_secret_is_rejected() {
        let token = sign(&valid_claims(), "some-other-secret");
        assert!(matches!(
            decode(&token, SECRET),
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
            decode(&forged, SECRET),
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
                matches!(decode(&token, SECRET), Err(AuthError::InvalidToken(_))),
                "token {token:?} should be rejected"
            );
        }
    }

    #[test]
    fn expired_token_is_rejected() {
        let mut claims = valid_claims();
        // Past the library's default sixty-second leeway for clock skew.
        claims["exp"] = serde_json::json!(chrono::Utc::now().timestamp() - 3600);
        let token = sign(&claims, SECRET);
        assert!(matches!(
            decode(&token, SECRET),
            Err(AuthError::TokenExpired)
        ));
    }

    #[test]
    fn malformed_token_is_rejected() {
        assert!(matches!(
            decode("not-a-jwt", SECRET),
            Err(AuthError::InvalidToken(_))
        ));
    }
}
