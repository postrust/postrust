# Realtime Subscriptions

Postrust streams table changes to clients over GraphQL subscriptions, carried
by PostgreSQL's own `LISTEN`/`NOTIFY`. A trigger publishes the changed row; the
server is listening; the client sees it.

## Overview

Subscriptions let an application:

- receive live updates when data changes
- build reactive dashboards and UIs
- implement collaborative features
- stream data to clients without polling

## Architecture

```
┌──────────────┐     WebSocket      ┌──────────────┐   LISTEN/NOTIFY   ┌──────────────┐
│   Client     │◀──────────────────▶│   Postrust   │◀─────────────────▶│  PostgreSQL  │
│  (Browser)   │                    │   Server     │                   │   Database   │
└──────────────┘                    └──────────────┘                   └──────────────┘
```

One connection to PostgreSQL is shared by every subscriber: the server listens
on a channel per table and fans each notification out to whoever asked for it.

## What a subscription is here

**A subscription streams the rows that changed, one notification at a time.**
It is not a live query: there is no initial result, no `where`, no `order_by`,
no `limit`, and no aggregate. The field takes no arguments at all, and each
message is the row a trigger published.

That is worth being plain about, because the two models look alike and behave
differently. A live query re-runs when anything it depends on changes and
answers with the whole current result; this answers with one row, when that row
changes. If you need the current state at connect time, query for it and then
subscribe.

Views are not subscribable — a view has no rows of its own for a trigger to
fire on.

## Enabling subscriptions

Subscriptions are served over WebSocket wherever GraphQL is:

```
ws://localhost:3000/v1/graphql/ws
```

`ws://localhost:3000/api/graphql/ws` serves the same thing.

The server listens on a channel per table at startup, so a table needs its
trigger before anything is streamed from it. See [PostgreSQL
setup](#postgresql-setup).

## GraphQL subscriptions

The field is named after the table, and its type is the table's type — the same
type a query returns, so the same fields select from it:

```graphql
subscription {
  orders {
    id
    total
    status
  }
}
```

Each message carries one row: the new row for an insert or an update, the old
row for a delete.

### Narrowing what arrives

Since the field takes no arguments, narrowing happens where the notification is
published — in the trigger's `WHEN` clause. This is stricter than a client-side
filter and cheaper than either: a row that does not qualify never becomes a
notification, never crosses the socket, and never wakes the client.

```sql
CREATE TRIGGER postrust_notify_public_orders
    AFTER INSERT OR UPDATE OR DELETE ON public.orders
    FOR EACH ROW
    WHEN (COALESCE(NEW.total, OLD.total) > 1000)
    EXECUTE FUNCTION public.postrust_notify_public_orders_fn();
```

## PostgreSQL setup

The server listens on `postrust_<schema>_<table>` and expects a JSON payload
with `operation`, `table`, `schema` and the row under `old` or `new`. This is
the trigger that produces it:

```sql
CREATE OR REPLACE FUNCTION public.postrust_notify_public_orders_fn()
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

    PERFORM pg_notify('postrust_public_orders', payload::text);

    RETURN COALESCE(NEW, OLD);
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS postrust_notify_public_orders ON public.orders;
CREATE TRIGGER postrust_notify_public_orders
    AFTER INSERT OR UPDATE OR DELETE ON public.orders
    FOR EACH ROW
    EXECUTE FUNCTION public.postrust_notify_public_orders_fn();
```

`postrust_graphql::subscription::create_notify_trigger_sql(schema, table)`
generates exactly this, if you would rather not write it per table.

### Two limits worth knowing

`pg_notify` refuses a payload over 8000 bytes — it raises `payload string too
long`, which fails the transaction that wrote the row. A wide row, or one with
a large `jsonb` column, will not fit: publish the key and let the client fetch
the rest.

A notification is delivered when the transaction commits, and is not delivered
at all if it rolls back. That is the behaviour you want, and it means a
subscriber sees nothing from a mutation that failed.

## Client integration

### JavaScript/TypeScript

Using `graphql-ws`:

```typescript
import { createClient } from 'graphql-ws';

const client = createClient({
  url: 'ws://localhost:3000/v1/graphql/ws',
  connectionParams: {
    authorization: `Bearer ${token}`,
  },
});

const unsubscribe = client.subscribe(
  {
    query: `
      subscription {
        orders {
          id
          total
          status
        }
      }
    `,
  },
  {
    next: (data) => console.log('Order changed:', data),
    error: (err) => console.error('Subscription error:', err),
    complete: () => console.log('Subscription complete'),
  }
);

// Later: unsubscribe()
```

### React with Apollo Client

```tsx
import { useSubscription, gql } from '@apollo/client';

const ORDER_CHANGES = gql`
  subscription OnOrderChange {
    orders {
      id
      total
      status
    }
  }
`;

function OrderFeed() {
  const [seen, setSeen] = useState<Order[]>([]);
  const { error } = useSubscription(ORDER_CHANGES, {
    onData: ({ data }) => setSeen((rows) => [data.data.orders, ...rows]),
  });

  if (error) return <p>Error: {error.message}</p>;

  return (
    <ul>
      {seen.map((order) => (
        <li key={order.id}>Order #{order.id} — {order.status}</li>
      ))}
    </ul>
  );
}
```

Each message is one row, so the client accumulates them. Apollo's cache will
not assemble a list for you here the way it does for a live query.

### React with urql

```tsx
import { useSubscription } from 'urql';

const OrderChanges = `
  subscription {
    orders { id total status }
  }
`;

function OrderList() {
  const [result] = useSubscription(
    { query: OrderChanges },
    (rows: Order[] = [], data) => [data.orders, ...rows]
  );

  if (result.error) return <p>Error!</p>;
  return <ul>{(result.data ?? []).map((o) => <li key={o.id}>#{o.id}</li>)}</ul>;
}
```

## Authentication

Subscriptions use the same JWT as queries, sent in the connection parameters:

```typescript
const client = createClient({
  url: 'ws://localhost:3000/v1/graphql/ws',
  connectionParams: () => ({
    authorization: `Bearer ${getToken()}`,
  }),
});
```

**Row-level security does not filter a notification.** A policy is evaluated
against a query, and a notification is not a query — it comes from a trigger
that ran as whoever wrote the row. A subscriber on a table therefore sees every
change published on it. Where that matters, publish only what everyone may see:
narrow the trigger, or notify a key and let the client read the row back
through a query, where the policy does apply.

## Performance

- **One PostgreSQL connection** serves every subscriber. Adding subscribers
  costs a channel entry and a broadcast, not a connection.
- **Publish narrowly.** A trigger `WHEN` clause is the cheapest filter
  available: it stops the notification from existing.
- **Select only the fields you need** — the payload is what the trigger built,
  but what crosses the socket is what the selection asked for.
- **Debounce at the database** for high-frequency tables, by making the trigger
  statement-level or by rate-limiting inside it.

```sql
-- One notification per statement rather than per row
CREATE TRIGGER postrust_notify_public_ticks
    AFTER INSERT OR UPDATE OR DELETE ON public.ticks
    FOR EACH STATEMENT
    EXECUTE FUNCTION public.postrust_notify_public_ticks_fn();
```

## Troubleshooting

**Nothing arrives.** Check the trigger exists and that its channel name matches
`postrust_<schema>_<table>` exactly, then watch it by hand:

```sql
LISTEN postrust_public_orders;
-- then, from another session, change a row
```

**The server is not listening on the channel.** Channels are collected at
startup from the tables in the exposed schemas. A table created since the
server started has no channel; restart it, or reload the schema cache.

**Connection drops.** Reconnect with backoff — `graphql-ws` does this for you:

```typescript
const client = createClient({
  url: 'ws://localhost:3000/v1/graphql/ws',
  retryAttempts: 5,
  retryWait: async (retries) => {
    await new Promise((r) => setTimeout(r, retries * 1000));
  },
  on: {
    connected: () => console.log('Connected'),
    closed: () => console.log('Disconnected'),
    error: (err) => console.error('Error:', err),
  },
});
```

**Debug logging:**

```env
RUST_LOG=postrust=debug,postrust_graphql=debug
```

## Next Steps

- [GraphQL API](./api-reference.md#graphql-api) — queries and mutations
- [Authentication](./authentication.md) — JWT configuration
- [Custom Routes](./custom-routes.md) — extending functionality
