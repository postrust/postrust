//! What one role can see.
//!
//! Hasura builds a separate GraphQL schema for every role, and the difference
//! between them is not cosmetic. A role with no `select` permission on a table
//! does not get a table it is refused access to -- it gets a schema with no
//! such field to read it by, and naming one is a validation failure rather
//! than a denial. The same goes one level down: a column outside the
//! permission is absent from the type, not merely null.
//!
//! That shape is what the corpus tests. `artist_select_query_Track_fail.yaml`
//! expects `field "Track" not found in type: 'query_root'`, which no amount of
//! filtering at execution time produces.
//!
//! # Why this is a filtered cache rather than a flag threaded through the
//! builders
//!
//! A select permission is a view of a table, and a set of them is a view of
//! the database. Reducing the schema cache to what a role may see and then
//! building the schema from *that* means the builders need to know nothing
//! about permissions: a table that was dropped generates no root field, no
//! relationship pointing at it and no boolean expression naming it, because
//! the code that would have generated them cannot see it either.
//!
//! The alternative -- asking "may this role?" at each of the several dozen
//! places a name is emitted -- has to be right every time, and is wrong the
//! moment someone adds a place. This has to be right once.
//!
//! # The one thing a cache cannot hold: two column sets
//!
//! Hasura grants reading and writing separately, so a role may be allowed to
//! set a column it is not allowed to see -- ten permissions in the corpus do
//! -- and may be allowed to write a table it cannot read at all, which eight
//! more do. A schema cache has one column list per table and no place to say
//! which half of it is which.
//!
//! The union goes in the cache, and the split is made once, where the object
//! type's fields are built: [`crate::schema::object::TableObjectType::fields`]
//! is what may be read and `writable_fields` is what may be written. That
//! keeps the property above -- everything a read is made of (the type, its
//! boolean expression, its ordering, its column enum, its aggregates) is
//! derived from `fields` and narrows with it, and the write inputs are built
//! from the other list and narrowed again by the permission that names them.
//!
//! A table with no readable field is the limit of that: it has no GraphQL type
//! at all, because a type with no fields is not a legal one. Its bulk writes
//! remain and answer with `affected_rows` alone -- there is no `returning` to
//! hang rows on -- and everything that would return a row of it (the query
//! roots, `insert_one`, the `_by_pk` writes, a relationship pointing at it, a
//! function returning it) is left out with the type.
//!
//! What is left over goes on [`SchemaConfig`]: `allow_aggregations`, which is
//! a fact about a permission rather than about a table.
//!
//! [`SchemaConfig`]: crate::schema::SchemaConfig

use crate::names::NameOverrides;
use postrust_core::schema_cache::{SchemaCache, Table};

/// Reduce a schema cache to what one role may see.
///
/// Returns the cache unchanged when the document grants that role nothing to
/// narrow -- which is the case for `admin`, and for every role when the
/// document carries no permissions at all.
pub fn cache_for_role(
    cache: &SchemaCache,
    names: &NameOverrides,
    role: &str,
    backend: bool,
) -> SchemaCache {
    let mut view = cache.clone();

    view.tables.retain(|_, table| {
        let Some(granted) = names.permissions(&table.schema, &table.name, role) else {
            // Named nothing about this table. With a permission document in
            // play that is a statement, not a silence: the role cannot see it.
            return false;
        };

        // Which of the four grants apply, decided before the columns because
        // the columns depend on the answer.
        //
        // `backend_only` narrows the insert once more: such a field is
        // reachable only by a caller that proved it holds the admin secret and
        // asked for it by name, so for everyone else it is not there at all --
        // which is the point of the flag. A client must not be able to reach
        // it by naming a role.
        table.insertable = table.insertable
            && granted
                .insert
                .as_ref()
                .is_some_and(|insert| backend || !insert.backend_only);
        table.updatable = table.updatable && granted.update.is_some();
        table.deletable = table.deletable && granted.delete.is_some();
        let writes = table.insertable || table.updatable || table.deletable;

        // Reading is not what makes a table exist here, and that is the whole
        // of this change. Hasura lets a role write a table it cannot read, and
        // eight permissions in its corpus do exactly that: `insert_account`
        // for `user`, `insert_leads` for `sales`, `insert_author` for
        // `student`. Such a mutation answers `affected_rows` and has no
        // `returning` to hang rows on, because there is no type to read the
        // written row out of.
        if granted.select.is_none() && !writes {
            return false;
        }

        // The columns the role may read, together with the ones it may write
        // without reading. Everything else is not merely unreadable but
        // absent, which is what makes a permission a statement about the
        // schema rather than about a request.
        //
        // This is the one place both column sets are held at once, and the
        // reason they can be: a schema cache has a single column list, so the
        // union goes in it and the narrower read set is applied once, where
        // the object type's fields are built. Everything a read is made of --
        // the type, its boolean expression, its ordering, its column enum, its
        // aggregates -- is derived from those fields and narrows with them.
        // The write inputs are built from the union instead and narrowed again
        // by the write permission that names them.
        let (insertable, updatable) = (table.insertable, table.updatable);
        table.columns.retain(|name, _| {
            let read = granted
                .select
                .as_ref()
                .is_some_and(|select| select.columns.allows(name));
            let written = insertable
                && granted
                    .insert
                    .as_ref()
                    .is_some_and(|insert| insert.columns.allows(name))
                || updatable
                    && granted
                        .update
                        .as_ref()
                        .is_some_and(|update| update.columns.allows(name));
            read || written
        });

        // A computed field runs a function over the whole row, so it can
        // answer questions the column list was written to prevent. Hasura
        // grants them one at a time and grants none by default -- and grants
        // none at all to a role that cannot read the table, since there is
        // nowhere for the answer to appear.
        table.computed_columns.retain(|name, _| {
            granted
                .select
                .as_ref()
                .is_some_and(|select| select.computed_fields.iter().any(|f| f == name))
        });

        // A select permission naming no columns at all, with
        // `allow_aggregations`, is how Hasura grants "how many" without
        // granting "which": the table is there to be counted and has nothing
        // to read. It gets an aggregate root and no row type, the same shape a
        // table this role may only write has.
        let counts = granted
            .select
            .as_ref()
            .is_some_and(|select| select.allow_aggregations);

        // Nothing to read, nothing to write and nothing to count is nothing at
        // all: the role is told the table is not there, which is the same
        // answer in the direction that withholds.
        !table.columns.is_empty() || !table.computed_columns.is_empty() || writes || counts
    });

    // A relationship whose other end the role cannot *read* is not a field it
    // gets a null for -- there is nothing to name it after, because a table
    // this role may only write has no type. Both ends are checked: the map is
    // keyed by the source, and the target may have gone even where the source
    // stayed.
    let readable: std::collections::HashSet<_> = view
        .tables
        .iter()
        .filter(|(_, table)| {
            names
                .permissions(&table.schema, &table.name, role)
                .is_some_and(|granted| reads_anything(granted, table))
        })
        .map(|(key, _)| key.clone())
        .collect();
    view.relationships.retain(|(source, _), links| {
        if !view.tables.contains_key(source) {
            return false;
        }
        links.retain(|link| readable.contains(link.foreign_table()));
        !links.is_empty()
    });

    view
}

/// Whether this role can read anything of this table at all.
///
/// The question a relationship target has to answer, and the question that
/// decides whether the table gets a GraphQL type: a role with a write grant
/// and no select permission keeps the table in the cache, and a type with no
/// fields is not a legal type. Asked of an already-reduced table, so the
/// computed fields it still carries are the granted ones.
pub fn reads_anything(granted: &crate::names::RolePermissions, table: &Table) -> bool {
    let Some(select) = &granted.select else {
        return false;
    };
    !table.computed_columns.is_empty()
        || table.columns.keys().any(|name| select.columns.allows(name))
}

/// Whether any of this role's permissions is reachable only by a backend.
///
/// Which decides whether the role needs a second schema at all: without one,
/// what a backend caller may name is what everyone may name.
pub fn has_backend_only(names: &NameOverrides, role: &str) -> bool {
    names.tables_with_permissions(role).any(|granted| {
        granted
            .insert
            .as_ref()
            .is_some_and(|insert| insert.backend_only)
    })
}

/// Who is asking, for the places that have to narrow rows to them.
///
/// The two request-scoped facts a permission needs, carried together because
/// they are always needed together and because the SQL builders they reach are
/// several calls below the context that holds them. `role: None` is the
/// unrestricted read: no permission document, or an administrator.
#[derive(Clone, Copy)]
pub struct Caller<'a> {
    pub role: Option<&'a str>,
    pub session: &'a std::collections::HashMap<String, String>,
}

impl Caller<'_> {
    /// Whether any permission applies to this caller at all.
    pub fn unrestricted(&self) -> bool {
        match self.role {
            None => true,
            Some(role) => role == postrust_auth::hasura::ADMIN_ROLE,
        }
    }
}

/// Why a read could not be narrowed to the caller.
///
/// Two different answers with two different codes in Hasura, so they are kept
/// apart here rather than collapsed into one message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fault {
    /// The role has no select permission on the table. Unreachable through a
    /// role's own schema, which has no such field -- so reaching it means
    /// something was built that should not have been, and the safe answer is
    /// to refuse rather than to read every row.
    NoPermission { role: String, table: String },
    /// A permission names a session variable the caller does not carry.
    MissingSessionVariable(String),
}

impl Fault {
    /// Hasura's `extensions.code` for this.
    pub fn code(&self) -> &'static str {
        match self {
            Self::NoPermission { .. } => "permission-error",
            Self::MissingSessionVariable(_) => "not-found",
        }
    }
}

impl std::fmt::Display for Fault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoPermission { role, table } => write!(
                f,
                "role \"{}\" has no permission to read \"{}\"",
                role, table
            ),
            Self::MissingSessionVariable(message) => f.write_str(message),
        }
    }
}

/// Hasura's wording for a written row that a permission refuses.
///
/// One message for both verbs, which is Hasura's choice and not a shortcut
/// taken here: `check constraint of an insert/update permission has failed`.
pub const CHECK_FAILED: &str = "check constraint of an insert/update permission has failed";

/// Which of the four grants is being asked about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verb {
    Select,
    Insert,
    Update,
    Delete,
}

impl Verb {
    fn of<'a>(&self, granted: &'a crate::names::RolePermissions) -> Option<Grant<'a>> {
        match self {
            Self::Select => granted.select.as_ref().map(Grant::Select),
            Self::Insert => granted.insert.as_ref().map(Grant::Insert),
            Self::Update => granted.update.as_ref().map(Grant::Update),
            Self::Delete => granted.delete.as_ref().map(Grant::Delete),
        }
    }
}

/// One grant, whichever it is.
enum Grant<'a> {
    Select(&'a crate::names::SelectPermission),
    Insert(&'a crate::names::InsertPermission),
    Update(&'a crate::names::UpdatePermission),
    Delete(&'a crate::names::DeletePermission),
}

impl Grant<'_> {
    /// Which rows this grant applies to, read before the operation.
    ///
    /// An insert has none: there are no rows to choose from yet, which is
    /// exactly why it has a `check` instead.
    fn filter(&self) -> Option<&serde_json::Value> {
        match self {
            Self::Select(p) => Some(&p.filter),
            Self::Update(p) => Some(&p.filter),
            Self::Delete(p) => Some(&p.filter),
            Self::Insert(_) => None,
        }
    }

    /// What a written row has to satisfy afterwards.
    ///
    /// An update with no `check` of its own is held to its `filter`, which is
    /// Hasura's rule: a row you may change is a row you may change into
    /// something you may still change.
    fn check(&self) -> Option<&serde_json::Value> {
        match self {
            Self::Insert(p) => Some(&p.check),
            Self::Update(p) => match p.check.is_null() {
                false => Some(&p.check),
                true => Some(&p.filter),
            },
            Self::Select(_) | Self::Delete(_) => None,
        }
    }

    /// Columns the server fills in, whatever the request said.
    fn set(&self) -> Option<&std::collections::HashMap<String, serde_json::Value>> {
        match self {
            Self::Insert(p) => Some(&p.set),
            Self::Update(p) => Some(&p.set),
            Self::Select(_) | Self::Delete(_) => None,
        }
    }
}

fn grant_for<'a>(
    caller: &Caller<'_>,
    names: &'a NameOverrides,
    schema: &str,
    table: &str,
    verb: Verb,
) -> Result<Option<Grant<'a>>, Fault> {
    if caller.unrestricted() {
        return Ok(None);
    }
    let role = caller.role.unwrap_or_default();
    match names
        .permissions(schema, table, role)
        .and_then(|granted| verb.of(granted))
    {
        Some(grant) => Ok(Some(grant)),
        // Reaching here means a schema was built naming an operation this role
        // has no permission for, which should not happen -- and the safe
        // answer to "which rows" is none of them, not all of them.
        None => Err(Fault::NoPermission {
            role: role.to_string(),
            table: table.to_string(),
        }),
    }
}

/// Which rows a role's permission lets one operation reach.
///
/// `Ok(None)` where nothing restricts it: an unrestricted caller, a filter of
/// `{}`, or an insert, which has no rows to choose from.
pub fn row_filter(
    caller: &Caller<'_>,
    names: &NameOverrides,
    schema: &str,
    table: &str,
    verb: Verb,
) -> Result<Option<serde_json::Value>, Fault> {
    let Some(grant) = grant_for(caller, names, schema, table, verb)? else {
        return Ok(None);
    };
    let Some(filter) = grant.filter() else {
        return Ok(None);
    };
    permission_where(filter, caller.session, names, schema, table)
        .map_err(Fault::MissingSessionVariable)
}

/// What a written row has to satisfy for the write to stand.
pub fn write_check(
    caller: &Caller<'_>,
    names: &NameOverrides,
    schema: &str,
    table: &str,
    verb: Verb,
) -> Result<Option<serde_json::Value>, Fault> {
    let Some(grant) = grant_for(caller, names, schema, table, verb)? else {
        return Ok(None);
    };
    let Some(check) = grant.check() else {
        return Ok(None);
    };
    permission_where(check, caller.session, names, schema, table)
        .map_err(Fault::MissingSessionVariable)
}

/// The columns the server fills in itself, with their values.
///
/// This is how `author_id` comes from the caller's identity rather than from
/// the caller: a preset overrides whatever the request said, so a client that
/// names the column anyway does not get to choose it.
pub fn presets(
    caller: &Caller<'_>,
    names: &NameOverrides,
    schema: &str,
    table: &str,
    verb: Verb,
) -> Result<Vec<(String, serde_json::Value)>, Fault> {
    let Some(grant) = grant_for(caller, names, schema, table, verb)? else {
        return Ok(Vec::new());
    };
    let Some(set) = grant.set() else {
        return Ok(Vec::new());
    };

    let mut filled = Vec::with_capacity(set.len());
    for (column, value) in set {
        // A preset's value is a session variable as often as it is a literal,
        // and it is resolved the same way a filter's operand is.
        let resolved = match value {
            serde_json::Value::String(text) => match session_variable(text) {
                Some(name) => {
                    resolve(name, caller.session, false).map_err(Fault::MissingSessionVariable)?
                }
                None => value.clone(),
            },
            other => other.clone(),
        };
        filled.push((column.clone(), resolved));
    }
    // Sorted, so the SQL a preset produces does not depend on how a hash map
    // felt about its keys.
    filled.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(filled)
}

/// The predicate a role's select permission adds to a read of one table.
pub fn read_predicate(
    caller: &Caller<'_>,
    names: &NameOverrides,
    schema: &str,
    table: &str,
) -> Result<Option<serde_json::Value>, Fault> {
    row_filter(caller, names, schema, table, Verb::Select)
}

/// How many rows a role's permission lets one read return.
///
/// A ceiling rather than a default, and `None` where the permission names one
/// no more than the configuration does.
pub fn read_limit(
    caller: &Caller<'_>,
    names: &NameOverrides,
    schema: &str,
    table: &str,
) -> Option<i64> {
    if caller.unrestricted() {
        return None;
    }
    names
        .permissions(schema, table, caller.role?)
        .and_then(|granted| granted.select.as_ref())
        .and_then(|select| select.limit)
        .map(|limit| limit as i64)
}

/// Turn a permission's row filter into the boolean expression a `where`
/// argument would have been.
///
/// The two are the same language. Hasura's `filter` is written in the same
/// shape a client writes `where` in, so there is no second predicate compiler
/// here and no second set of operators to keep in step -- the filter is
/// rewritten into a `<table>_bool_exp` and handed to the one that already
/// exists. Everything it can express, a permission can express, including a
/// predicate over a relationship.
///
/// Three things metadata spells differently, and they are all this does:
///
///   * `{"author_id": 1}` is `_eq` written without saying so. Metadata omits
///     the operator where it is equality; a `where` argument never does.
///   * `$and`, `$or` and `$not` are the older spellings of `_and`, `_or` and
///     `_not`, and the corpus uses both -- sometimes in one file.
///   * `"X-Hasura-User-Id"` is not a string to compare against but the name of
///     a session variable to compare against the value of. This is the half
///     that makes a permission about the caller rather than about the table.
///
/// `Ok(None)` means the filter restricts nothing, which is what `{}` says and
/// is a real answer: a role may be granted every row. It is distinct from
/// having no permission at all, which is settled before this is reached.
///
/// An `Err` names a session variable the caller does not carry. That is a
/// refusal rather than an empty result, because a filter that silently
/// compares against nothing is a filter that matches nothing for a reason the
/// client cannot see -- and Hasura says so too, in these words.
pub fn permission_where(
    filter: &serde_json::Value,
    session: &std::collections::HashMap<String, String>,
    names: &NameOverrides,
    schema: &str,
    table: &str,
) -> Result<Option<serde_json::Value>, String> {
    let rewritten = rewrite(filter, session, names, schema, table, None)?;
    Ok(match rewritten {
        serde_json::Value::Object(map) if map.is_empty() => None,
        serde_json::Value::Null => None,
        value => Some(value),
    })
}

/// The older spellings, and what they are now.
fn connective(key: &str) -> Option<&'static str> {
    match key {
        "$and" | "_and" => Some("_and"),
        "$or" | "_or" => Some("_or"),
        "$not" | "_not" => Some("_not"),
        _ => None,
    }
}

/// Whether an operator takes a list, which decides how a session variable
/// spells itself.
fn takes_a_list(operator: &str) -> bool {
    matches!(operator, "_in" | "_nin" | "_has_keys_any" | "_has_keys_all")
}

fn rewrite(
    value: &serde_json::Value,
    session: &std::collections::HashMap<String, String>,
    names: &NameOverrides,
    schema: &str,
    table: &str,
    // The operator this value is the operand of, where it is one. Only used to
    // decide whether a session variable is one value or a list of them.
    operator: Option<&str>,
) -> Result<serde_json::Value, String> {
    match value {
        serde_json::Value::Object(map) => {
            let mut out = serde_json::Map::with_capacity(map.len());
            for (key, child) in map {
                if let Some(spelling) = connective(key) {
                    out.insert(
                        spelling.to_string(),
                        rewrite(child, session, names, schema, table, None)?,
                    );
                    continue;
                }

                // `$in`, `$gt`, `$is_null`: the whole operator family has an
                // older spelling, not just the three connectives above, and
                // the corpus writes both. Everything after the sigil is the
                // operator this server already knows.
                let key = match key.strip_prefix('$') {
                    Some(rest) if !rest.is_empty() => std::borrow::Cow::Owned(format!("_{}", rest)),
                    _ => std::borrow::Cow::Borrowed(key.as_str()),
                };

                // A predicate over another table, so what is inside it is
                // spelled in *that* table's names rather than in this one's.
                // Everything else here carries the outer table down.
                if key == "_exists" {
                    out.insert(
                        "_exists".to_string(),
                        rewrite_exists(child, session, names, schema)?,
                    );
                    continue;
                }

                if key.starts_with('_') {
                    // An operator. Its operand may be a session variable, and
                    // whether that is one value or several depends on which
                    // operator it is.
                    out.insert(
                        key.to_string(),
                        rewrite(child, session, names, schema, table, Some(&key))?,
                    );
                    continue;
                }

                // A column, or a relationship. A renamed column is exposed
                // under the other name and the compiler below reads exposed
                // names, so the rename is applied here. A relationship is not
                // in the column map and so is left alone -- along with the
                // columns inside it, which belong to a table this does not
                // know the name of. A rename on the far side of a relationship
                // inside a permission is the one spelling this does not
                // follow.
                let exposed = names
                    .column(schema, table, &key)
                    .unwrap_or(&key)
                    .to_string();

                let rewritten = rewrite(child, session, names, schema, table, None)?;
                // Equality written without saying so: metadata omits the
                // operator, a boolean expression never does.
                let wrapped = match &rewritten {
                    serde_json::Value::Object(_) => rewritten,
                    scalar => serde_json::json!({ "_eq": scalar }),
                };
                out.insert(exposed, wrapped);
            }
            Ok(serde_json::Value::Object(out))
        }

        serde_json::Value::Array(items) => items
            .iter()
            .map(|item| rewrite(item, session, names, schema, table, operator))
            .collect::<Result<Vec<_>, _>>()
            .map(serde_json::Value::Array),

        serde_json::Value::String(text) => match session_variable(text) {
            Some(name) => resolve(name, session, operator.is_some_and(takes_a_list)),
            None => Ok(value.clone()),
        },

        other => Ok(other.clone()),
    }
}

/// The table `_exists` looks in, as `(schema, table)`.
///
/// Hasura's metadata writes the name two ways -- `_table: user` and `_table:
/// {schema: public, name: user}` -- and its own corpus uses the first. An
/// unqualified name is read in the schema of the table the permission is on,
/// which is where a permission written without a schema means to look.
pub fn exists_target(spec: &serde_json::Value, default_schema: &str) -> Option<(String, String)> {
    match spec.get("_table")? {
        serde_json::Value::String(name) => Some((default_schema.to_string(), name.clone())),
        serde_json::Value::Object(qualified) => {
            let name = qualified.get("name").and_then(|v| v.as_str())?;
            let schema = qualified
                .get("schema")
                .and_then(|v| v.as_str())
                .unwrap_or(default_schema);
            Some((schema.to_string(), name.to_string()))
        }
        _ => None,
    }
}

/// `_exists`, rewritten so that its predicate reads against the table it names.
///
/// The `_table` is a name and not a predicate, so it is copied through
/// untouched: rewriting it would read a table called `x-hasura-anything` as a
/// session variable, and would wrap it in an `_eq` besides.
fn rewrite_exists(
    spec: &serde_json::Value,
    session: &std::collections::HashMap<String, String>,
    names: &NameOverrides,
    default_schema: &str,
) -> Result<serde_json::Value, String> {
    let Some(map) = spec.as_object() else {
        return Err("\"_exists\" takes a table and a predicate over it".to_string());
    };
    let Some((schema, table)) = exists_target(spec, default_schema) else {
        return Err("\"_exists\" needs a \"_table\" to look in".to_string());
    };

    let mut out = serde_json::Map::with_capacity(2);
    if let Some(named) = map.get("_table") {
        out.insert("_table".to_string(), named.clone());
    }
    let predicate = map
        .get("_where")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    out.insert(
        "_where".to_string(),
        rewrite(&predicate, session, names, &schema, &table, None)?,
    );
    Ok(serde_json::Value::Object(out))
}

/// The session variable a string names, if it names one.
///
/// Case does not matter: the corpus writes `X-HASURA-USER-ID` and
/// `X-Hasura-User-Id` for the same variable, sometimes in adjacent lines of
/// one file.
fn session_variable(text: &str) -> Option<String> {
    let lowered = text.to_ascii_lowercase();
    let bare = lowered.strip_prefix("x-hasura-")?;
    match bare.is_empty() {
        true => None,
        false => Some(bare.replace('-', "_")),
    }
}

/// The value a session variable stands for.
fn resolve(
    name: String,
    session: &std::collections::HashMap<String, String>,
    as_a_list: bool,
) -> Result<serde_json::Value, String> {
    let Some(value) = session.get(&name) else {
        // Hasura's wording, and Hasura's decision to refuse rather than to
        // compare against nothing.
        return Err(format!(
            "missing session variable: \"x-hasura-{}\"",
            name.replace('_', "-")
        ));
    };

    if !as_a_list {
        return Ok(serde_json::Value::String(value.clone()));
    }

    // A list arrives as a PostgreSQL array literal, because a header carries
    // one string and that is the spelling both ends already agree on:
    // `X-Hasura-Allowed-Ids: {1,2,3}`. Unbraced is read as a single-item list
    // rather than refused, since a one-element list is the case a caller is
    // most likely to write bare.
    let inner = value
        .strip_prefix('{')
        .and_then(|rest| rest.strip_suffix('}'));
    Ok(match inner {
        None => serde_json::json!([value]),
        Some("") => serde_json::json!([]),
        Some(items) => serde_json::Value::Array(
            items
                .split(',')
                .map(|item| serde_json::Value::String(item.trim().trim_matches('"').to_string()))
                .collect(),
        ),
    })
}

/// Whether a role may ask for aggregates over a table.
///
/// Separate from reading its rows because counting rows you cannot see is a
/// way of seeing them, and Hasura keeps the two apart for that reason. A role
/// the document says nothing about is not reached here: it has no table.
pub fn allows_aggregations(names: &NameOverrides, schema: &str, table: &str, role: &str) -> bool {
    names
        .permissions(schema, table, role)
        .and_then(|granted| granted.select.as_ref())
        .is_some_and(|select| select.allow_aggregations)
}

#[cfg(test)]
mod tests {
    use super::*;
    use postrust_core::schema_cache::{Column, Table};
    use postrust_core::QualifiedIdentifier;
    use std::collections::{HashMap, HashSet};

    fn column(name: &str, position: i32) -> Column {
        Column {
            name: name.to_string(),
            description: None,
            nullable: false,
            data_type: "text".to_string(),
            nominal_type: "text".to_string(),
            max_len: None,
            default: None,
            enum_values: vec![],
            is_pk: name == "id",
            position,
            domain_type: None,
            always_generated: false,
        }
    }

    fn table(name: &str, columns: &[&str]) -> Table {
        Table {
            schema: "public".to_string(),
            name: name.to_string(),
            description: None,
            is_view: false,
            insertable: true,
            updatable: true,
            deletable: true,
            pk_cols: vec!["id".to_string()],
            unique_constraints: Vec::new(),
            columns: columns
                .iter()
                .enumerate()
                .map(|(at, c)| (c.to_string(), column(c, at as i32 + 1)))
                .collect(),
            computed_columns: Default::default(),
            is_partitioned: false,
        }
    }

    fn cache(tables: Vec<Table>) -> SchemaCache {
        let mut map = HashMap::new();
        for table in tables {
            map.insert(table.qualified_identifier(), table);
        }
        SchemaCache {
            tables: map,
            relationships: HashMap::new(),
            routines: HashMap::new(),
            timezones: HashSet::new(),
            media_handlers: HashMap::new(),
            pg_version: 150000,
            representations: Default::default(),
        }
    }

    const DOCUMENT: &str = r#"{
        "tables": {
            "public.article": {
                "permissions": {
                    "user": {
                        "select": {"columns": ["id", "title"], "filter": {},
                                   "allow_aggregations": true},
                        "insert": {"columns": "*", "check": {}}
                    },
                    "reader": {
                        "select": {"columns": "*", "filter": {}}
                    },
                    "narrow": {
                        "select": {"columns": ["id", "title"], "filter": {}},
                        "insert": {"columns": ["title"], "check": {}}
                    }
                }
            },
            "public.secret": {
                "permissions": {
                    "reader": {"select": {"columns": "*", "filter": {}}}
                }
            }
        }
    }"#;

    fn names() -> NameOverrides {
        NameOverrides::parse(DOCUMENT).unwrap()
    }

    #[test]
    fn a_table_the_role_was_told_nothing_about_is_not_there() {
        let view = cache_for_role(
            &cache(vec![table("article", &["id"]), table("secret", &["id"])]),
            &names(),
            "user",
            false,
        );
        assert!(view
            .tables
            .contains_key(&QualifiedIdentifier::new("public", "article")));
        // `secret` names `reader` and not `user`, so for `user` it does not
        // exist -- which is a different answer from existing and refusing.
        assert!(!view
            .tables
            .contains_key(&QualifiedIdentifier::new("public", "secret")));
    }

    #[test]
    fn a_column_outside_the_permission_is_absent_rather_than_null() {
        // `narrow` reads id and title and writes title. `content` is named by
        // neither, so it is not in the cache under any heading.
        let view = cache_for_role(
            &cache(vec![table("article", &["id", "title", "content"])]),
            &names(),
            "narrow",
            false,
        );
        let article = &view.tables[&QualifiedIdentifier::new("public", "article")];
        assert!(article.has_column("title"));
        assert!(!article.has_column("content"));
    }

    #[test]
    fn a_column_the_role_may_write_and_not_read_is_kept_for_the_write() {
        // `user` reads id and title and inserts every column. `content` is in
        // the cache because the insert names it -- the read half is taken out
        // where the object type's fields are built, not here.
        let view = cache_for_role(
            &cache(vec![table("article", &["id", "title", "content"])]),
            &names(),
            "user",
            false,
        );
        let article = &view.tables[&QualifiedIdentifier::new("public", "article")];
        assert!(article.has_column("content"));
    }

    #[test]
    fn a_wildcard_keeps_every_column() {
        let view = cache_for_role(
            &cache(vec![table("article", &["id", "title", "content"])]),
            &names(),
            "reader",
            false,
        );
        let article = &view.tables[&QualifiedIdentifier::new("public", "article")];
        assert_eq!(article.columns.len(), 3);
    }

    #[test]
    fn a_write_the_role_was_not_granted_is_not_a_root_field() {
        let view = cache_for_role(
            &cache(vec![table("article", &["id"])]),
            &names(),
            "user",
            false,
        );
        let article = &view.tables[&QualifiedIdentifier::new("public", "article")];
        // Granted insert; told nothing about update or delete.
        assert!(article.insertable);
        assert!(!article.updatable);
        assert!(!article.deletable);
    }

    #[test]
    fn a_permission_cannot_grant_a_write_the_database_refuses() {
        let mut read_only = table("article", &["id"]);
        read_only.insertable = false;
        let view = cache_for_role(&cache(vec![read_only]), &names(), "user", false);
        let article = &view.tables[&QualifiedIdentifier::new("public", "article")];
        // The document grants `insert` and the database does not. Metadata
        // above the database cannot widen what is beneath it.
        assert!(!article.insertable);
    }

    #[test]
    fn a_computed_field_is_granted_one_at_a_time() {
        let mut article = table("article", &["id"]);
        article.computed_columns.insert(
            "word_count".to_string(),
            postrust_core::schema_cache::ComputedColumn {
                function: QualifiedIdentifier::new("public", "word_count"),
                data_type: "integer".to_string(),
                description: None,
                row_argument: None,
                session_argument: None,
                takes_arguments: false,
            },
        );
        let view = cache_for_role(&cache(vec![article]), &names(), "user", false);
        let article = &view.tables[&QualifiedIdentifier::new("public", "article")];
        // The permission names no computed fields, and none is the default.
        assert!(article.computed_columns.is_empty());
    }

    #[test]
    fn a_table_that_can_only_be_written_keeps_the_write_and_gets_no_type() {
        // `writer` may insert and not read. The table stays, with the columns
        // the insert may name and nothing to read them back with: nothing this
        // role asks can produce a row of it.
        let names = NameOverrides::parse(
            r#"{"tables": {"public.article": {"permissions":
                 {"writer": {"insert": {"columns": "*", "check": {}}}}}}}"#,
        )
        .unwrap();
        let view = cache_for_role(
            &cache(vec![table("article", &["id"])]),
            &names,
            "writer",
            false,
        );
        let article = &view.tables[&QualifiedIdentifier::new("public", "article")];
        assert!(article.insertable);
        assert!(!article.updatable);
        assert!(!article.deletable);
        assert!(article.has_column("id"));
        let granted = names.permissions("public", "article", "writer").unwrap();
        assert!(!reads_anything(granted, article));
    }

    #[test]
    fn a_relationship_pointing_at_a_table_the_role_may_only_write_is_dropped() {
        // There is nothing to name such a field after: the far side has no
        // type, and naming one that was never registered is a schema that will
        // not build.
        let names = NameOverrides::parse(
            r#"{"tables": {
                 "public.article": {"permissions":
                   {"writer": {"select": {"columns": "*", "filter": {}}}}},
                 "public.author": {"permissions":
                   {"writer": {"insert": {"columns": "*", "check": {}}}}}}}"#,
        )
        .unwrap();
        let mut base = cache(vec![table("article", &["id"]), table("author", &["id"])]);
        base.relationships.insert(
            (
                QualifiedIdentifier::new("public", "article"),
                "public".to_string(),
            ),
            vec![postrust_core::schema_cache::Relationship::ForeignKey {
                table: QualifiedIdentifier::new("public", "article"),
                foreign_table: QualifiedIdentifier::new("public", "author"),
                is_self: false,
                cardinality: postrust_core::schema_cache::Cardinality::M2O {
                    constraint: "article_author_id_fkey".to_string(),
                    columns: vec![("author_id".to_string(), "id".to_string())],
                },
                table_is_view: false,
                foreign_table_is_view: false,
                constraint_name: "article_author_id_fkey".to_string(),
            }],
        );
        let view = cache_for_role(&base, &names, "writer", false);
        assert!(view
            .tables
            .contains_key(&QualifiedIdentifier::new("public", "author")));
        assert!(view.relationships.is_empty());
    }

    #[test]
    fn exists_keeps_its_table_and_rewrites_its_predicate() {
        // The table is a name, not a value: it must survive untouched, while
        // the predicate beside it gets the session variable and the `_eq` that
        // metadata leaves implicit.
        let session = session(&[("user_id", "2")]);
        let rewritten = compiled(
            serde_json::json!({
                "_exists": {"_table": "user",
                            "_where": {"id": "X-Hasura-User-Id", "is_admin": true}}
            }),
            &session,
        );
        assert_eq!(
            rewritten,
            serde_json::json!({
                "_exists": {"_table": "user",
                            "_where": {"id": {"_eq": "2"},
                                       "is_admin": {"_eq": true}}}
            })
        );
    }

    #[test]
    fn exists_names_its_table_either_way() {
        assert_eq!(
            exists_target(&serde_json::json!({"_table": "user"}), "public"),
            Some(("public".to_string(), "user".to_string()))
        );
        assert_eq!(
            exists_target(
                &serde_json::json!({"_table": {"schema": "auth", "name": "user"}}),
                "public"
            ),
            Some(("auth".to_string(), "user".to_string()))
        );
        assert_eq!(exists_target(&serde_json::json!({}), "public"), None);
    }

    fn session(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn compiled(filter: serde_json::Value, session: &HashMap<String, String>) -> serde_json::Value {
        permission_where(
            &filter,
            session,
            &NameOverrides::default(),
            "public",
            "article",
        )
        .unwrap()
        .unwrap()
    }

    #[test]
    fn equality_written_without_saying_so_becomes_an_operator() {
        assert_eq!(
            compiled(serde_json::json!({"is_published": true}), &session(&[])),
            serde_json::json!({"is_published": {"_eq": true}})
        );
    }

    #[test]
    fn the_older_connectives_are_the_newer_ones() {
        let filter = serde_json::json!({
            "$or": [{"author_id": "X-HASURA-USER-ID"}, {"is_published": true}]
        });
        assert_eq!(
            compiled(filter, &session(&[("user_id", "1")])),
            serde_json::json!({"_or": [
                {"author_id": {"_eq": "1"}},
                {"is_published": {"_eq": true}}
            ]})
        );
    }

    #[test]
    fn the_older_operator_spellings_are_the_newer_ones_too() {
        // Not only the three connectives: the whole family has a `$` spelling
        // and the corpus writes both.
        assert_eq!(
            compiled(
                serde_json::json!({"id": {"$in": "X-Hasura-Allowed-Ids"}}),
                &session(&[("allowed_ids", "{1,2}")])
            ),
            serde_json::json!({"id": {"_in": ["1", "2"]}})
        );
        assert_eq!(
            compiled(serde_json::json!({"age": {"$gt": 18}}), &session(&[])),
            serde_json::json!({"age": {"_gt": 18}})
        );
    }

    #[test]
    fn a_session_variable_is_read_however_it_is_spelled() {
        // The corpus writes both, sometimes in one file.
        let by_shouting = compiled(
            serde_json::json!({"id": {"_eq": "X-HASURA-USER-ID"}}),
            &session(&[("user_id", "7")]),
        );
        let by_title = compiled(
            serde_json::json!({"id": {"_eq": "X-Hasura-User-Id"}}),
            &session(&[("user_id", "7")]),
        );
        assert_eq!(by_shouting, by_title);
        assert_eq!(by_shouting, serde_json::json!({"id": {"_eq": "7"}}));
    }

    #[test]
    fn a_variable_the_caller_does_not_carry_is_refused() {
        let error = permission_where(
            &serde_json::json!({"id": "X-Hasura-User-Id"}),
            &session(&[]),
            &NameOverrides::default(),
            "public",
            "article",
        )
        .unwrap_err();
        // Refused rather than compared against nothing: a filter that matches
        // no rows for a reason the client cannot see is worse than a refusal.
        assert!(error.contains("x-hasura-user-id"), "unhelpful: {}", error);
    }

    #[test]
    fn a_list_operator_reads_its_variable_as_a_list() {
        assert_eq!(
            compiled(
                serde_json::json!({"name": {"_in": "X-Hasura-Free-Artists"}}),
                &session(&[("free_artists", "{Ann,Bob}")])
            ),
            serde_json::json!({"name": {"_in": ["Ann", "Bob"]}})
        );
        // An empty array literal is no items, not one empty item.
        assert_eq!(
            compiled(
                serde_json::json!({"name": {"_in": "X-Hasura-Free-Artists"}}),
                &session(&[("free_artists", "{}")])
            ),
            serde_json::json!({"name": {"_in": []}})
        );
        // And the same variable under an operator that takes one value stays
        // one value.
        assert_eq!(
            compiled(
                serde_json::json!({"name": {"_eq": "X-Hasura-Free-Artists"}}),
                &session(&[("free_artists", "{Ann,Bob}")])
            ),
            serde_json::json!({"name": {"_eq": "{Ann,Bob}"}})
        );
    }

    #[test]
    fn a_predicate_over_a_relationship_survives_unchanged() {
        // Nothing special to do: a relationship in a permission is the same
        // shape a relationship in `where` is, and the compiler below already
        // turns one into a correlated EXISTS.
        assert_eq!(
            compiled(
                serde_json::json!({"articles": {"is_published": {"_eq": true}}}),
                &session(&[])
            ),
            serde_json::json!({"articles": {"is_published": {"_eq": true}}})
        );
    }

    #[test]
    fn an_empty_filter_restricts_nothing() {
        let none = permission_where(
            &serde_json::json!({}),
            &session(&[]),
            &NameOverrides::default(),
            "public",
            "article",
        )
        .unwrap();
        // A real answer, and not the same as having no permission at all --
        // which is settled before this is reached.
        assert!(none.is_none());
    }

    #[test]
    fn a_renamed_column_is_named_the_way_the_schema_exposes_it() {
        let names =
            NameOverrides::parse(r#"{"public.article": {"columns": {"author_id": "writtenBy"}}}"#)
                .unwrap();
        let compiled = permission_where(
            &serde_json::json!({"author_id": "X-Hasura-User-Id"}),
            &session(&[("user_id", "1")]),
            &names,
            "public",
            "article",
        )
        .unwrap()
        .unwrap();
        // The permission names the column; the compiler below reads the field.
        assert_eq!(compiled, serde_json::json!({"writtenBy": {"_eq": "1"}}));
    }

    const WRITES: &str = r#"{
        "tables": {"public.article": {"permissions": {"user": {
            "select": {"columns": "*", "filter": {}},
            "insert": {"columns": "*", "check": {"is_published": false},
                       "set": {"author_id": "X-Hasura-User-Id", "source": "web"},
                       "backend_only": true},
            "update": {"columns": "*", "filter": {"author_id": "X-Hasura-User-Id"}},
            "delete": {"filter": {"author_id": "X-Hasura-User-Id"}}
        }}}}
    }"#;

    fn writer() -> (NameOverrides, HashMap<String, String>) {
        (
            NameOverrides::parse(WRITES).unwrap(),
            session(&[("user_id", "3")]),
        )
    }

    #[test]
    fn each_verb_is_asked_about_its_own_grant() {
        let (names, sess) = writer();
        let caller = Caller {
            role: Some("user"),
            session: &sess,
        };
        // An insert has no rows to choose from, which is why it has a check.
        assert_eq!(
            row_filter(&caller, &names, "public", "article", Verb::Insert).unwrap(),
            None
        );
        for verb in [Verb::Update, Verb::Delete] {
            assert_eq!(
                row_filter(&caller, &names, "public", "article", verb).unwrap(),
                Some(serde_json::json!({"author_id": {"_eq": "3"}}))
            );
        }
    }

    #[test]
    fn an_update_with_no_check_of_its_own_is_held_to_its_filter() {
        // Hasura's rule: a row you may change is a row you may change into
        // something you may still change.
        let (names, sess) = writer();
        let caller = Caller {
            role: Some("user"),
            session: &sess,
        };
        assert_eq!(
            write_check(&caller, &names, "public", "article", Verb::Update).unwrap(),
            Some(serde_json::json!({"author_id": {"_eq": "3"}}))
        );
        assert_eq!(
            write_check(&caller, &names, "public", "article", Verb::Insert).unwrap(),
            Some(serde_json::json!({"is_published": {"_eq": false}}))
        );
    }

    #[test]
    fn a_preset_resolves_a_session_variable_and_keeps_a_literal() {
        let (names, sess) = writer();
        let caller = Caller {
            role: Some("user"),
            session: &sess,
        };
        assert_eq!(
            presets(&caller, &names, "public", "article", Verb::Insert).unwrap(),
            vec![
                ("author_id".to_string(), serde_json::json!("3")),
                ("source".to_string(), serde_json::json!("web")),
            ]
        );
    }

    #[test]
    fn a_backend_only_insert_is_absent_until_a_backend_asks() {
        let (names, _) = writer();
        assert!(has_backend_only(&names, "user"));
        let of = |backend| {
            cache_for_role(
                &cache(vec![table("article", &["id"])]),
                &names,
                "user",
                backend,
            )
            .tables[&QualifiedIdentifier::new("public", "article")]
                .insertable
        };
        assert!(!of(false));
        assert!(of(true));
    }

    #[test]
    fn aggregates_are_asked_for_separately_from_rows() {
        let names = names();
        assert!(allows_aggregations(&names, "public", "article", "user"));
        // `reader` may read every column and was not granted aggregates.
        assert!(!allows_aggregations(&names, "public", "article", "reader"));
        assert!(!allows_aggregations(
            &names, "public", "article", "stranger"
        ));
    }
}
