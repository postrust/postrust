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
pub use table::{Column, ColumnMap, ComputedColumn, Table, TablesMap};

use crate::api_request::QualifiedIdentifier;
use crate::error::{Error, Result};
use sqlx::PgPool;
use std::collections::{HashMap, HashSet};
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
    /// Casts between a domain and `json`/`text`, by (source, target) type.
    ///
    /// A schema uses these to decide how a value of one of its domains is
    /// written on the wire and how one written by a client is read back.
    pub representations: std::collections::HashMap<(String, String), String>,
}

impl SchemaCache {
    /// Load schema cache from the database.
    pub async fn load(pool: &PgPool, schemas: &[String]) -> Result<Self> {
        Self::load_with_search_path(pool, schemas, &[]).await
    }

    /// Load the cache, resolving functions along `extra_search_path` as well.
    ///
    /// A computed column's function is reached the way the database reaches
    /// it, which is along the search path -- so it may live in a schema the
    /// API does not expose. The table it reads must still be exposed.
    pub async fn load_with_search_path(
        pool: &PgPool,
        schemas: &[String],
        extra_search_path: &[String],
    ) -> Result<Self> {
        info!("Loading schema cache for schemas: {:?}", schemas);

        let function_schemas: Vec<String> = schemas
            .iter()
            .chain(extra_search_path)
            .cloned()
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();

        // Get PostgreSQL version
        let pg_version = queries::get_pg_version(pool).await?;
        info!("PostgreSQL version: {}", pg_version);

        // Load tables and columns
        let mut tables = queries::load_tables(pool, schemas).await?;
        info!("Loaded {} tables/views", tables.len());

        // Which base-table columns each view carries. Loaded before the
        // relationships, because a view's base table may live in a schema
        // that is not itself exposed -- `private.articles` behind a public
        // view -- and its foreign keys are needed all the same.
        let view_columns = queries::load_view_columns(pool, schemas).await?;

        // Functions that read as columns. Attached to the tables they belong
        // to, so resolving a field name has one place to look.
        for computed in queries::load_computed_columns(pool, schemas, &function_schemas).await? {
            if let Some(table) = tables.get_mut(&computed.table) {
                table.computed_columns.insert(
                    computed.name,
                    ComputedColumn {
                        function: computed.function,
                        data_type: computed.return_type,
                        description: computed.description,
                        row_argument: computed.row_argument,
                        session_argument: computed.session_argument,
                        takes_arguments: computed.takes_arguments,
                    },
                );
            }
        }

        let mut relationship_schemas: Vec<String> = schemas.to_vec();
        for column in &view_columns {
            if !relationship_schemas.contains(&column.base.schema) {
                relationship_schemas.push(column.base.schema.clone());
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

        // Junctions first, then views: a many-to-many derived from a
        // junction is itself a relationship a view can carry, and the
        // junction may live in a schema that is not exposed.
        let primary_keys = queries::load_primary_keys(pool, &relationship_schemas).await?;

        // Which uniqueness an upsert resolves against has to be named, and a
        // table may have several.
        let mut tables = tables;
        for (qi, constraints) in queries::load_unique_constraints(pool, schemas).await? {
            if let Some(table) = tables.get_mut(&qi) {
                table.unique_constraints = constraints;
            }
        }

        let mut relationships = relationships;
        add_junction_relationships(&primary_keys, &mut relationships);
        substitute_hidden_junctions(&tables, &view_columns, &mut relationships);
        add_view_relationships(&tables, &view_columns, &mut relationships);

        // A view over a junction is a junction. It has no key of its own, so
        // this runs only once the view's own relationships exist -- which is
        // why the pass is here and not with the first one.
        let view_keys = view_primary_keys(&primary_keys, &view_columns);
        add_junction_relationships(&view_keys, &mut relationships);

        let media_handlers = queries::load_media_handlers(pool, schemas).await?;
        info!("Loaded {} media type handlers", media_handlers.len());

        let representations = queries::load_representations(pool).await?;
        info!("Loaded {} data representations", representations.len());

        Ok(Self {
            tables,
            relationships,
            routines,
            timezones,
            pg_version,
            media_handlers,
            representations,
        })
    }

    /// Get a table by qualified identifier.
    pub fn get_table(&self, qi: &QualifiedIdentifier) -> Option<&Table> {
        self.tables.get(qi)
    }

    /// Get a table, returning an error if not found.
    pub fn require_table(&self, qi: &QualifiedIdentifier) -> Result<&Table> {
        self.get_table(qi).ok_or_else(|| Error::TableNotFound {
            name: qi.to_string(),
            suggestion: self.similar_table(qi),
        })
    }

    /// The exposed table whose name is closest to one that does not exist.
    ///
    /// PostgREST answers a schema-cache miss with a suggestion where there is
    /// an obviously intended table -- `projectx` for `projects` -- and with
    /// nothing where there is not, which is what the similarity floor decides.
    /// Below it the "suggestion" would be noise, and would also say more about
    /// the schema than the client asked.
    fn similar_table(&self, qi: &QualifiedIdentifier) -> Option<String> {
        const MIN_SIMILARITY: f64 = 0.5;

        // Scored by shared character n-grams rather than by edit distance.
        // A Levenshtein ratio cannot tell `items` from `items3` as the table
        // `itemsx` was meant to be -- both are one edit away, so both score
        // 0.833 and the winner is whichever the map happened to yield last.
        // Counting 3-grams separates them, because it notices that one
        // candidate is the query's own length and the other is not: `items`
        // scores 0.73 against `itemsx` and `items3` scores 0.67.
        let asked = grams(&qi.name, 3);

        self.tables
            .keys()
            .filter(|candidate| candidate.schema == qi.schema)
            .map(|candidate| (cosine(&asked, &grams(&candidate.name, 3)), candidate))
            .filter(|(score, _)| *score >= MIN_SIMILARITY)
            // Ties go to the shorter name, and then to the earlier one, so
            // that the suggestion does not depend on iteration order.
            .max_by(|(a, left), (b, right)| {
                a.total_cmp(b)
                    .then_with(|| right.name.len().cmp(&left.name.len()))
                    .then_with(|| right.name.cmp(&left.name))
            })
            .map(|(_, candidate)| candidate.to_string())
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
        self.returned_table(self.get_routines(qi)?.first()?)
    }

    /// The same, for a routine already chosen.
    ///
    /// A name may carry several signatures returning different things, so
    /// which overload was selected decides which table the result is shaped
    /// by -- and only the caller knows that.
    pub fn returned_table(&self, routine: &Routine) -> Option<&Table> {
        // Only a row type names a table. Without this, a function returning
        // `xml` is shaped by whatever relation happens to be called `xml` --
        // and the fixtures have one, so this is not hypothetical.
        if !routine.returns_composite {
            return None;
        }
        let type_name = routine.return_type.type_name()?;
        let candidate = match type_name.split_once('.') {
            Some((schema, name)) => QualifiedIdentifier::new(schema, name),
            None => QualifiedIdentifier::new(&routine.schema, type_name),
        };
        self.get_table(&candidate)
    }

    /// The function that renders a value of `pg_type` as `target`, if the
    /// schema declared one.
    pub fn representation(&self, pg_type: &str, target: &str) -> Option<&str> {
        self.representations
            .get(&(pg_type.to_string(), target.to_string()))
            .map(String::as_str)
    }

    /// Find a user-defined renderer for a media type on a given table.
    ///
    /// A handler declared for the table itself wins over one taking
    /// `anyelement`, which is the schema-wide fallback.
    /// A schema may also declare a handler for `*/*`, which renders whatever
    /// was asked for. What comes back is the handler and the media type the
    /// response should be labelled with -- not always the one requested, since
    /// `*/*` is not a type a response can claim to be. PostgREST resolves it
    /// to `application/octet-stream`, which is what a body of unnamed bytes
    /// is, and this follows it.
    /// A handler declared for one table renders that table as it is, so it
    /// only applies to a request that asked for the table as it is:
    /// `?select=id` is a different shape from the one the handler was written
    /// against, and running it over that shape produces either nonsense or an
    /// error. PostgREST guards both table-specific lookups on the selection
    /// being the default one, and `default_select` is that guard.
    pub fn media_handler<'a>(
        &'a self,
        schema: &str,
        media_type: &'a str,
        table: &QualifiedIdentifier,
        default_select: bool,
    ) -> Option<(&'a MediaHandler, &'a str)> {
        let named = |media: &str| {
            let candidates = self
                .media_handlers
                .get(&(schema.to_string(), media.to_string()))?;
            candidates
                .iter()
                .find(|h| default_select && h.table.as_ref() == Some(table))
                .or_else(|| candidates.iter().find(|h| h.table.is_none()))
        };

        match named(media_type) {
            Some(handler) => Some((handler, media_type)),
            // `*/*` is declared on a table, so it is a table lookup too.
            None if default_select => {
                named("*/*").map(|handler| (handler, "application/octet-stream"))
            }
            None => None,
        }
    }

    /// The error to report when no relationship connects two resources.
    ///
    /// Built here rather than at the call site because the suggestion needs
    /// the relationships the origin actually has, which is what the client
    /// most likely meant to name.
    pub fn relationship_not_found(
        &self,
        from: &QualifiedIdentifier,
        to_name: &str,
        hint: Option<&str>,
        schema: &str,
    ) -> Error {
        // Looser than the table suggestion: a relationship is named by the far
        // table, and a client that got it wrong tends to have got it wrong by
        // a whole word -- `car_model_sales_202101` for `car_model_sales`.
        const MIN_SIMILARITY: f64 = 0.4;

        let related: Vec<&str> = self
            .get_relationships(from, schema)
            .into_iter()
            .flatten()
            .map(|rel| rel.foreign_table().name.as_str())
            .collect();

        // A relationship of exactly that name exists, so the name is not what
        // went wrong -- the hint is, and there is nothing to suggest. Dropping
        // only the exact match and then offering the next-nearest name told a
        // client that asked for `person!space` that it had perhaps meant
        // `person_detail`, when what it had meant was `person` all along.
        let suggestion = match related.contains(&to_name) {
            true => None,
            false => related
                .iter()
                .map(|name| (similarity(name, to_name), *name))
                .filter(|(score, _)| *score >= MIN_SIMILARITY)
                .max_by(|(a, _), (b, _)| a.total_cmp(b))
                .map(|(_, name)| name.to_string()),
        };

        Error::RelationshipNotFound {
            origin: from.name.clone(),
            target: to_name.to_string(),
            hint: hint.map(str::to_string),
            schema: schema.to_string(),
            suggestion,
        }
    }

    /// The exposed function whose name is closest to one that does not exist.
    ///
    /// Only when the name itself is wrong: an existing name whose arguments do
    /// not match is a different mistake, and the overloads answer it better.
    pub fn similar_routine(&self, qi: &QualifiedIdentifier) -> Option<String> {
        const MIN_SIMILARITY: f64 = 0.75;

        self.routines
            .keys()
            .filter(|candidate| candidate.schema == qi.schema)
            .map(|candidate| (similarity(&candidate.name, &qi.name), candidate))
            .filter(|(score, _)| *score >= MIN_SIMILARITY)
            .max_by(|(a, _), (b, _)| a.total_cmp(b))
            .map(|(_, candidate)| candidate.to_string())
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
                // One foreign key pointing at its own table is two
                // relationships, and the table's name alone cannot tell them
                // apart. The convention is PostgREST's: the table's name means
                // the rows that point here -- the children -- and the key's own
                // column means the row this one points at. A hint names the
                // column the children point with, and so always means children.
                r if r.is_self_referential() => match hint {
                    None => {
                        (r.foreign_table().name == to_name && r.is_one_to_many())
                            || (r.single_local_column() == Some(to_name) && !r.is_one_to_many())
                    }
                    Some(hint) => {
                        r.foreign_table().name == to_name
                            && r.is_one_to_many()
                            && r.single_foreign_column() == Some(hint)
                    }
                },
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

        // A computed relationship declared under a name overrides whatever the
        // catalogue found under it. That is deliberate on the schema author's
        // part -- naming the function after the table is how a schema replaces
        // the derived relationship with one of its own -- so the two are not
        // an ambiguity to report back.
        let by_target = match by_target
            .iter()
            .any(|r| matches!(r, Relationship::Computed { .. }))
        {
            true => by_target
                .into_iter()
                .filter(|r| matches!(r, Relationship::Computed { .. }))
                .collect(),
            false => by_target,
        };

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

        // A self-referential match has already accounted for the hint, and the
        // general rule would reject it: both directions join on the same pair
        // of columns, so the hint names them both.
        if let Some(hint) = hint {
            matches.retain(|r| r.is_self_referential() || r.matches_hint(hint));
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
                // What the client would have to write instead. The details
                // say which relationships were found; this says how to ask for
                // one of them, which is the part it can act on.
                disambiguated: matches
                    .iter()
                    .map(|r| format!("{}!{}", to_name, r.disambiguator()))
                    .collect(),
            }),
        }
    }
}

/// How alike two names are, from 0 (nothing in common) to 1 (identical).
///
/// Edit distance over the longer of the two, which is enough to tell a typo
/// from a different word.
pub(crate) fn similarity(a: &str, b: &str) -> f64 {
    let longest = a.chars().count().max(b.chars().count());
    if longest == 0 {
        return 1.0;
    }
    1.0 - edit_distance(a, b) as f64 / longest as f64
}

/// The closest of `candidates` to `query`, or `None` if none is close enough.
///
/// Scored the way PostgREST scores its hints: cosine similarity over the
/// character n-grams of both strings, three at a time and falling back to two
/// where three finds nothing, with the ends marked so that a shared prefix
/// counts for something. A Levenshtein ratio, which is the obvious thing to
/// reach for, answers differently on the cases that matter -- it calls
/// `(any_arg)` and `(name)` a third alike, because they are both short, where
/// sharing no letter sequence at all is what a client needs to be told.
///
/// The floor is strict, and applies to the similarity; among what clears it,
/// the nearest by edit distance wins. A floor of zero therefore means "shares
/// any sequence of characters at all", which is a real filter -- two strings
/// with no run of two characters in common score nothing and never reach the
/// ranking.
pub(crate) fn closest<'a>(
    candidates: impl IntoIterator<Item = &'a str> + Clone,
    query: &str,
    min_score: f64,
) -> Option<&'a str> {
    for size in [3usize, 2] {
        let asked = grams(query, size);
        let mut fitting: Vec<&str> = candidates
            .clone()
            .into_iter()
            .filter(|candidate| cosine(&asked, &grams(candidate, size)) > min_score)
            .collect();

        // Ranked by edit distance only among those the similarity admitted,
        // which is why a short unrelated name never surfaces: it never got
        // this far.
        fitting.sort_by(|a, b| {
            similarity(&normalize(b), &normalize(query))
                .total_cmp(&similarity(&normalize(a), &normalize(query)))
        });
        if let Some(best) = fitting.first() {
            return Some(best);
        }
    }
    None
}

/// A string reduced to what a comparison should see.
///
/// Punctuation is dropped, so `(name)` and `name` are the same word; commas
/// and spaces are kept, because a parameter list is compared as the list it
/// is written as.
fn normalize(value: &str) -> String {
    value
        .chars()
        .flat_map(char::to_lowercase)
        .filter(|c| c.is_alphanumeric() || *c == ',' || *c == ' ')
        .collect()
}

/// How many times each run of `size` characters occurs, ends marked.
fn grams(value: &str, size: usize) -> HashMap<String, usize> {
    let mut padded: Vec<char> = std::iter::once('-')
        .chain(normalize(value).chars())
        .chain(std::iter::once('-'))
        .collect();
    while padded.len() < size {
        padded.push('-');
    }

    let mut counts: HashMap<String, usize> = HashMap::new();
    for window in padded.windows(size) {
        *counts.entry(window.iter().collect()).or_insert(0) += 1;
    }
    counts
}

/// The cosine of the angle between two gram counts.
fn cosine(a: &HashMap<String, usize>, b: &HashMap<String, usize>) -> f64 {
    let magnitude = |counts: &HashMap<String, usize>| -> f64 {
        counts.values().map(|n| (n * n) as f64).sum::<f64>().sqrt()
    };
    let (a_size, b_size) = (magnitude(a), magnitude(b));
    if a_size == 0.0 || b_size == 0.0 {
        return 0.0;
    }
    let shared: f64 = a
        .iter()
        .map(|(gram, count)| (count * b.get(gram).copied().unwrap_or(0)) as f64)
        .sum();
    shared / (a_size * b_size)
}

/// Levenshtein distance, computed one row at a time.
fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    let mut current = vec![0; b.len() + 1];

    for (i, a_char) in a.chars().enumerate() {
        current[0] = i + 1;
        for (j, b_char) in b.iter().enumerate() {
            let substitution = previous[j] + usize::from(a_char != *b_char);
            current[j + 1] = substitution.min(previous[j + 1] + 1).min(current[j] + 1);
        }
        std::mem::swap(&mut previous, &mut current);
    }

    previous[b.len()]
}

/// Route a many-to-many through the exposed view of its junction.
///
/// The junction is often the one table of the three that the API does not
/// expose -- `private.personnages` behind `test.personnages` -- and joining
/// through the base table is a `permission denied for schema private` at
/// request time even though the relationship itself is real. Where a view over
/// it is exposed and carries both keys, that view is joined through instead.
fn substitute_hidden_junctions(
    tables: &TablesMap,
    view_columns: &[queries::ViewColumn],
    relationships: &mut RelationshipsMap,
) {
    use std::collections::HashMap;

    // Which name an exposed view gives each of a base table's columns.
    type ColumnRenames<'a> = HashMap<&'a str, &'a str>;
    /// The exposed views over one base table, each with its renames.
    type ExposedViews<'a> = Vec<(&'a QualifiedIdentifier, ColumnRenames<'a>)>;

    let mut views_over: HashMap<&QualifiedIdentifier, ExposedViews<'_>> = HashMap::new();
    for column in view_columns {
        if !tables.contains_key(&column.view) {
            continue;
        }
        let entry = views_over.entry(&column.base).or_default();
        match entry.iter_mut().find(|(view, _)| *view == &column.view) {
            Some((_, mapping)) => {
                mapping.insert(&column.base_column, &column.view_column);
            }
            None => {
                let mut mapping = HashMap::new();
                mapping.insert(column.base_column.as_str(), column.view_column.as_str());
                entry.push((&column.view, mapping));
            }
        }
    }

    for rel in relationships.values_mut().flatten() {
        let Relationship::ForeignKey {
            cardinality: Cardinality::M2M(junction),
            ..
        } = rel
        else {
            continue;
        };
        if tables.contains_key(&junction.table) {
            continue;
        }

        let needed: Vec<&str> = junction
            .source_columns
            .iter()
            .map(|(_, through)| through.as_str())
            .chain(
                junction
                    .target_columns
                    .iter()
                    .map(|(through, _)| through.as_str()),
            )
            .collect();

        let Some((view, mapping)) = views_over.get(&junction.table).and_then(|views| {
            views
                .iter()
                .find(|(_, mapping)| needed.iter().all(|name| mapping.contains_key(name)))
        }) else {
            // Nothing exposes the junction, so nothing can join through it.
            // Marked rather than removed here, since the relationship is being
            // iterated; the sweep below drops them.
            junction.table = QualifiedIdentifier::new("", "");
            continue;
        };

        junction.table = (*view).clone();
        for (_, through) in junction.source_columns.iter_mut() {
            if let Some(renamed) = mapping.get(through.as_str()) {
                *through = (*renamed).to_string();
            }
        }
        for (through, _) in junction.target_columns.iter_mut() {
            if let Some(renamed) = mapping.get(through.as_str()) {
                *through = (*renamed).to_string();
            }
        }
    }

    // Offering an unreachable relationship would answer `permission denied for
    // schema private` where PostgREST answers that no relationship was found,
    // which is both the truer answer and the one that says nothing about a
    // schema the client was never shown.
    for rels in relationships.values_mut() {
        rels.retain(|rel| {
            !matches!(
                rel,
                Relationship::ForeignKey {
                    cardinality: Cardinality::M2M(junction),
                    ..
                } if junction.table.name.is_empty()
            )
        });
    }
    relationships.retain(|_, rels| !rels.is_empty());
}

/// Project relationships from base tables onto the views that select them.
///
/// A view carrying both ends of a foreign key can stand in for either side of
/// it, so `/articleStars?select=*,articles(*)` embeds through the key on
/// `private.article_stars` without the client ever naming the private table.
///
/// The join columns are rewritten to the names the view gives them: a view is
/// free to rename what it selects, and the projected relationship has to join
/// on the names the client can actually see. Where a view selects one base
/// column twice under different names, each spelling is a relationship of its
/// own -- they are genuinely different joins, and `!t1_id1` is how a client
/// tells them apart.
fn add_view_relationships(
    tables: &TablesMap,
    view_columns: &[queries::ViewColumn],
    relationships: &mut RelationshipsMap,
) {
    use std::collections::HashMap;

    // base table -> view -> base column -> the names that view gives it
    type ColumnNames = HashMap<String, Vec<String>>;
    let mut views_over: HashMap<&QualifiedIdentifier, HashMap<&QualifiedIdentifier, ColumnNames>> =
        HashMap::new();
    for column in view_columns {
        views_over
            .entry(&column.base)
            .or_default()
            .entry(&column.view)
            .or_default()
            .entry(column.base_column.clone())
            .or_default()
            .push(column.view_column.clone());
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

        // Which columns each end has to carry for the view to stand in for
        // it. A many-to-many joins through a junction, so the two ends are
        // described by different halves of it -- the near side by the columns
        // going into the junction, the far side by the ones coming out.
        let (near_needs, far_needs): (Vec<String>, Vec<String>) = match cardinality {
            Cardinality::M2M(junction) => (
                junction
                    .source_columns
                    .iter()
                    .map(|(near, _)| near.clone())
                    .collect(),
                junction
                    .target_columns
                    .iter()
                    .map(|(_, far)| far.clone())
                    .collect(),
            ),
            _ => {
                let columns = cardinality.columns();
                (
                    columns.iter().map(|(near, _)| near.clone()).collect(),
                    columns.iter().map(|(_, far)| far.clone()).collect(),
                )
            }
        };

        // Every way the view can spell the columns this end joins on. Empty
        // when it does not carry all of them, which is what disqualifies it
        // from standing in at all.
        let spellings = |view_cols: &ColumnNames, needed: &[String]| -> Vec<Vec<String>> {
            let mut combinations: Vec<Vec<String>> = vec![vec![]];
            for name in needed {
                let Some(candidates) = view_cols.get(name) else {
                    return vec![];
                };
                combinations = combinations
                    .into_iter()
                    .flat_map(|prefix| {
                        candidates.iter().map(move |candidate| {
                            let mut extended = prefix.clone();
                            extended.push(candidate.clone());
                            extended
                        })
                    })
                    .collect();
            }
            combinations
        };

        // The view stands in for the near side.
        if let Some(views) = views_over.get(source) {
            for (view, view_cols) in views {
                for near in spellings(view_cols, &near_needs) {
                    derived.push((
                        ((*view).clone(), view.schema.clone()),
                        Relationship::ForeignKey {
                            table: (*view).clone(),
                            foreign_table: foreign_table.clone(),
                            is_self: *view == foreign_table,
                            cardinality: rename_cardinality(cardinality, Some(&near), None),
                            table_is_view: true,
                            foreign_table_is_view: *foreign_table_is_view,
                            constraint_name: constraint_name.clone(),
                        },
                    ));
                }
            }
        }

        // The view stands in for the far side, and likewise.
        if let Some(views) = views_over.get(foreign_table) {
            for (view, view_cols) in views {
                for far in spellings(view_cols, &far_needs) {
                    derived.push((
                        (source.clone(), source.schema.clone()),
                        Relationship::ForeignKey {
                            table: source.clone(),
                            foreign_table: (*view).clone(),
                            is_self: source == *view,
                            cardinality: rename_cardinality(cardinality, None, Some(&far)),
                            table_is_view: *table_is_view,
                            foreign_table_is_view: true,
                            constraint_name: constraint_name.clone(),
                        },
                    ));
                }
            }
        }

        // Both sides are views. Neither of the cases above covers it: each
        // replaces one end and leaves the other as the base table.
        if let (Some(near_views), Some(far_views)) =
            (views_over.get(source), views_over.get(foreign_table))
        {
            for (near_view, near_cols) in near_views {
                for near in spellings(near_cols, &near_needs) {
                    for (far_view, far_cols) in far_views {
                        for far in spellings(far_cols, &far_needs) {
                            derived.push((
                                ((*near_view).clone(), near_view.schema.clone()),
                                Relationship::ForeignKey {
                                    table: (*near_view).clone(),
                                    foreign_table: (*far_view).clone(),
                                    is_self: near_view == far_view,
                                    cardinality: rename_cardinality(
                                        cardinality,
                                        Some(&near),
                                        Some(&far),
                                    ),
                                    table_is_view: true,
                                    foreign_table_is_view: true,
                                    constraint_name: constraint_name.clone(),
                                },
                            ));
                        }
                    }
                }
            }
        }
    }

    // A projection is only useful if the client can name both ends. The base
    // table behind a view is often in a schema that is not exposed, and
    // offering it as a target would produce a relationship that resolves to a
    // table the request has no permission to read.
    for (key, rel) in derived {
        if !tables.contains_key(&key.0) || !tables.contains_key(rel.foreign_table()) {
            continue;
        }
        relationships.entry(key).or_default().push(rel);
    }
}

/// A cardinality with one or both ends' columns renamed.
///
/// `near` and `far` are given in the order the cardinality lists them, so the
/// nth name replaces the nth column. `None` leaves that end alone.
fn rename_cardinality(
    cardinality: &Cardinality,
    near: Option<&[String]>,
    far: Option<&[String]>,
) -> Cardinality {
    let rename_pairs = |columns: &[(String, String)]| -> Vec<(String, String)> {
        columns
            .iter()
            .enumerate()
            .map(|(i, (local, foreign))| {
                (
                    near.and_then(|names| names.get(i)).unwrap_or(local).clone(),
                    far.and_then(|names| names.get(i))
                        .unwrap_or(foreign)
                        .clone(),
                )
            })
            .collect()
    };

    match cardinality {
        Cardinality::O2M {
            constraint,
            columns,
        } => Cardinality::O2M {
            constraint: constraint.clone(),
            columns: rename_pairs(columns),
        },
        Cardinality::M2O {
            constraint,
            columns,
        } => Cardinality::M2O {
            constraint: constraint.clone(),
            columns: rename_pairs(columns),
        },
        Cardinality::O2O {
            constraint,
            columns,
            is_parent,
        } => Cardinality::O2O {
            constraint: constraint.clone(),
            columns: rename_pairs(columns),
            is_parent: *is_parent,
        },
        // Only the outer ends move: the junction keeps its own column names,
        // since the view stands in for a side of the join and not for the
        // table doing the joining.
        Cardinality::M2M(junction) => {
            let mut junction = junction.clone();
            if let Some(names) = near {
                for (i, (source, _)) in junction.source_columns.iter_mut().enumerate() {
                    if let Some(name) = names.get(i) {
                        *source = name.clone();
                    }
                }
            }
            if let Some(names) = far {
                for (i, (_, target)) in junction.target_columns.iter_mut().enumerate() {
                    if let Some(name) = names.get(i) {
                        *target = name.clone();
                    }
                }
            }
            Cardinality::M2M(junction)
        }
    }
}

/// The key a view inherits from the table it selects from.
///
/// A view over a junction is a junction: `main_jobs` selects the columns of
/// `jobs` that make it one, and PostgREST embeds `sites` through it and
/// through `jobs` alike -- reporting both as candidates when the two cannot be
/// told apart. A view has no key of its own, so the base table's is mapped
/// through the names the view gives those columns.
///
/// Only where the view exposes every one of them: a key missing a column is
/// not that key, and a junction is exactly the relation whose key *is* the
/// pairing.
fn view_primary_keys(
    primary_keys: &std::collections::HashMap<QualifiedIdentifier, Vec<String>>,
    view_columns: &[queries::ViewColumn],
) -> std::collections::HashMap<QualifiedIdentifier, Vec<String>> {
    use std::collections::HashMap;

    // view -> base table -> base column -> the name the view gives it
    let mut by_view: HashMap<
        &QualifiedIdentifier,
        HashMap<&QualifiedIdentifier, HashMap<&str, &str>>,
    > = HashMap::new();
    for column in view_columns {
        by_view
            .entry(&column.view)
            .or_default()
            .entry(&column.base)
            .or_default()
            .entry(column.base_column.as_str())
            .or_insert(column.view_column.as_str());
    }

    let mut keys = HashMap::new();
    for (view, bases) in by_view {
        for (base, columns) in bases {
            let Some(key) = primary_keys.get(base).filter(|key| !key.is_empty()) else {
                continue;
            };
            let mapped: Option<Vec<String>> = key
                .iter()
                .map(|column| columns.get(column.as_str()).map(|name| name.to_string()))
                .collect();
            if let Some(mapped) = mapped {
                keys.insert(view.clone(), mapped);
            }
        }
    }

    keys
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
fn add_junction_relationships(
    primary_keys: &std::collections::HashMap<QualifiedIdentifier, Vec<String>>,
    relationships: &mut RelationshipsMap,
) {
    let mut derived: Vec<((QualifiedIdentifier, String), Relationship)> = Vec::new();

    for (junction_qi, junction_pk) in primary_keys {
        let key = (junction_qi.clone(), junction_qi.schema.clone());
        let Some(rels) = relationships.get(&key) else {
            continue;
        };

        // The keys out of the junction, however many columns each is written
        // over. Requiring one column apiece looked like what separates a
        // junction from a table that merely references two others, but it is
        // not: `touched_files` joins `files(project_id, filename)` to
        // `users_tasks(user_id, task_id)` and is as much a junction as any
        // other. What separates them is the primary-key test below, which this
        // duplicated and then over-applied -- so a composite junction was
        // answered "no relationship" for a relationship that is plainly there.
        let outgoing: Vec<(&Relationship, Vec<(String, String)>)> = rels
            .iter()
            .filter(|r| matches!(r, Relationship::ForeignKey { table, .. } if table == junction_qi))
            .filter(|r| r.is_to_one())
            .filter_map(|r| {
                let mut columns = r.join_columns();
                columns.dedup();
                match columns.is_empty() {
                    true => None,
                    false => Some((r, columns)),
                }
            })
            .collect();

        let pk: Vec<&str> = junction_pk.iter().map(String::as_str).collect();

        // The pair whose columns are part of the primary key is what makes
        // this a junction: keying on both of them is what says a row of it is
        // one pairing. A junction may carry more in its key than the pair --
        // `group_yard` keys on `(id, group_id, yard_id)` -- and requiring the
        // key to be exactly the two columns missed those.
        for near in 0..outgoing.len() {
            for far in 0..outgoing.len() {
                if near == far {
                    continue;
                }
                let (near_rel, near_columns) = &outgoing[near];
                let (far_rel, far_columns) = &outgoing[far];
                if near_rel.foreign_table() == far_rel.foreign_table() {
                    continue;
                }
                // Every column either key is written over has to be part of
                // the junction's own key: that is what says a row of it is one
                // pairing and nothing more.
                if !near_columns
                    .iter()
                    .chain(far_columns)
                    .all(|(local, _)| pk.contains(&local.as_str()))
                {
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
                            // Source to junction, then junction to target,
                            // each in the order the constraint declares so a
                            // composite key joins column for column.
                            source_columns: near_columns
                                .iter()
                                .map(|(local, foreign)| (foreign.clone(), local.clone()))
                                .collect(),
                            target_columns: far_columns
                                .iter()
                                .map(|(local, foreign)| (local.clone(), foreign.clone()))
                                .collect(),
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
