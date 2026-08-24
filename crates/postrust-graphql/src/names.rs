//! Names the schema does not carry.
//!
//! Almost everything in the generated API is derived: a table's own name gives
//! the root fields, a foreign key gives a relationship, a function gives a
//! computed field. Hasura derives none of it. Every one of those names is
//! written down in metadata by a person -- `add_computed_field` says the
//! function is `fetch_articles_plain` and the field is `get_articles`,
//! `create_array_relationship` says the relationship is `posts` -- and
//! reflection cannot recover a name nobody wrote down.
//!
//! That is the single largest remaining divergence from Hasura, and it does
//! not shrink by implementing anything. Around 25 cases in Hasura's own corpus
//! fail on it across four groups, every one of them by answering correctly
//! under a different name.
//!
//! So names can be given. Only names: this is a lookup table, not a metadata
//! model. It grants no permissions, tracks no tables, and a table absent from
//! it is exposed exactly as before.
//!
//! ```json
//! {
//!   "public.author": {
//!     "name": "Author",
//!     "relationships": { "article_author_id_fkey": "posts" },
//!     "computed_fields": { "automatic_comment_in_db_upper_name": "upper_name" }
//!   }
//! }
//! ```
//!
//! Relationships and computed fields are keyed by the thing the database
//! actually has -- a constraint name, or a function name -- rather than by the
//! name being replaced. A derived name is what this exists to change, so
//! keying by it would mean writing down the answer to ask the question; and
//! where two foreign keys point at one table the derived names collide, which
//! is one of the cases this is for.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// The names given to one table.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct TableNames {
    /// The base name for this table's root fields and types.
    ///
    /// `"name": "Author"` gives `Author`, `Author_by_pk`, `insert_Author`,
    /// `Author_bool_exp` and the rest, since all of them are derived from one
    /// base name.
    #[serde(default)]
    pub name: Option<String>,

    /// Relationship field names, keyed by constraint name -- or by function
    /// name for a computed relationship, which has no constraint.
    #[serde(default)]
    pub relationships: HashMap<String, String>,

    /// Computed field names, keyed by the function behind them.
    #[serde(default)]
    pub computed_fields: HashMap<String, String>,

    /// Whether this table is a set of allowed values rather than a set of
    /// rows.
    ///
    /// The one thing here that is not a name, and it is here for the same
    /// reason the names are: nothing in the schema says it. A table with a
    /// text primary key and a comment column is an ordinary table -- being an
    /// enumeration is a decision someone made about it, which Hasura records
    /// as `set_table_is_enum` and which reflection cannot recover.
    ///
    /// Marked so, its rows become the members of a GraphQL enum and every
    /// column with a foreign key to it is typed as that enum instead of as
    /// text.
    #[serde(default, rename = "enum")]
    pub is_enum: bool,
}

/// Every name given, keyed by `schema.table`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NameOverrides {
    tables: HashMap<String, TableNames>,
}

impl NameOverrides {
    /// Read from JSON: either the document itself, or a path to a file holding
    /// it.
    ///
    /// Told apart by the first non-space character, because a JSON object
    /// cannot begin with anything but `{` and a path cannot begin with it.
    /// Both spellings exist because both places are natural: a handful of
    /// names belongs in the environment beside every other setting, and a
    /// migrated schema's worth of them belongs in a file under review.
    pub fn parse(value: &str) -> Result<Self, String> {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Ok(Self::default());
        }

        let document = if trimmed.starts_with('{') {
            trimmed.to_string()
        } else {
            std::fs::read_to_string(trimmed)
                .map_err(|e| format!("cannot read GraphQL names from \"{}\": {}", trimmed, e))?
        };

        let tables: HashMap<String, TableNames> = serde_json::from_str(&document)
            .map_err(|e| format!("GraphQL names are not valid JSON: {}", e))?;

        for (key, names) in &tables {
            if !key.contains('.') {
                return Err(format!(
                    "\"{}\" does not name a table; keys are \"schema.table\", \
                     so a table in the default schema is still \"public.{}\"",
                    key, key
                ));
            }
            if names.name.as_deref() == Some("") {
                return Err(format!("\"{}\" is given an empty name", key));
            }
        }

        Ok(Self { tables })
    }

    /// Whether any name was given at all.
    pub fn is_empty(&self) -> bool {
        self.tables.is_empty()
    }

    /// How many tables were named, for the line the server logs at startup.
    pub fn len(&self) -> usize {
        self.tables.len()
    }

    fn table(&self, schema: &str, table: &str) -> Option<&TableNames> {
        self.tables.get(&format!("{}.{}", schema, table))
    }

    /// Whether a table was marked as a set of allowed values.
    pub fn is_enum(&self, schema: &str, table: &str) -> bool {
        self.table(schema, table).map(|t| t.is_enum).unwrap_or(false)
    }

    /// Every table marked as one, as `(schema, table)`.
    pub fn enum_tables(&self) -> Vec<(String, String)> {
        self.tables
            .iter()
            .filter(|(_, names)| names.is_enum)
            .filter_map(|(key, _)| {
                key.split_once('.')
                    .map(|(schema, table)| (schema.to_string(), table.to_string()))
            })
            .collect()
    }

    /// The base name given to a table, if one was.
    pub fn base_name(&self, schema: &str, table: &str) -> Option<&str> {
        self.table(schema, table)?.name.as_deref()
    }

    /// The name given to a relationship, by its constraint, column or
    /// function.
    ///
    /// Falls back to `computed_fields` because Hasura has one command for
    /// both: `add_computed_field` covers a function returning a value and a
    /// function returning rows, and which it is cannot be told from the
    /// metadata -- only from the database. A converter should not have to
    /// connect to one to put an entry in the right map, so either map answers.
    pub fn relationship(&self, schema: &str, table: &str, source: &str) -> Option<&str> {
        let names = self.table(schema, table)?;
        names
            .relationships
            .get(source)
            .or_else(|| names.computed_fields.get(source))
            .map(String::as_str)
    }

    /// The name given to a computed field, by its function.
    ///
    /// Reads both maps, for the reason [`Self::relationship`] gives.
    pub fn computed_field(&self, schema: &str, table: &str, function: &str) -> Option<&str> {
        let names = self.table(schema, table)?;
        names
            .computed_fields
            .get(function)
            .or_else(|| names.relationships.get(function))
            .map(String::as_str)
    }

    /// The function behind a computed field, given the name it is exposed
    /// under.
    ///
    /// The resolver reads the other direction: it is handed a field name from
    /// the selection and has to write the call. Searched rather than indexed
    /// because a table has a handful of computed fields, not thousands, and a
    /// second map would be one more thing to keep in step.
    pub fn computed_source(&self, schema: &str, table: &str, field: &str) -> Option<&str> {
        let names = self.table(schema, table)?;
        names
            .computed_fields
            .iter()
            .chain(names.relationships.iter())
            .find(|(_, exposed)| exposed.as_str() == field)
            .map(|(function, _)| function.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    const SAMPLE: &str = r#"{
        "public.author": {
            "name": "Author",
            "relationships": {"article_author_id_fkey": "posts",
                              "fetch_articles_plain": "get_articles"},
            "computed_fields": {"author_upper_name": "upper_name"}
        }
    }"#;

    #[test]
    fn names_are_read_from_a_document() {
        let names = NameOverrides::parse(SAMPLE).unwrap();
        assert_eq!(names.base_name("public", "author"), Some("Author"));
        assert_eq!(
            names.relationship("public", "author", "article_author_id_fkey"),
            Some("posts")
        );
        assert_eq!(
            names.computed_field("public", "author", "author_upper_name"),
            Some("upper_name")
        );
    }

    #[test]
    fn a_computed_field_can_be_looked_up_from_either_side() {
        let names = NameOverrides::parse(SAMPLE).unwrap();
        assert_eq!(
            names.computed_source("public", "author", "upper_name"),
            Some("author_upper_name")
        );
        assert_eq!(names.computed_source("public", "author", "nothing"), None);
    }

    #[test]
    fn a_computed_field_is_found_in_either_map() {
        // Hasura has one command for both, and which one an entry belongs in
        // cannot be told from its metadata.
        let names = NameOverrides::parse(
            r#"{"public.author": {"computed_fields": {"fetch_articles_plain": "get_articles"}}}"#,
        )
        .unwrap();
        assert_eq!(
            names.relationship("public", "author", "fetch_articles_plain"),
            Some("get_articles")
        );

        let other = NameOverrides::parse(
            r#"{"public.author": {"relationships": {"author_upper_name": "upper_name"}}}"#,
        )
        .unwrap();
        assert_eq!(
            other.computed_field("public", "author", "author_upper_name"),
            Some("upper_name")
        );
        assert_eq!(
            other.computed_source("public", "author", "upper_name"),
            Some("author_upper_name")
        );
    }

    #[test]
    fn a_table_can_be_marked_as_a_set_of_values() {
        let names = NameOverrides::parse(
            r#"{"public.colors": {"enum": true}, "public.author": {"name": "Author"}}"#,
        )
        .unwrap();
        assert!(names.is_enum("public", "colors"));
        assert!(!names.is_enum("public", "author"));
        assert_eq!(
            names.enum_tables(),
            vec![("public".to_string(), "colors".to_string())]
        );
    }

    #[test]
    fn a_table_that_was_not_named_is_left_alone() {
        let names = NameOverrides::parse(SAMPLE).unwrap();
        assert_eq!(names.base_name("public", "article"), None);
        assert_eq!(names.relationship("other", "author", "anything"), None);
    }

    #[test]
    fn nothing_given_is_not_an_error() {
        assert!(NameOverrides::parse("").unwrap().is_empty());
        assert!(NameOverrides::parse("   ").unwrap().is_empty());
        assert!(NameOverrides::parse("{}").unwrap().is_empty());
    }

    #[test]
    fn a_key_without_a_schema_says_so() {
        // The likeliest mistake, and one that would otherwise be silent: the
        // table is simply never found and every name is ignored.
        let error = NameOverrides::parse(r#"{"author": {"name": "Author"}}"#).unwrap_err();
        assert!(error.contains("public.author"), "unhelpful: {}", error);
    }

    #[test]
    fn a_document_that_is_not_one_says_so() {
        let error = NameOverrides::parse("{not json}").unwrap_err();
        assert!(error.contains("valid JSON"), "unhelpful: {}", error);
    }

    #[test]
    fn a_path_that_is_not_there_names_itself() {
        let error = NameOverrides::parse("/no/such/names.json").unwrap_err();
        assert!(error.contains("/no/such/names.json"), "unhelpful: {}", error);
    }
}
