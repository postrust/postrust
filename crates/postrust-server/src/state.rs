//! Application state.

use postrust_auth::{JwtCache, JwtConfig};
use postrust_core::{AppConfig, SchemaCache};
use sqlx::PgPool;
use tokio::sync::RwLock;

/// Shared application state.
pub struct AppState {
    /// Database connection pool
    pub pool: PgPool,
    /// Cached schema metadata
    pub schema_cache: RwLock<SchemaCache>,
    /// Application configuration
    pub config: AppConfig,
    /// JWT configuration
    pub jwt_config: JwtConfig,
    /// Cache of validated tokens, when `jwt_cache_enabled` asks for one.
    pub jwt_cache: Option<JwtCache>,
}

impl AppState {
    /// Get a read lock on the schema cache.
    pub async fn schema_cache(&self) -> tokio::sync::RwLockReadGuard<'_, SchemaCache> {
        self.schema_cache.read().await
    }

    /// Reload the schema cache. Used by the `db_channel_enabled` listener.
    pub async fn reload_schema(&self) -> Result<(), postrust_core::Error> {
        let new_cache = SchemaCache::load_with_search_path(
            &self.pool,
            &self.config.db_schemas,
            &self.config.db_extra_search_path,
        )
        .await?;
        let mut guard = self.schema_cache.write().await;
        *guard = new_cache;
        Ok(())
    }

    /// Get the default schema.
    pub fn default_schema(&self) -> &str {
        self.config.default_schema()
    }

    /// Get exposed schemas.
    pub fn schemas(&self) -> &[String] {
        &self.config.db_schemas
    }
}
