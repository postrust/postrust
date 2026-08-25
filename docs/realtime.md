# Realtime Subscriptions

A subscription in Postrust is a **live query**: the client asks the question
once and is answered whenever the answer changes. It carries the same fields
the query root does, takes the same arguments, and comes back with the same
rows — the only difference is that it keeps coming back.

What wakes it is PostgreSQL's own `LISTEN`/`NOTIFY`. A trigger publishes on the
changed table, the server reads the query again, and it sends the new answer
only if it differs from the one it sent last.

## Overview

Subscriptions let an application:

- keep a view of the data current without polling for it
- build reactive dashboards and UIs
- implement collaborative features
- know that what is on screen is what is in the database

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

**A subscription is a live query.** `subscription { orders(where: {status:
{_eq: "open"}}, order_by: [{id: desc}], limit: 20) { id total } }` answers
immediately with those twenty rows, and answers again with the whole current
set whenever it stops being right. It is the same contract Hasura's
subscriptions have, and a client generated against one works against this.

The subscription root mirrors the query root, field for field:

| Field | What it answers |
|---|---|
| `orders(where:, order_by:, distinct_on:, limit:, offset:)` | the rows, again on every change |
| `orders_by_pk(id: 1)` | one row, again whenever it changes |
| `orders_aggregate(where: …)` | `aggregate { count sum { … } }`, again on every change |

**Two things wake it.** A trigger on the table publishes when a row is written,
which is instant and costs nothing while nothing is being written. Beside it, a
slow refresh re-reads the query every `PGRST_SUBSCRIPTION_REFRESH` seconds — 30
by default — because a trigger cannot see everything a query can:

- a **view** has no rows of its own for a trigger to fire on;
- an **embedded row** may live in a table that carries no trigger, so
  `orders { customer { name } }` would not notice the customer being renamed;
- a predicate written against the **clock** — `where: {expires_at: {_lt:
  "now()"}}` — changes with no write at all.

Set the refresh to `0` to turn it off and leave only the notifications, which
is the right setting when every subscribable table carries a trigger and no
subscription depends on time. Set it lower to close the gap faster, at the cost
of one query per subscriber per interval.

**A wake is not a message.** Every wake re-reads the query and compares the
answer with the one last sent; a write that changes a row the subscription does
not select sends nothing. Clients see a message only when what they are looking
at actually changed.

## Enabling subscriptions

Subscriptions are served over WebSocket wherever GraphQL is:

```
ws://localhost:3000/v1/graphql/ws
```

`ws://localhost:3000/api/graphql/ws` serves the same thing.

Every table the query root exposes has a subscription, whether or not it
carries a trigger: without one it is answered by the refresh alone, which is
slower and works. Adding the trigger is what makes it instant. See [PostgreSQL
setup](#postgresql-setup).

## GraphQL subscriptions

The field is named after the table and takes what the query field takes:

```graphql
subscription {
  orders(
    where: { status: { _eq: "open" } }
    order_by: [{ created_at: desc }]
    limit: 20
  ) {
    id
    total
    status
    customer { name }
  }
}
```

Every message carries the whole current answer — the twenty rows as they are
now — not a delta. That is what makes a live query simple to hold on the client
side: replace what you had with what arrived.

### Narrowing what arrives

`where` narrows the answer, exactly as it does on a query. Narrowing what
*wakes* the server is a second thing, and it happens in the trigger's `WHEN`
clause: a row that does not qualify never becomes a notification, so the query
is not re-read for it at all.

```sql
CREATE TRIGGER postrust_notify_public_orders
    AFTER INSERT OR UPDATE OR DELETE ON public.orders
    FOR EACH ROW
    WHEN (COALESCE(NEW.total, OLD.total) > 1000)
    EXECUTE FUNCTION public.postrust_notify_public_orders_fn();
```

The two are independent: a `WHEN` clause that is too narrow makes a
subscription miss changes until the refresh catches them, and one that is too
wide only costs a re-read that sends nothing.

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
a large `jsonb` column, will not fit. Nothing here reads the payload — it is
the *signal* that matters, and the answer is read from the table afterwards —
so a trigger that publishes only the key is enough, and safer:

```sql
PERFORM pg_notify('postrust_public_orders', jsonb_build_object(
    'operation', TG_OP,
    'table', TG_TABLE_NAME,
    'schema', TG_TABLE_SCHEMA
)::text);
```

A notification is delivered when the transaction commits, and is not delivered
at all if it rolls back. That is the behaviour you want, and it means a
subscriber sees nothing from a mutation that failed.

`NOTIFY` is not replicated to a physical standby. Every server instance serving
subscriptions has to be connected to the primary; one pointed at a read replica
would be answered by the refresh alone.

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
        orders(where: { status: { _eq: "open" } }, limit: 20) {
          id
          total
          status
        }
      }
    `,
  },
  {
    next: (data) => console.log('Open orders now:', data.data.orders),
    error: (err) => console.error('Subscription error:', err),
    complete: () => console.log('Subscription complete'),
  }
);

// Later: unsubscribe()
```

### React with Apollo Client

Every message is the whole current answer, so the component renders what
arrived rather than accumulating it:

```tsx
import { useSubscription, gql } from '@apollo/client';

const OPEN_ORDERS = gql`
  subscription OpenOrders {
    orders(where: { status: { _eq: "open" } }, order_by: [{ id: desc }]) {
      id
      total
      status
    }
  }
`;

function OrderList() {
  const { data, error } = useSubscription(OPEN_ORDERS);

  if (error) return <p>Error: {error.message}</p>;
  return (
    <ul>
      {(data?.orders ?? []).map((order) => (
        <li key={order.id}>Order #{order.id} — {order.status}</li>
      ))}
    </ul>
  );
}
```

### React with urql

```tsx
import { useSubscription } from 'urql';

const OpenOrders = `
  subscription {
    orders(where: { status: { _eq: "open" } }) { id total status }
  }
`;

function OrderList() {
  const [result] = useSubscription({ query: OpenOrders });

  if (result.error) return <p>Error!</p>;
  return <ul>{(result.data?.orders ?? []).map((o) => <li key={o.id}>#{o.id}</li>)}</ul>;
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

**Row-level security applies, because the answer is a query.** Each re-read
runs as the subscriber's own role in its own transaction, so a policy filters a
live query exactly as it filters the query it was written from. A notification
only says "look again"; it never carries a row to a client that could not have
read it.

## Performance

- **One PostgreSQL connection carries every notification**, whatever the
  number of subscribers or of server instances: `NOTIFY` is a broadcast, and
  each instance holds one `LISTEN` connection. Adding an instance costs one
  connection and a copy of the signal.
- **What a wake costs is one re-read of that subscriber's query.** Idle costs
  nothing: with no writes and the refresh off, a thousand subscribers generate
  no database work at all.
- **Publish narrowly.** A trigger `WHEN` clause is the cheapest filter
  available: it stops the wake from existing, so the query is never re-read.
- **Raise `PGRST_SUBSCRIPTION_REFRESH`** where every subscribable table has a
  trigger and nothing depends on the clock — or set it to `0`. It is a floor
  under correctness, not the main mechanism, and every tick is a query per
  subscriber.
- **Narrow the subscription itself.** A `limit` bounds what is re-read and what
  crosses the socket, and an answer that is smaller is also less likely to have
  changed.
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

**The first answer arrives and nothing else does.** That is the trigger, not
the subscription: without one, changes are noticed by the refresh, so the delay
is up to `PGRST_SUBSCRIPTION_REFRESH` seconds. Check the trigger exists and
that its channel name matches `postrust_<schema>_<table>` exactly, then watch it
by hand:

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
