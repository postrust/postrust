# Realtime Subscriptions

Postrust provides realtime data synchronization through GraphQL subscriptions, powered by PostgreSQL's native `LISTEN/NOTIFY` mechanism. Subscribe to changes on any table or view and receive updates instantly.

## Overview

Realtime subscriptions allow your application to:

- Receive live updates when data changes
- Build reactive dashboards and UIs
- Implement collaborative features
- Stream data to clients without polling

## Architecture

```
┌──────────────┐     WebSocket      ┌──────────────┐   LISTEN/NOTIFY   ┌──────────────┐
│   Client     │◀──────────────────▶│   Postrust   │◀────────────────▶│  PostgreSQL  │
│  (Browser)   │                    │   Server     │                   │   Database   │
└──────────────┘                    └──────────────┘                   └──────────────┘
                                          │
                                    GraphQL Subscriptions
                                    over WebSocket
```

## Enabling Subscriptions

Subscriptions are enabled by default when using the GraphQL endpoint. Connect via WebSocket to start subscribing:

```
ws://localhost:3000/graphql
```

## GraphQL Subscriptions

### Basic Subscription

Subscribe to all changes on a table:

```graphql
subscription {
  users {
    id
    name
    email
    updatedAt
  }
}
```

### Filtered Subscriptions

Subscribe to specific records:

```graphql
subscription {
  orders(filter: { status: { eq: "pending" } }) {
    id
    total
    status
    customer {
      name
      email
    }
  }
}
```

### Subscribe to Views

Subscribe to PostgreSQL views for computed/aggregated data:

```graphql
subscription {
  salesDashboard {
    totalRevenue
    orderCount
    averageOrderValue
    topProducts {
      name
      salesCount
    }
  }
}
```

## Client Integration

### JavaScript/TypeScript

Using `graphql-ws`:

```typescript
import { createClient } from 'graphql-ws';

const client = createClient({
  url: 'ws://localhost:3000/graphql',
  connectionParams: {
    authorization: `Bearer ${token}`,
  },
});

// Subscribe to orders
const unsubscribe = client.subscribe(
  {
    query: `
      subscription {
        orders(filter: { status: { eq: "pending" } }) {
          id
          total
          status
        }
      }
    `,
  },
  {
    next: (data) => {
      console.log('Order update:', data);
    },
    error: (err) => {
      console.error('Subscription error:', err);
    },
    complete: () => {
      console.log('Subscription complete');
    },
  }
);

// Later: unsubscribe()
```

### React with Apollo Client

```tsx
import { useSubscription, gql } from '@apollo/client';

const ORDERS_SUBSCRIPTION = gql`
  subscription OnOrderUpdate {
    orders(filter: { status: { eq: "pending" } }) {
      id
      total
      status
      customer { name }
    }
  }
`;

function PendingOrders() {
  const { data, loading, error } = useSubscription(ORDERS_SUBSCRIPTION);

  if (loading) return <p>Connecting...</p>;
  if (error) return <p>Error: {error.message}</p>;

  return (
    <ul>
      {data.orders.map(order => (
        <li key={order.id}>
          Order #{order.id} - ${order.total} ({order.customer.name})
        </li>
      ))}
    </ul>
  );
}
```

### React with urql

```tsx
import { useSubscription } from 'urql';

const OrdersSubscription = `
  subscription {
    orders(filter: { status: { eq: "pending" } }) {
      id
      total
      status
    }
  }
`;

function OrdersList() {
  const [result] = useSubscription({ query: OrdersSubscription });

  if (result.fetching) return <p>Loading...</p>;
  if (result.error) return <p>Error!</p>;

  return (
    <ul>
      {result.data.orders.map(order => (
        <li key={order.id}>Order #{order.id}</li>
      ))}
    </ul>
  );
}
```

## PostgreSQL Setup

### Trigger-Based Notifications

For fine-grained control, create triggers that publish changes:

```sql
-- Create notification function
CREATE OR REPLACE FUNCTION notify_table_change()
RETURNS TRIGGER AS $$
BEGIN
  PERFORM pg_notify(
    'table_change',
    json_build_object(
      'table', TG_TABLE_NAME,
      'operation', TG_OP,
      'data', CASE
        WHEN TG_OP = 'DELETE' THEN row_to_json(OLD)
        ELSE row_to_json(NEW)
      END
    )::text
  );
  RETURN COALESCE(NEW, OLD);
END;
$$ LANGUAGE plpgsql;

-- Apply to tables
CREATE TRIGGER orders_notify
  AFTER INSERT OR UPDATE OR DELETE ON orders
  FOR EACH ROW EXECUTE FUNCTION notify_table_change();
```

### Filtered Notifications

Only notify for specific conditions:

```sql
CREATE OR REPLACE FUNCTION notify_high_value_order()
RETURNS TRIGGER AS $$
BEGIN
  IF NEW.total > 1000 THEN
    PERFORM pg_notify(
      'high_value_order',
      row_to_json(NEW)::text
    );
  END IF;
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER high_value_orders_notify
  AFTER INSERT ON orders
  FOR EACH ROW EXECUTE FUNCTION notify_high_value_order();
```

## Real-World Examples

### Live Dashboard

```graphql
subscription LiveMetrics {
  dashboardMetrics {
    activeUsers
    ordersPerMinute
    revenue {
      today
      thisWeek
      thisMonth
    }
    recentOrders(limit: 5) {
      id
      total
      customer { name }
    }
  }
}
```

### Chat Application

```graphql
subscription ChatMessages($roomId: Int!) {
  messages(filter: { roomId: { eq: $roomId } }, orderBy: { createdAt: DESC }) {
    id
    content
    createdAt
    author {
      id
      name
      avatarUrl
    }
  }
}
```

### Collaborative Editing

```graphql
subscription DocumentChanges($documentId: Int!) {
  documentEdits(filter: { documentId: { eq: $documentId } }) {
    id
    content
    cursorPosition
    lastEditedBy {
      id
      name
    }
    updatedAt
  }
}
```

### Notifications Feed

```graphql
subscription UserNotifications($userId: Int!) {
  notifications(
    filter: { userId: { eq: $userId }, read: { eq: false } }
    orderBy: { createdAt: DESC }
  ) {
    id
    type
    message
    createdAt
    relatedEntity {
      ... on Order { id total }
      ... on Comment { id content }
    }
  }
}
```

## Authentication

Subscriptions respect the same JWT authentication as queries:

```typescript
const client = createClient({
  url: 'ws://localhost:3000/graphql',
  connectionParams: () => ({
    authorization: `Bearer ${getToken()}`,
  }),
});
```

Row-Level Security policies apply to subscriptions:

```sql
-- Users only receive updates for their own orders
CREATE POLICY orders_subscription ON orders
  FOR SELECT
  USING (user_id = current_setting('request.jwt.claims')::json->>'sub');
```

## Performance Considerations

### Connection Limits

Configure maximum WebSocket connections:

```env
POSTRUST_MAX_SUBSCRIPTIONS=1000
POSTRUST_SUBSCRIPTION_TIMEOUT=300  # seconds
```

### Debouncing Updates

For high-frequency changes, consider debouncing at the database level:

```sql
-- Aggregate changes over a time window
CREATE OR REPLACE FUNCTION debounced_notify()
RETURNS TRIGGER AS $$
DECLARE
  last_notify timestamptz;
BEGIN
  -- Check last notification time
  SELECT last_notified INTO last_notify
  FROM notification_state
  WHERE table_name = TG_TABLE_NAME;

  -- Only notify if more than 100ms since last
  IF last_notify IS NULL OR
     NOW() - last_notify > interval '100 milliseconds' THEN
    PERFORM pg_notify(TG_TABLE_NAME || '_change', '');
    INSERT INTO notification_state (table_name, last_notified)
    VALUES (TG_TABLE_NAME, NOW())
    ON CONFLICT (table_name) DO UPDATE SET last_notified = NOW();
  END IF;

  RETURN NEW;
END;
$$ LANGUAGE plpgsql;
```

### Subscription Best Practices

1. **Use filters**: Subscribe to specific records, not entire tables
2. **Select only needed fields**: Minimize payload size
3. **Implement reconnection logic**: Handle network interruptions gracefully
4. **Unsubscribe when done**: Clean up subscriptions to free resources

## Troubleshooting

### Connection Issues

```typescript
const client = createClient({
  url: 'ws://localhost:3000/graphql',
  retryAttempts: 5,
  retryWait: async (retries) => {
    await new Promise(r => setTimeout(r, retries * 1000));
  },
  on: {
    connected: () => console.log('Connected'),
    closed: () => console.log('Disconnected'),
    error: (err) => console.error('Error:', err),
  },
});
```

### Debugging Subscriptions

Enable debug logging:

```env
RUST_LOG=postrust=debug
```

## Next Steps

- See [GraphQL](./graphql.md) for query and mutation documentation
- See [Authentication](./authentication.md) for JWT configuration
- See [Custom Routes](./custom-routes.md) for extending functionality
