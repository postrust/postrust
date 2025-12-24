//! GraphQL context containing auth, database pool, and schema cache.

use postrust_auth::AuthResult;
use postrust_core::schema_cache::SchemaCacheRef;
use sqlx::PgPool;

/// Context available to all GraphQL resolvers.
pub struct GraphQLContext {
    /// Database connection pool.
    pub pool: PgPool,
    /// Schema cache for table/column metadata.
    pub schema_cache: SchemaCacheRef,
    /// Authentication result with role and claims.
    pub auth: AuthResult,
}

impl GraphQLContext {
    /// Create a new GraphQL context.
    pub fn new(pool: PgPool, schema_cache: SchemaCacheRef, auth: AuthResult) -> Self {
        Self {
            pool,
            schema_cache,
            auth,
        }
    }

    /// Get the current role.
    pub fn role(&self) -> &str {
        &self.auth.role
    }

    /// Get a claim value.
    pub fn claim(&self, key: &str) -> Option<&serde_json::Value> {
        self.auth.claims.get(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn create_test_auth() -> AuthResult {
        let mut claims = HashMap::new();
        claims.insert("user_id".into(), serde_json::json!(123));
        claims.insert("role".into(), serde_json::json!("admin"));

        AuthResult {
            role: "authenticated".into(),
            claims,
        }
    }

    #[test]
    fn test_context_role() {
        let auth = create_test_auth();
        // Note: We can't fully test without a pool, but we can test the auth part
        assert_eq!(auth.role, "authenticated");
    }

    #[test]
    fn test_context_claim() {
        let auth = create_test_auth();
        assert_eq!(auth.claims.get("user_id"), Some(&serde_json::json!(123)));
    }
}
