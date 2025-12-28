import { component$ } from "@builder.io/qwik";
import type { DocumentHead } from "@builder.io/qwik-city";
import { Link } from "@builder.io/qwik-city";

export default component$(() => {
  return (
    <div class="min-h-screen bg-white">
      <div class="bg-gradient-to-b from-neutral-50 to-white border-b border-neutral-200">
        <div class="container-wide py-12">
          <div class="flex items-center gap-2 text-sm text-neutral-500 mb-4">
            <Link href="/docs" class="hover:text-primary-600">Docs</Link>
            <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 5l7 7-7 7"/>
            </svg>
            <span class="text-neutral-900">Realtime</span>
          </div>
          <h1 class="text-4xl font-bold text-neutral-900 mb-4">Realtime Subscriptions</h1>
          <p class="text-lg text-neutral-600 max-w-2xl">
            Live data synchronization through GraphQL subscriptions powered by PostgreSQL LISTEN/NOTIFY.
          </p>
        </div>
      </div>

      <div class="container-wide py-12">
        <div class="max-w-3xl">
          {/* Overview */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">Overview</h2>
            <p class="text-neutral-600 mb-4">
              Postrust provides realtime data synchronization through GraphQL subscriptions.
              Subscribe to changes on any table or view and receive updates instantly.
            </p>
            <ul class="list-disc list-inside text-neutral-600 space-y-2">
              <li>Live updates when data changes</li>
              <li>Reactive dashboards and UIs</li>
              <li>Collaborative features</li>
              <li>No polling required</li>
            </ul>
          </section>

          {/* Architecture */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">Architecture</h2>
            <div class="bg-neutral-900 rounded-xl overflow-hidden">
              <div class="px-4 py-2 bg-neutral-800 border-b border-neutral-700">
                <span class="text-sm text-neutral-400">Architecture</span>
              </div>
              <pre class="p-4 text-sm overflow-x-auto">
                <code class="text-neutral-100">{`┌──────────────┐   WebSocket   ┌──────────────┐   LISTEN/NOTIFY   ┌────────────┐
│   Client     │◀────────────▶│   Postrust   │◀────────────────▶│  PostgreSQL│
│  (Browser)   │              │   Server     │                   │  Database  │
└──────────────┘              └──────────────┘                   └────────────┘`}</code>
              </pre>
            </div>
          </section>

          {/* Connection */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">Connecting</h2>
            <p class="text-neutral-600 mb-4">
              Connect via WebSocket to the GraphQL endpoint:
            </p>
            <div class="p-4 bg-neutral-50 rounded-lg">
              <code class="font-mono text-primary-600">ws://localhost:3000/graphql</code>
            </div>
          </section>

          {/* Basic Subscription */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">Basic Subscription</h2>
            <div class="bg-neutral-900 rounded-xl overflow-hidden">
              <div class="px-4 py-2 bg-neutral-800 border-b border-neutral-700">
                <span class="text-sm text-neutral-400">GraphQL</span>
              </div>
              <pre class="p-4 text-sm overflow-x-auto">
                <code class="text-neutral-100">{`# Subscribe to all changes on a table
subscription {
  users {
    id
    name
    email
    updatedAt
  }
}

# Subscribe with filters
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
}`}</code>
              </pre>
            </div>
          </section>

          {/* Subscribe to Views */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">Subscribe to Views</h2>
            <p class="text-neutral-600 mb-4">
              Subscribe to PostgreSQL views for computed or aggregated data:
            </p>
            <div class="bg-neutral-900 rounded-xl overflow-hidden">
              <div class="px-4 py-2 bg-neutral-800 border-b border-neutral-700">
                <span class="text-sm text-neutral-400">GraphQL</span>
              </div>
              <pre class="p-4 text-sm overflow-x-auto">
                <code class="text-neutral-100">{`subscription {
  salesDashboard {
    totalRevenue
    orderCount
    averageOrderValue
    topProducts {
      name
      salesCount
    }
  }
}`}</code>
              </pre>
            </div>
          </section>

          {/* JavaScript Client */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">JavaScript Client</h2>
            <div class="bg-neutral-900 rounded-xl overflow-hidden">
              <div class="px-4 py-2 bg-neutral-800 border-b border-neutral-700">
                <span class="text-sm text-neutral-400">TypeScript</span>
              </div>
              <pre class="p-4 text-sm overflow-x-auto">
                <code class="text-neutral-100">{`import { createClient } from 'graphql-ws';

const client = createClient({
  url: 'ws://localhost:3000/graphql',
  connectionParams: {
    authorization: \`Bearer \${token}\`,
  },
});

// Subscribe to orders
const unsubscribe = client.subscribe(
  {
    query: \`
      subscription {
        orders(filter: { status: { eq: "pending" } }) {
          id
          total
          status
        }
      }
    \`,
  },
  {
    next: (data) => console.log('Update:', data),
    error: (err) => console.error('Error:', err),
    complete: () => console.log('Done'),
  }
);

// Later: unsubscribe()`}</code>
              </pre>
            </div>
          </section>

          {/* React Example */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">React with Apollo</h2>
            <div class="bg-neutral-900 rounded-xl overflow-hidden">
              <div class="px-4 py-2 bg-neutral-800 border-b border-neutral-700">
                <span class="text-sm text-neutral-400">React</span>
              </div>
              <pre class="p-4 text-sm overflow-x-auto">
                <code class="text-neutral-100">{`import { useSubscription, gql } from '@apollo/client';

const ORDERS_SUBSCRIPTION = gql\`
  subscription OnOrderUpdate {
    orders(filter: { status: { eq: "pending" } }) {
      id
      total
      status
      customer { name }
    }
  }
\`;

function PendingOrders() {
  const { data, loading, error } = useSubscription(ORDERS_SUBSCRIPTION);

  if (loading) return <p>Connecting...</p>;
  if (error) return <p>Error: {error.message}</p>;

  return (
    <ul>
      {data.orders.map(order => (
        <li key={order.id}>
          Order #{order.id} - \${order.total}
        </li>
      ))}
    </ul>
  );
}`}</code>
              </pre>
            </div>
          </section>

          {/* PostgreSQL Triggers */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">PostgreSQL Triggers</h2>
            <p class="text-neutral-600 mb-4">
              Create triggers for fine-grained notifications:
            </p>
            <div class="bg-neutral-900 rounded-xl overflow-hidden">
              <div class="px-4 py-2 bg-neutral-800 border-b border-neutral-700">
                <span class="text-sm text-neutral-400">SQL</span>
              </div>
              <pre class="p-4 text-sm overflow-x-auto">
                <code class="text-neutral-100">{`-- Notification function
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

-- Apply to table
CREATE TRIGGER orders_notify
  AFTER INSERT OR UPDATE OR DELETE ON orders
  FOR EACH ROW EXECUTE FUNCTION notify_table_change();`}</code>
              </pre>
            </div>
          </section>

          {/* Use Cases */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">Use Cases</h2>
            <div class="grid md:grid-cols-2 gap-4">
              <div class="p-4 bg-neutral-50 rounded-lg">
                <h3 class="font-semibold text-neutral-900 mb-2">Live Dashboards</h3>
                <p class="text-sm text-neutral-600">Real-time metrics, KPIs, and analytics updates</p>
              </div>
              <div class="p-4 bg-neutral-50 rounded-lg">
                <h3 class="font-semibold text-neutral-900 mb-2">Chat Applications</h3>
                <p class="text-sm text-neutral-600">Instant message delivery and presence</p>
              </div>
              <div class="p-4 bg-neutral-50 rounded-lg">
                <h3 class="font-semibold text-neutral-900 mb-2">Collaborative Editing</h3>
                <p class="text-sm text-neutral-600">Multi-user document editing and cursors</p>
              </div>
              <div class="p-4 bg-neutral-50 rounded-lg">
                <h3 class="font-semibold text-neutral-900 mb-2">Notifications</h3>
                <p class="text-sm text-neutral-600">Push alerts and activity feeds</p>
              </div>
            </div>
          </section>

          {/* Best Practices */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">Best Practices</h2>
            <ul class="space-y-3">
              {[
                { title: "Use filters", desc: "Subscribe to specific records, not entire tables" },
                { title: "Select only needed fields", desc: "Minimize payload size" },
                { title: "Handle reconnection", desc: "Implement retry logic for network issues" },
                { title: "Clean up subscriptions", desc: "Unsubscribe when components unmount" },
              ].map((item) => (
                <li key={item.title} class="flex items-start gap-3">
                  <svg class="w-5 h-5 text-primary-600 mt-0.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 13l4 4L19 7"/>
                  </svg>
                  <div>
                    <span class="font-medium text-neutral-900">{item.title}</span>
                    <span class="text-neutral-600"> - {item.desc}</span>
                  </div>
                </li>
              ))}
            </ul>
          </section>

          {/* Next */}
          <div class="flex items-center justify-between pt-8 border-t border-neutral-200">
            <Link
              href="/docs/pgvector"
              class="flex items-center gap-2 text-neutral-600 hover:text-primary-600"
            >
              <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 19l-7-7 7-7"/>
              </svg>
              pgvector
            </Link>
            <Link
              href="/docs/custom-routes"
              class="flex items-center gap-2 text-neutral-600 hover:text-primary-600"
            >
              Custom Routes
              <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 5l7 7-7 7"/>
              </svg>
            </Link>
          </div>
        </div>
      </div>
    </div>
  );
});

export const head: DocumentHead = {
  title: "Realtime Subscriptions - Postrust Documentation",
  meta: [
    {
      name: "description",
      content: "Live data synchronization through GraphQL subscriptions powered by PostgreSQL LISTEN/NOTIFY.",
    },
  ],
};
