//! Response formatting for Postrust.
//!
//! Handles content negotiation and response formatting for JSON, CSV, and other formats.

mod headers;
pub use headers::parse_guc_headers;
mod json;

pub use headers::{build_response_headers, ContentRange};
pub use json::format_json_response;

use http::{HeaderMap, HeaderValue, StatusCode};
use postrust_core::{ApiRequest, MediaType};
use serde::Serialize;

/// A formatted HTTP response.
#[derive(Clone, Debug)]
pub struct Response {
    /// HTTP status code
    pub status: StatusCode,
    /// Response headers
    pub headers: HeaderMap,
    /// Response body
    pub body: bytes::Bytes,
}

impl Response {
    /// Create a new response.
    pub fn new(status: StatusCode, body: impl Into<bytes::Bytes>) -> Self {
        Self {
            status,
            headers: HeaderMap::new(),
            body: body.into(),
        }
    }

    /// Create a JSON response.
    pub fn json<T: Serialize>(status: StatusCode, value: &T) -> Result<Self, serde_json::Error> {
        let body = serde_json::to_vec(value)?;
        let mut response = Self::new(status, body);
        response.set_content_type("application/json; charset=utf-8");
        Ok(response)
    }

    /// Create an empty response.
    pub fn empty(status: StatusCode) -> Self {
        Self::new(status, bytes::Bytes::new())
    }

    /// Set a header.
    /// Add a header without replacing one of the same name.
    ///
    /// A response may legitimately carry two `Set-Cookie`s, which `set_header`
    /// -- being a replace -- cannot express.
    pub fn append_header(&mut self, name: &str, value: &str) {
        if let (Ok(name), Ok(value)) = (
            http::header::HeaderName::from_bytes(name.as_bytes()),
            HeaderValue::from_str(value),
        ) {
            self.headers.append(name, value);
        }
    }

    pub fn set_header(&mut self, name: &str, value: &str) {
        if let Ok(v) = HeaderValue::from_str(value) {
            self.headers.insert(
                http::header::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                v,
            );
        }
    }

    /// Set Content-Type header.
    pub fn set_content_type(&mut self, content_type: &str) {
        self.set_header("content-type", content_type);
    }

    /// Set Content-Range header.
    pub fn set_content_range(&mut self, range: &ContentRange) {
        self.set_header("content-range", &range.to_string());
    }

    /// Set Location header.
    pub fn set_location(&mut self, location: &str) {
        self.set_header("location", location);
    }
}

/// Format a query result as a response.
pub fn format_response(
    request: &ApiRequest,
    result: &QueryResult,
) -> Result<Response, FormatError> {
    let media_type = request
        .accept_media_types
        .first()
        .cloned()
        .unwrap_or(MediaType::ApplicationJson);

    // A mutation the caller wanted no representation from sends no body at
    // all -- not an empty JSON array, which is what an empty row set would
    // otherwise render as. The headers still apply.
    // A media type the schema renders itself: the database produced the whole
    // payload, so there is nothing here to serialise.
    if let Some((media_type, body)) = &result.raw_body {
        let mut response = Response::new(result.status, body.clone().into_bytes());
        response.set_content_type(media_type);
        if let Some(range) = &result.content_range {
            response.set_header("Content-Range", &range.to_string());
        }
        return Ok(response);
    }

    if result.omit_body {
        let mut response = Response::new(result.status, bytes::Bytes::new());
        add_common_headers(&mut response, request, result);
        return Ok(response);
    }

    // `;nulls=stripped` asks for keys with a null value to be left out
    // entirely, rather than sent as nulls.
    let rows = match &media_type {
        MediaType::SingularJson {
            strip_nulls: true, ..
        }
        | MediaType::ArrayJson {
            strip_nulls: true, ..
        } => result.rows.iter().cloned().map(strip_nulls).collect(),
        _ => result.rows.clone(),
    };
    let result = &QueryResult {
        rows,
        ..result.clone()
    };

    match &media_type {
        MediaType::ApplicationJson => {
            let body = if result.singular {
                format_singular_or_null(&result.rows)?
            } else {
                format_json_response(&result.rows)?
            };
            let mut response = Response::new(result.status, body);
            response.set_content_type("application/json; charset=utf-8");
            add_common_headers(&mut response, request, result);
            Ok(response)
        }
        MediaType::TextCsv => {
            // CSV formatting would go here
            let body = format_csv_response(&result.rows)?;
            let mut response = Response::new(result.status, body);
            response.set_content_type("text/csv; charset=utf-8");
            add_common_headers(&mut response, request, result);
            Ok(response)
        }
        MediaType::SingularJson { nullable, .. } => {
            let body = format_singular_json(&result.rows, *nullable)?;
            let mut response = Response::new(result.status, body);
            response.set_content_type("application/vnd.pgrst.object+json; charset=utf-8");
            add_common_headers(&mut response, request, result);
            Ok(response)
        }
        other => {
            // Default to JSON (covers `*/*`, e.g. a default curl request).
            let body = if result.singular {
                format_singular_or_null(&result.rows)?
            } else {
                format_json_response(&result.rows)?
            };
            let mut response = Response::new(result.status, body);
            // The body is JSON either way, but a client that asked for one of
            // PostgREST's own JSON media types is told it got that: the type
            // is how it knows the shape it negotiated was honoured.
            let content_type = match other {
                MediaType::ArrayJson { .. } => "application/vnd.pgrst.array+json; charset=utf-8",
                MediaType::OpenApi => "application/openapi+json; charset=utf-8",
                _ => "application/json; charset=utf-8",
            };
            response.set_content_type(content_type);
            add_common_headers(&mut response, request, result);
            Ok(response)
        }
    }
}

/// Which preferences this request could honour.
pub(crate) fn preference_scope(
    request: &ApiRequest,
) -> postrust_core::api_request::preferences::PreferenceScope {
    use postrust_core::api_request::preferences::PreferenceScope;
    use postrust_core::api_request::{Action, DbAction, Mutation};

    let Action::Db(action) = &request.action else {
        return PreferenceScope::read();
    };

    match action {
        DbAction::RelationMut { mutation, .. } => PreferenceScope {
            resolution: matches!(mutation, Mutation::Create),
            representation: true,
            missing: matches!(mutation, Mutation::Create | Mutation::Update),
            max_affected: matches!(mutation, Mutation::Update | Mutation::Delete),
        },
        DbAction::Routine { .. } => PreferenceScope {
            max_affected: true,
            ..PreferenceScope::read()
        },
        _ => PreferenceScope::read(),
    }
}

/// Remove every key whose value is null, at every depth.
///
/// `Accept: application/vnd.pgrst.array+json;nulls=stripped` asks for a body
/// carrying only what a row actually has, which for a wide table with few
/// populated columns is most of the response.
fn strip_nulls(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(fields) => serde_json::Value::Object(
            fields
                .into_iter()
                .filter(|(_, value)| !value.is_null())
                .map(|(key, value)| (key, strip_nulls(value)))
                .collect(),
        ),
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.into_iter().map(strip_nulls).collect())
        }
        other => other,
    }
}

/// Add common response headers.
fn add_common_headers(response: &mut Response, request: &ApiRequest, result: &QueryResult) {
    // A function's own headers, added rather than replaced: the shape of the
    // setting is an array precisely so that a name may repeat.
    if let Some(guc) = &result.guc_headers {
        if let Ok(headers) = crate::headers::parse_guc_headers(guc) {
            for (name, value) in headers {
                response.append_header(&name, &value);
            }
        }
    }

    // Content-Range
    if let Some(range) = &result.content_range {
        response.set_content_range(range);
    }

    // Location (for POST)
    if let Some(location) = &result.location {
        response.set_location(location);
    }

    // Preference-Applied
    if let Some(applied) = postrust_core::api_request::preferences::preference_applied(
        &request.preferences,
        preference_scope(request),
    ) {
        response.set_header("preference-applied", &applied);
    }

    // Content-Profile
    if request.negotiated_by_profile {
        response.set_header("content-profile", &request.schema);
    }
}

/// Format a result as a single value (PostgREST-compatibility RPC responses).
///
/// A single row is emitted bare (not array-wrapped), no rows becomes `null`,
/// and the (unexpected) multi-row case falls back to a JSON array so no data
/// is silently dropped.
fn format_singular_or_null(rows: &[serde_json::Value]) -> Result<bytes::Bytes, FormatError> {
    match rows.len() {
        0 => Ok(bytes::Bytes::from_static(b"null")),
        1 => Ok(bytes::Bytes::from(serde_json::to_vec(&rows[0])?)),
        _ => format_json_response(rows),
    }
}

/// Format singular JSON (single object or null).
fn format_singular_json(
    rows: &[serde_json::Value],
    nullable: bool,
) -> Result<bytes::Bytes, FormatError> {
    match rows.len() {
        0 if nullable => Ok(bytes::Bytes::from_static(b"null")),
        0 => Err(FormatError::NotFound),
        1 => Ok(bytes::Bytes::from(serde_json::to_vec(&rows[0])?)),
        _ => Err(FormatError::MultipleRows),
    }
}

/// Format CSV response.
fn format_csv_response(rows: &[serde_json::Value]) -> Result<bytes::Bytes, FormatError> {
    if rows.is_empty() {
        return Ok(bytes::Bytes::new());
    }

    let mut output = Vec::new();

    // Get headers from first row
    if let Some(serde_json::Value::Object(map)) = rows.first() {
        let headers: Vec<&str> = map.keys().map(|s| s.as_str()).collect();
        output.extend_from_slice(headers.join(",").as_bytes());
        output.push(b'\n');

        // Write rows
        for row in rows {
            if let serde_json::Value::Object(row_map) = row {
                let values: Vec<String> = headers
                    .iter()
                    .map(|h| row_map.get(*h).map(csv_escape).unwrap_or_default())
                    .collect();
                output.extend_from_slice(values.join(",").as_bytes());
                output.push(b'\n');
            }
        }
    }

    Ok(bytes::Bytes::from(output))
}

/// Escape a value for CSV.
fn csv_escape(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => {
            if s.contains(',') || s.contains('"') || s.contains('\n') {
                format!("\"{}\"", s.replace('"', "\"\""))
            } else {
                s.clone()
            }
        }
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// Query result for response formatting.
#[derive(Clone, Debug, Default)]
pub struct QueryResult {
    /// HTTP status code
    pub status: StatusCode,
    /// Result rows
    pub rows: Vec<serde_json::Value>,
    /// Total row count (for pagination)
    pub total_count: Option<i64>,
    /// Content range
    pub content_range: Option<ContentRange>,
    /// Location header (for POST)
    pub location: Option<String>,
    /// Custom headers from GUC
    pub guc_headers: Option<String>,
    /// Custom status from GUC
    pub guc_status: Option<String>,
    /// Whether to send no body at all.
    ///
    /// Set for a mutation the caller asked no representation from: the
    /// response carries headers and a status but no payload, where an empty
    /// row set would otherwise render as `[]`.
    pub omit_body: bool,
    /// Whether the result should be rendered as a single (un-arrayed) value.
    ///
    /// Set for PostgREST-compatibility RPC responses where the underlying
    /// function is not set-returning: the bare object/scalar is returned
    /// instead of a one-element array.
    pub singular: bool,
    /// A body the database rendered in full, with the media type it is in.
    ///
    /// Set when the request asked for a media type the schema declares its own
    /// renderer for. The value is the whole payload, so it replaces the usual
    /// JSON rendering rather than being wrapped in it.
    pub raw_body: Option<(String, String)>,
}

/// Response formatting error.
#[derive(Debug, thiserror::Error)]
pub enum FormatError {
    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Resource not found")]
    NotFound,

    #[error("Multiple rows returned for singular response")]
    MultipleRows,
}

impl FormatError {
    pub fn status_code(&self) -> StatusCode {
        match self {
            Self::Json(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::MultipleRows => StatusCode::NOT_ACCEPTABLE,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn result(rows: Vec<serde_json::Value>, singular: bool) -> QueryResult {
        QueryResult {
            status: StatusCode::OK,
            rows,
            singular,
            ..Default::default()
        }
    }

    #[test]
    fn singular_result_renders_bare_object() {
        let req = ApiRequest::default();
        let resp = format_response(&req, &result(vec![json!({"ok": true})], true)).unwrap();
        assert_eq!(&resp.body[..], br#"{"ok":true}"#);
    }

    #[test]
    fn singular_empty_result_renders_null() {
        let req = ApiRequest::default();
        let resp = format_response(&req, &result(vec![], true)).unwrap();
        assert_eq!(&resp.body[..], b"null");
    }

    #[test]
    fn non_singular_result_renders_array() {
        let req = ApiRequest::default();
        let resp = format_response(&req, &result(vec![json!(1), json!(2)], false)).unwrap();
        assert_eq!(&resp.body[..], b"[1,2]");
    }
}
