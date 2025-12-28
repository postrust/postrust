//! GraphQL subscriptions using PostgreSQL LISTEN/NOTIFY.
//!
//! This module provides realtime data synchronization through GraphQL subscriptions,
//! powered by PostgreSQL's native LISTEN/NOTIFY mechanism.
//!
//! ## Architecture
//!
//! ```text
//! ┌──────────────┐   WebSocket   ┌──────────────┐   LISTEN/NOTIFY   ┌────────────┐
//! │   Client     │◀────────────▶│   Postrust   │◀────────────────▶│  PostgreSQL│
//! │  (Browser)   │              │   Server     │                   │  Database  │
//! └──────────────┘              └──────────────┘                   └────────────┘
//! ```
//!
//! ## Usage
//!
//! 1. Create notification triggers on your tables using [`broker::create_notify_trigger_sql`]
//! 2. Start the [`NotifyBroker`] to listen for database notifications
//! 3. Use GraphQL subscriptions to receive realtime updates
//!
//! ## Example
//!
//! ```graphql
//! subscription {
//!   users {
//!     id
//!     name
//!     email
//!   }
//! }
//! ```

pub mod broker;

pub use broker::{
    create_notify_trigger_sql, drop_notify_trigger_sql, table_channel_name, BrokerError,
    NotifyBroker, PgNotification,
};

use crate::schema::GeneratedSchema;
use postrust_core::schema_cache::SchemaCache;

/// A subscription field in the GraphQL schema.
#[derive(Debug, Clone)]
pub struct SubscriptionField {
    /// The GraphQL field name (e.g., "users", "orders")
    pub name: String,
    /// The source table name
    pub table_name: String,
    /// The source schema name
    pub schema_name: String,
    /// The return type (e.g., "Users", "Orders")
    pub return_type: String,
    /// Description for documentation
    pub description: Option<String>,
}

impl SubscriptionField {
    /// Create a new subscription field for a table.
    pub fn for_table(schema: &str, table: &str, type_name: &str) -> Self {
        Self {
            name: to_camel_case(table),
            table_name: table.to_string(),
            schema_name: schema.to_string(),
            return_type: type_name.to_string(),
            description: Some(format!("Subscribe to changes on the {} table", table)),
        }
    }

    /// Get the PostgreSQL channel name for this subscription.
    pub fn channel_name(&self) -> String {
        table_channel_name(&self.schema_name, &self.table_name)
    }
}

/// Generate subscription fields for all tables in the schema.
pub fn generate_subscription_fields(
    _schema_cache: &SchemaCache,
    generated: &GeneratedSchema,
) -> Vec<SubscriptionField> {
    let mut fields = Vec::new();

    for (type_name, obj_type) in &generated.object_types {
        let table = &obj_type.table;

        // Only create subscriptions for tables, not views (views can be added later)
        if !table.is_view {
            fields.push(SubscriptionField::for_table(
                &table.schema,
                &table.name,
                type_name,
            ));
        }
    }

    fields
}

/// Convert a snake_case string to camelCase.
fn to_camel_case(s: &str) -> String {
    let mut result = String::new();
    let mut capitalize_next = false;

    for (i, c) in s.chars().enumerate() {
        if c == '_' {
            capitalize_next = true;
        } else if capitalize_next {
            result.push(c.to_ascii_uppercase());
            capitalize_next = false;
        } else if i == 0 {
            result.push(c.to_ascii_lowercase());
        } else {
            result.push(c);
        }
    }

    result
}

/// Payload structure for table change notifications.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct TableChangePayload {
    /// The operation type: INSERT, UPDATE, or DELETE
    pub operation: String,
    /// The table name
    pub table: String,
    /// The schema name
    pub schema: String,
    /// The old row data (for UPDATE and DELETE)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old: Option<serde_json::Value>,
    /// The new row data (for INSERT and UPDATE)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new: Option<serde_json::Value>,
}

impl TableChangePayload {
    /// Parse a notification payload.
    pub fn from_payload(payload: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(payload)
    }

    /// Get the data to return to the client.
    ///
    /// For INSERT and UPDATE, returns the new row.
    /// For DELETE, returns the old row.
    pub fn data(&self) -> Option<&serde_json::Value> {
        match self.operation.as_str() {
            "DELETE" => self.old.as_ref(),
            _ => self.new.as_ref(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_camel_case() {
        assert_eq!(to_camel_case("users"), "users");
        assert_eq!(to_camel_case("user_orders"), "userOrders");
        assert_eq!(to_camel_case("order_items"), "orderItems");
        // PostgreSQL identifiers are typically lowercase, but function preserves case
        assert_eq!(to_camel_case("my_table_name"), "myTableName");
    }

    #[test]
    fn test_subscription_field_channel_name() {
        let field = SubscriptionField::for_table("public", "users", "Users");
        assert_eq!(field.channel_name(), "postrust_public_users");
    }

    #[test]
    fn test_table_change_payload_parsing() {
        let json = r#"{
            "operation": "INSERT",
            "table": "users",
            "schema": "public",
            "new": {"id": 1, "name": "Alice"}
        }"#;

        let payload = TableChangePayload::from_payload(json).unwrap();
        assert_eq!(payload.operation, "INSERT");
        assert_eq!(payload.table, "users");
        assert!(payload.new.is_some());
        assert!(payload.old.is_none());
    }

    #[test]
    fn test_table_change_payload_data() {
        let insert_payload = TableChangePayload {
            operation: "INSERT".to_string(),
            table: "users".to_string(),
            schema: "public".to_string(),
            old: None,
            new: Some(serde_json::json!({"id": 1})),
        };
        assert!(insert_payload.data().is_some());

        let delete_payload = TableChangePayload {
            operation: "DELETE".to_string(),
            table: "users".to_string(),
            schema: "public".to_string(),
            old: Some(serde_json::json!({"id": 1})),
            new: None,
        };
        assert!(delete_payload.data().is_some());
    }
}
