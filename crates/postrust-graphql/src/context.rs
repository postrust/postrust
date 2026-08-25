//! GraphQL context containing auth, database pool, and schema cache.

use postrust_auth::AuthResult;
use postrust_core::schema_cache::SchemaCacheRef;
use sqlx::PgPool;
use std::collections::HashMap;
use std::sync::Arc;

/// The transaction every write in one operation shares.
///
/// A GraphQL mutation may name several root fields, and the specification
/// resolves them one after another. Hasura runs the whole set in one
/// transaction, so a mutation whose second root field violates a constraint
/// leaves nothing behind from its first -- which is what a client sending
/// "create the order and its lines" is relying on. Opening a transaction per
/// resolver, as this used to, half-applies that mutation and reports failure.
///
/// Opened lazily by the first write and settled once the operation is answered:
/// committed when the response carries no errors, rolled back otherwise. A
/// query never touches it.
pub type SharedWrite =
    Arc<tokio::sync::Mutex<Option<sqlx::Transaction<'static, sqlx::Postgres>>>>;

/// Context available to all GraphQL resolvers.
pub struct GraphQLContext {
    /// Database connection pool.
    pub pool: PgPool,
    /// Schema cache for table/column metadata.
    pub schema_cache: SchemaCacheRef,
    /// Authentication result with role and claims.
    pub auth: AuthResult,
    /// Session variables carried into the transaction, without their
    /// `x-hasura-` prefix and lowercased: `user_id`, `org_id`.
    ///
    /// This is the half of Hasura's permission model that transfers. Its rules
    /// live in metadata and are compiled into every query; here permissions
    /// live in the database as roles and row level security, and what a policy
    /// needs is the caller's identity. A policy written against
    /// `current_setting('hasura.user_id')` sees what the Hasura permission
    /// would have seen.
    pub session: HashMap<String, String>,
    /// The transaction every write in this operation shares. See [`SharedWrite`].
    pub write: SharedWrite,
}

impl GraphQLContext {
    /// Create a new GraphQL context.
    pub fn new(pool: PgPool, schema_cache: SchemaCacheRef, auth: AuthResult) -> Self {
        Self {
            pool,
            schema_cache,
            auth,
            session: HashMap::new(),
            write: Arc::new(tokio::sync::Mutex::new(None)),
        }
    }

    /// Share one transaction with whoever is going to settle it.
    ///
    /// The caller keeps the other handle: it is the only thing that knows
    /// whether the operation ended in errors, and so the only thing that can
    /// decide between commit and rollback.
    pub fn with_write(mut self, write: SharedWrite) -> Self {
        self.write = write;
        self
    }

    /// Carry session variables into every transaction this request opens.
    pub fn with_session(mut self, session: HashMap<String, String>) -> Self {
        self.session = session;
        self
    }

    /// The `SET LOCAL` statements this request's session variables need.
    ///
    /// The setting name is built from the variable's own name after it has
    /// been checked, and the value is bound rather than interpolated:
    /// `set_config` takes both as arguments, where `SET LOCAL` would need the
    /// value written into the statement.
    pub fn session_settings(&self) -> Vec<(String, String)> {
        self.session
            .iter()
            .filter(|(name, _)| {
                !name.is_empty()
                    && name
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_')
            })
            .map(|(name, value)| (format!("hasura.{}", name), value.clone()))
            .collect()
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
