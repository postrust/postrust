//! Response header building.

use http::{HeaderMap, HeaderValue};
use postrust_core::ApiRequest;
use std::fmt;

/// Content-Range header value.
///
/// Rendered exactly as PostgREST's `contentRangeH`: no unit prefix, `*` for an
/// unknown total, and `*` in place of the range itself when the response is
/// empty. See `RangeQuery.hs` in the PostgREST source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentRange {
    /// Start of range (0-based)
    pub start: i64,
    /// End of range (inclusive)
    pub end: i64,
    /// Total count (or None if unknown)
    pub total: Option<i64>,
}

impl ContentRange {
    /// Create a new content range.
    pub fn new(start: i64, end: i64, total: Option<i64>) -> Self {
        Self { start, end, total }
    }

    /// Build the range for a response that returned `count` rows starting at
    /// `offset`.
    ///
    /// `count` is what the query actually returned, so any limit has already
    /// been applied; an empty result yields `end < start`, which renders as
    /// `*` rather than a zero-length range.
    pub fn from_pagination(offset: i64, count: i64, total: Option<i64>) -> Self {
        Self::new(offset, offset + count - 1, total)
    }

    /// The HTTP status this range implies, following PostgREST's `rangeStatus`.
    ///
    /// Without a total there is nothing to be partial about, so the answer is
    /// always 200.
    pub fn status(&self) -> http::StatusCode {
        match self.total {
            None => http::StatusCode::OK,
            Some(total) => {
                if self.start > total {
                    http::StatusCode::RANGE_NOT_SATISFIABLE
                } else if (1 + self.end - self.start) < total {
                    http::StatusCode::PARTIAL_CONTENT
                } else {
                    http::StatusCode::OK
                }
            }
        }
    }
}

impl fmt::Display for ContentRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // An empty result, or a known-zero total, reports `*` for the range.
        if self.total == Some(0) || self.start > self.end {
            write!(f, "*")?;
        } else {
            write!(f, "{}-{}", self.start, self.end)?;
        }
        match self.total {
            Some(total) => write!(f, "/{total}"),
            None => write!(f, "/*"),
        }
    }
}

/// Build response headers based on request and result.
///
/// # Deprecated
///
/// Nothing in this workspace calls this. The headers a response actually
/// carries are built by `add_common_headers`, and the two have already drifted:
/// `Allow` is sent for an OPTIONS by that path and not by this one, and the
/// `Content-Range` a rendered media type reports is likewise decided there.
/// Keeping a second implementation means every header fix has to be made
/// twice, and the one that is missed is the one nobody is calling.
///
/// Deprecated rather than removed because this crate is published to
/// crates.io, so the name is public API somebody outside this repository may
/// be using. It will be removed in a future breaking release.
#[deprecated(
    since = "0.4.0",
    note = "unused within the workspace and drifted from the live header path; \
            responses are built by postrust_response::format_response"
)]
pub fn build_response_headers(
    request: &ApiRequest,
    content_type: &str,
    content_range: Option<&ContentRange>,
    location: Option<&str>,
) -> HeaderMap {
    let mut headers = HeaderMap::new();

    // Content-Type
    if let Ok(v) = HeaderValue::from_str(content_type) {
        headers.insert(http::header::CONTENT_TYPE, v);
    }

    // Content-Range
    if let Some(range) = content_range {
        if let Ok(v) = HeaderValue::from_str(&range.to_string()) {
            headers.insert(http::header::CONTENT_RANGE, v);
        }
    }

    // Location
    if let Some(loc) = location {
        if let Ok(v) = HeaderValue::from_str(loc) {
            headers.insert(http::header::LOCATION, v);
        }
    }

    // Content-Profile
    if request.negotiated_by_profile {
        if let Ok(v) = HeaderValue::from_str(&request.schema) {
            headers.insert(http::header::HeaderName::from_static("content-profile"), v);
        }
    }

    // Preference-Applied
    if let Some(applied) = postrust_core::api_request::preferences::preference_applied(
        &request.preferences,
        crate::preference_scope(request),
    ) {
        if let Ok(v) = HeaderValue::from_str(&applied) {
            headers.insert(
                http::header::HeaderName::from_static("preference-applied"),
                v,
            );
        }
    }

    headers
}

/// Parse the `response.headers` setting a function may have set.
///
/// PostgREST's shape: a JSON array of objects, each with exactly one key and
/// a string value. An array rather than one object because a response may
/// carry the same header twice -- two `Set-Cookie`s, say -- which an object
/// cannot express.
///
/// `Err` when it is anything else, which is a fault in the function rather
/// than in the request and is reported as one.
pub fn parse_guc_headers(guc_headers: &str) -> Option<Vec<(String, String)>> {
    let Ok(serde_json::Value::Array(entries)) = serde_json::from_str(guc_headers) else {
        return None;
    };

    let mut headers = Vec::with_capacity(entries.len());
    for entry in entries {
        let serde_json::Value::Object(fields) = entry else {
            return None;
        };
        if fields.len() != 1 {
            return None;
        }
        for (name, value) in fields {
            let serde_json::Value::String(value) = value else {
                return None;
            };
            headers.push((name, value));
        }
    }

    Some(headers)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_content_range_display() {
        // No unit prefix, and `*` stands in for an unknown total.
        assert_eq!(ContentRange::new(0, 9, Some(100)).to_string(), "0-9/100");
        assert_eq!(ContentRange::new(10, 19, None).to_string(), "10-19/*");
    }

    #[test]
    fn test_content_range_display_empty() {
        // An empty result reports `*` for the range, not a zero-length one.
        assert_eq!(ContentRange::from_pagination(0, 0, None).to_string(), "*/*");
        assert_eq!(
            ContentRange::from_pagination(0, 0, Some(0)).to_string(),
            "*/0"
        );
    }

    #[test]
    fn test_content_range_from_pagination() {
        // A full page: `count` is what came back, so the limit is already applied.
        let range = ContentRange::from_pagination(0, 10, Some(100));
        assert_eq!((range.start, range.end), (0, 9));

        // Partial last page.
        let range = ContentRange::from_pagination(90, 5, Some(95));
        assert_eq!((range.start, range.end), (90, 94));
    }

    #[test]
    fn test_content_range_status() {
        // Without a total there is nothing to be partial about.
        assert_eq!(
            ContentRange::from_pagination(0, 10, None).status(),
            http::StatusCode::OK
        );
        // A window narrower than the total is partial content.
        assert_eq!(
            ContentRange::from_pagination(0, 10, Some(100)).status(),
            http::StatusCode::PARTIAL_CONTENT
        );
        // The whole set is a plain 200.
        assert_eq!(
            ContentRange::from_pagination(0, 15, Some(15)).status(),
            http::StatusCode::OK
        );
        // Asking past the end is not satisfiable.
        assert_eq!(
            ContentRange::from_pagination(50, 0, Some(15)).status(),
            http::StatusCode::RANGE_NOT_SATISFIABLE
        );
    }

    #[test]
    fn test_parse_guc_headers() {
        let guc = r#"[{"X-Custom-Header": "value1"}, {"X-Another": "value2"}]"#;
        let headers = parse_guc_headers(guc).unwrap();

        assert_eq!(headers.len(), 2);
        assert_eq!(headers[0], ("X-Custom-Header".into(), "value1".into()));
        assert_eq!(headers[1], ("X-Another".into(), "value2".into()));
    }

    /// The same header twice is the reason it is an array.
    #[test]
    fn guc_headers_may_repeat_a_name() {
        let guc = r#"[{"Set-Cookie": "a=1"}, {"Set-Cookie": "b=2"}]"#;
        let headers = parse_guc_headers(guc).unwrap();

        assert_eq!(headers.len(), 2);
        assert!(headers.iter().all(|(name, _)| name == "Set-Cookie"));
    }

    /// Anything else is a fault in the function that set it.
    #[test]
    fn guc_headers_must_be_single_key_objects_of_strings() {
        for bad in [
            r#"{"X": "y"}"#,
            r#"[{"X": "y", "Z": "w"}]"#,
            r#"[{"X": 1}]"#,
            r#"["X: y"]"#,
            "not json",
        ] {
            assert!(parse_guc_headers(bad).is_none(), "accepted {bad}");
        }
    }
}
