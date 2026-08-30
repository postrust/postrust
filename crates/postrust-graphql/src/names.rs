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
//! So names can be given -- and, since a permission is the same kind of thing,
//! permissions too. Both are decisions a person wrote into metadata that no
//! schema remembers. This is still a lookup table rather than a metadata model:
//! it tracks no tables, offers no API to change it, and a table absent from it
//! is exposed exactly as before.
//!
//! ```json
//! {
//!   "public.author": {
//!     "name": "Authors",
//!     "roots": { "select_by_pk": "Author" },
//!     "columns": { "id": "AuthorId" },
//!     "relationships": { "article_author_id_fkey": "posts" },
//!     "computed_fields": { "automatic_comment_in_db_upper_name": "upper_name" },
//!     "permissions": {
//!       "user": {
//!         "select": {
//!           "columns": ["id", "name"],
//!           "filter": { "id": "X-Hasura-User-Id" },
//!           "limit": 10
//!         }
//!       }
//!     }
//!   }
//! }
//! ```
//!
//! The module is called `names` because names came first, and the document's
//! variable keeps both spellings for the same reason. What it holds is now
//! wider than that.
//!
//! Relationships and computed fields are keyed by the thing the database
//! actually has -- a constraint name, or a function name -- rather than by the
//! name being replaced. A derived name is what this exists to change, so
//! keying by it would mean writing down the answer to ask the question; and
//! where two foreign keys point at one table the derived names collide, which
//! is one of the cases this is for.
//!
//! Columns are keyed by the column, which is the same rule: the column is what
//! the database has. It is also the entry that reaches furthest -- a renamed
//! column is a name in the schema and in nothing else, so every path from a
//! request to SQL translates it back.

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

    /// The relationships this table has, where metadata says which.
    ///
    /// `relationships` above renames what reflection found; this replaces it.
    /// Two things need that, and they are the same thing seen from either end.
    ///
    /// A **manual** relationship maps column to column and has no constraint
    /// to be keyed by, so `relationships` cannot carry it at all -- and a
    /// permission written through one could not be compiled, which is a read
    /// refused rather than narrowed.
    ///
    /// And **which** relationships exist is metadata's to say. Reflection
    /// offers one per foreign key; Hasura offers the ones its metadata
    /// declares, which is usually fewer. A table whose keys are all declared
    /// looks the same either way, and one that tracks only some of them does
    /// not.
    ///
    /// Absent means "reflect", which is what a document that says nothing
    /// about a table has always meant. Present means "these, and no others".
    #[serde(default)]
    pub declared_relationships: Option<Vec<DeclaredRelationship>>,

    /// Computed field names, keyed by the function behind them.
    #[serde(default)]
    pub computed_fields: HashMap<String, String>,

    /// Root field names, keyed by the root they replace: `select`,
    /// `select_by_pk`, `select_aggregate`, `insert`, `insert_one`, `update`,
    /// `update_by_pk`, `delete`, `delete_by_pk`.
    ///
    /// Separate from `name` because Hasura names each root independently --
    /// `select: Authors`, `select_by_pk: Author`, `select_aggregate:
    /// AuthorAgg` -- where a base name derives all of them from one word. A
    /// set that agrees on a base could be written as `name`; one that does not
    /// can only be written here.
    #[serde(default)]
    pub roots: HashMap<String, String>,

    /// Descriptions Hasura keeps in metadata rather than in the database.
    ///
    /// A comment is not a name, and is here for the same reason the names are:
    /// where metadata carries one, the database's own comment is not what a
    /// client sees. An empty string is "no description", which is how
    /// `set_table_customization` suppresses a comment the database has.
    #[serde(default)]
    pub comments: Comments,

    /// Field names for columns, keyed by the column.
    ///
    /// The one rename that is not a name for something derived: a column has a
    /// name, and this is a different one to expose it under. It reaches
    /// further than the others because a column appears in the projection, in
    /// `where`, in `order_by`, in `distinct_on`, in both mutation inputs and
    /// in every embed -- everywhere a field name has to become SQL.
    #[serde(default)]
    pub columns: HashMap<String, String>,

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

    /// What each role may do with this table, keyed by role.
    ///
    /// A role absent from this map has no permission on the table at all,
    /// which in Hasura is not a refusal at execution but an absence in the
    /// schema: the root fields are not there to be named. The same is true one
    /// level down -- a role with `select` and no `insert` has no `insert_x`
    /// field, and asking for one is a validation failure rather than a denial.
    ///
    /// Empty for every table until a document says otherwise, and a document
    /// that says nothing about permissions leaves the server behaving as it did
    /// before they existed: database roles and row level security, with no
    /// second layer above them.
    #[serde(default)]
    pub permissions: HashMap<String, RolePermissions>,
}

impl RolePermissions {
    /// The columns one verb may name, where that verb names columns at all.
    pub fn write_columns(&self, verb: crate::role::Verb) -> Option<&ColumnSet> {
        match verb {
            crate::role::Verb::Insert => self.insert.as_ref().map(|p| &p.columns),
            crate::role::Verb::Update => self.update.as_ref().map(|p| &p.columns),
            crate::role::Verb::Select | crate::role::Verb::Delete => None,
        }
    }
}

/// What one role may do with one table.
///
/// Four independent grants, and the absence of one is meaningful: no `select`
/// means the table cannot be read by this role, not that it can be read
/// without restriction.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct RolePermissions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub select: Option<SelectPermission>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub insert: Option<InsertPermission>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update: Option<UpdatePermission>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delete: Option<DeletePermission>,
}

/// Which rows a role may read, and which of their columns.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SelectPermission {
    /// The columns this role can see. Every other column of the table is not
    /// merely unreadable but absent from the type, which is what makes a
    /// permission a statement about the schema rather than about a request.
    #[serde(default)]
    pub columns: ColumnSet,

    /// Which rows. A boolean expression in the same shape a `where` argument
    /// takes, with one addition: a string like `X-Hasura-User-Id` stands for
    /// the caller's session variable of that name.
    ///
    /// Left uninterpreted here. Reading it is the query builder's job, and a
    /// document that carries a predicate this server cannot compile should
    /// fail where the compilation is, not where the parsing is.
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub filter: serde_json::Value,

    /// The most rows a single request may read. A ceiling, not a default: a
    /// request asking for more gets this, and one asking for fewer gets what it
    /// asked for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u64>,

    /// Whether this role may ask for aggregates. Hasura keeps it separate from
    /// reading rows because counting rows you cannot see is a way of seeing
    /// them.
    #[serde(default)]
    pub allow_aggregations: bool,

    /// Which computed fields this role may ask for. Empty is none, not all:
    /// a computed field runs a function over a row, so it can answer questions
    /// the column permissions were written to prevent.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub computed_fields: Vec<String>,
}

/// What a role may write, and what the result has to satisfy.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct InsertPermission {
    /// The columns a request may supply.
    #[serde(default)]
    pub columns: ColumnSet,

    /// What every written row must satisfy. Unlike a `filter`, this is checked
    /// against the row *after* it is built, which is what stops a caller
    /// inserting a row it would not then be allowed to read.
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub check: serde_json::Value,

    /// Columns the server fills in, overriding whatever the request said. The
    /// value may be a session variable, which is how `author_id` comes from
    /// the caller's identity rather than from the caller.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub set: HashMap<String, serde_json::Value>,

    /// Reachable only by a caller that proved it holds the admin secret,
    /// whatever role it then claims. A mutation a client must not be able to
    /// reach by naming a role.
    #[serde(default)]
    pub backend_only: bool,
}

/// Which rows a role may change, which columns of them, and what the result
/// has to satisfy.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct UpdatePermission {
    #[serde(default)]
    pub columns: ColumnSet,

    /// Which rows may be changed, read before the change.
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub filter: serde_json::Value,

    /// What they must satisfy afterwards. Absent means the `filter` stands in,
    /// which is Hasura's rule: a row you may change is a row you may change
    /// into something you may still change.
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub check: serde_json::Value,

    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub set: HashMap<String, serde_json::Value>,
}

/// Which rows a role may delete.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct DeletePermission {
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub filter: serde_json::Value,
}

/// The columns a permission covers.
///
/// Hasura writes either a list or `"*"`, and the two are not the same thing
/// even when they name the same columns today: `"*"` follows the table, so a
/// column added tomorrow is covered, and a list does not.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum ColumnSet {
    /// Every column the table has, now and later.
    #[default]
    All,
    /// Exactly these.
    Named(Vec<String>),
}

impl ColumnSet {
    /// Whether a column is covered.
    pub fn allows(&self, column: &str) -> bool {
        match self {
            Self::All => true,
            Self::Named(columns) => columns.iter().any(|name| name == column),
        }
    }

    /// Whether this covers nothing at all.
    ///
    /// A permission naming no columns is one Hasura accepts and which grants
    /// only the ability to know the table exists.
    pub fn is_empty(&self) -> bool {
        matches!(self, Self::Named(columns) if columns.is_empty())
    }
}

impl Serialize for ColumnSet {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::All => serializer.serialize_str("*"),
            Self::Named(columns) => columns.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for ColumnSet {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Either {
            Wildcard(String),
            Named(Vec<String>),
        }

        match Either::deserialize(deserializer)? {
            Either::Named(columns) => Ok(Self::Named(columns)),
            Either::Wildcard(text) if text == "*" => Ok(Self::All),
            Either::Wildcard(text) => Err(serde::de::Error::custom(format!(
                "\"{}\" is not a column set; write a list of columns, or \"*\" for all of them",
                text
            ))),
        }
    }
}

/// Descriptions given to one table and the things on it.
///
/// Every value is a description as written: an empty string means the field
/// has none, which is a different answer from having said nothing.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Comments {
    /// The description of the table's own type.
    #[serde(default)]
    pub table: Option<String>,
    /// Column descriptions, keyed by the column.
    #[serde(default)]
    pub columns: HashMap<String, String>,
    /// Computed field descriptions, keyed by the function behind them.
    #[serde(default)]
    pub computed_fields: HashMap<String, String>,
    /// Root field descriptions, keyed by the root: `select`, `insert_one`.
    #[serde(default)]
    pub roots: HashMap<String, String>,
}

/// Every name given, keyed by `schema.table`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NameOverrides {
    tables: HashMap<String, TableNames>,
    functions: HashMap<String, FunctionNames>,
    introspection_disabled_for: Vec<String>,
}

/// The document in its sectioned shape.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
struct Sections {
    #[serde(default)]
    tables: HashMap<String, TableNames>,
    #[serde(default)]
    functions: HashMap<String, FunctionNames>,
    /// Roles that may not read the schema as data.
    ///
    /// Not per table, unlike everything else here, because the question is not
    /// about a table: Hasura's `set_graphql_schema_introspection_options` names
    /// roles and nothing else.
    #[serde(default)]
    introspection_disabled_for: Vec<String>,
}

/// One relationship metadata declares, and how it is reached.
///
/// Either by a key reflection already found -- `using` names the constraint or
/// the column, the same spellings [`TableNames::relationships`] accepts -- or
/// by a column mapping, which is a join no key describes.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct DeclaredRelationship {
    /// The field name this relationship is exposed under.
    pub name: String,
    /// The key it is reached by, where it is reached by one.
    #[serde(default)]
    pub using: Option<String>,
    /// The table it points at, as `schema.table`, for a mapped join.
    #[serde(default)]
    pub table: Option<String>,
    /// The join, as this table's column to the other table's column.
    #[serde(default)]
    pub columns: std::collections::BTreeMap<String, String>,
    /// Whether it yields one row rather than many.
    ///
    /// A key says this for itself; a column mapping does not, which is why
    /// Hasura declares object and array relationships separately.
    #[serde(default)]
    pub to_one: bool,
    /// Whether a nested insert writes this row *after* the row it hangs off.
    ///
    /// A key says which side holds it: `article.author_id` references
    /// `author`, so the author is written first and its key fills the
    /// article's column. A column mapping says only that two columns are
    /// equal, and `author.id = author_detail.id` is equal in both directions
    /// -- so which row can be written first is not recoverable from it, and
    /// Hasura writes `insertion_order` down for exactly that reason.
    ///
    /// `false` is `before_parent`, which is what an object relationship means
    /// without being told otherwise. Ignored for a relationship to many,
    /// which can only be written after the row it hangs off.
    #[serde(default)]
    pub after_parent: bool,
}

impl DeclaredRelationship {
    /// The table a mapped join points at, as `(schema, table)`.
    ///
    /// `None` for one reached by a key, which needs no target written down:
    /// the key already says where it goes.
    pub fn target(&self, default_schema: &str) -> Option<(String, String)> {
        let written = self.table.as_deref()?;
        Some(match written.split_once('.') {
            Some((schema, table)) => (schema.to_string(), table.to_string()),
            None => (default_schema.to_string(), written.to_string()),
        })
    }
}

/// What metadata says about one function, beyond what the catalogue knows.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct FunctionNames {
    /// The root this function is exposed on: `query` or `mutation`.
    #[serde(default)]
    pub exposed_as: Option<String>,
    /// The roles a function permission names.
    ///
    /// Hasura infers a *query* function's permission from the select
    /// permission on the table it returns -- a role that may read the rows may
    /// ask the function for them. A function exposed as a **mutation** is not
    /// inferred from anything: it has side effects, and a permission to read a
    /// table is not a permission to change it, so the role has to be named by
    /// a `pg_create_function_permission`.
    #[serde(default)]
    pub roles: Vec<String>,
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

        // Two shapes, and which one this is can be told from the keys: a
        // table key is `schema.table` and always has a dot, so `tables` and
        // `functions` at the top level cannot be tables and can only be the
        // sections they name. The flat shape came first and stays valid.
        let parsed: serde_json::Value = serde_json::from_str(&document)
            .map_err(|e| format!("GraphQL names are not valid JSON: {}", e))?;
        let sectioned = parsed.as_object().is_some_and(|map| {
            map.contains_key("tables")
                || map.contains_key("functions")
                || map.contains_key("introspection_disabled_for")
        });

        let (tables, functions, introspection_disabled_for) = match sectioned {
            true => {
                let document: Sections = serde_json::from_value(parsed)
                    .map_err(|e| format!("GraphQL names are not valid JSON: {}", e))?;
                (
                    document.tables,
                    document.functions,
                    document.introspection_disabled_for,
                )
            }
            false => (
                serde_json::from_value(parsed)
                    .map_err(|e| format!("GraphQL names are not valid JSON: {}", e))?,
                HashMap::new(),
                Vec::new(),
            ),
        };

        for key in functions.keys() {
            if !key.contains('.') {
                return Err(format!(
                    "\"{}\" does not name a function; keys are \"schema.function\", \
                     so a function in the default schema is still \"public.{}\"",
                    key, key
                ));
            }
        }

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

        Ok(Self {
            tables,
            functions,
            introspection_disabled_for,
        })
    }

    /// Whether any name was given at all.
    pub fn is_empty(&self) -> bool {
        self.tables.is_empty() && self.functions.is_empty()
    }

    /// Which root a function was placed on, if metadata said.
    ///
    /// `query` or `mutation`. Reflection places a function by its volatility,
    /// which is the only thing the catalogue records; Hasura lets metadata
    /// override it -- `track_function` with `configuration: {exposed_as:
    /// query}` puts a VOLATILE function on the query root, which is a thing a
    /// person decided and no schema remembers.
    pub fn exposed_as(&self, schema: &str, function: &str) -> Option<&str> {
        self.functions
            .get(&format!("{}.{}", schema, function))?
            .exposed_as
            .as_deref()
    }

    /// Whether a role was granted this function by a function permission.
    ///
    /// Only asked of a function exposed as a mutation -- see [`FunctionNames`]
    /// for why the query side is inferred instead.
    pub fn function_grants(&self, schema: &str, function: &str, role: &str) -> bool {
        self.functions
            .get(&format!("{}.{}", schema, function))
            .is_some_and(|given| given.roles.iter().any(|named| named == role))
    }

    /// The relationships metadata declares for a table, where it declares any.
    pub fn declared_relationships(
        &self,
        schema: &str,
        table: &str,
    ) -> Option<&[DeclaredRelationship]> {
        self.table(schema, table)?.declared_relationships.as_deref()
    }

    /// How many tables were named, for the line the server logs at startup.
    pub fn len(&self) -> usize {
        self.tables.len()
    }

    /// How many functions were placed, for the same line.
    pub fn placed_functions(&self) -> usize {
        self.functions.len()
    }

    fn table(&self, schema: &str, table: &str) -> Option<&TableNames> {
        self.tables.get(&format!("{}.{}", schema, table))
    }

    /// What one role may do with one table.
    ///
    /// `None` means the role was named nothing about this table, which is not
    /// the same as being named an empty permission: the first has no access,
    /// the second has access to nothing. Both refuse, and only the second is a
    /// decision someone wrote down.
    pub fn permissions(&self, schema: &str, table: &str, role: &str) -> Option<&RolePermissions> {
        self.table(schema, table)?.permissions.get(role)
    }

    /// What one role was granted, across every table that grants it anything.
    pub fn tables_with_permissions<'a>(
        &'a self,
        role: &'a str,
    ) -> impl Iterator<Item = &'a RolePermissions> + 'a {
        self.tables
            .values()
            .filter_map(move |table| table.permissions.get(role))
    }

    /// Whether this role may read the schema as data.
    ///
    /// `__typename` is not this question: it names the type of a row a client
    /// is already reading, every client library sends it, and Hasura answers it
    /// for a role introspection is disabled for. `__schema` and `__type` are
    /// the two that read the schema itself.
    pub fn introspection_disabled(&self, role: &str) -> bool {
        self.introspection_disabled_for.iter().any(|it| it == role)
    }

    /// Whether the document says anything about permissions at all.
    ///
    /// The switch for the whole layer. A document carrying only names leaves
    /// the server as it was: database roles and row level security, with
    /// nothing above them. One permission anywhere turns the layer on for
    /// every table, because a document that grants `user` a filtered view of
    /// `article` and says nothing about `author` is saying `user` cannot read
    /// `author` -- not that `author` is open to everyone.
    pub fn has_permissions(&self) -> bool {
        self.tables
            .values()
            .any(|table| !table.permissions.is_empty())
    }

    /// Every role the document names, sorted.
    ///
    /// Sorted because a schema is built per role and the order they are built
    /// in should not depend on how a hash map felt. `admin` is not among them
    /// unless a permission names it: an administrator is not a role with
    /// permissions but a caller the permissions do not apply to.
    pub fn roles(&self) -> Vec<&str> {
        let mut roles: Vec<&str> = self
            .tables
            .values()
            .flat_map(|table| table.permissions.keys())
            .map(String::as_str)
            .collect();
        roles.sort_unstable();
        roles.dedup();
        roles
    }

    /// How many table-and-role pairs were granted something, for the line the
    /// server logs at startup.
    pub fn granted(&self) -> usize {
        self.tables
            .values()
            .map(|table| table.permissions.len())
            .sum()
    }

    /// Whether a table was marked as a set of allowed values.
    pub fn is_enum(&self, schema: &str, table: &str) -> bool {
        self.table(schema, table)
            .map(|t| t.is_enum)
            .unwrap_or(false)
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

    /// The name given to one root field, if one was.
    ///
    /// `kind` is Hasura's own key -- `select`, `insert_one`, `delete_by_pk`.
    pub fn root(&self, schema: &str, table: &str, kind: &str) -> Option<&str> {
        self.table(schema, table)?
            .roots
            .get(kind)
            .map(String::as_str)
    }

    /// The field a column is exposed as, if it was renamed.
    pub fn column(&self, schema: &str, table: &str, column: &str) -> Option<&str> {
        self.table(schema, table)?
            .columns
            .get(column)
            .map(String::as_str)
    }

    /// The column behind a field name, if that name is a rename.
    ///
    /// The other direction, which is the one every resolver needs: it is
    /// handed a field name from the request and has to write a column. `None`
    /// means the name is already the column's, which is the ordinary case.
    pub fn column_source(&self, schema: &str, table: &str, field: &str) -> Option<&str> {
        let names = self.table(schema, table)?;
        names
            .columns
            .iter()
            .find(|(_, exposed)| exposed.as_str() == field)
            .map(|(column, _)| column.as_str())
    }

    /// Whether any column of this table is exposed under another name.
    ///
    /// Worth asking before building a projection: a table with no renames
    /// keeps the `SELECT *` it had, and only one that has them pays for a
    /// column list.
    pub fn renames_columns(&self, schema: &str, table: &str) -> bool {
        self.table(schema, table)
            .map(|names| !names.columns.is_empty())
            .unwrap_or(false)
    }

    /// The description given to a table's type, if one was.
    ///
    /// `Some("")` is a description that was given and is empty, which means
    /// the type has none. `None` means nothing was said and the database's own
    /// comment stands.
    pub fn table_comment(&self, schema: &str, table: &str) -> Option<&str> {
        self.table(schema, table)?.comments.table.as_deref()
    }

    /// The description given to a column, by the column.
    pub fn column_comment(&self, schema: &str, table: &str, column: &str) -> Option<&str> {
        self.table(schema, table)?
            .comments
            .columns
            .get(column)
            .map(String::as_str)
    }

    /// The description given to a computed field, by its function.
    pub fn computed_comment(&self, schema: &str, table: &str, function: &str) -> Option<&str> {
        self.table(schema, table)?
            .comments
            .computed_fields
            .get(function)
            .map(String::as_str)
    }

    /// The description given to one root field, by its kind.
    pub fn root_comment(&self, schema: &str, table: &str, kind: &str) -> Option<&str> {
        self.table(schema, table)?
            .comments
            .roots
            .get(kind)
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

    /// A declaration is the whole list, and it may describe a join.
    #[test]
    fn declared_relationships_carry_a_join_and_not_just_a_name() {
        let names = NameOverrides::parse(
            r#"{"tables": {"public.message": {"declared_relationships": [
                 {"name": "members", "table": "public.channel_member",
                  "columns": {"channel_id": "channel_id"}},
                 {"name": "author", "using": "message_author_id_fkey", "to_one": true}
               ]}}}"#,
        )
        .unwrap();
        let declared = names
            .declared_relationships("public", "message")
            .expect("the table declares some");
        assert_eq!(declared.len(), 2);
        assert_eq!(declared[0].name, "members");
        assert_eq!(
            declared[0].target("public"),
            Some(("public".to_string(), "channel_member".to_string()))
        );
        assert_eq!(
            declared[0].columns.get("channel_id").map(String::as_str),
            Some("channel_id")
        );
        assert!(!declared[0].to_one, "an array relationship by default");
        assert_eq!(declared[1].using.as_deref(), Some("message_author_id_fkey"));
        assert!(declared[1].to_one);
        // An unqualified table is read in the table's own schema.
        assert_eq!(
            crate::names::DeclaredRelationship {
                table: Some("channel_member".to_string()),
                ..Default::default()
            }
            .target("other"),
            Some(("other".to_string(), "channel_member".to_string()))
        );
        // And a table that declares nothing is left to reflection.
        assert_eq!(names.declared_relationships("public", "author"), None);
    }

    #[test]
    fn a_table_that_was_not_named_is_left_alone() {
        let names = NameOverrides::parse(SAMPLE).unwrap();
        assert_eq!(names.base_name("public", "article"), None);
        assert_eq!(names.relationship("other", "author", "anything"), None);
    }

    /// The sectioned shape, told apart from the flat one by a key that
    /// cannot be a table: a table key always carries a dot.
    #[test]
    fn a_function_can_be_placed_on_a_root() {
        let names = NameOverrides::parse(
            r#"{"tables": {"public.author": {"name": "Authors"}},
                "functions": {"public.volatile_func1": {"exposed_as": "query"}}}"#,
        )
        .unwrap();
        assert_eq!(names.base_name("public", "author"), Some("Authors"));
        assert_eq!(names.exposed_as("public", "volatile_func1"), Some("query"));
        assert_eq!(names.exposed_as("public", "something_else"), None);
    }

    #[test]
    fn the_flat_shape_is_still_read() {
        let names = NameOverrides::parse(SAMPLE).unwrap();
        assert!(!names.is_empty());
        assert_eq!(names.exposed_as("public", "anything"), None);
    }

    #[test]
    fn a_function_key_names_its_schema() {
        let error = NameOverrides::parse(r#"{"functions": {"volatile_func1": {}}}"#)
            .expect_err("a bare function name is not a key");
        assert!(error.contains("schema.function"), "{}", error);
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
        assert!(
            error.contains("/no/such/names.json"),
            "unhelpful: {}",
            error
        );
    }

    const GRANTED: &str = r#"{
        "tables": {
            "public.article": {
                "permissions": {
                    "user": {
                        "select": {
                            "columns": ["id", "title", "content"],
                            "filter": {"$or": [{"author_id": "X-HASURA-USER-ID"},
                                               {"is_published": true}]},
                            "limit": 10,
                            "allow_aggregations": true,
                            "computed_fields": ["get_articles"]
                        },
                        "insert": {
                            "check": {"author_id": "X-Hasura-User-Id"},
                            "columns": ["title", "content"],
                            "set": {"author_id": "X-Hasura-User-Id"},
                            "backend_only": true
                        },
                        "update": {"columns": ["title"], "filter": {}},
                        "delete": {"filter": {"is_published": false}}
                    },
                    "anonymous": {
                        "select": {"columns": "*", "filter": {"is_published": true}}
                    }
                }
            }
        }
    }"#;

    #[test]
    fn what_a_role_may_do_is_read_from_the_document() {
        let names = NameOverrides::parse(GRANTED).unwrap();
        let user = names.permissions("public", "article", "user").unwrap();

        let select = user.select.as_ref().unwrap();
        assert!(select.columns.allows("title"));
        assert!(!select.columns.allows("is_published"));
        assert_eq!(select.limit, Some(10));
        assert!(select.allow_aggregations);
        assert_eq!(select.computed_fields, vec!["get_articles".to_string()]);
        // The predicate is carried, not read: compiling it is the query
        // builder's job and a failure there should say so there.
        assert_eq!(
            select.filter["$or"][1]["is_published"],
            serde_json::json!(true)
        );

        let insert = user.insert.as_ref().unwrap();
        assert!(insert.backend_only);
        assert_eq!(
            insert.set.get("author_id"),
            Some(&serde_json::json!("X-Hasura-User-Id"))
        );

        assert!(user.update.is_some());
        assert!(user.delete.is_some());
    }

    #[test]
    fn a_wildcard_column_set_is_not_a_list_that_happens_to_be_long() {
        let names = NameOverrides::parse(GRANTED).unwrap();
        let anonymous = names.permissions("public", "article", "anonymous").unwrap();
        let select = anonymous.select.as_ref().unwrap();
        assert_eq!(select.columns, ColumnSet::All);
        // Which means a column nobody has thought of yet is covered too.
        assert!(select.columns.allows("a_column_added_next_year"));
        // And this role was told nothing about writing.
        assert!(anonymous.insert.is_none());
    }

    #[test]
    fn a_role_named_nothing_has_nothing() {
        let names = NameOverrides::parse(GRANTED).unwrap();
        assert!(names.permissions("public", "article", "stranger").is_none());
        assert!(names.permissions("public", "author", "user").is_none());
    }

    #[test]
    fn the_roles_come_back_sorted_and_once_each() {
        let names = NameOverrides::parse(GRANTED).unwrap();
        assert_eq!(names.roles(), vec!["anonymous", "user"]);
        assert_eq!(names.granted(), 2);
    }

    #[test]
    fn a_document_of_names_alone_leaves_the_layer_off() {
        assert!(!NameOverrides::parse(SAMPLE).unwrap().has_permissions());
        assert!(NameOverrides::parse(GRANTED).unwrap().has_permissions());
        assert!(NameOverrides::parse("").unwrap().roles().is_empty());
    }

    #[test]
    fn a_column_set_that_is_neither_says_so() {
        let error = NameOverrides::parse(
            r#"{"tables": {"public.a": {"permissions":
                 {"user": {"select": {"columns": "all"}}}}}}"#,
        )
        .unwrap_err();
        assert!(error.contains("not a column set"), "unhelpful: {}", error);
    }

    #[test]
    fn a_permission_survives_the_round_trip() {
        // The converter writes this document and the server reads it, so the
        // two spellings of a column set have to mean the same thing in both
        // directions -- `"*"` must not come back as the one-element list
        // `["*"]`, which would name a column no table has.
        let names = NameOverrides::parse(GRANTED).unwrap();
        let written = serde_json::to_string(&Sections {
            tables: names.tables.clone(),
            functions: HashMap::new(),
            introspection_disabled_for: Vec::new(),
        })
        .unwrap();
        let again = NameOverrides::parse(&written).unwrap();
        assert_eq!(again, names);
    }
}
