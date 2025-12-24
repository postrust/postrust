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
    pub schema: String,
    pub name: String,
}

impl QualifiedIdentifier {
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

pub type FieldName = String;
pub type Schema = String;
pub type Alias = String;
pub type Cast = String;
pub type Hint = String;
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
    pub name: FieldName,
    pub json_path: JsonPath,
}

impl Field {
    pub fn simple(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            json_path: Vec::new(),
        }
    }

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
    InvRead { headers_only: bool },
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
        qi: QualifiedIdentifier,
        headers_only: bool,
    },
    /// INSERT/UPDATE/DELETE on a table
    RelationMut {
        qi: QualifiedIdentifier,
        mutation: Mutation,
    },
    /// Call a stored function
    Routine {
        qi: QualifiedIdentifier,
        invoke_method: InvokeMethod,
    },
    /// Read schema metadata
    SchemaRead { schema: Schema, headers_only: bool },
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
        qi: QualifiedIdentifier,
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
    Null,
    True,
    False,
    Unknown,
}

impl IsValue {
    pub fn to_sql(&self) -> &'static str {
        match self {
            Self::Null => "NULL",
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
    Simple { op: SimpleOperator, value: String },
    /// Quantified operation: `col.eq.value` or `col.eq(any).{arr}`
    Quant {
        op: QuantOperator,
        quantifier: Option<OpQuantifier>,
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
        op: FtsOperator,
        language: Option<Language>,
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
    pub fn new(operation: Operation) -> Self {
        Self {
            negated: false,
            operation,
        }
    }

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
    pub field: Field,
    pub op_expr: OpExpr,
}

impl Filter {
    pub fn new(field: Field, op_expr: OpExpr) -> Self {
        Self { field, op_expr }
    }
}

/// Boolean logic operator.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LogicOperator {
    And,
    Or,
}

/// A tree of boolean logic combining filters.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum LogicTree {
    /// A boolean expression combining children
    Expr {
        negated: bool,
        op: LogicOperator,
        children: Vec<LogicTree>,
    },
    /// A leaf filter
    Stmt(Filter),
}

impl LogicTree {
    pub fn and(children: Vec<LogicTree>) -> Self {
        Self::Expr {
            negated: false,
            op: LogicOperator::And,
            children,
        }
    }

    pub fn or(children: Vec<LogicTree>) -> Self {
        Self::Expr {
            negated: false,
            op: LogicOperator::Or,
            children,
        }
    }

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
    Sum,
    Avg,
    Max,
    Min,
    Count,
}

impl AggregateFunction {
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
        field: Field,
        aggregate: Option<AggregateFunction>,
        aggregate_cast: Option<Cast>,
        cast: Option<Cast>,
        alias: Option<Alias>,
    },
    /// Embed a related resource
    Relation {
        relation: FieldName,
        alias: Option<Alias>,
        hint: Option<Hint>,
        join_type: Option<JoinType>,
    },
    /// Spread a related resource's columns (horizontal embedding)
    SpreadRelation {
        relation: FieldName,
        hint: Option<Hint>,
        join_type: Option<JoinType>,
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
        }
    }
}

// ============================================================================
// Ordering
// ============================================================================

/// Sort direction.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum OrderDirection {
    #[default]
    Asc,
    Desc,
}

impl OrderDirection {
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
    First,
    Last,
}

impl OrderNulls {
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
        field: Field,
        direction: Option<OrderDirection>,
        nulls: Option<OrderNulls>,
    },
    /// Order by a field from an embedded relation
    Relation {
        relation: FieldName,
        field: Field,
        direction: Option<OrderDirection>,
        nulls: Option<OrderNulls>,
    },
}

impl OrderTerm {
    pub fn field(name: impl Into<String>) -> Self {
        Self::Field {
            field: Field::simple(name),
            direction: None,
            nulls: None,
        }
    }

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
    pub offset: i64,
    pub limit: Option<i64>,
}

impl Range {
    pub fn new(offset: i64, limit: Option<i64>) -> Self {
        Self { offset, limit }
    }

    /// Create a range from HTTP Range header format (0-9 means rows 0-9 inclusive).
    pub fn from_bounds(start: i64, end: Option<i64>) -> Self {
        Self {
            offset: start,
            limit: end.map(|e| e - start + 1),
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
        raw: bytes::Bytes,
        keys: HashSet<String>,
    },
    /// URL-encoded form data
    ProcessedUrlEncoded {
        data: Vec<(String, String)>,
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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MediaType {
    /// application/json
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
    SingularJson { nullable: bool },
    /// Array JSON with nulls stripped
    ArrayJsonStrip,
    /// EXPLAIN plan output
    Plan {
        base: Box<MediaType>,
        format: PlanFormat,
        options: Vec<PlanOption>,
    },
}

impl Default for MediaType {
    fn default() -> Self {
        Self::ApplicationJson
    }
}

impl MediaType {
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
            Self::ArrayJsonStrip => "application/vnd.pgrst.array+json",
            Self::Plan { .. } => "application/vnd.pgrst.plan+json",
        }
    }
}

/// EXPLAIN plan format.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlanFormat {
    Json,
    Text,
}

/// EXPLAIN plan options.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlanOption {
    Analyze,
    Verbose,
    Settings,
    Buffers,
    Wal,
}

// ============================================================================
// Preferences
// ============================================================================

/// Resolution strategy for upsert conflicts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PreferResolution {
    MergeDuplicates,
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
    #[default]
    Commit,
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
    #[default]
    Strict,
    /// Lenient - ignore unknown parameters
    Lenient,
}

/// Parsed Prefer headers.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Preferences {
    pub resolution: Option<PreferResolution>,
    pub representation: PreferRepresentation,
    pub count: Option<PreferCount>,
    pub transaction: PreferTransaction,
    pub missing: PreferMissing,
    pub handling: PreferHandling,
    pub timezone: Option<String>,
    pub max_affected: Option<i64>,
    pub invalid: Vec<String>,
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
