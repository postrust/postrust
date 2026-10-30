//! Application state.

use crate::error::Error;
use crate::user::User;
use postrust_auth::{JwtCache, JwtConfig, JwtKeyError};
use postrust_core::{AppConfig, IsolationLevel, SchemaCache};
use postrust_engine::State;
use sqlx::{PgPool, Postgres, Transaction};
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
    /// secret does not parse; see [`postrust_engine::jwt_config`]. With
    /// `PGRST_JWT_JWKS_URL` set the keys are fetched when first needed, by
    /// the request that needs them; [`build`](crate::build) also fetches them
    /// before serving, and keeps them fresh in the background.
    pub fn new(
        pool: PgPool,
        schema_cache: SchemaCache,
        config: AppConfig,
    ) -> Result<Self, JwtKeyError> {
        Ok(Self {
            pool,
            schema_cache: RwLock::new(schema_cache),
            jwt_config: crate::jwks::attach(postrust_engine::jwt_config(&config)?, &config)?,
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

    /// A transaction on the pool that runs as `user`, as a REST request
    /// would: read-write, at the isolation level the configuration gives the
    /// role, with `db_pre_request` run. See [`AppState::begin_as_with`].
    pub async fn begin_as(&self, user: &User) -> Result<Transaction<'static, Postgres>, Error> {
        self.begin_as_with(user, TxOptions::default()).await
    }

    /// A transaction on the pool that runs as `user`.
    ///
    /// The pool is logged in as the authenticator, and a query sent on it
    /// runs as the authenticator: with every grant it has, and with no
    /// `request.jwt.claims` for a row-level security policy to read, so the
    /// policies written for the API's roles do not apply. This makes the
    /// transaction the caller's before you send anything on it. The same
    /// statements REST sends for a request, worked out by the same
    /// [`Session`](postrust_core::Session): `SET TRANSACTION` where one is
    /// asked for, the role, the claims, the facts about the request `user`
    /// carries, the `search_path` and `app.settings.*`, and `db_pre_request`.
    ///
    /// Every one of them is local to the transaction. Commit it or drop it --
    /// a dropped one is rolled back -- and the connection goes back to the
    /// pool as the authenticator, with nothing set.
    ///
    /// What this does not do is make your handler untrusted code. It keeps
    /// what a request *sends* inside the policy: every statement you run with
    /// the caller's values in it runs as the caller. But the handler runs as
    /// the authenticator, which may always switch back -- `RESET ROLE` on the
    /// transaction is enough -- and it holds the pool. Treat it as you would a
    /// `SECURITY DEFINER` function: code you wrote and reviewed, not something
    /// the client controls.
    ///
    /// Fails as REST fails: a connection that cannot be had from the pool in
    /// time is logged at error and answered 500 (`PGRST500`), and a session the database
    /// refuses -- a role that does not exist, a pre-request function that
    /// raises -- is answered as the database's error.
    pub async fn begin_as_with(
        &self,
        user: &User,
        options: TxOptions,
    ) -> Result<Transaction<'static, Postgres>, Error> {
        // Logged here, at error, as the REST pipeline logs it: the pool
        // running dry or the database out of reach is the operator's to fix,
        // and the client is told only that the service is unavailable. The
        // line carries the cause, and the request span's `request_id` and
        // `trace_id` with it.
        let mut tx = self.pool.begin().await.map_err(|e| {
            let e = postrust_core::db::DbError::from(e);
            tracing::error!("Could not begin a transaction: {}", e);
            postrust_core::Error::ConnectionPool(e.to_string())
        })?;

        user.session(&self.config)
            .read_only(options.read_only)
            .isolation(options.isolation)
            .pre_request(options.run_pre_request)
            .apply_sqlx(&mut tx)
            .await
            .map_err(|e| {
                tracing::debug!("Refused before the query: {}", e);
                Error::from(e)
            })?;
        Ok(tx)
    }
}

/// How [`AppState::begin_as_with`] begins a transaction.
///
/// `#[non_exhaustive]`, so a knob can be added in a minor release: start from
/// the default and change what should differ.
///
/// ```
/// use postrust_core::IsolationLevel;
/// use postrust_server::TxOptions;
///
/// let options = TxOptions::default()
///     .read_only(true)
///     .isolation(IsolationLevel::RepeatableRead);
/// assert!(options.read_only && options.run_pre_request);
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct TxOptions {
    /// Whether the transaction is read-only, which PostgreSQL enforces for
    /// everything the transaction reaches -- a volatile function, a trigger.
    /// A write is refused with `25006`, answered 405 as REST answers it.
    /// Off by default.
    pub read_only: bool,
    /// The isolation level, over the role's (`role_settings`) and the
    /// server's (`PGRST_DB_TX_ISOLATION`). `None`, the default, leaves the
    /// choice to them, as REST does.
    pub isolation: Option<IsolationLevel>,
    /// Whether to call `db_pre_request`, where one is configured. On by
    /// default, as it is for every REST request: it is commonly where a
    /// deployment refuses a caller, and a route that skips it skips that.
    pub run_pre_request: bool,
}

impl Default for TxOptions {
    fn default() -> Self {
        Self {
            read_only: false,
            isolation: None,
            run_pre_request: true,
        }
    }
}

impl TxOptions {
    /// With `read_only` set. See [`TxOptions::read_only`](#structfield.read_only).
    pub fn read_only(mut self, read_only: bool) -> Self {
        self.read_only = read_only;
        self
    }

    /// At `isolation`, whatever the configuration says.
    pub fn isolation(mut self, isolation: IsolationLevel) -> Self {
        self.isolation = Some(isolation);
        self
    }

    /// With `run_pre_request` set.
    pub fn run_pre_request(mut self, run_pre_request: bool) -> Self {
        self.run_pre_request = run_pre_request;
        self
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
