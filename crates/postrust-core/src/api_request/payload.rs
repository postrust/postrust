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
        MediaType::TextCsv => parse_csv_payload(body),
        MediaType::OctetStream | MediaType::TextPlain | MediaType::TextXml => {
            Ok(Some(Payload::RawPayload(body)))
        }
        _ => parse_json_payload(body),
    }
}

/// Parse JSON body and extract keys.
fn parse_json_payload(body: Bytes) -> Result<Option<Payload>> {
    // Parse to extract keys
    // What went wrong with the JSON is not the client's to act on -- the
    // parser's offset says nothing it can use -- so this is the one message,
    // and it is the one PostgREST gives.
    let value: serde_json::Value = serde_json::from_slice(&body)
        .map_err(|_| Error::InvalidBody("Empty or invalid json".into()))?;

    let keys = extract_json_keys(&value);

    Ok(Some(Payload::ProcessedJson { raw: body, keys }))
}

/// Parse a CSV body into the same shape a JSON array body would have taken.
///
/// The header row names the columns. Everything after it is a row of text,
/// except an unquoted empty field, which is null -- that is the only way CSV
/// can express one, and it is what PostgREST reads it as.
fn parse_csv_payload(body: Bytes) -> Result<Option<Payload>> {
    let text = std::str::from_utf8(&body)
        .map_err(|_| Error::InvalidBody("Empty or invalid json".into()))?;

    let mut records = read_csv(text);
    if records.is_empty() {
        return Ok(None);
    }

    let headers: Vec<String> = records
        .remove(0)
        .into_iter()
        .map(|(value, _)| value)
        .collect();

    let mut rows = Vec::with_capacity(records.len());
    for record in records {
        if record.len() != headers.len() {
            return Err(Error::InvalidBody(
                "All lines must have same number of fields".into(),
            ));
        }
        let mut row = serde_json::Map::with_capacity(headers.len());
        for (name, (value, quoted)) in headers.iter().zip(record) {
            let value = match value.is_empty() && !quoted {
                true => serde_json::Value::Null,
                false => serde_json::Value::String(value),
            };
            row.insert(name.clone(), value);
        }
        rows.push(serde_json::Value::Object(row));
    }

    let raw = serde_json::to_vec(&serde_json::Value::Array(rows))
        .map_err(|e| Error::Internal(e.to_string()))?;

    Ok(Some(Payload::ProcessedJson {
        raw: Bytes::from(raw),
        keys: headers.into_iter().collect(),
    }))
}

/// Split CSV text into records of `(field, was quoted)`.
///
/// A quoted field may contain commas and newlines, and `""` inside one is a
/// literal quote. Whether a field was quoted is carried out because it is the
/// only thing separating an empty string from a null.
fn read_csv(text: &str) -> Vec<Vec<(String, bool)>> {
    let mut records = Vec::new();
    let mut record = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut in_quotes = false;
    let mut chars = text.chars().peekable();

    while let Some(ch) = chars.next() {
        if in_quotes {
            match ch {
                '"' if chars.peek() == Some(&'"') => {
                    chars.next();
                    field.push('"');
                }
                '"' => in_quotes = false,
                other => field.push(other),
            }
            continue;
        }
        match ch {
            '"' => {
                in_quotes = true;
                quoted = true;
            }
            ',' => record.push((std::mem::take(&mut field), std::mem::take(&mut quoted))),
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                record.push((std::mem::take(&mut field), std::mem::take(&mut quoted)));
                records.push(std::mem::take(&mut record));
            }
            '\n' => {
                record.push((std::mem::take(&mut field), std::mem::take(&mut quoted)));
                records.push(std::mem::take(&mut record));
            }
            other => field.push(other),
        }
    }

    if !field.is_empty() || quoted || !record.is_empty() {
        record.push((field, quoted));
        records.push(record);
    }

    records
}

/// Check that every row of a bulk insert names the same columns.
///
/// The insert is one statement with one column list, so a row naming fewer
/// columns would silently take defaults for the rest. `?columns=` is how a
/// client says which columns to write when the rows genuinely differ, and this
/// check is skipped when it does.
pub fn validate_uniform_keys(payload: &Payload) -> Result<()> {
    let Payload::ProcessedJson { raw, .. } = payload else {
        return Ok(());
    };
    let Ok(serde_json::Value::Array(rows)) = serde_json::from_slice::<serde_json::Value>(raw)
    else {
        return Ok(());
    };

    let mut expected: Option<HashSet<&String>> = None;
    for row in &rows {
        let Some(object) = row.as_object() else {
            return Err(Error::InvalidBody("All object keys must match".into()));
        };
        let keys: HashSet<&String> = object.keys().collect();
        match &expected {
            None => expected = Some(keys),
            Some(first) if *first != keys => {
                return Err(Error::InvalidBody("All object keys must match".into()))
            }
            Some(_) => {}
        }
    }

    Ok(())
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
pub fn validate_payload_columns(payload: &Payload, expected: &HashSet<String>) -> Result<()> {
    let keys = match payload {
        Payload::ProcessedJson { keys, .. } => keys,
        Payload::ProcessedUrlEncoded { keys, .. } => keys,
        _ => return Ok(()),
    };

    for key in keys {
        if !expected.contains(key) {
            return Err(Error::UnknownColumn {
                column: key.clone(),
                relation: String::new(),
            });
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
