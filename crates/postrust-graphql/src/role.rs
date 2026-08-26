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
