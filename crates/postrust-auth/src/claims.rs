//! JWT claims handling.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Parsed JWT claims for use in requests.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Claims {
    /// All claims as key-value pairs
    pub values: HashMap<String, serde_json::Value>,
}

impl Claims {
    /// Create empty claims.
    pub fn new() -> Self {
        Self::default()
    }

    /// Get a claim value as a string.
    pub fn get_str(&self, key: &str) -> Option<&str> {
        self.values.get(key).and_then(|v| v.as_str())
    }

    /// Get a claim value as an integer.
    pub fn get_i64(&self, key: &str) -> Option<i64> {
        self.values.get(key).and_then(|v| v.as_i64())
    }

    /// Get a claim value as a boolean.
    pub fn get_bool(&self, key: &str) -> Option<bool> {
        self.values.get(key).and_then(|v| v.as_bool())
    }

    /// Get a claim value.
    pub fn get(&self, key: &str) -> Option<&serde_json::Value> {
        self.values.get(key)
    }

    /// Set a claim value.
    pub fn set(&mut self, key: impl Into<String>, value: serde_json::Value) {
        self.values.insert(key.into(), value);
    }

    /// Convert to JSON string for GUC.
    pub fn to_json(&self) -> String {
        serde_json::to_string(&self.values).unwrap_or_else(|_| "{}".to_string())
    }

    /// Get claims for a specific prefix (e.g., "request.jwt.claims.").
    pub fn prefixed_entries(&self, prefix: &str) -> Vec<(String, String)> {
        self.values
            .iter()
            .map(|(k, v)| {
                let key = format!("{}{}", prefix, k);
                let value = match v {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                (key, value)
            })
            .collect()
    }
}

impl From<HashMap<String, serde_json::Value>> for Claims {
    fn from(values: HashMap<String, serde_json::Value>) -> Self {
        Self { values }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_claims_get_str() {
        let mut claims = Claims::new();
        claims.set("role", serde_json::Value::String("admin".into()));

        assert_eq!(claims.get_str("role"), Some("admin"));
        assert_eq!(claims.get_str("missing"), None);
    }

    #[test]
    fn test_claims_get_i64() {
        let mut claims = Claims::new();
        claims.set("user_id", serde_json::Value::Number(42.into()));

        assert_eq!(claims.get_i64("user_id"), Some(42));
    }

    #[test]
    fn test_claims_to_json() {
        let mut claims = Claims::new();
        claims.set("role", serde_json::Value::String("user".into()));
        claims.set("id", serde_json::Value::Number(123.into()));

        let json = claims.to_json();
        assert!(json.contains("role"));
        assert!(json.contains("user"));
    }

    #[test]
    fn test_claims_prefixed_entries() {
        let mut claims = Claims::new();
        claims.set("role", serde_json::Value::String("admin".into()));
        claims.set("email", serde_json::Value::String("test@example.com".into()));

        let entries = claims.prefixed_entries("request.jwt.claims.");
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().any(|(k, _)| k == "request.jwt.claims.role"));
    }
}
