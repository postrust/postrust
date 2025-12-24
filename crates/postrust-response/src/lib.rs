//! Response formatting for Postrust.
//!
//! Handles content negotiation and response formatting for JSON, CSV, and other formats.

mod json;
mod headers;

pub use json::format_json_response;
pub use headers::{build_response_headers, ContentRange};

use http::{HeaderMap, HeaderValue, StatusCode};
use postrust_core::{ActionPlan, ApiRequest, MediaType, PreferRepresentation};
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

    match &media_type {
        MediaType::ApplicationJson => {
            let body = format_json_response(&result.rows)?;
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
        MediaType::SingularJson { nullable } => {
            let body = format_singular_json(&result.rows, *nullable)?;
            let mut response = Response::new(result.status, body);
            response.set_content_type("application/vnd.pgrst.object+json; charset=utf-8");
            add_common_headers(&mut response, request, result);
            Ok(response)
        }
        _ => {
            // Default to JSON
            let body = format_json_response(&result.rows)?;
            let mut response = Response::new(result.status, body);
            response.set_content_type("application/json; charset=utf-8");
            add_common_headers(&mut response, request, result);
            Ok(response)
        }
    }
}

/// Add common response headers.
fn add_common_headers(response: &mut Response, request: &ApiRequest, result: &QueryResult) {
    // Content-Range
    if let Some(range) = &result.content_range {
        response.set_content_range(range);
    }

    // Location (for POST)
    if let Some(location) = &result.location {
        response.set_location(location);
    }

    // Preference-Applied
    if let Some(applied) = postrust_core::api_request::preferences::preference_applied(&request.preferences) {
        response.set_header("preference-applied", &applied);
    }

    // Content-Profile
    if request.negotiated_by_profile {
        response.set_header("content-profile", &request.schema);
    }
}

/// Format singular JSON (single object or null).
fn format_singular_json(rows: &[serde_json::Value], nullable: bool) -> Result<bytes::Bytes, FormatError> {
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
    if let Some(first) = rows.first() {
        if let serde_json::Value::Object(map) = first {
            let headers: Vec<&str> = map.keys().map(|s| s.as_str()).collect();
            output.extend_from_slice(headers.join(",").as_bytes());
            output.push(b'\n');

            // Write rows
            for row in rows {
                if let serde_json::Value::Object(row_map) = row {
                    let values: Vec<String> = headers
                        .iter()
                        .map(|h| {
                            row_map
                                .get(*h)
                                .map(|v| csv_escape(v))
                                .unwrap_or_default()
                        })
                        .collect();
                    output.extend_from_slice(values.join(",").as_bytes());
                    output.push(b'\n');
                }
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
