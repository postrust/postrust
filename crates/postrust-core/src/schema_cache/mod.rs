//! PostgreSQL schema introspection and caching.
//!
//! This module provides functionality to discover and cache database schema
//! metadata including tables, columns, relationships, and functions.

mod queries;
mod relationship;
mod routine;
mod table;

pub use relationship::{
    Cardinality, Junction, MediaHandler, MediaHandlerMap, Relationship, RelationshipsMap,
};
pub use routine::{FuncVolatility, RetType, Routine, RoutineMap, RoutineParam};
pub use table::{Column, ColumnMap, Table, TablesMap};

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
    /// User-defined renderers for media types, by (schema, media type).
    pub media_handlers: MediaHandlerMap,
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

        // Which base-table columns each view carries. Loaded before the
        // relationships, because a view's base table may live in a schema
        // that is not itself exposed -- `private.articles` behind a public
        // view -- and its foreign keys are needed all the same.
        let view_columns = queries::load_view_columns(pool, schemas).await?;

        let mut relationship_schemas: Vec<String> = schemas.to_vec();
        for (_, base, _) in &view_columns {
            if !relationship_schemas.contains(&base.schema) {
                relationship_schemas.push(base.schema.clone());
            }
        }

        // Load relationships
        let relationships = queries::load_relationships(pool, &relationship_schemas).await?;
        info!("Loaded {} relationship sets", relationships.len());

        // Load routines
        let routines = queries::load_routines(pool, schemas).await?;
        info!("Loaded {} routines", routines.len());

        // Load timezone names
        let timezones = queries::load_timezones(pool).await?;
        info!("Loaded {} timezones", timezones.len());

        let mut relationships = relationships;
        add_view_relationships(&view_columns, &mut relationships);
        add_junction_relationships(&tables, &mut relationships);

        let media_handlers = queries::load_media_handlers(pool, schemas).await?;
        info!("Loaded {} media type handlers", media_handlers.len());

        Ok(Self {
            tables,
            relationships,
            routines,
            timezones,
            pg_version,
            media_handlers,
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
    pub fn get_relationships(
        &self,
        qi: &QualifiedIdentifier,
        schema: &str,
    ) -> Option<&Vec<Relationship>> {
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

    /// The table whose rows a function returns, if it returns rows of one.
    ///
    /// This is what makes a function behave like a table for everything after
    /// the call: its result can be selected from, filtered, ordered, paged and
    /// embedded on, all of which need a table to resolve columns against.
    ///
    /// `format_type` may or may not qualify the name, depending on the search
    /// path it was rendered against, so both spellings are tried -- and an
    /// unqualified one is looked for in the function's own schema.
    pub fn routine_returned_table(&self, qi: &QualifiedIdentifier) -> Option<&Table> {
        let routine = self.get_routines(qi)?.first()?;
        let type_name = routine.return_type.type_name()?;
        let candidate = match type_name.split_once('.') {
            Some((schema, name)) => QualifiedIdentifier::new(schema, name),
            None => QualifiedIdentifier::new(&qi.schema, type_name),
        };
        self.get_table(&candidate)
    }

    /// Find a user-defined renderer for a media type on a given table.
    ///
    /// A handler declared for the table itself wins over one taking
    /// `anyelement`, which is the schema-wide fallback.
    pub fn media_handler(
        &self,
        schema: &str,
        media_type: &str,
        table: &QualifiedIdentifier,
    ) -> Option<&MediaHandler> {
        let candidates = self
            .media_handlers
            .get(&(schema.to_string(), media_type.to_string()))?;
        candidates
            .iter()
            .find(|h| h.table.as_ref() == Some(table))
            .or_else(|| candidates.iter().find(|h| h.table.is_none()))
    }

    /// Find a relationship between two tables by name.
    ///
    /// `hint` is the `!hint` of `person_detail!message_sender_fkey(name)`,
    /// which names the relationship when the two resources are connected by
    /// more than one.
    ///
    /// Returns `Ok(None)` when nothing matches, and an error when several do
    /// and no hint told them apart -- picking one arbitrarily would answer a
    /// question the client did not ask.
    pub fn find_relationship(
        &self,
        from: &QualifiedIdentifier,
        to_name: &str,
        hint: Option<&str>,
        schema: &str,
    ) -> Result<Option<&Relationship>> {
        let Some(candidates) = self.get_relationships(from, schema) else {
            return Ok(None);
        };

        // An embedding may be named three ways, and they are tried in the
        // order of how specifically they identify one relationship.
        //
        // The target table's name is the usual spelling, but it stops
        // identifying anything as soon as two foreign keys point at the same
        // table. The foreign key constraint names exactly one relationship,
        // and so does the column that joins them -- `/messages?select=*,
        // sender(*)` embeds through the `sender` column, and
        // `/projects?select=*,client(*)` through the constraint called
        // `client`.
        let by_target: Vec<&Relationship> = candidates
            .iter()
            .filter(|r| match r {
                // A computed relationship is named by its function. Two
                // functions may return the same table, so the table's name
                // would not tell them apart.
                Relationship::Computed { function, .. } => function.name == to_name,
                Relationship::ForeignKey { foreign_table, .. } => foreign_table.name == to_name,
            })
            .collect();

        // A relationship projected onto a view carries the constraint and
        // columns of the base one it came from, so naming either of those
        // would match both. Only the view's own name selects the projection;
        // the constraint and the column mean the relationship they belong to.
        // PostgreSQL cannot point a foreign key at a view, so a view on the
        // far side is exactly what marks a projection.
        let declared = |r: &&Relationship| !matches!(r, Relationship::ForeignKey { foreign_table_is_view, .. } if *foreign_table_is_view);

        let mut matches = match by_target.is_empty() {
            false => by_target,
            true => {
                let by_constraint: Vec<&Relationship> = candidates
                    .iter()
                    .filter(declared)
                    .filter(|r| r.constraint_name() == to_name)
                    .collect();
                match by_constraint.is_empty() {
                    false => by_constraint,
                    true => candidates
                        .iter()
                        .filter(declared)
                        .filter(|r| r.join_columns().iter().any(|(local, _)| local == to_name))
                        .collect(),
                }
            }
        };

        if let Some(hint) = hint {
            matches.retain(|r| r.matches_hint(hint));
        }

        match matches.len() {
            0 => Ok(None),
            1 => Ok(Some(matches[0])),
            _ => Err(Error::AmbiguousRelationship {
                origin: from.name.clone(),
                target: to_name.to_string(),
                candidates: matches
                    .iter()
                    .map(|r| (r.cardinality_name().to_string(), r.describe()))
                    .collect(),
            }),
        }
    }
}

/// Project a base table's relationships onto the views that select from it.
///
/// A view has no foreign keys, but embedding one is ordinary usage: what makes
/// it possible is that the view carries the columns the key joins on. So every
/// relationship of a base table is offered to each view over it that keeps
/// those columns, on both sides -- a view can be embedded, and can embed.
fn add_view_relationships(
    view_columns: &[(QualifiedIdentifier, QualifiedIdentifier, String)],
    relationships: &mut RelationshipsMap,
) {
    use std::collections::HashMap;

    // base table -> the views over it, each with the columns it carries
    let mut views_over: HashMap<&QualifiedIdentifier, HashMap<&QualifiedIdentifier, Vec<&str>>> =
        HashMap::new();
    for (view, base, column) in view_columns {
        views_over
            .entry(base)
            .or_default()
            .entry(view)
            .or_default()
            .push(column);
    }

    let existing: Vec<((QualifiedIdentifier, String), Relationship)> = relationships
        .iter()
        .flat_map(|(key, rels)| rels.iter().map(move |rel| (key.clone(), rel.clone())))
        .collect();

    let mut derived: Vec<((QualifiedIdentifier, String), Relationship)> = Vec::new();

    for ((source, _), rel) in &existing {
        let Relationship::ForeignKey {
            foreign_table,
            cardinality,
            table_is_view,
            foreign_table_is_view,
            constraint_name,
            ..
        } = rel
        else {
            continue;
        };
        // A many-to-many is derived from two others; projecting it as well
        // would double up on whatever those produce.
        if matches!(cardinality, Cardinality::M2M(_)) {
            continue;
        }

        let columns = cardinality.columns();
        let carries = |view_cols: &Vec<&str>, wanted: &dyn Fn(&(String, String)) -> String| {
            columns
                .iter()
                .all(|pair| view_cols.iter().any(|c| *c == wanted(pair)))
        };

        // The view stands in for the near side.
        if let Some(views) = views_over.get(source) {
            for (view, view_cols) in views {
                if !carries(view_cols, &|(local, _)| local.clone()) {
                    continue;
                }
                derived.push((
                    ((*view).clone(), view.schema.clone()),
                    Relationship::ForeignKey {
                        table: (*view).clone(),
                        foreign_table: foreign_table.clone(),
                        is_self: *view == foreign_table,
                        cardinality: cardinality.clone(),
                        table_is_view: true,
                        foreign_table_is_view: *foreign_table_is_view,
                        constraint_name: constraint_name.clone(),
                    },
                ));
            }
        }

        // The view stands in for the far side.
        if let Some(views) = views_over.get(foreign_table) {
            for (view, view_cols) in views {
                if !carries(view_cols, &|(_, foreign)| foreign.clone()) {
                    continue;
                }
                derived.push((
                    (source.clone(), source.schema.clone()),
                    Relationship::ForeignKey {
                        table: source.clone(),
                        foreign_table: (*view).clone(),
                        is_self: source == *view,
                        cardinality: cardinality.clone(),
                        table_is_view: *table_is_view,
                        foreign_table_is_view: true,
                        constraint_name: constraint_name.clone(),
                    },
                ));
            }
        }

        // Both sides are views. Neither of the cases above covers it: each
        // replaces one end and leaves the other as the base table.
        if let (Some(near), Some(far)) = (views_over.get(source), views_over.get(foreign_table)) {
            for (near_view, near_cols) in near {
                if !carries(near_cols, &|(local, _)| local.clone()) {
                    continue;
                }
                for (far_view, far_cols) in far {
                    if !carries(far_cols, &|(_, foreign)| foreign.clone()) {
                        continue;
                    }
                    derived.push((
                        ((*near_view).clone(), near_view.schema.clone()),
                        Relationship::ForeignKey {
                            table: (*near_view).clone(),
                            foreign_table: (*far_view).clone(),
                            is_self: near_view == far_view,
                            cardinality: cardinality.clone(),
                            table_is_view: true,
                            foreign_table_is_view: true,
                            constraint_name: constraint_name.clone(),
                        },
                    ));
                }
            }
        }
    }

    for (key, rel) in derived {
        relationships.entry(key).or_default().push(rel);
    }
}

/// Derive many-to-many relationships from junction tables.
///
/// A junction is a table that exists only to join two others: it has exactly
/// two foreign keys, to two different tables, and its primary key is precisely
/// the columns of those keys. That last condition is what separates a junction
/// from an ordinary table that happens to reference two others, which would
/// otherwise sprout a relationship it has no business having.
///
/// Both sides get the relationship, named after the table across the junction,
/// so `/users?select=name,tasks(name)` works from either end.
fn add_junction_relationships(tables: &TablesMap, relationships: &mut RelationshipsMap) {
    let mut derived: Vec<((QualifiedIdentifier, String), Relationship)> = Vec::new();

    for (junction_qi, junction) in tables {
        let key = (junction_qi.clone(), junction_qi.schema.clone());
        let Some(rels) = relationships.get(&key) else {
            continue;
        };

        // The single-column keys out of the junction. A junction joins two
        // tables by one column each; a composite key to somewhere else is a
        // different relationship that happens to share the table.
        let outgoing: Vec<(&Relationship, (String, String))> = rels
            .iter()
            .filter(|r| matches!(r, Relationship::ForeignKey { table, .. } if table == junction_qi))
            .filter(|r| r.is_to_one())
            .filter_map(|r| {
                let mut columns = r.join_columns();
                columns.dedup();
                match columns.len() {
                    1 => Some((r, columns.remove(0))),
                    _ => None,
                }
            })
            .collect();

        let mut pk: Vec<&str> = junction.pk_cols.iter().map(String::as_str).collect();
        pk.sort_unstable();
        if pk.len() != 2 {
            continue;
        }

        // The pair whose columns are exactly the primary key is the one that
        // makes this a junction. Requiring the whole key is what separates a
        // junction from a table that merely references two others.
        for near in 0..outgoing.len() {
            for far in 0..outgoing.len() {
                if near == far {
                    continue;
                }
                let (near_rel, (near_local, near_foreign)) = &outgoing[near];
                let (far_rel, (far_local, far_foreign)) = &outgoing[far];
                if near_rel.foreign_table() == far_rel.foreign_table() {
                    continue;
                }
                let mut covered = vec![near_local.as_str(), far_local.as_str()];
                covered.sort_unstable();
                if covered != pk {
                    continue;
                }

                let source = near_rel.foreign_table().clone();
                let target = far_rel.foreign_table().clone();

                derived.push((
                    (source.clone(), source.schema.clone()),
                    Relationship::ForeignKey {
                        table: source,
                        foreign_table: target,
                        is_self: false,
                        cardinality: Cardinality::M2M(Junction {
                            table: junction_qi.clone(),
                            constraint1: near_rel.constraint_name().to_string(),
                            constraint2: far_rel.constraint_name().to_string(),
                            // Source to junction, then junction to target.
                            source_columns: vec![(near_foreign.clone(), near_local.clone())],
                            target_columns: vec![(far_local.clone(), far_foreign.clone())],
                        }),
                        table_is_view: false,
                        foreign_table_is_view: false,
                        constraint_name: String::new(),
                    },
                ));
            }
        }
    }

    for (key, rel) in derived {
        relationships.entry(key).or_default().push(rel);
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
