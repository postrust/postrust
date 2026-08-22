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

/// Parse GUC headers from database response.
#[allow(dead_code)] // Reserved for GUC-driven response headers (`response.headers`); not yet wired.
pub fn parse_guc_headers(guc_headers: &str) -> Vec<(String, String)> {
    // Format: "header1: value1\nheader2: value2"
    guc_headers
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(2, ':');
            let key = parts.next()?.trim().to_string();
            let value = parts.next()?.trim().to_string();
            Some((key, value))
        })
        .collect()
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
        let guc = "X-Custom-Header: value1\nX-Another: value2";
        let headers = parse_guc_headers(guc);

        assert_eq!(headers.len(), 2);
        assert_eq!(headers[0], ("X-Custom-Header".into(), "value1".into()));
        assert_eq!(headers[1], ("X-Another".into(), "value2".into()));
    }
}
