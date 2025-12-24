//! Response header building.

use http::{HeaderMap, HeaderValue};
use postrust_core::ApiRequest;
use std::fmt;

/// Content-Range header value.
#[derive(Clone, Debug)]
pub struct ContentRange {
    /// Start of range (0-based)
    pub start: i64,
    /// End of range (inclusive)
    pub end: i64,
    /// Total count (or None if unknown)
    pub total: Option<i64>,
    /// Unit name
    pub unit: String,
}

impl ContentRange {
    /// Create a new content range.
    pub fn new(start: i64, end: i64, total: Option<i64>) -> Self {
        Self {
            start,
            end,
            total,
            unit: "items".to_string(),
        }
    }

    /// Create from offset, limit, and total.
    pub fn from_pagination(offset: i64, limit: Option<i64>, count: i64, total: Option<i64>) -> Self {
        let end = match limit {
            Some(l) => (offset + l - 1).min(offset + count - 1).max(offset),
            None => offset + count - 1,
        };

        Self::new(offset, end, total)
    }
}

impl fmt::Display for ContentRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.total {
            Some(total) => write!(f, "{} {}-{}/{}", self.unit, self.start, self.end, total),
            None => write!(f, "{} {}-{}/*", self.unit, self.start, self.end),
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
            headers.insert(
                http::header::HeaderName::from_static("content-profile"),
                v,
            );
        }
    }

    // Preference-Applied
    if let Some(applied) = postrust_core::api_request::preferences::preference_applied(&request.preferences) {
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
        let range = ContentRange::new(0, 9, Some(100));
        assert_eq!(range.to_string(), "items 0-9/100");

        let range = ContentRange::new(10, 19, None);
        assert_eq!(range.to_string(), "items 10-19/*");
    }

    #[test]
    fn test_content_range_from_pagination() {
        // First page of 10
        let range = ContentRange::from_pagination(0, Some(10), 10, Some(100));
        assert_eq!(range.start, 0);
        assert_eq!(range.end, 9);

        // Partial last page
        let range = ContentRange::from_pagination(90, Some(10), 5, Some(95));
        assert_eq!(range.start, 90);
        assert_eq!(range.end, 94);
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
