//! What a request runs against: a database, a schema cache, configuration.

use postrust_auth::{JwtCache, JwtConfig};
use postrust_core::db::{Db, MaybeSend, MaybeSync};
use postrust_core::{AppConfig, SchemaCache};
use std::future::Future;
use std::ops::Deref;

/// Everything the pipeline needs from its host.
///
/// The server's `AppState` is one, over a sqlx pool. A host with no sqlx -- a
/// Cloudflare Worker -- brings its own, over its own [`Db`]. The pipeline is
/// generic over this, so nothing on the native path goes through a vtable.
pub trait State: MaybeSync {
    /// The database requests run on.
    type Db: Db;

    /// A borrow of the schema cache, held for as long as a request needs it.
    type SchemaCache<'a>: Deref<Target = SchemaCache> + MaybeSend + MaybeSync
    where
        Self: 'a;

    /// The database requests run on.
    fn db(&self) -> &Self::Db;

    /// The schema cache.
    fn schema_cache(&self) -> impl Future<Output = Self::SchemaCache<'_>> + MaybeSend;

    /// Configuration.
    fn config(&self) -> &AppConfig;

    /// How tokens are verified.
    fn jwt_config(&self) -> &JwtConfig;

    /// The cache of verified tokens, where one is configured.
    fn jwt_cache(&self) -> Option<&JwtCache>;

    /// The schema a request names when it names none.
    fn default_schema(&self) -> &str {
        self.config().default_schema()
    }

    /// The schemas the API exposes.
    fn schemas(&self) -> &[String] {
        &self.config().db_schemas
    }
}
