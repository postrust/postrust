//! Error types for Postrust.
//!
//! Provides comprehensive error handling with HTTP status code mapping.

use http::StatusCode;
use thiserror::Error;

/// Result type for Postrust operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Main error type for Postrust.
#[derive(Error, Debug)]
pub enum Error {
    // ========================================================================
    // Request Parsing Errors (4xx)
    // ========================================================================
    #[error("Invalid path: {0}")]
    InvalidPath(String),

    /// An aggregate was requested while `db-aggregates-enabled` is off.
    ///
    /// Off is the default, matching PostgREST: an unrestricted aggregate over
    /// a large table is an easy way to make a server do far more work than the
    /// request looks like it asks for.
    #[error("Use of aggregate functions is not allowed")]
    AggregatesNotAllowed,

    #[error("Invalid query parameter: {0}")]
    InvalidQueryParam(String),

    #[error("Invalid header: {0}")]
    InvalidHeader(&'static str),

    #[error("{0}")]
    InvalidBody(String),

    #[error("Unsupported HTTP method: {0}")]
    UnsupportedMethod(String),

    /// A method a function cannot be called with.
    ///
    /// A function is invoked with GET, HEAD or POST. The others are refused
    /// with a message that says so rather than one about the method in
    /// general, since the method is fine elsewhere in the API.
    #[error("Cannot use the {0} method on RPC")]
    InvalidRpcMethod(String),

    /// A profile header named a schema the server does not expose.
    ///
    /// Carries the schemas that *are* exposed, because the client needs them
    /// to correct the request and they are part of the API's own contract.
    #[error("Invalid schema: {requested}")]
    UnacceptableSchema {
        requested: String,
        exposed: Vec<String>,
    },

    #[error("Could not find the '{column}' column of '{relation}' in the schema cache")]
    UnknownColumn {
        /// The column the request named.
        column: String,
        /// The relation it was looked for on.
        relation: String,
    },

    #[error("Requested range not satisfiable")]
    InvalidRange(String),

    #[error("None of these media types are available: {0}")]
    InvalidMediaType(String),

    #[error("Missing required parameter: {0}")]
    MissingParameter(String),

    #[error("Ambiguous request: {0}")]
    AmbiguousRequest(String),

    /// Several signatures of a function fit the arguments equally well.
    #[error("Could not choose the best candidate function between: {}", .candidates.join(", "))]
    AmbiguousFunction { candidates: Vec<String> },

    /// Several relationships connect the two resources and none was named.
    #[error("Could not embed because more than one relationship was found for '{origin}' and '{target}'")]
    AmbiguousRelationship {
        origin: String,
        target: String,
        /// Each candidate as `(cardinality, description)`.
        candidates: Vec<(String, String)>,
    },

    // ========================================================================
    // Authentication/Authorization Errors (401/403)
    // ========================================================================
    /// The token could not be decoded or its signature did not verify.
    ///
    /// Distinct from a claims error: nothing about the token was readable, so
    /// there is nothing to say about what it claimed.
    #[error("{0}")]
    InvalidJwt(String),

    /// The token decoded, but a claim was missing, malformed or unsatisfied.
    #[error("{0}")]
    JwtClaim(String),

    #[error("Anonymous access is disabled")]
    MissingAuth,

    #[error("Insufficient permissions: {0}")]
    InsufficientPermissions(String),

    // ========================================================================
    // Resource Errors (404)
    // ========================================================================
    #[error("Resource not found: {0}")]
    NotFound(String),

    #[error("Table not found: {name}")]
    TableNotFound {
        /// The name the request asked for, schema-qualified.
        name: String,
        /// A table of a very similar name, where the schema has one.
        suggestion: Option<String>,
    },

    /// No function of that name accepts the arguments supplied.
    ///
    /// Carries what was looked for so the client can see why nothing matched:
    /// the name, the argument names it was called with, and the signature of
    /// an overload that does exist, where one does.
    #[error("Could not find the function {name}")]
    FunctionNotFound {
        /// The function's qualified name, without arguments.
        name: String,
        /// The argument names it was called with.
        params: Vec<String>,
        /// A signature that does exist, where one does.
        candidate: Option<String>,
        /// The type of a single unnamed parameter that would also have
        /// matched, spelled as the message spells it.
        ///
        /// Over `POST` the whole body can be one argument, and which type it
        /// could be is decided by the body's media type: `json/jsonb` for a
        /// JSON body, `text`, `xml` or `bytea` for the others. Naming it says
        /// what else was looked for besides the arguments themselves.
        single_param: Option<String>,
    },

    /// A `RAISE sqlstate 'PGRST'` the schema wrote wrongly.
    ///
    /// The message and the detail are both JSON documents with required keys,
    /// and a function that gets one wrong has not asked for a response -- it
    /// has a bug. Reporting it as the 500 it is says so; applying half of what
    /// it asked for would not.
    #[error("Could not parse JSON in the \"RAISE SQLSTATE 'PGRST'\" error")]
    RaiseNotUnderstood(RaiseFault),

    #[error("Column not found: {0}")]
    ColumnNotFound(String),

    #[error("Could not find a relationship between '{origin}' and '{target}' in the schema cache")]
    RelationshipNotFound {
        /// The table the embedding started from.
        origin: String,
        /// The resource the request asked to embed.
        target: String,
        /// The `!hint` that was meant to identify it, if any.
        hint: Option<String>,
        /// The schema that was searched.
        schema: String,
        /// A relationship of a very similar name, where there is one.
        suggestion: Option<String>,
    },

    /// A filter or order names something the request never embedded.
    ///
    /// `?non_existent.name=like.*x*` is not a column with a dot in it, and
    /// answering it as though the prefix were meaningless would silently
    /// return the unfiltered table.
    #[error("'{0}' is not an embedded resource in this request")]
    NotAnEmbeddedResource(String),

    /// Ordering by a column of a resource that yields many rows per parent.
    ///
    /// There is no single value to order on: the parent has a list.
    #[error("A related order on '{relation}' is not possible")]
    RelatedOrderNotPossible {
        /// The table the request started from.
        origin: String,
        /// The embedded resource the order named.
        relation: String,
    },

    /// A single object was asked for and the result is not one.
    ///
    /// `Accept: application/vnd.pgrst.object+json` says the client will accept
    /// exactly one row; anything else is a negotiation failure rather than a
    /// missing resource, which is why it is 406 and not 404.
    #[error("Cannot coerce the result to a single JSON object")]
    NotSingular {
        /// How many rows the query actually returned.
        rows: usize,
    },

    /// A path with more segments than any resource has.
    #[error("Invalid path specified in request URL")]
    InvalidResourcePath,

    /// A `PUT` whose filters do not name exactly one row by its key.
    ///
    /// `PUT` writes the row the URL names, so the URL has to name exactly one:
    /// every primary key column, with `eq`, and nothing else.
    #[error("Filters must include all and only primary key columns with 'eq' operators")]
    InvalidPutFilters,

    /// A `PUT` carrying a page.
    #[error("limit/offset querystring parameters are not allowed for PUT")]
    PutLimitNotAllowed,

    /// A `PUT` whose body names a different row from its URL.
    #[error("Payload values do not match URL in primary key column(s)")]
    PutMatchingPk,

    /// A function set `response.headers` to something that is not headers.
    #[error(
        "response.headers guc must be a JSON array composed of objects with a single key and a \
         string value"
    )]
    InvalidGucHeaders,

    /// A function set `response.status` to something that is not a status.
    #[error("response.status guc must be a valid status code")]
    InvalidGucStatus,

    /// A mutation touched more rows than `Prefer: max-affected` allowed.
    ///
    /// The preference is a guard against a filter that turned out to match
    /// more than the client meant; nothing is committed when it trips.
    #[error("Query result exceeds max-affected preference constraint")]
    MaxAffectedExceeded(i64),

    /// Preferences the server does not know, sent with `handling=strict`.
    ///
    /// Strict handling is a request to be told rather than have them ignored.
    #[error("Invalid preferences given with handling=strict")]
    InvalidPreferences(Vec<String>),

    // ========================================================================
    // Schema Cache Errors
    // ========================================================================
    #[error("Schema cache not loaded")]
    SchemaCacheNotLoaded,

    #[error("Schema cache load failed: {0}")]
    SchemaCacheLoadFailed(String),

    // ========================================================================
    // Database Errors (500/4xx depending on type)
    // ========================================================================
    #[error("Database error: {0}")]
    Database(#[from] DatabaseError),

    #[error("Connection pool error: {0}")]
    ConnectionPool(String),

    // ========================================================================
    // Internal Errors (500)
    // ========================================================================
    #[error("Internal error: {0}")]
    Internal(String),

    #[error("Configuration error: {0}")]
    Config(String),

    // ========================================================================
    // Plan Errors
    // ========================================================================
    #[error("Invalid plan: {0}")]
    InvalidPlan(String),

    #[error("Embedding error: {0}")]
    EmbeddingError(String),
}

impl Error {
    /// Get the HTTP status code for this error.
    pub fn status_code(&self) -> StatusCode {
        match self {
            // 400 Bad Request
            Self::InvalidPath(_)
            | Self::InvalidQueryParam(_)
            | Self::InvalidHeader(_)
            | Self::InvalidBody(_)
            | Self::MissingParameter(_)
            | Self::AmbiguousRequest(_)
            | Self::UnknownColumn { .. }
            | Self::NotAnEmbeddedResource(_)
            | Self::RelatedOrderNotPossible { .. }
            | Self::InvalidPreferences(_)
            | Self::PutLimitNotAllowed
            | Self::PutMatchingPk
            | Self::MaxAffectedExceeded(_)
            | Self::InvalidPlan(_)
            | Self::EmbeddingError(_)
            | Self::AggregatesNotAllowed => StatusCode::BAD_REQUEST,

            // The request names something real, but not which one of it.
            Self::AmbiguousFunction { .. } | Self::AmbiguousRelationship { .. } => {
                StatusCode::MULTIPLE_CHOICES
            }

            // 401 Unauthorized
            Self::InvalidJwt(_) | Self::JwtClaim(_) | Self::MissingAuth => StatusCode::UNAUTHORIZED,

            // 403 Forbidden
            Self::InsufficientPermissions(_) => StatusCode::FORBIDDEN,

            // 404 Not Found
            Self::NotFound(_) | Self::TableNotFound { .. } | Self::FunctionNotFound { .. } => {
                StatusCode::NOT_FOUND
            }

            // A relationship or a column the schema does not have is a fault
            // in the request, not a missing resource: the resource addressed
            // by the URL exists, and `?select=` asked it for something it
            // cannot give. The table it names is what a 404 would be about.
            Self::ColumnNotFound(_) | Self::RelationshipNotFound { .. } => StatusCode::BAD_REQUEST,

            // 405 Method Not Allowed
            Self::UnsupportedMethod(_) | Self::InvalidRpcMethod(_) | Self::InvalidPutFilters => {
                StatusCode::METHOD_NOT_ALLOWED
            }

            // 406 Not Acceptable
            Self::UnacceptableSchema { .. }
            | Self::NotSingular { .. }
            | Self::InvalidMediaType(_) => StatusCode::NOT_ACCEPTABLE,

            Self::InvalidResourcePath => StatusCode::NOT_FOUND,

            // A range the server cannot satisfy, which is what 416 is for --
            // a negative limit asks for a window that does not exist rather
            // than for one the server merely declines.
            Self::InvalidRange(_) => StatusCode::RANGE_NOT_SATISFIABLE,

            // 500 Internal Server Error
            Self::RaiseNotUnderstood(_)
            | Self::InvalidGucHeaders
            | Self::InvalidGucStatus
            | Self::SchemaCacheNotLoaded
            | Self::SchemaCacheLoadFailed(_)
            | Self::ConnectionPool(_)
            | Self::Internal(_)
            | Self::Config(_) => StatusCode::INTERNAL_SERVER_ERROR,

            // Database errors map based on type
            Self::Database(db_err) => db_err.status_code(),
        }
    }

    /// Get the error code for API responses.
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidPath(_) => "PGRST100",
            Self::AggregatesNotAllowed => "PGRST123",
            // PGRST100 is the parse failure in PostgREST's taxonomy, covering
            // both the path and the query string; PGRST101 is a function
            // invoked with a method its volatility does not allow.
            Self::InvalidQueryParam(_) => "PGRST100",
            // Not one of PostgREST's own: a malformed header is a request
            // that did not parse, which is what PGRST100 covers.
            Self::InvalidHeader(_) => "PGRST100",
            Self::InvalidBody(_) => "PGRST102",
            Self::UnsupportedMethod(_) => "PGRST117",
            Self::InvalidRpcMethod(_) => "PGRST101",
            Self::UnacceptableSchema { .. } => "PGRST106",
            Self::UnknownColumn { .. } => "PGRST204",
            Self::InvalidRange(_) => "PGRST103",
            Self::InvalidMediaType(_) => "PGRST107",
            Self::MissingParameter(_) => "PGRST109",
            Self::AmbiguousRequest(_) => "PGRST110",
            Self::AmbiguousFunction { .. } => "PGRST203",
            Self::AmbiguousRelationship { .. } => "PGRST201",

            Self::InvalidJwt(_) => "PGRST301",
            Self::JwtClaim(_) => "PGRST303",
            Self::MissingAuth => "PGRST302",
            Self::InsufficientPermissions(_) => "PGRST203",

            Self::NotFound(_) => "PGRST205",
            Self::TableNotFound { .. } => "PGRST205",
            Self::FunctionNotFound { .. } => "PGRST202",
            Self::ColumnNotFound(_) => "PGRST204",
            Self::RelationshipNotFound { .. } => "PGRST200",
            Self::NotAnEmbeddedResource(_) => "PGRST108",
            Self::NotSingular { .. } => "PGRST116",
            Self::InvalidResourcePath => "PGRST125",
            Self::InvalidPutFilters => "PGRST105",
            Self::PutLimitNotAllowed => "PGRST114",
            Self::PutMatchingPk => "PGRST115",
            Self::MaxAffectedExceeded(_) => "PGRST124",
            Self::RaiseNotUnderstood(_) => "PGRST121",
            Self::InvalidGucHeaders => "PGRST111",
            Self::InvalidGucStatus => "PGRST112",
            Self::RelatedOrderNotPossible { .. } => "PGRST118",
            Self::InvalidPreferences(_) => "PGRST122",

            Self::SchemaCacheNotLoaded => "PGRST400",
            Self::SchemaCacheLoadFailed(_) => "PGRST401",

            Self::Database(e) => e.code(),
            Self::ConnectionPool(_) => "PGRST500",

            Self::Internal(_) => "PGRST900",
            Self::Config(_) => "PGRST901",

            Self::InvalidPlan(_) => "PGRST600",
            Self::EmbeddingError(_) => "PGRST601",
        }
    }

    /// Convert to JSON error response.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "code": self.code(),
            "message": self.to_string(),
            "details": self.details(),
            "hint": self.hint(),
        })
    }

    /// Get additional details for the error.
    pub fn details(&self) -> Option<serde_json::Value> {
        match self {
            Self::Database(db_err) => db_err.details.clone().map(serde_json::Value::String),
            // A structured list rather than a sentence: each candidate is
            // named so the client can pick one and say which.
            Self::AmbiguousRelationship {
                origin,
                target,
                candidates,
            } => Some(serde_json::Value::Array(
                candidates
                    .iter()
                    .map(|(cardinality, description)| {
                        serde_json::json!({
                            "cardinality": cardinality,
                            "embedding": format!("{} with {}", origin, target),
                            "relationship": description,
                        })
                    })
                    .collect(),
            )),
            Self::RelationshipNotFound {
                origin,
                target,
                hint,
                schema,
                ..
            } => Some(serde_json::Value::String(format!(
                "Searched for a foreign key relationship between '{}' and '{}'{} in the schema \
                 '{}', but no matches were found.",
                origin,
                target,
                match hint {
                    Some(hint) => format!(" using the hint '{}'", hint),
                    None => String::new(),
                },
                schema
            ))),
            Self::RelatedOrderNotPossible { origin, relation } => {
                Some(serde_json::Value::String(format!(
                    "'{}' and '{}' do not form a many-to-one or one-to-one relationship",
                    origin, relation
                )))
            }
            Self::InvalidRange(reason) => Some(serde_json::Value::String(reason.clone())),
            Self::MaxAffectedExceeded(affected) => Some(serde_json::Value::String(format!(
                "The query affects {} rows",
                affected
            ))),
            Self::NotSingular { rows } => Some(serde_json::Value::String(format!(
                "The result contains {} rows",
                rows
            ))),
            Self::InvalidPreferences(invalid) => Some(serde_json::Value::String(format!(
                "Invalid preferences: {}",
                invalid.join(", ")
            ))),
            Self::RaiseNotUnderstood(fault) => Some(serde_json::Value::String(fault.details())),
            Self::FunctionNotFound {
                name,
                params,
                single_param,
                ..
            } => Some(serde_json::Value::String(format!(
                "Searched for the function {} {}, but no matches were found in the schema cache.",
                name,
                match single_param.as_deref() {
                    // A body that is one value carries no argument names, so
                    // the only thing that could have matched is the parameter
                    // that takes it.
                    Some(other) if other != JSON_PARAM =>
                        format!("with a single unnamed {} parameter", other),
                    Some(_) => format!(
                        "{} or with a single unnamed {} parameter",
                        describe_params(params),
                        JSON_PARAM
                    ),
                    None => describe_params(params),
                }
            ))),
            _ => None,
        }
    }

    /// Get a hint for resolving the error.
    pub fn hint(&self) -> Option<String> {
        match self {
            Self::NotAnEmbeddedResource(name) => Some(format!(
                "Verify that '{}' is included in the 'select' query parameter.",
                name
            )),
            Self::TableNotFound { suggestion, .. } => suggestion
                .as_ref()
                .map(|name| format!("Perhaps you meant the table '{}'", name)),
            Self::RelationshipNotFound {
                target, suggestion, ..
            } => suggestion
                .as_ref()
                .map(|name| format!("Perhaps you meant '{}' instead of '{}'.", name, target)),
            Self::AmbiguousFunction { .. } => Some(
                "Try renaming the parameters or the function itself in the database so function \
                 overloading can be resolved"
                    .into(),
            ),
            Self::RaiseNotUnderstood(fault) => Some(fault.hint().into()),
            // A body that is one value names no arguments, so there is no
            // near miss to point at: any function of that name would have
            // done, and none of them takes this body.
            Self::FunctionNotFound {
                candidate,
                single_param,
                ..
            } if !names_only_one_param(single_param.as_deref()) => candidate
                .as_ref()
                .map(|c| format!("Perhaps you meant to call the function {}", c)),
            Self::FunctionNotFound { .. } => None,
            // The schemas a server exposes are part of its contract, not an
            // internal detail, so naming them is how the client is told what
            // to ask for instead.
            Self::UnacceptableSchema { exposed, .. } => Some(format!(
                "Only the following schemas are exposed: {}",
                exposed.join(", ")
            )),
            Self::Database(db_err) => db_err.hint.clone(),
            _ => None,
        }
    }
}

/// How a function's argument list reads in an error message.
///
/// PostgREST words the no-argument case differently rather than printing an
/// empty list, and the messages are matched verbatim by clients.
/// How the message spells the type of a single unnamed JSON parameter.
///
/// It is the one media type that can also carry named arguments, so it reads
/// as an alternative rather than as the only thing looked for.
pub const JSON_PARAM: &str = "json/jsonb";

/// Whether the request could only ever have matched one unnamed parameter.
///
/// True for a body that is one value -- text, xml or bytes -- which carries no
/// argument names at all.
pub fn names_only_one_param(single_param: Option<&str>) -> bool {
    matches!(single_param, Some(other) if other != JSON_PARAM)
}

pub fn function_signature(name: &str, params: &[String]) -> String {
    match params.is_empty() {
        true => format!("{} without parameters", name),
        false => format!("{}({})", name, params.join(", ")),
    }
}

fn describe_params(params: &[String]) -> String {
    match params.len() {
        0 => "without parameters".to_string(),
        1 => format!("with parameter {}", params[0]),
        _ => format!("with parameters {}", params.join(", ")),
    }
}

/// Database-specific error type.
#[derive(Error, Debug)]
#[error("Database error [{code}]: {message}")]
pub struct DatabaseError {
    pub code: String,
    pub message: String,
    pub details: Option<String>,
    pub hint: Option<String>,
    pub constraint: Option<String>,
    pub table: Option<String>,
    pub column: Option<String>,
}

/// What was wrong with a `RAISE sqlstate 'PGRST'`.
#[derive(Clone, Debug)]
pub enum RaiseFault {
    /// The MESSAGE was not the JSON object it has to be.
    Message(String),
    /// The DETAIL was not the JSON object it has to be.
    Detail(String),
    /// There was no DETAIL, and the status has to come from somewhere.
    NoDetail,
}

impl RaiseFault {
    /// What the schema author has to look at.
    pub fn details(&self) -> String {
        match self {
            Self::Message(raw) => format!("Invalid JSON value for MESSAGE: '{}'", raw),
            Self::Detail(raw) => format!("Invalid JSON value for DETAIL: '{}'", raw),
            Self::NoDetail => "DETAIL is missing in the RAISE statement".to_string(),
        }
    }

    /// What the document should have contained.
    pub fn hint(&self) -> &'static str {
        match self {
            Self::Message(_) => {
                "MESSAGE must be a JSON object with obligatory keys: 'code', 'message' and \
                 optional keys: 'details', 'hint'."
            }
            _ => {
                "DETAIL must be a JSON object with obligatory keys: 'status', 'headers' and \
                 optional key: 'status_text'."
            }
        }
    }
}

/// A response a database function asked for outright.
#[derive(Clone, Debug)]
pub struct RaisedResponse {
    /// The HTTP status to answer with.
    pub status: u16,
    /// The reason phrase, where the schema named one of its own.
    pub status_text: Option<String>,
    /// Headers to add to the response.
    pub headers: Vec<(String, String)>,
    /// The response body, verbatim.
    pub body: serde_json::Value,
}

impl DatabaseError {
    /// The response a `RAISE sqlstate 'PGRST'` asked for.
    ///
    /// A function can take over the whole response by raising that SQLSTATE:
    /// the message is the body, and the detail carries the status, an optional
    /// status text and any headers. It is how a schema returns a 402 with its
    /// own payload without the API layer knowing anything about billing.
    ///
    /// `None` when this is not that error. `Err` when it is and the schema
    /// wrote it wrongly, which is a bug in the schema rather than a response
    /// to half-apply.
    pub fn raised_response(&self) -> Option<std::result::Result<RaisedResponse, RaiseFault>> {
        if self.code != "PGRST" {
            return None;
        }
        Some(self.parse_raise())
    }

    fn parse_raise(&self) -> std::result::Result<RaisedResponse, RaiseFault> {
        let object = |raw: &str| -> Option<serde_json::Map<String, serde_json::Value>> {
            match serde_json::from_str(raw) {
                Ok(serde_json::Value::Object(map)) => Some(map),
                _ => None,
            }
        };

        // The message is the body, and a body has to say what went wrong and
        // under what code -- the two keys a client reads first.
        let message =
            object(&self.message).ok_or_else(|| RaiseFault::Message(self.message.clone()))?;
        if !message.contains_key("code") || !message.contains_key("message") {
            return Err(RaiseFault::Message(self.message.clone()));
        }

        let raw_detail = self.details.as_deref().ok_or(RaiseFault::NoDetail)?;
        let detail =
            object(raw_detail).ok_or_else(|| RaiseFault::Detail(raw_detail.to_string()))?;
        let status = detail
            .get("status")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| RaiseFault::Detail(raw_detail.to_string()))?;
        let headers = detail
            .get("headers")
            .and_then(serde_json::Value::as_object)
            .ok_or_else(|| RaiseFault::Detail(raw_detail.to_string()))?;

        // Every key the error body has, present whether the schema wrote it or
        // not: a client reading `details` should not have to tell "no detail"
        // from "no such key".
        let key = |name: &str| {
            message
                .get(name)
                .cloned()
                .unwrap_or(serde_json::Value::Null)
        };

        Ok(RaisedResponse {
            status: status as u16,
            status_text: detail
                .get("status_text")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string),
            headers: headers
                .iter()
                .filter_map(|(name, value)| Some((name.clone(), value.as_str()?.to_string())))
                .collect(),
            body: serde_json::json!({
                "code": key("code"),
                "message": key("message"),
                "details": key("details"),
                "hint": key("hint"),
            }),
        })
    }

    /// The HTTP status a PostgreSQL error code maps to.
    ///
    /// PostgREST's table (`mapSQLtoHTTP` in `Error.hs`), which is part of its
    /// contract rather than an implementation detail: a client distinguishes a
    /// missing function from a bad argument by the status.
    ///
    /// The default is 400, not 500: the great majority of SQLSTATEs a request
    /// can provoke are provoked by the request. Reporting them as server
    /// errors tells the client to retry something that will never succeed.
    pub fn status_code(&self) -> StatusCode {
        let code = self.code.as_str();
        let message = self.message.as_str();

        match self.raised_response() {
            Some(Ok(raised)) => {
                if let Ok(status) = StatusCode::from_u16(raised.status) {
                    return status;
                }
            }
            // A raise the schema wrote wrongly is the schema's fault, and a
            // fault in the server's own data is a 500.
            Some(Err(_)) => return StatusCode::INTERNAL_SERVER_ERROR,
            None => {}
        }

        // `PT<nnn>` is a status code raised by the schema itself: a function
        // says `RAISE sqlstate 'PT402'` to answer 402, with the message as the
        // status text.
        if let Some(digits) = code.strip_prefix("PT") {
            if let Ok(status) = digits.parse::<u16>() {
                if let Ok(status) = StatusCode::from_u16(status) {
                    return status;
                }
            }
            return StatusCode::INTERNAL_SERVER_ERROR;
        }

        match code {
            "23503" | "23505" => StatusCode::CONFLICT,
            // A write attempted inside the read-only transaction a read
            // request runs in. The request was not wrong, the method was.
            "25006" => StatusCode::METHOD_NOT_ALLOWED,
            // Cardinality violation. `pg-safeupdate` raises it for an
            // unqualified UPDATE or DELETE, which is the client's doing;
            // anything else is a function or view misbehaving.
            "21000" => match message.ends_with("requires a WHERE clause") {
                true => StatusCode::BAD_REQUEST,
                false => StatusCode::INTERNAL_SERVER_ERROR,
            },
            // Invalid parameter value, which is also how PostgreSQL reports a
            // `SET ROLE` to a role that does not exist.
            "22023" => match message.starts_with("role") && message.ends_with("does not exist") {
                true => StatusCode::UNAUTHORIZED,
                false => StatusCode::BAD_REQUEST,
            },
            "53400" => StatusCode::INTERNAL_SERVER_ERROR,
            "57P01" => StatusCode::SERVICE_UNAVAILABLE,
            "P0001" => StatusCode::BAD_REQUEST,
            // `xmlagg` is missing only when the request asked for XML the
            // server cannot produce, which is a negotiation failure.
            "42883" => match message.starts_with("function xmlagg(") {
                true => StatusCode::NOT_ACCEPTABLE,
                false => StatusCode::NOT_FOUND,
            },
            "42P01" => StatusCode::NOT_FOUND,
            "42P17" => StatusCode::INTERNAL_SERVER_ERROR,
            // Without authentication the client may simply not have said who
            // it is; with it, it has and is still not permitted.
            "42501" => StatusCode::UNAUTHORIZED,
            _ => match code.as_bytes() {
                [b'0', b'8', ..] => StatusCode::SERVICE_UNAVAILABLE,
                [b'0', b'9', ..] => StatusCode::INTERNAL_SERVER_ERROR,
                [b'0', b'L', ..] | [b'0', b'P', ..] => StatusCode::FORBIDDEN,
                [b'2', b'5', ..] => StatusCode::INTERNAL_SERVER_ERROR,
                [b'2', b'8', ..] => StatusCode::FORBIDDEN,
                [b'2', b'D', ..] => StatusCode::INTERNAL_SERVER_ERROR,
                [b'3', b'8', ..] | [b'3', b'9', ..] | [b'3', b'B', ..] => {
                    StatusCode::INTERNAL_SERVER_ERROR
                }
                [b'4', b'0', ..] => StatusCode::INTERNAL_SERVER_ERROR,
                [b'5', b'3', ..] => StatusCode::SERVICE_UNAVAILABLE,
                [b'5', b'4', ..] | [b'5', b'5', ..] | [b'5', b'7', ..] | [b'5', b'8', ..] => {
                    StatusCode::INTERNAL_SERVER_ERROR
                }
                [b'F', b'0', ..] | [b'H', b'V', ..] => StatusCode::INTERNAL_SERVER_ERROR,
                [b'P', b'0', ..] => StatusCode::INTERNAL_SERVER_ERROR,
                [b'X', b'X', ..] => StatusCode::INTERNAL_SERVER_ERROR,
                _ => StatusCode::BAD_REQUEST,
            },
        }
    }

    /// Get error code for API response.
    pub fn code(&self) -> &'static str {
        match self.code.as_str() {
            c if c.starts_with("23") => "PGRST503", // Constraint violation
            c if c.starts_with("42") => "PGRST504", // SQL error
            c if c.starts_with("28") => "PGRST505", // Auth error
            _ => "PGRST500",                        // Generic database error
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_status_codes() {
        assert_eq!(
            Error::InvalidQueryParam("test".into()).status_code(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(Error::MissingAuth.status_code(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            Error::TableNotFound {
                name: "users".into(),
                suggestion: None
            }
            .status_code(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            Error::UnsupportedMethod("TRACE".into()).status_code(),
            StatusCode::METHOD_NOT_ALLOWED
        );
    }

    #[test]
    fn test_error_codes() {
        // A malformed query string is a parse failure, PGRST100 -- the same
        // code as a malformed path. PGRST101 means something else entirely.
        assert_eq!(Error::InvalidQueryParam("test".into()).code(), "PGRST100");
        assert_eq!(Error::MissingAuth.code(), "PGRST302");
        assert_eq!(
            Error::TableNotFound {
                name: "users".into(),
                suggestion: None
            }
            .code(),
            "PGRST205"
        );
    }

    #[test]
    fn test_database_error_status() {
        let constraint_error = DatabaseError {
            code: "23505".into(), // unique_violation
            message: "Duplicate key".into(),
            details: None,
            hint: None,
            constraint: Some("users_pkey".into()),
            table: Some("users".into()),
            column: None,
        };
        assert_eq!(constraint_error.status_code(), StatusCode::CONFLICT);
    }

    #[test]
    fn test_error_to_json() {
        let error = Error::InvalidQueryParam("bad filter".into());
        let json = error.to_json();
        assert_eq!(json["code"], "PGRST100");
        assert!(json["message"].as_str().unwrap().contains("bad filter"));
    }
}
