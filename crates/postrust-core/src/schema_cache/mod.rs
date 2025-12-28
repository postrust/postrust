//! PostgreSQL schema introspection and caching.
//!
//! This module provides functionality to discover and cache database schema
//! metadata including tables, columns, relationships, and functions.

mod table;
mod relationship;
mod routine;
mod queries;

pub use table::{Table, Column, ColumnMap, TablesMap};
pub use relationship::{Relationship, Cardinality, Junction, RelationshipsMap};
pub use routine::{Routine, RoutineParam, RetType, FuncVolatility, RoutineMap};

use crate::api_request::QualifiedIdentifier;
use crate::error::{Error, Result};
use sqlx::PgPool;
use std::collections::HashSet;
use std::sync::Arc;
use tracing::info;

/// Cached PostgreSQL schema metadata.
#[derive(Clone, Debug)]
pub struct SchemaCache {
    /// Tables and views by qualified identifier.
    pub tables: TablesMap,
    /// Relationships between tables.
    pub relationships: RelationshipsMap,
    /// Stored functions/procedures.
    pub routines: RoutineMap,
    /// Valid timezone names.
    pub timezones: HashSet<String>,
    /// PostgreSQL version.
    pub pg_version: i32,
}

impl SchemaCache {
    /// Load schema cache from the database.
    pub async fn load(pool: &PgPool, schemas: &[String]) -> Result<Self> {
        info!("Loading schema cache for schemas: {:?}", schemas);

        // Get PostgreSQL version
        let pg_version = queries::get_pg_version(pool).await?;
        info!("PostgreSQL version: {}", pg_version);

        // Load tables and columns
        let tables = queries::load_tables(pool, schemas).await?;
        info!("Loaded {} tables/views", tables.len());

        // Load relationships
        let relationships = queries::load_relationships(pool, schemas).await?;
        info!("Loaded {} relationship sets", relationships.len());

        // Load routines
        let routines = queries::load_routines(pool, schemas).await?;
        info!("Loaded {} routines", routines.len());

        // Load timezone names
        let timezones = queries::load_timezones(pool).await?;
        info!("Loaded {} timezones", timezones.len());

        Ok(Self {
            tables,
            relationships,
            routines,
            timezones,
            pg_version,
        })
    }

    /// Get a table by qualified identifier.
    pub fn get_table(&self, qi: &QualifiedIdentifier) -> Option<&Table> {
        self.tables.get(qi)
    }

    /// Get a table, returning an error if not found.
    pub fn require_table(&self, qi: &QualifiedIdentifier) -> Result<&Table> {
        self.get_table(qi)
            .ok_or_else(|| Error::TableNotFound(qi.to_string()))
    }

    /// Get relationships for a table.
    pub fn get_relationships(&self, qi: &QualifiedIdentifier, schema: &str) -> Option<&Vec<Relationship>> {
        self.relationships.get(&(qi.clone(), schema.to_string()))
    }

    /// Get a routine by qualified identifier.
    pub fn get_routines(&self, qi: &QualifiedIdentifier) -> Option<&Vec<Routine>> {
        self.routines.get(qi)
    }

    /// Check if a timezone is valid.
    pub fn is_valid_timezone(&self, tz: &str) -> bool {
        self.timezones.contains(tz)
    }

    /// Get a summary of the cached schema.
    pub fn summary(&self) -> String {
        format!(
            "SchemaCache: {} tables, {} relationship sets, {} routines, PG {}",
            self.tables.len(),
            self.relationships.len(),
            self.routines.len(),
            self.pg_version
        )
    }

    /// Find a relationship between two tables by name.
    pub fn find_relationship(
        &self,
        from: &QualifiedIdentifier,
        to_name: &str,
        schema: &str,
    ) -> Option<&Relationship> {
        self.get_relationships(from, schema)?
            .iter()
            .find(|r| match r {
                Relationship::ForeignKey { foreign_table, .. } => {
                    foreign_table.name == to_name
                }
                Relationship::Computed { foreign_table, .. } => {
                    foreign_table.name == to_name
                }
            })
    }
}

/// Thread-safe schema cache wrapper.
#[derive(Clone)]
pub struct SchemaCacheRef(Arc<tokio::sync::RwLock<Option<SchemaCache>>>);

impl SchemaCacheRef {
    /// Create a new empty schema cache reference.
    pub fn new() -> Self {
        Self(Arc::new(tokio::sync::RwLock::new(None)))
    }

    /// Create a schema cache reference from a static cache.
    pub fn from_static(cache: SchemaCache) -> Self {
        Self(Arc::new(tokio::sync::RwLock::new(Some(cache))))
    }

    /// Load or reload the schema cache.
    pub async fn load(&self, pool: &PgPool, schemas: &[String]) -> Result<()> {
        let cache = SchemaCache::load(pool, schemas).await?;
        let mut guard = self.0.write().await;
        *guard = Some(cache);
        Ok(())
    }

    /// Get a read reference to the schema cache.
    pub async fn get(&self) -> Result<tokio::sync::RwLockReadGuard<'_, Option<SchemaCache>>> {
        let guard = self.0.read().await;
        if guard.is_none() {
            return Err(Error::SchemaCacheNotLoaded);
        }
        Ok(guard)
    }

    /// Check if the cache is loaded.
    pub async fn is_loaded(&self) -> bool {
        self.0.read().await.is_some()
    }
}

impl Default for SchemaCacheRef {
    fn default() -> Self {
        Self::new()
    }
}
