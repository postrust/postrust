//! Application state.

use postrust_auth::{JwtCache, JwtConfig, JwtKeyError};
use postrust_core::{AppConfig, SchemaCache};
use postrust_engine::State;
use sqlx::PgPool;
use tokio::sync::{RwLock, RwLockReadGuard};

/// Shared application state.
///
/// `#[non_exhaustive]` from 3.0.0: build one with [`AppState::new`], then
/// assign any field that should differ from what the configuration says.
#[non_exhaustive]
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
    /// State for `config`, serving `schema_cache` through `pool`.
    ///
    /// The JWT configuration and token cache are derived from `config`, as
    /// [`build`](crate::build) derives them. Fails when the configured JWT
    /// secret does not parse; see [`postrust_engine::jwt_config`].
    pub fn new(
        pool: PgPool,
        schema_cache: SchemaCache,
        config: AppConfig,
    ) -> Result<Self, JwtKeyError> {
        Ok(Self {
            pool,
            schema_cache: RwLock::new(schema_cache),
            jwt_config: postrust_engine::jwt_config(&config)?,
            jwt_cache: JwtCache::new(config.jwt_cache_enabled, config.jwt_cache_max_lifetime),
            config,
        })
    }

    /// Get a read lock on the schema cache.
    pub async fn schema_cache(&self) -> RwLockReadGuard<'_, SchemaCache> {
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

impl State for AppState {
    type Db = PgPool;
    type SchemaCache<'a> = RwLockReadGuard<'a, SchemaCache>;

    fn db(&self) -> &PgPool {
        &self.pool
    }

    async fn schema_cache(&self) -> RwLockReadGuard<'_, SchemaCache> {
        self.schema_cache.read().await
    }

    fn config(&self) -> &AppConfig {
        &self.config
    }

    fn jwt_config(&self) -> &JwtConfig {
        &self.jwt_config
    }

    fn jwt_cache(&self) -> Option<&JwtCache> {
        self.jwt_cache.as_ref()
    }
}
