//! What one role can see.
//!
//! Hasura builds a separate GraphQL schema for every role, and the difference
//! between them is not cosmetic. A role with no `select` permission on a table
//! does not get a table it is refused access to -- it gets a schema with no
//! such field in it, and naming one is a validation failure rather than a
//! denial. The same goes one level down: a column outside the permission is
//! absent from the type, not merely null.
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
//! What a cache cannot express is carried on [`SchemaConfig`] instead, and
//! there is one such thing: `allow_aggregations`, which is a fact about a
//! permission rather than about a table.
//!
//! [`SchemaConfig`]: crate::schema::SchemaConfig

use crate::names::NameOverrides;
use postrust_core::schema_cache::SchemaCache;

/// Reduce a schema cache to what one role may see.
///
/// Returns the cache unchanged when the document grants that role nothing to
/// narrow -- which is the case for `admin`, and for every role when the
/// document carries no permissions at all.
pub fn cache_for_role(cache: &SchemaCache, names: &NameOverrides, role: &str) -> SchemaCache {
    let mut view = cache.clone();

    view.tables.retain(|_, table| {
        let Some(granted) = names.permissions(&table.schema, &table.name, role) else {
            // Named nothing about this table. With a permission document in
            // play that is a statement, not a silence: the role cannot see it.
            return false;
        };
        let Some(select) = &granted.select else {
            // Reading is what makes a table exist here.
            //
            // Hasura is looser: a role may insert into a table it cannot read,
            // and 6 permissions in its corpus do exactly that. Reproducing it
            // means two column sets per table -- one for the type, one for the
            // input -- where a schema cache has one, and until the write
            // permissions are compiled there is nothing to hang the second on.
            //
            // So this is deliberately the stricter reading, in the direction
            // that withholds: such a role loses the write rather than gaining
            // a readable column. Recorded rather than pretended, and the
            // corpus cases it costs are counted.
            return false;
        };

        // The columns the role may see. Everything else is not merely
        // unreadable but absent, which is what makes this a schema question.
        //
        // The same one-column-set limit as above, and the same direction: 27
        // permissions in the corpus name a write column the role cannot read,
        // and here such a column is not writable either. Narrower than Hasura,
        // never wider.
        table.columns.retain(|name, _| select.columns.allows(name));

        // A computed field runs a function over the whole row, so it can
        // answer questions the column list was written to prevent. Hasura
        // grants them one at a time and grants none by default.
        table
            .computed_columns
            .retain(|name, _| select.computed_fields.iter().any(|f| f == name));

        // A write the role was not granted is a root field that does not
        // exist, and the builders already gate on these three.
        table.insertable = table.insertable && granted.insert.is_some();
        table.updatable = table.updatable && granted.update.is_some();
        table.deletable = table.deletable && granted.delete.is_some();

        true
    });

    // A relationship whose other end the role cannot see is not a field it
    // gets a null for -- there is nothing to name it after. Both ends are
    // checked: the map is keyed by the source, and the target may have gone
    // even where the source stayed.
    view.relationships.retain(|(source, _), links| {
        if !view.tables.contains_key(source) {
            return false;
        }
        links.retain(|link| view.tables.contains_key(link.foreign_table()));
        !links.is_empty()
    });

    view
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

/// The predicate a role's select permission adds to a read of one table.
///
/// `Ok(None)` where nothing restricts it.
pub fn read_predicate(
    caller: &Caller<'_>,
    names: &NameOverrides,
    schema: &str,
    table: &str,
) -> Result<Option<serde_json::Value>, Fault> {
    if caller.unrestricted() {
        return Ok(None);
    }
    let role = caller.role.unwrap_or_default();
    let Some(select) = names
        .permissions(schema, table, role)
        .and_then(|granted| granted.select.as_ref())
    else {
        return Err(Fault::NoPermission {
            role: role.to_string(),
            table: table.to_string(),
        });
    };
    permission_where(&select.filter, caller.session, names, schema, table)
        .map_err(Fault::MissingSessionVariable)
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

                if key.starts_with('_') {
                    // An operator. Its operand may be a session variable, and
                    // whether that is one value or several depends on which
                    // operator it is.
                    out.insert(
                        key.clone(),
                        rewrite(child, session, names, schema, table, Some(key))?,
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
                    .column(schema, table, key)
                    .unwrap_or(key.as_str())
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
        let view = cache_for_role(
            &cache(vec![table("article", &["id", "title", "content"])]),
            &names(),
            "user",
        );
        let article = &view.tables[&QualifiedIdentifier::new("public", "article")];
        assert!(article.has_column("title"));
        assert!(!article.has_column("content"));
    }

    #[test]
    fn a_wildcard_keeps_every_column() {
        let view = cache_for_role(
            &cache(vec![table("article", &["id", "title", "content"])]),
            &names(),
            "reader",
        );
        let article = &view.tables[&QualifiedIdentifier::new("public", "article")];
        assert_eq!(article.columns.len(), 3);
    }

    #[test]
    fn a_write_the_role_was_not_granted_is_not_a_root_field() {
        let view = cache_for_role(&cache(vec![table("article", &["id"])]), &names(), "user");
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
        let view = cache_for_role(&cache(vec![read_only]), &names(), "user");
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
        let view = cache_for_role(&cache(vec![article]), &names(), "user");
        let article = &view.tables[&QualifiedIdentifier::new("public", "article")];
        // The permission names no computed fields, and none is the default.
        assert!(article.computed_columns.is_empty());
    }

    #[test]
    fn a_table_that_can_only_be_written_is_withheld_rather_than_half_built() {
        // `writer` may insert and not read. Hasura exposes the insert; here
        // the table has no readable columns to build a type from, and the
        // stricter answer is the one that withholds. See `cache_for_role`.
        let names = NameOverrides::parse(
            r#"{"tables": {"public.article": {"permissions":
                 {"writer": {"insert": {"columns": "*", "check": {}}}}}}}"#,
        )
        .unwrap();
        let view = cache_for_role(&cache(vec![table("article", &["id"])]), &names, "writer");
        assert!(view.tables.is_empty());
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
