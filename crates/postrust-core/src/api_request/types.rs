//! Core types for API request parsing, mirroring PostgREST's type system.
//!
//! These types represent the parsed structure of an HTTP request before
//! it's converted into an execution plan.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

// ============================================================================
// Identifiers
// ============================================================================

/// A fully qualified identifier with schema and name.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct QualifiedIdentifier {
    /// The schema. Empty when unqualified, leaving resolution to `search_path`.
    pub schema: String,
    /// The relation or routine's own name.
    pub name: String,
}

impl QualifiedIdentifier {
    /// A `schema.name` identifier.
    pub fn new(schema: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            schema: schema.into(),
            name: name.into(),
        }
    }

    /// Create an identifier without a schema (uses default search path).
    pub fn unqualified(name: impl Into<String>) -> Self {
        Self {
            schema: String::new(),
            name: name.into(),
        }
    }
}

impl std::fmt::Display for QualifiedIdentifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.schema.is_empty() {
            write!(f, "{}", self.name)
        } else {
            write!(f, "{}.{}", self.schema, self.name)
        }
    }
}

/// A column name, as the request wrote it.
pub type FieldName = String;
/// A schema name.
pub type Schema = String;
/// The name a selected field is returned under, where the request renamed it.
pub type Alias = String;
/// A PostgreSQL type name a selected field is cast to, from `::type`.
pub type Cast = String;
/// The `!hint` naming which relationship or which function signature to use.
pub type Hint = String;
/// A text-search configuration name, from `fts(english)`.
pub type Language = String;

// ============================================================================
// JSON Path
// ============================================================================

/// Operand for JSON path operations.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum JsonOperand {
    /// Object key access: `->key`
    Key(String),
    /// Array index access: `->0`
    Idx(i32),
}

/// JSON path operation type.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum JsonOperation {
    /// Returns JSON: `->`
    Arrow(JsonOperand),
    /// Returns text: `->>`
    DoubleArrow(JsonOperand),
}

/// A path into a JSON column.
pub type JsonPath = Vec<JsonOperation>;

/// A field reference, optionally with JSON path.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Field {
    /// The column.
    pub name: FieldName,
    /// Steps into the column's JSON, empty for an ordinary column.
    pub json_path: JsonPath,
}

impl Field {
    /// A plain column reference, with no JSON path.
    pub fn simple(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            json_path: Vec::new(),
        }
    }

    /// A column reference that reaches into the column's JSON.
    pub fn with_json_path(name: impl Into<String>, json_path: JsonPath) -> Self {
        Self {
            name: name.into(),
            json_path,
        }
    }
}

// ============================================================================
// Actions
// ============================================================================

/// How an RPC function is invoked.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InvokeMethod {
    /// POST invocation (can have side effects)
    Inv,
    /// GET/HEAD invocation (read-only)
    InvRead {
        /// The request was HEAD, so the body is computed and discarded.
        headers_only: bool,
    },
}

/// Type of mutation operation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mutation {
    /// POST - Insert new records
    Create,
    /// PATCH - Update existing records (partial)
    Update,
    /// DELETE - Remove records
    Delete,
    /// PUT - Upsert a single record
    SingleUpsert,
}

/// The parsed resource from the URL path.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Resource {
    /// A table or view: `/table_name`
    Relation(String),
    /// An RPC function: `/rpc/function_name`
    Routine(String),
    /// The root schema: `/`
    Schema,
}

/// Database action derived from HTTP method and resource.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DbAction {
    /// SELECT from a table/view
    RelationRead {
        /// The relation being read.
        qi: QualifiedIdentifier,
        /// The request was HEAD: headers are computed, the body discarded.
        headers_only: bool,
    },
    /// INSERT/UPDATE/DELETE on a table
    RelationMut {
        /// The relation being written.
        qi: QualifiedIdentifier,
        /// Which write.
        mutation: Mutation,
    },
    /// Call a stored function
    Routine {
        /// The function being called.
        qi: QualifiedIdentifier,
        /// How it was invoked, which decides whether it may write.
        invoke_method: InvokeMethod,
    },
    /// Read schema metadata
    SchemaRead {
        /// The schema being described.
        schema: Schema,
        /// The request was HEAD.
        headers_only: bool,
    },
}

/// The action to perform, which may or may not require database access.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Action {
    /// Database operation
    Db(DbAction),
    /// OPTIONS on a table (returns metadata)
    RelationInfo(QualifiedIdentifier),
    /// OPTIONS on a function
    RoutineInfo {
        /// The function being described.
        qi: QualifiedIdentifier,
        /// How it would be invoked.
        invoke_method: InvokeMethod,
    },
    /// OPTIONS on root (returns OpenAPI spec)
    SchemaInfo,
}

// ============================================================================
// Filter Operations
// ============================================================================

/// Simple binary operators (always single value).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SimpleOperator {
    /// `neq` - Not equal
    NotEqual,
    /// `cs` - Contains (array/range)
    Contains,
    /// `cd` - Contained by (array/range)
    Contained,
    /// `ov` - Overlaps (array/range)
    Overlap,
    /// `sl` - Strictly left of (range)
    StrictlyLeft,
    /// `sr` - Strictly right of (range)
    StrictlyRight,
    /// `nxr` - Does not extend to the right (range)
    NotExtendsRight,
    /// `nxl` - Does not extend to the left (range)
    NotExtendsLeft,
    /// `adj` - Adjacent to (range)
    Adjacent,
}

impl SimpleOperator {
    /// Get the SQL operator for this simple operator.
    pub fn to_sql(&self) -> &'static str {
        match self {
            Self::NotEqual => "<>",
            Self::Contains => "@>",
            Self::Contained => "<@",
            Self::Overlap => "&&",
            Self::StrictlyLeft => "<<",
            Self::StrictlyRight => ">>",
            Self::NotExtendsRight => "&<",
            Self::NotExtendsLeft => "&>",
            Self::Adjacent => "-|-",
        }
    }
}

/// Quantified operators (can use `any` or `all` modifiers).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum QuantOperator {
    /// `eq` - Equal
    Equal,
    /// `gte` - Greater than or equal
    GreaterThanEqual,
    /// `gt` - Greater than
    GreaterThan,
    /// `lte` - Less than or equal
    LessThanEqual,
    /// `lt` - Less than
    LessThan,
    /// `like` - LIKE pattern match
    Like,
    /// `ilike` - Case-insensitive LIKE
    ILike,
    /// `match` - Regex match (~)
    Match,
    /// `imatch` - Case-insensitive regex (~*)
    IMatch,
}

impl QuantOperator {
    /// Get the SQL operator for this quantified operator.
    pub fn to_sql(&self) -> &'static str {
        match self {
            Self::Equal => "=",
            Self::GreaterThanEqual => ">=",
            Self::GreaterThan => ">",
            Self::LessThanEqual => "<=",
            Self::LessThan => "<",
            Self::Like => "LIKE",
            Self::ILike => "ILIKE",
            Self::Match => "~",
            Self::IMatch => "~*",
        }
    }
}

/// Quantifier for array operations.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OpQuantifier {
    /// Match any element
    Any,
    /// Match all elements
    All,
}

/// Full-text search operators.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FtsOperator {
    /// `fts` - to_tsquery
    Fts,
    /// `plfts` - plainto_tsquery
    Plain,
    /// `phfts` - phraseto_tsquery
    Phrase,
    /// `wfts` - websearch_to_tsquery
    Websearch,
}

impl FtsOperator {
    /// Get the PostgreSQL function name for this FTS operator.
    pub fn to_function(&self) -> &'static str {
        match self {
            Self::Fts => "to_tsquery",
            Self::Plain => "plainto_tsquery",
            Self::Phrase => "phraseto_tsquery",
            Self::Websearch => "websearch_to_tsquery",
        }
    }
}

/// Value for IS comparisons.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum IsValue {
    /// `is.null`
    Null,
    /// `is.notnull`
    NotNull,
    /// `is.true`
    True,
    /// `is.false`
    False,
    /// `is.unknown`
    Unknown,
}

impl IsValue {
    /// The SQL keyword this compares against.
    pub fn to_sql(&self) -> &'static str {
        match self {
            Self::Null => "NULL",
            Self::NotNull => "NOT NULL",
            Self::True => "TRUE",
            Self::False => "FALSE",
            Self::Unknown => "UNKNOWN",
        }
    }
}

/// A filter operation with its value(s).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Operation {
    /// Simple binary operation: `col.neq.value`
    Simple {
        /// The operator.
        op: SimpleOperator,
        /// Its right-hand operand, unparsed: the column's type decides how it
        /// is read.
        value: String,
    },
    /// Quantified operation: `col.eq.value` or `col.eq(any).{arr}`
    Quant {
        /// The operator.
        op: QuantOperator,
        /// `any` or `all`, where the request supplied one.
        quantifier: Option<OpQuantifier>,
        /// The operand, unparsed.
        value: String,
    },
    /// IN list: `col.in.(a,b,c)`
    In(Vec<String>),
    /// IS comparison: `col.is.null`
    Is(IsValue),
    /// IS DISTINCT FROM: `col.isdistinct.value`
    IsDistinctFrom(String),
    /// Full-text search: `col.fts(english).query`
    Fts {
        /// Which of the text-search functions to build the query with.
        op: FtsOperator,
        /// The text-search configuration, where the request named one.
        language: Option<Language>,
        /// The search text.
        value: String,
    },
}

/// An operator expression, possibly negated.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OpExpr {
    /// Whether this expression is negated (NOT)
    pub negated: bool,
    /// The operation to perform
    pub operation: Operation,
}

impl OpExpr {
    /// The operation as written.
    pub fn new(operation: Operation) -> Self {
        Self {
            negated: false,
            operation,
        }
    }

    /// The operation under `not.`.
    pub fn negated(operation: Operation) -> Self {
        Self {
            negated: true,
            operation,
        }
    }
}

// ============================================================================
// Filters and Logic Trees
// ============================================================================

/// A single filter on a field.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Filter {
    /// The column being filtered, with any JSON path into it.
    pub field: Field,
    /// The comparison applied to it.
    pub op_expr: OpExpr,
}

impl Filter {
    /// One column compared one way.
    pub fn new(field: Field, op_expr: OpExpr) -> Self {
        Self { field, op_expr }
    }
}

/// Boolean logic operator.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LogicOperator {
    /// Every child must hold.
    And,
    /// At least one child must hold.
    Or,
}

/// A tree of boolean logic combining filters.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum LogicTree {
    /// A boolean expression combining children
    Expr {
        /// Whether the whole node is negated, as in `not.and(...)`.
        negated: bool,
        /// How the children combine.
        op: LogicOperator,
        /// The operands.
        children: Vec<LogicTree>,
    },
    /// A leaf filter
    Stmt(Filter),
}

/// The names embedded resources answer to in a selection.
///
/// The name a filter would use is the one the response uses: the alias where
/// there is one, the relation's own name otherwise. A spread has neither --
/// its columns land in the parent and nothing is named -- so there is nothing
/// to filter by.
pub fn embedded_names(select: &[SelectItem]) -> Vec<String> {
    select
        .iter()
        .filter_map(|item| match item {
            SelectItem::Relation {
                relation, alias, ..
            } => Some(alias.clone().unwrap_or_else(|| relation.clone())),
            _ => None,
        })
        .collect()
}

impl LogicTree {
    /// Whether every leaf of this tree names one of `embeds`.
    ///
    /// `or=(clientinfo.not.is.null,contact.not.is.null)` asks about embedded
    /// resources rather than about columns: whether the related row was
    /// there. Those are not names the table has, so a tree made only of them
    /// cannot be evaluated where the table's own filters are -- it has to
    /// wait until the embeds themselves are in scope. Answering this decides
    /// which of the two places the tree belongs to.
    pub fn names_only(&self, embeds: &[String]) -> bool {
        match self {
            Self::Expr { children, .. } => {
                !children.is_empty() && children.iter().all(|child| child.names_only(embeds))
            }
            Self::Stmt(filter) => embeds.contains(&filter.field.name),
        }
    }

    /// Combine children with `AND`, not negated.
    pub fn and(children: Vec<LogicTree>) -> Self {
        Self::Expr {
            negated: false,
            op: LogicOperator::And,
            children,
        }
    }

    /// Combine children with `OR`, not negated.
    pub fn or(children: Vec<LogicTree>) -> Self {
        Self::Expr {
            negated: false,
            op: LogicOperator::Or,
            children,
        }
    }

    /// A tree of exactly one filter.
    pub fn filter(filter: Filter) -> Self {
        Self::Stmt(filter)
    }
}

// ============================================================================
// Select Items
// ============================================================================

/// Aggregate functions.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AggregateFunction {
    /// `sum()`
    Sum,
    /// `avg()`
    Avg,
    /// `max()`
    Max,
    /// `min()`
    Min,
    /// `count()`
    Count,
}

impl AggregateFunction {
    /// The SQL function name.
    pub fn to_sql(&self) -> &'static str {
        match self {
            Self::Sum => "SUM",
            Self::Avg => "AVG",
            Self::Max => "MAX",
            Self::Min => "MIN",
            Self::Count => "COUNT",
        }
    }
}

/// Join type for embedded resources.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum JoinType {
    /// INNER JOIN - only matching rows
    Inner,
    /// LEFT JOIN - all parent rows (default)
    #[default]
    Left,
}

/// An item in the select list.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum SelectItem {
    /// Select a column, possibly with aggregation
    Field {
        /// The column, with any JSON path into it.
        field: Field,
        /// An aggregate applied to it, from `col.sum()`.
        aggregate: Option<AggregateFunction>,
        /// A cast applied to the aggregate's result, distinct from `cast`,
        /// which applies to the column before aggregating.
        aggregate_cast: Option<Cast>,
        /// A cast applied to the column, from `col::text`.
        cast: Option<Cast>,
        /// The key this is returned under, where the request renamed it.
        alias: Option<Alias>,
    },
    /// Embed a related resource
    Relation {
        /// The related resource, as the request named it.
        relation: FieldName,
        /// The key the embed is returned under, where renamed.
        alias: Option<Alias>,
        /// Which relationship to follow, where several connect the two.
        hint: Option<Hint>,
        /// Whether a parent with no children is kept. Defaults to `Left`.
        join_type: Option<JoinType>,
        /// Columns and further embeds selected on the related resource.
        ///
        /// Empty means every column, matching a bare `relation()`.
        select: Vec<SelectItem>,
    },
    /// Spread a related resource's columns (horizontal embedding)
    ///
    /// The related resource's columns land in the parent object itself rather
    /// than under a key of their own, so unlike [`Self::Relation`] there is no
    /// alias: nothing is being named.
    SpreadRelation {
        /// The related resource, as the request named it.
        relation: FieldName,
        /// Which relationship to follow, where several connect the two.
        hint: Option<Hint>,
        /// Whether a parent with no children is kept. Defaults to `Left`.
        join_type: Option<JoinType>,
        /// Columns and further embeds selected on the related resource.
        ///
        /// Empty means every column, matching a bare `...relation()`.
        select: Vec<SelectItem>,
    },
}

impl SelectItem {
    /// Create a simple field selection.
    pub fn field(name: impl Into<String>) -> Self {
        Self::Field {
            field: Field::simple(name),
            aggregate: None,
            aggregate_cast: None,
            cast: None,
            alias: None,
        }
    }

    /// Create a relation embedding.
    pub fn relation(name: impl Into<String>) -> Self {
        Self::Relation {
            relation: name.into(),
            alias: None,
            hint: None,
            join_type: None,
            select: Vec::new(),
        }
    }
}

// ============================================================================
// Ordering
// ============================================================================

/// Sort direction.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum OrderDirection {
    /// Ascending, PostgreSQL's default.
    #[default]
    Asc,
    /// Descending.
    Desc,
}

impl OrderDirection {
    /// The `ORDER BY` keyword.
    pub fn to_sql(&self) -> &'static str {
        match self {
            Self::Asc => "ASC",
            Self::Desc => "DESC",
        }
    }
}

/// NULL ordering.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderNulls {
    /// Nulls sort before non-nulls.
    First,
    /// Nulls sort after non-nulls.
    Last,
}

impl OrderNulls {
    /// The `NULLS FIRST` / `NULLS LAST` clause.
    pub fn to_sql(&self) -> &'static str {
        match self {
            Self::First => "NULLS FIRST",
            Self::Last => "NULLS LAST",
        }
    }
}

/// An ORDER BY term.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum OrderTerm {
    /// Order by a field
    Field {
        /// The column to sort on.
        field: Field,
        /// Ascending or descending. Absent leaves PostgreSQL's default.
        direction: Option<OrderDirection>,
        /// Where nulls sort. Absent leaves PostgreSQL's default, which depends
        /// on the direction.
        nulls: Option<OrderNulls>,
    },
    /// Order by a field from an embedded relation
    Relation {
        /// The embedded resource holding the column.
        relation: FieldName,
        /// The column to sort on.
        field: Field,
        /// Ascending or descending.
        direction: Option<OrderDirection>,
        /// Where nulls sort.
        nulls: Option<OrderNulls>,
    },
}

impl OrderTerm {
    /// Sort on a column, leaving direction and null placement to PostgreSQL.
    pub fn field(name: impl Into<String>) -> Self {
        Self::Field {
            field: Field::simple(name),
            direction: None,
            nulls: None,
        }
    }

    /// Sort on a column, descending.
    pub fn field_desc(name: impl Into<String>) -> Self {
        Self::Field {
            field: Field::simple(name),
            direction: Some(OrderDirection::Desc),
            nulls: None,
        }
    }
}

// ============================================================================
// Pagination
// ============================================================================

/// A range for pagination (offset and limit).
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Range {
    /// Rows to skip before the first returned.
    pub offset: i64,
    /// How many rows at most. `None` is unbounded.
    pub limit: Option<i64>,
    /// Whether `offset` was asked for rather than defaulted to.
    ///
    /// A query parameter takes precedence over the `Range` header, and
    /// `?offset=0` is a request to start at the first row -- not the absence
    /// of one. Without this the two are the same value and `?limit=10` with a
    /// `Range: 5-9` header starts at the header's row, which is neither what
    /// was asked for nor where the reported `Content-Range` says it began.
    #[serde(default)]
    pub offset_explicit: bool,
}

impl Range {
    /// A range from an explicit offset and limit, as query parameters give.
    pub fn new(offset: i64, limit: Option<i64>) -> Self {
        Self {
            offset,
            limit,
            offset_explicit: false,
        }
    }

    /// Create a range from HTTP Range header format (0-9 means rows 0-9 inclusive).
    pub fn from_bounds(start: i64, end: Option<i64>) -> Self {
        Self {
            offset: start,
            limit: end.map(|e| e - start + 1),
            offset_explicit: false,
        }
    }

    /// Check if this range has a limit.
    pub fn has_limit(&self) -> bool {
        self.limit.is_some()
    }
}

// ============================================================================
// Payload
// ============================================================================

/// Request body payload.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Payload {
    /// Parsed JSON with extracted keys
    ProcessedJson {
        /// The body as received, passed to PostgreSQL unaltered.
        raw: bytes::Bytes,
        /// The union of the keys across every object in the body, which is
        /// what decides the column list when `?columns=` was not given.
        keys: HashSet<String>,
    },
    /// URL-encoded form data
    ProcessedUrlEncoded {
        /// The decoded pairs, in the order they were sent.
        data: Vec<(String, String)>,
        /// The distinct keys among them.
        keys: HashSet<String>,
    },
    /// Raw JSON (used with &columns parameter)
    RawJson(bytes::Bytes),
    /// Raw binary payload (for RPC)
    RawPayload(bytes::Bytes),
}

// ============================================================================
// Media Types
// ============================================================================

/// Supported media types for content negotiation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum MediaType {
    /// application/json
    #[default]
    ApplicationJson,
    /// application/geo+json
    GeoJson,
    /// text/csv
    TextCsv,
    /// text/plain
    TextPlain,
    /// text/xml
    TextXml,
    /// application/openapi+json
    OpenApi,
    /// application/x-www-form-urlencoded
    UrlEncoded,
    /// application/octet-stream
    OctetStream,
    /// */*
    Any,
    /// Custom media type
    Other(String),
    /// Singular JSON object (vnd.pgrst.object)
    SingularJson {
        /// `;nulls=null`: an empty result is `null` rather than a 406.
        nullable: bool,
        /// `;nulls=stripped`: omit keys whose value is null.
        strip_nulls: bool,
    },
    /// Array JSON (vnd.pgrst.array)
    ArrayJson {
        /// `;nulls=stripped`: omit keys whose value is null.
        strip_nulls: bool,
    },
    /// EXPLAIN plan output
    Plan {
        /// The media type the plan is *for*: the plan describes the query that
        /// would have produced this.
        base: Box<MediaType>,
        /// How the plan itself is rendered.
        format: PlanFormat,
        /// `EXPLAIN` options the request asked for.
        options: Vec<PlanOption>,
    },
}

impl MediaType {
    /// The `Content-Type` this is answered with.
    pub fn content_type(&self) -> &str {
        match self {
            Self::ApplicationJson => "application/json",
            Self::GeoJson => "application/geo+json",
            Self::TextCsv => "text/csv",
            Self::TextPlain => "text/plain",
            Self::TextXml => "text/xml",
            Self::OpenApi => "application/openapi+json",
            Self::UrlEncoded => "application/x-www-form-urlencoded",
            Self::OctetStream => "application/octet-stream",
            Self::Any => "*/*",
            Self::Other(s) => s,
            Self::SingularJson { .. } => "application/vnd.pgrst.object+json",
            Self::ArrayJson { .. } => "application/vnd.pgrst.array+json",
            Self::Plan { .. } => "application/vnd.pgrst.plan+json",
        }
    }

    /// The type's full name, parameters included.
    ///
    /// [`Self::content_type`] answers what a response body *is*, which never
    /// needs the parameters. Naming a type back to a client does: a plan is
    /// not the same request as a plan for CSV with `analyze` on, and a client
    /// told only `application/vnd.pgrst.plan+json` has not been told which of
    /// its requests was refused.
    pub fn to_mime(&self) -> String {
        match self {
            Self::SingularJson {
                strip_nulls: true, ..
            } => "application/vnd.pgrst.object+json;nulls=stripped".to_string(),
            Self::ArrayJson { strip_nulls: true } => {
                "application/vnd.pgrst.array+json;nulls=stripped".to_string()
            }
            // Without the parameter there is nothing to distinguish it from
            // plain JSON, which is the name it is known by.
            Self::ArrayJson { strip_nulls: false } => "application/json".to_string(),
            Self::Plan {
                base,
                format,
                options,
            } => {
                let mut mime = format!(
                    "application/vnd.pgrst.plan+{}; for=\"{}\"",
                    match format {
                        PlanFormat::Json => "json",
                        PlanFormat::Text => "text",
                    },
                    base.to_mime()
                );
                if !options.is_empty() {
                    let named: Vec<&str> = options
                        .iter()
                        .map(|option| match option {
                            PlanOption::Analyze => "analyze",
                            PlanOption::Verbose => "verbose",
                            PlanOption::Settings => "settings",
                            PlanOption::Buffers => "buffers",
                            PlanOption::Wal => "wal",
                        })
                        .collect();
                    mime.push_str("; options=");
                    mime.push_str(&named.join("|"));
                }
                mime
            }
            other => other.content_type().to_string(),
        }
    }
}

/// EXPLAIN plan format.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlanFormat {
    /// `EXPLAIN (FORMAT JSON)`.
    Json,
    /// `EXPLAIN (FORMAT TEXT)`, PostgreSQL's default rendering.
    Text,
}

/// EXPLAIN plan options.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlanOption {
    /// Run the query and report actual timings, not just the plan.
    Analyze,
    /// Include additional per-node detail.
    Verbose,
    /// Report planner settings that differ from their defaults.
    Settings,
    /// Report buffer usage. Requires `Analyze`.
    Buffers,
    /// Report WAL usage. Requires `Analyze`.
    Wal,
}

// ============================================================================
// Preferences
// ============================================================================

/// Resolution strategy for upsert conflicts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PreferResolution {
    /// `resolution=merge-duplicates`: a conflicting row is updated.
    MergeDuplicates,
    /// `resolution=ignore-duplicates`: a conflicting row is left alone.
    IgnoreDuplicates,
}

/// What to return from mutations.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PreferRepresentation {
    /// Return full response body
    Full,
    /// Return headers only
    HeadersOnly,
    /// Return nothing
    #[default]
    None,
}

/// How to count rows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PreferCount {
    /// Exact count (may be slow)
    Exact,
    /// Use query planner estimate
    Planned,
    /// Use statistics estimate
    Estimated,
}

/// Transaction handling.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PreferTransaction {
    /// `tx=commit`, the default.
    #[default]
    Commit,
    /// `tx=rollback`: the work is done, reported, and thrown away. Useful for
    /// seeing what a mutation would do without doing it.
    Rollback,
}

/// How to handle missing values.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PreferMissing {
    /// Use column defaults
    #[default]
    ApplyDefaults,
    /// Use NULL
    ApplyNulls,
}

/// Strictness of request handling.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PreferHandling {
    /// Strict - fail on unknown parameters
    Strict,
    /// Lenient - ignore unknown parameters.
    ///
    /// The default, as in PostgREST: a `Prefer` the server does not recognise
    /// is a preference, and RFC 7240 says a preference may be ignored.
    #[default]
    Lenient,
}

/// Parsed Prefer headers.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Preferences {
    /// How an upsert resolves a conflicting row.
    pub resolution: Option<PreferResolution>,
    /// Whether a mutation returns the rows it touched.
    pub representation: PreferRepresentation,
    /// Whether, and how exactly, to count the full result set.
    pub count: Option<PreferCount>,
    /// Whether the transaction commits.
    pub transaction: PreferTransaction,
    /// What an omitted column means on insert: its default, or null.
    pub missing: PreferMissing,
    /// Whether an unrecognised preference is an error.
    pub handling: PreferHandling,
    /// `timezone=`: the session time zone for this request.
    pub timezone: Option<String>,
    /// `max-affected=`: refuse the write if it would touch more rows.
    pub max_affected: Option<i64>,
    /// Preferences that were sent and not understood. Reported under
    /// `handling=strict`, ignored otherwise.
    pub invalid: Vec<String>,
    /// Every preference the server understood, in the order it was sent.
    ///
    /// Reported back as `Preference-Applied`. Kept as written rather than
    /// rebuilt from the fields above, because a field cannot say whether its
    /// value was asked for or is merely its default -- and `handling=lenient`
    /// is worth echoing exactly when it was asked for.
    #[serde(default)]
    pub applied: Vec<String>,
}

// ============================================================================
// Query Parameters
// ============================================================================

/// Path into an embedded resource.
pub type EmbedPath = Vec<FieldName>;

/// Parsed query parameters.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct QueryParams {
    /// Canonical query string (sorted)
    pub canonical: String,
    /// RPC parameters
    pub params: Vec<(String, String)>,
    /// Range per embedded resource
    pub ranges: HashMap<String, Range>,
    /// Order by per embedded resource
    pub order: Vec<(EmbedPath, Vec<OrderTerm>)>,
    /// Logic trees per embedded resource
    pub logic: Vec<(EmbedPath, LogicTree)>,
    /// Columns to include (for CSV/upsert)
    pub columns: Option<HashSet<FieldName>>,
    /// Select items (parsed from &select)
    pub select: Vec<SelectItem>,
    /// Filters
    pub filters: Vec<(EmbedPath, Filter)>,
    /// Root-level filters
    pub filters_root: Vec<Filter>,
    /// Fields being filtered (for optimization)
    pub filter_fields: HashSet<FieldName>,
    /// Conflict columns for upsert
    pub on_conflict: Option<Vec<FieldName>>,
}

// ============================================================================
// Main ApiRequest
// ============================================================================

/// A fully parsed API request ready for planning.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ApiRequest {
    /// The action to perform
    pub action: Action,
    /// Target schema
    pub schema: Schema,
    /// Request body
    pub payload: Option<Payload>,
    /// Parsed query parameters
    pub query_params: QueryParams,
    /// Accepted response formats
    pub accept_media_types: Vec<MediaType>,
    /// Request body format
    pub content_media_type: MediaType,
    /// Prefer headers
    pub preferences: Preferences,
    /// Explicitly requested columns
    pub columns: HashSet<FieldName>,
    /// Top-level pagination range
    pub top_level_range: Range,
    /// Ranges for embedded resources
    pub range_map: HashMap<String, Range>,
    /// Server-configured ceiling on returned rows (`PGRST_MAX_ROWS`).
    ///
    /// Applied on top of whatever the request asked for, so a request with no
    /// `limit` cannot pull an entire table into memory.
    pub max_rows: Option<i64>,
    /// Whether schema was negotiated from Accept-Profile header
    pub negotiated_by_profile: bool,
    /// Raw HTTP method
    pub method: String,
    /// Raw path
    pub path: String,
    /// Request headers (for GUC passthrough)
    pub headers: IndexMap<String, String>,
    /// Request cookies
    pub cookies: IndexMap<String, String>,
}

impl Default for ApiRequest {
    fn default() -> Self {
        Self {
            action: Action::SchemaInfo,
            schema: String::new(),
            payload: None,
            query_params: QueryParams::default(),
            accept_media_types: vec![MediaType::ApplicationJson],
            content_media_type: MediaType::ApplicationJson,
            preferences: Preferences::default(),
            columns: HashSet::new(),
            top_level_range: Range::default(),
            range_map: HashMap::new(),
            max_rows: None,
            negotiated_by_profile: false,
            method: String::new(),
            path: String::new(),
            headers: IndexMap::new(),
            cookies: IndexMap::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_qualified_identifier() {
        let qi = QualifiedIdentifier::new("public", "users");
        assert_eq!(qi.to_string(), "public.users");

        let unqual = QualifiedIdentifier::unqualified("users");
        assert_eq!(unqual.to_string(), "users");
    }

    #[test]
    fn test_simple_operator_sql() {
        assert_eq!(SimpleOperator::NotEqual.to_sql(), "<>");
        assert_eq!(SimpleOperator::Contains.to_sql(), "@>");
        assert_eq!(SimpleOperator::Overlap.to_sql(), "&&");
    }

    #[test]
    fn test_quant_operator_sql() {
        assert_eq!(QuantOperator::Equal.to_sql(), "=");
        assert_eq!(QuantOperator::GreaterThan.to_sql(), ">");
        assert_eq!(QuantOperator::Like.to_sql(), "LIKE");
    }

    #[test]
    fn test_range_from_bounds() {
        let range = Range::from_bounds(0, Some(9));
        assert_eq!(range.offset, 0);
        assert_eq!(range.limit, Some(10));

        let range = Range::from_bounds(10, Some(19));
        assert_eq!(range.offset, 10);
        assert_eq!(range.limit, Some(10));
    }
}
