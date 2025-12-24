//! Request body payload parsing.
//!
//! Handles JSON and URL-encoded request bodies.

use super::types::*;
use crate::error::{Error, Result};
use bytes::Bytes;
use std::collections::HashSet;

/// Parse request body based on content type.
pub fn parse_payload(body: Bytes, content_type: &MediaType) -> Result<Option<Payload>> {
    if body.is_empty() {
        return Ok(None);
    }

    match content_type {
        MediaType::ApplicationJson => parse_json_payload(body),
        MediaType::UrlEncoded => parse_urlencoded_payload(body),
        MediaType::TextCsv => {
            // CSV is handled as raw JSON for processing
            Ok(Some(Payload::RawJson(body)))
        }
        MediaType::OctetStream | MediaType::TextPlain | MediaType::TextXml => {
            Ok(Some(Payload::RawPayload(body)))
        }
        _ => parse_json_payload(body),
    }
}

/// Parse JSON body and extract keys.
fn parse_json_payload(body: Bytes) -> Result<Option<Payload>> {
    // Parse to extract keys
    let value: serde_json::Value =
        serde_json::from_slice(&body).map_err(|e| Error::InvalidBody(e.to_string()))?;

    let keys = extract_json_keys(&value);

    Ok(Some(Payload::ProcessedJson { raw: body, keys }))
}

/// Extract top-level keys from JSON value.
fn extract_json_keys(value: &serde_json::Value) -> HashSet<String> {
    match value {
        serde_json::Value::Object(map) => map.keys().cloned().collect(),
        serde_json::Value::Array(arr) => {
            // For arrays, collect keys from all objects
            arr.iter()
                .filter_map(|v| v.as_object())
                .flat_map(|map| map.keys().cloned())
                .collect()
        }
        _ => HashSet::new(),
    }
}

/// Parse URL-encoded body.
fn parse_urlencoded_payload(body: Bytes) -> Result<Option<Payload>> {
    let body_str =
        std::str::from_utf8(&body).map_err(|_| Error::InvalidBody("Invalid UTF-8".into()))?;

    let data: Vec<(String, String)> = url::form_urlencoded::parse(body_str.as_bytes())
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();

    let keys: HashSet<String> = data.iter().map(|(k, _)| k.clone()).collect();

    Ok(Some(Payload::ProcessedUrlEncoded { data, keys }))
}

/// Check if payload keys match the expected columns.
pub fn validate_payload_columns(
    payload: &Payload,
    expected: &HashSet<String>,
) -> Result<()> {
    let keys = match payload {
        Payload::ProcessedJson { keys, .. } => keys,
        Payload::ProcessedUrlEncoded { keys, .. } => keys,
        _ => return Ok(()),
    };

    for key in keys {
        if !expected.contains(key) {
            return Err(Error::UnknownColumn(key.clone()));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_json_object() {
        let body = Bytes::from(r#"{"name": "John", "age": 30}"#);
        let payload = parse_payload(body, &MediaType::ApplicationJson)
            .unwrap()
            .unwrap();

        match payload {
            Payload::ProcessedJson { keys, .. } => {
                assert!(keys.contains("name"));
                assert!(keys.contains("age"));
            }
            _ => panic!("Expected ProcessedJson"),
        }
    }

    #[test]
    fn test_parse_json_array() {
        let body = Bytes::from(r#"[{"id": 1}, {"id": 2, "name": "test"}]"#);
        let payload = parse_payload(body, &MediaType::ApplicationJson)
            .unwrap()
            .unwrap();

        match payload {
            Payload::ProcessedJson { keys, .. } => {
                assert!(keys.contains("id"));
                assert!(keys.contains("name"));
            }
            _ => panic!("Expected ProcessedJson"),
        }
    }

    #[test]
    fn test_parse_urlencoded() {
        let body = Bytes::from("name=John&age=30");
        let payload = parse_payload(body, &MediaType::UrlEncoded)
            .unwrap()
            .unwrap();

        match payload {
            Payload::ProcessedUrlEncoded { data, keys } => {
                assert_eq!(data.len(), 2);
                assert!(keys.contains("name"));
                assert!(keys.contains("age"));
            }
            _ => panic!("Expected ProcessedUrlEncoded"),
        }
    }

    #[test]
    fn test_parse_empty_body() {
        let body = Bytes::new();
        let payload = parse_payload(body, &MediaType::ApplicationJson).unwrap();
        assert!(payload.is_none());
    }

    #[test]
    fn test_parse_octet_stream() {
        let body = Bytes::from(vec![0u8, 1, 2, 3]);
        let payload = parse_payload(body.clone(), &MediaType::OctetStream)
            .unwrap()
            .unwrap();

        match payload {
            Payload::RawPayload(data) => {
                assert_eq!(data, body);
            }
            _ => panic!("Expected RawPayload"),
        }
    }
}
