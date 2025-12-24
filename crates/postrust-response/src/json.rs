//! JSON response formatting.

use super::FormatError;
use bytes::Bytes;

/// Format rows as a JSON array.
pub fn format_json_response(rows: &[serde_json::Value]) -> Result<Bytes, FormatError> {
    let json = serde_json::to_vec(rows)?;
    Ok(Bytes::from(json))
}

/// Format a single row as JSON object.
pub fn format_json_object(row: &serde_json::Value) -> Result<Bytes, FormatError> {
    let json = serde_json::to_vec(row)?;
    Ok(Bytes::from(json))
}

/// Format rows with nulls stripped (for vnd.pgrst.array+json).
pub fn format_json_strip_nulls(rows: &[serde_json::Value]) -> Result<Bytes, FormatError> {
    let stripped: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| strip_nulls(row.clone()))
        .collect();
    let json = serde_json::to_vec(&stripped)?;
    Ok(Bytes::from(json))
}

/// Recursively strip null values from a JSON value.
fn strip_nulls(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let filtered: serde_json::Map<String, serde_json::Value> = map
                .into_iter()
                .filter(|(_, v)| !v.is_null())
                .map(|(k, v)| (k, strip_nulls(v)))
                .collect();
            serde_json::Value::Object(filtered)
        }
        serde_json::Value::Array(arr) => {
            let filtered: Vec<serde_json::Value> = arr
                .into_iter()
                .map(strip_nulls)
                .collect();
            serde_json::Value::Array(filtered)
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_format_json_response() {
        let rows = vec![
            json!({"id": 1, "name": "Alice"}),
            json!({"id": 2, "name": "Bob"}),
        ];

        let result = format_json_response(&rows).unwrap();
        let parsed: Vec<serde_json::Value> = serde_json::from_slice(&result).unwrap();

        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0]["name"], "Alice");
    }

    #[test]
    fn test_format_json_object() {
        let row = json!({"id": 1, "name": "Alice"});

        let result = format_json_object(&row).unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&result).unwrap();

        assert_eq!(parsed["name"], "Alice");
    }

    #[test]
    fn test_strip_nulls() {
        let value = json!({
            "id": 1,
            "name": "Alice",
            "email": null,
            "nested": {
                "a": 1,
                "b": null
            }
        });

        let stripped = strip_nulls(value);

        assert!(stripped.get("id").is_some());
        assert!(stripped.get("name").is_some());
        assert!(stripped.get("email").is_none());
        assert!(stripped["nested"].get("b").is_none());
    }

    #[test]
    fn test_format_empty_array() {
        let rows: Vec<serde_json::Value> = vec![];
        let result = format_json_response(&rows).unwrap();
        assert_eq!(&result[..], b"[]");
    }
}
