//! API key generation and validation service.

use crate::error::{ProxyError, ProxyResult};
use crate::saas::db;
use crate::saas::types::{ApiKey, CreateApiKeyRequest};
use rand::Rng;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

/// API key service for generating and validating API keys.
pub struct ApiKeyService {
    pool: PgPool,
}

impl ApiKeyService {
    /// Create a new API key service.
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Generate a new API key for a tenant.
    ///
    /// Returns the API key with the raw key value (only available at creation time).
    pub async fn create_api_key(
        &self,
        tenant_id: Uuid,
        req: CreateApiKeyRequest,
    ) -> ProxyResult<ApiKey> {
        // Generate a secure random key
        let raw_key = generate_api_key();
        let key_hash = hash_api_key(&raw_key);
        let key_prefix = &raw_key[..8];

        // Store in database
        let row = db::create_api_key(&self.pool, tenant_id, req, &key_hash, key_prefix).await?;

        // Return with the raw key (only time it's available)
        Ok(ApiKey {
            id: row.id,
            tenant_id: row.tenant_id,
            name: row.name,
            key: Some(raw_key), // Only returned on creation
            key_prefix: row.key_prefix,
            scopes: row.scopes,
            last_used_at: row.last_used_at,
            expires_at: row.expires_at,
            enabled: row.enabled,
            created_at: row.created_at,
        })
    }

    /// Validate an API key and return the tenant ID and scopes.
    pub async fn validate_api_key(&self, raw_key: &str) -> ProxyResult<ValidatedApiKey> {
        let key_hash = hash_api_key(raw_key);

        let validation = db::validate_api_key_by_hash(&self.pool, &key_hash)
            .await?
            .ok_or_else(|| ProxyError::Auth("Invalid or expired API key".into()))?;

        if !validation.enabled {
            return Err(ProxyError::Auth("API key is disabled".into()));
        }

        // Update last used timestamp asynchronously
        let pool = self.pool.clone();
        let key_id = validation.id;
        tokio::spawn(async move {
            let _ = db::update_last_used(&pool, key_id).await;
        });

        Ok(ValidatedApiKey {
            key_id: validation.id,
            tenant_id: validation.tenant_id,
            scopes: validation.scopes,
        })
    }

    /// List API keys for a tenant.
    pub async fn list_api_keys(&self, tenant_id: Uuid) -> ProxyResult<Vec<ApiKey>> {
        db::list_api_keys(&self.pool, tenant_id).await
    }

    /// Get an API key by ID.
    pub async fn get_api_key(&self, id: Uuid, tenant_id: Uuid) -> ProxyResult<Option<ApiKey>> {
        db::get_api_key_for_tenant(&self.pool, id, tenant_id).await
    }

    /// Revoke (delete) an API key.
    pub async fn revoke_api_key(&self, id: Uuid, tenant_id: Uuid) -> ProxyResult<bool> {
        db::delete_api_key(&self.pool, id, tenant_id).await
    }

    /// Disable an API key without deleting it.
    pub async fn disable_api_key(&self, id: Uuid, tenant_id: Uuid) -> ProxyResult<bool> {
        db::disable_api_key(&self.pool, id, tenant_id).await
    }

    /// Enable a disabled API key.
    pub async fn enable_api_key(&self, id: Uuid, tenant_id: Uuid) -> ProxyResult<bool> {
        db::enable_api_key(&self.pool, id, tenant_id).await
    }
}

/// Result of API key validation.
#[derive(Debug, Clone)]
pub struct ValidatedApiKey {
    pub key_id: Uuid,
    pub tenant_id: Uuid,
    pub scopes: Vec<String>,
}

impl ValidatedApiKey {
    /// Check if the key has a specific scope.
    pub fn has_scope(&self, scope: &str) -> bool {
        self.scopes.iter().any(|s| s == scope || s == "*")
    }

    /// Check if the key has read access for a resource.
    pub fn can_read(&self, resource: &str) -> bool {
        self.has_scope(&format!("{}:read", resource))
            || self.has_scope(&format!("{}:write", resource))
            || self.has_scope("*")
    }

    /// Check if the key has write access for a resource.
    pub fn can_write(&self, resource: &str) -> bool {
        self.has_scope(&format!("{}:write", resource)) || self.has_scope("*")
    }
}

/// Generate a secure random API key.
///
/// Format: `pr_live_` + 32 random alphanumeric characters
fn generate_api_key() -> String {
    const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut rng = rand::rng();

    let random_part: String = (0..32)
        .map(|_| {
            let idx = rng.random_range(0..CHARSET.len());
            CHARSET[idx] as char
        })
        .collect();

    format!("pr_live_{}", random_part)
}

/// Hash an API key using SHA-256.
pub fn hash_api_key(key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(key.as_bytes());
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_api_key() {
        let key = generate_api_key();
        assert!(key.starts_with("pr_live_"));
        assert_eq!(key.len(), 8 + 32); // prefix + random
    }

    #[test]
    fn test_hash_api_key() {
        let key = "pr_live_test123456789012345678901234";
        let hash = hash_api_key(key);
        assert_eq!(hash.len(), 64); // SHA-256 produces 32 bytes = 64 hex chars

        // Same input should produce same hash
        let hash2 = hash_api_key(key);
        assert_eq!(hash, hash2);

        // Different input should produce different hash
        let hash3 = hash_api_key("pr_live_different12345678901234567");
        assert_ne!(hash, hash3);
    }

    #[test]
    fn test_validated_api_key_scopes() {
        let validated = ValidatedApiKey {
            key_id: Uuid::new_v4(),
            tenant_id: Uuid::new_v4(),
            scopes: vec!["domains:read".to_string(), "domains:write".to_string()],
        };

        assert!(validated.has_scope("domains:read"));
        assert!(validated.has_scope("domains:write"));
        assert!(!validated.has_scope("upstreams:write"));

        assert!(validated.can_read("domains"));
        assert!(validated.can_write("domains"));
        assert!(!validated.can_read("upstreams"));
        assert!(!validated.can_write("upstreams"));
    }

    #[test]
    fn test_wildcard_scope() {
        let validated = ValidatedApiKey {
            key_id: Uuid::new_v4(),
            tenant_id: Uuid::new_v4(),
            scopes: vec!["*".to_string()],
        };

        assert!(validated.can_read("domains"));
        assert!(validated.can_write("domains"));
        assert!(validated.can_read("upstreams"));
        assert!(validated.can_write("upstreams"));
    }
}
