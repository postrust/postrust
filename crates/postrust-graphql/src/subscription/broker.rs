//! PostgreSQL NOTIFY message broker for GraphQL subscriptions.
//!
//! This module provides a broker that listens to PostgreSQL NOTIFY events
//! and broadcasts them to GraphQL subscription clients.

use futures::stream::{Stream, StreamExt};
use sqlx::postgres::PgListener;
use sqlx::PgPool;
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::broadcast;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

/// Default channel capacity for broadcast channels
const DEFAULT_CHANNEL_CAPACITY: usize = 256;

/// A notification from PostgreSQL
#[derive(Debug, Clone)]
pub struct PgNotification {
    /// The channel name (table name or custom channel)
    pub channel: String,
    /// The payload (usually JSON)
    pub payload: String,
    /// Process ID that sent the notification
    pub process_id: u32,
}

/// Message broker that distributes PostgreSQL NOTIFY events to subscribers.
pub struct NotifyBroker {
    /// Database connection pool
    pool: PgPool,
    /// Channel senders keyed by channel name
    channels: Arc<RwLock<HashMap<String, broadcast::Sender<PgNotification>>>>,
    /// Capacity for new broadcast channels
    channel_capacity: usize,
    /// Whether the broker is running
    running: Arc<RwLock<bool>>,
}

impl NotifyBroker {
    /// Create a new notification broker.
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            channels: Arc::new(RwLock::new(HashMap::new())),
            channel_capacity: DEFAULT_CHANNEL_CAPACITY,
            running: Arc::new(RwLock::new(false)),
        }
    }

    /// Create a new notification broker with custom channel capacity.
    pub fn with_capacity(pool: PgPool, capacity: usize) -> Self {
        Self {
            pool,
            channels: Arc::new(RwLock::new(HashMap::new())),
            channel_capacity: capacity,
            running: Arc::new(RwLock::new(false)),
        }
    }

    /// Start listening for notifications on the given channels.
    ///
    /// This spawns a background task that listens for PostgreSQL NOTIFY events
    /// and broadcasts them to all subscribers.
    pub async fn start(&self, listen_channels: Vec<String>) -> Result<(), BrokerError> {
        // Check if already running
        {
            let running = self.running.read().await;
            if *running {
                return Err(BrokerError::AlreadyRunning);
            }
        }

        // Mark as running
        {
            let mut running = self.running.write().await;
            *running = true;
        }

        // Create channels for each listen channel
        {
            let mut channels = self.channels.write().await;
            for channel_name in &listen_channels {
                if !channels.contains_key(channel_name) {
                    let (tx, _) = broadcast::channel(self.channel_capacity);
                    channels.insert(channel_name.clone(), tx);
                }
            }
        }

        // Create listener
        let mut listener = PgListener::connect_with(&self.pool)
            .await
            .map_err(BrokerError::Database)?;

        // Subscribe to all channels
        for channel in &listen_channels {
            listener
                .listen(channel)
                .await
                .map_err(BrokerError::Database)?;
            info!("Listening on PostgreSQL channel: {}", channel);
        }

        // Clone for the spawned task
        let channels = Arc::clone(&self.channels);
        let running = Arc::clone(&self.running);

        // Spawn listener task
        tokio::spawn(async move {
            loop {
                // Check if we should stop
                {
                    let is_running = running.read().await;
                    if !*is_running {
                        info!("Broker stopped, exiting listener loop");
                        break;
                    }
                }

                match listener.try_recv().await {
                    Ok(Some(notification)) => {
                        let pg_notification = PgNotification {
                            channel: notification.channel().to_string(),
                            payload: notification.payload().to_string(),
                            process_id: notification.process_id() as u32,
                        };

                        debug!(
                            "Received notification on channel '{}': {}",
                            pg_notification.channel,
                            &pg_notification.payload[..pg_notification.payload.len().min(100)]
                        );

                        // Broadcast to subscribers
                        let channels_read = channels.read().await;
                        if let Some(sender) = channels_read.get(&pg_notification.channel) {
                            // Ignore send errors - means no active receivers
                            let _ = sender.send(pg_notification);
                        }
                    }
                    Ok(None) => {
                        // No notification available, continue
                        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
                    }
                    Err(e) => {
                        error!("Error receiving notification: {:?}", e);
                        // Try to reconnect after a delay
                        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                    }
                }
            }
        });

        Ok(())
    }

    /// Stop the broker.
    pub async fn stop(&self) {
        let mut running = self.running.write().await;
        *running = false;
        info!("Broker stop requested");
    }

    /// Subscribe to notifications for a specific channel.
    ///
    /// Returns a stream of notifications for the given channel.
    pub async fn subscribe(
        &self,
        channel: &str,
    ) -> Result<Pin<Box<dyn Stream<Item = PgNotification> + Send>>, BrokerError> {
        let channels = self.channels.read().await;

        let sender = channels
            .get(channel)
            .ok_or_else(|| BrokerError::ChannelNotFound(channel.to_string()))?;

        let receiver = sender.subscribe();

        // Convert broadcast receiver to stream
        let stream = tokio_stream::wrappers::BroadcastStream::new(receiver).filter_map(|result| {
            futures::future::ready(result.ok())
        });

        Ok(Box::pin(stream))
    }

    /// Subscribe to a channel, creating it if it doesn't exist.
    ///
    /// Note: This only creates a broadcast channel. You must also call
    /// `listen_channel` to start receiving PostgreSQL notifications.
    pub async fn subscribe_or_create(
        &self,
        channel: &str,
    ) -> Pin<Box<dyn Stream<Item = PgNotification> + Send>> {
        // First try to get existing channel
        {
            let channels = self.channels.read().await;
            if let Some(sender) = channels.get(channel) {
                let receiver = sender.subscribe();
                let stream = tokio_stream::wrappers::BroadcastStream::new(receiver)
                    .filter_map(|result| futures::future::ready(result.ok()));
                return Box::pin(stream);
            }
        }

        // Create new channel
        {
            let mut channels = self.channels.write().await;
            // Double-check after acquiring write lock
            if !channels.contains_key(channel) {
                let (tx, _) = broadcast::channel(self.channel_capacity);
                channels.insert(channel.to_string(), tx);
            }
        }

        // Now subscribe
        let channels = self.channels.read().await;
        let sender = channels.get(channel).expect("just created");
        let receiver = sender.subscribe();
        let stream = tokio_stream::wrappers::BroadcastStream::new(receiver)
            .filter_map(|result| futures::future::ready(result.ok()));
        Box::pin(stream)
    }

    /// Add a new channel to listen on dynamically.
    pub async fn listen_channel(&self, channel: &str) -> Result<(), BrokerError> {
        // Create a new listener for this channel
        let mut listener = PgListener::connect_with(&self.pool)
            .await
            .map_err(BrokerError::Database)?;

        listener
            .listen(channel)
            .await
            .map_err(BrokerError::Database)?;

        // Ensure broadcast channel exists
        {
            let mut channels = self.channels.write().await;
            if !channels.contains_key(channel) {
                let (tx, _) = broadcast::channel(self.channel_capacity);
                channels.insert(channel.to_string(), tx);
            }
        }

        let channels = Arc::clone(&self.channels);
        let running = Arc::clone(&self.running);
        let channel_name = channel.to_string();

        // Spawn a listener for this channel
        tokio::spawn(async move {
            info!("Started dynamic listener for channel: {}", channel_name);

            loop {
                {
                    let is_running = running.read().await;
                    if !*is_running {
                        break;
                    }
                }

                match listener.try_recv().await {
                    Ok(Some(notification)) => {
                        let pg_notification = PgNotification {
                            channel: notification.channel().to_string(),
                            payload: notification.payload().to_string(),
                            process_id: notification.process_id() as u32,
                        };

                        let channels_read = channels.read().await;
                        if let Some(sender) = channels_read.get(&pg_notification.channel) {
                            let _ = sender.send(pg_notification);
                        }
                    }
                    Ok(None) => {
                        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
                    }
                    Err(e) => {
                        warn!("Error on channel {}: {:?}", channel_name, e);
                        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                    }
                }
            }

            info!("Stopped dynamic listener for channel: {}", channel_name);
        });

        Ok(())
    }

    /// Check if the broker is currently running.
    pub async fn is_running(&self) -> bool {
        *self.running.read().await
    }

    /// Get the number of active channels.
    pub async fn channel_count(&self) -> usize {
        self.channels.read().await.len()
    }
}

/// Errors that can occur in the broker.
#[derive(Debug, thiserror::Error)]
pub enum BrokerError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Channel not found: {0}")]
    ChannelNotFound(String),

    #[error("Broker is already running")]
    AlreadyRunning,
}

/// Generate a channel name for table change notifications.
pub fn table_channel_name(schema: &str, table: &str) -> String {
    format!("postrust_{}_{}", schema, table)
}

/// Generate SQL to create a notification trigger for a table.
pub fn create_notify_trigger_sql(schema: &str, table: &str) -> String {
    let channel = table_channel_name(schema, table);
    let trigger_name = format!("postrust_notify_{}_{}", schema, table);
    let function_name = format!("postrust_notify_{}_{}_fn", schema, table);

    format!(
        r#"
-- Create notification function
CREATE OR REPLACE FUNCTION {schema}.{function_name}()
RETURNS TRIGGER AS $$
DECLARE
    payload jsonb;
BEGIN
    IF TG_OP = 'DELETE' THEN
        payload := jsonb_build_object(
            'operation', 'DELETE',
            'table', TG_TABLE_NAME,
            'schema', TG_TABLE_SCHEMA,
            'old', row_to_json(OLD)
        );
    ELSIF TG_OP = 'UPDATE' THEN
        payload := jsonb_build_object(
            'operation', 'UPDATE',
            'table', TG_TABLE_NAME,
            'schema', TG_TABLE_SCHEMA,
            'old', row_to_json(OLD),
            'new', row_to_json(NEW)
        );
    ELSIF TG_OP = 'INSERT' THEN
        payload := jsonb_build_object(
            'operation', 'INSERT',
            'table', TG_TABLE_NAME,
            'schema', TG_TABLE_SCHEMA,
            'new', row_to_json(NEW)
        );
    END IF;

    PERFORM pg_notify('{channel}', payload::text);

    RETURN COALESCE(NEW, OLD);
END;
$$ LANGUAGE plpgsql;

-- Create trigger
DROP TRIGGER IF EXISTS {trigger_name} ON {schema}.{table};
CREATE TRIGGER {trigger_name}
    AFTER INSERT OR UPDATE OR DELETE ON {schema}.{table}
    FOR EACH ROW
    EXECUTE FUNCTION {schema}.{function_name}();
"#,
        schema = schema,
        table = table,
        channel = channel,
        function_name = function_name,
        trigger_name = trigger_name
    )
}

/// Generate SQL to drop a notification trigger for a table.
pub fn drop_notify_trigger_sql(schema: &str, table: &str) -> String {
    let trigger_name = format!("postrust_notify_{}_{}", schema, table);
    let function_name = format!("postrust_notify_{}_{}_fn", schema, table);

    format!(
        r#"
DROP TRIGGER IF EXISTS {trigger_name} ON {schema}.{table};
DROP FUNCTION IF EXISTS {schema}.{function_name}();
"#,
        schema = schema,
        table = table,
        trigger_name = trigger_name,
        function_name = function_name
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_table_channel_name() {
        assert_eq!(
            table_channel_name("public", "users"),
            "postrust_public_users"
        );
        assert_eq!(
            table_channel_name("api", "orders"),
            "postrust_api_orders"
        );
    }

    #[test]
    fn test_create_notify_trigger_sql() {
        let sql = create_notify_trigger_sql("public", "users");
        assert!(sql.contains("CREATE OR REPLACE FUNCTION"));
        assert!(sql.contains("postrust_notify_public_users_fn"));
        assert!(sql.contains("CREATE TRIGGER"));
        assert!(sql.contains("pg_notify"));
        assert!(sql.contains("postrust_public_users"));
    }

    #[test]
    fn test_drop_notify_trigger_sql() {
        let sql = drop_notify_trigger_sql("public", "users");
        assert!(sql.contains("DROP TRIGGER IF EXISTS"));
        assert!(sql.contains("DROP FUNCTION IF EXISTS"));
    }
}
