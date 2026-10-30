import { component$, useSignal } from "@builder.io/qwik";
import { emailOff } from "~/components/email-off";

/*
 * EXAMPLES ROADMAP:
 * -----------------
 * TODO: Add more real-world examples:
 * - [x] Aggregations (count, sum, avg)
 * - [ ] Full-text search
 * - [ ] Bulk insert/upsert
 * - [x] Stored procedures (RPC)
 * - [ ] File uploads with storage
 * - [x] Realtime subscriptions
 * - [x] Custom routes examples
 * - [x] pgvector / vector search
 * - [ ] Multi-tenant patterns
 * - [ ] Audit logging setup
 */

const codeExamples = [
  {
    id: "rest",
    label: "REST API",
    examples: [
      {
        title: "Query with Filters",
        code: `# Get active users with pagination
curl "localhost:3000/users?status=eq.active&limit=10&offset=0"

# Response
[
  {"id": 1, "name": "Alice", "email": "alice@example.com"},
  {"id": 2, "name": "Bob", "email": "bob@example.com"}
]`,
      },
      {
        title: "Nested Resources",
        code: `# Get orders with customer and items
curl "localhost:3000/orders?select=*,customer(name,email),items(product(name,price))"

# Response
[{
  "id": 1,
  "total": 99.99,
  "customer": {"name": "Alice", "email": "alice@example.com"},
  "items": [{"product": {"name": "Widget", "price": 49.99}}]
}]`,
      },
    ],
  },
  {
    id: "graphql",
    label: "GraphQL",
    examples: [
      {
        title: "Query with Relations",
        code: `query {
  users(filter: {status: {eq: "active"}}, limit: 10) {
    id
    name
    email
    orders {
      id
      total
      items {
        product { name price }
      }
    }
  }
}`,
      },
      {
        title: "Mutations",
        code: `mutation {
  insertUsers(objects: [
    {name: "Charlie", email: "charlie@example.com"}
  ]) {
    id
    name
    createdAt
  }
}`,
      },
    ],
  },
  {
    id: "crud",
    label: "CRUD Operations",
    examples: [
      {
        title: "Create & Update",
        code: `# Insert a new product
curl -X POST "localhost:3000/products" \\
  -H "Content-Type: application/json" \\
  -d '{"name": "Widget", "price": 29.99, "stock": 100}'

# Update with filters (PATCH)
curl -X PATCH "localhost:3000/products?id=eq.1" \\
  -H "Content-Type: application/json" \\
  -d '{"price": 24.99}'`,
      },
      {
        title: "Upsert & Delete",
        code: `# Upsert (insert or update on conflict)
curl -X POST "localhost:3000/products" \\
  -H "Prefer: resolution=merge-duplicates" \\
  -d '{"sku": "WDG-001", "name": "Widget Pro", "price": 39.99}'

# Delete with filters
curl -X DELETE "localhost:3000/products?status=eq.discontinued"`,
      },
    ],
  },
  {
    id: "auth",
    label: "Authentication",
    examples: [
      {
        title: "JWT Authentication",
        code: `# Include JWT in Authorization header
curl -H "Authorization: Bearer eyJhbGciOiJIUzI1NiIs..." \\
  "localhost:3000/users"

# JWT payload for PostgreSQL RLS
{
  "role": "authenticated",
  "sub": "user_123",
  "email": "alice@example.com"
}`,
      },
      {
        title: "Row-Level Security",
        code: `-- PostgreSQL RLS Policy
CREATE POLICY user_isolation ON orders
  FOR ALL
  USING (user_id = current_setting('request.jwt.claims')::json->>'sub');

-- Users can only see their own orders
-- Postrust enforces this automatically`,
      },
    ],
  },
  {
    id: "advanced",
    label: "Advanced",
    examples: [
      {
        title: "Aggregations",
        code: `# Count orders by status
curl "localhost:3000/orders?select=status,count()&group=status"

# Sum and average
curl "localhost:3000/order_items?select=product_id,sum(quantity),avg(price)"

# Response
[
  {"status": "pending", "count": 42},
  {"status": "shipped", "count": 156}
]`,
      },
      {
        title: "Stored Procedures (RPC)",
        code: `# Call a PostgreSQL function
curl -X POST "localhost:3000/rpc/calculate_shipping" \\
  -H "Content-Type: application/json" \\
  -d '{"order_id": 123, "destination": "US"}'

# Response
{"shipping_cost": 12.99, "estimated_days": 3}`,
      },
    ],
  },
  {
    id: "realtime",
    label: "Realtime",
    examples: [
      {
        title: "GraphQL Subscriptions",
        code: `subscription {
  orders(filter: {status: {eq: "pending"}}) {
    id
    total
    status
    customer { name email }
  }
}

# Receive live updates when orders change
{"data": {"orders": {"id": 42, "status": "shipped", ...}}}`,
      },
      {
        title: "Subscribe to Any View",
        code: `# Subscribe to a PostgreSQL view for live dashboards
subscription {
  salesDashboard {
    totalRevenue
    orderCount
    topProducts { name salesCount }
  }
}

# Updates stream in real-time as underlying data changes`,
      },
    ],
  },
  {
    id: "vector",
    label: "pgvector",
    examples: [
      {
        title: "Semantic Search",
        code: `# Find similar documents using embeddings
curl "localhost:3000/documents?select=id,title,content&embedding=lt.0.5&order=embedding.asc"

# Or via GraphQL with vector distance
query {
  documents(
    filter: {embedding: {near: {vector: [0.1, 0.2, ...], distance: 0.5}}}
    order: {embedding: ASC}
    limit: 10
  ) {
    id title content similarity
  }
}`,
      },
      {
        title: "RAG Pipeline",
        code: `-- PostgreSQL: Create embeddings table
CREATE TABLE documents (
  id SERIAL PRIMARY KEY,
  content TEXT,
  embedding vector(1536)  -- OpenAI ada-002 dimensions
);
CREATE INDEX ON documents USING ivfflat (embedding vector_cosine_ops);

# Query similar content for LLM context
curl "localhost:3000/rpc/search_similar" \\
  -d '{"query_embedding": [0.1, 0.2, ...], "limit": 5}'`,
      },
    ],
  },
  {
    id: "custom",
    label: "Custom Routes",
    examples: [
      {
        title: "Routes of Your Own",
        code: `// src/main.rs -- a crate depending on postrust-server, not a fork
use postrust_server::{AppState, Builder, Host, User};

async fn handle_stripe(Json(event): Json<Value>) -> Json<Value> {
    Json(json!({ "received": true }))
}

// Runs as the caller: their role, their claims, row-level security
async fn my_orders(
    user: User,
    State(state): State<Arc<AppState>>,
) -> Result<Json<Value>, postrust_server::Error> {
    let mut tx = state.begin_as(&user).await?;
    let (n,): (i64,) = sqlx::query_as("SELECT count(*) FROM orders")
        .fetch_one(&mut *tx).await?;
    tx.commit().await?;
    Ok(Json(json!({ "orders": n })))
}

let app = Builder::new(AppConfig::try_from_env()?, Host::SERVER)
    .nest("/hooks", Router::new()
        .route("/stripe", post(handle_stripe))
        .route("/orders", get(my_orders)))
    .build()
    .await?;`,
      },
      {
        title: "Custom Middleware",
        code: `// Wrap everything Postrust serves, its own API included
use axum::middleware;

let app = Builder::new(config, Host::SERVER)
    .nest("/hooks", hooks())
    .layer(middleware::from_fn(rate_limit))
    .build()
    .await?;

async fn rate_limit(req: Request, next: Next) -> Response {
    // Check rate limits, return 429 if exceeded
    next.run(req).await
}`,
      },
    ],
  },
];

export const CodeExamplesSection = component$(() => {
  const activeTab = useSignal("rest");

  return (
    <section class="section-padding bg-white">
      <div class="container-wide">
        {/* Section Header */}
        <div class="mx-auto mb-12 max-w-3xl text-center">
          <div class="bg-accent-100 text-accent-700 mb-4 inline-flex items-center gap-2 rounded-full px-3 py-1 text-sm font-medium">
            <svg
              class="h-4 w-4"
              fill="none"
              stroke="currentColor"
              viewBox="0 0 24 24"
            >
              <path
                stroke-linecap="round"
                stroke-linejoin="round"
                stroke-width="2"
                d="M10 20l4-16m4 4l4 4-4 4M6 16l-4-4 4-4"
              />
            </svg>
            Developer Experience
          </div>
          <h2 class="mb-4 text-3xl font-bold text-neutral-900 md:text-4xl">
            Intuitive API, powerful results
          </h2>
          <p class="text-lg text-neutral-600">
            Whether you prefer REST or GraphQL, Postrust provides a clean,
            consistent API that's easy to use and powerful enough for
            production.
          </p>
        </div>

        {/* Tab Navigation */}
        <div class="mb-8 flex justify-center">
          <div class="inline-flex rounded-lg bg-neutral-100 p-1">
            {codeExamples.map((tab) => (
              <button
                key={tab.id}
                onClick$={() => (activeTab.value = tab.id)}
                class={`rounded-md px-6 py-2.5 text-sm font-medium transition-all ${
                  activeTab.value === tab.id
                    ? "bg-white text-neutral-900 shadow-sm"
                    : "text-neutral-600 hover:text-neutral-900"
                }`}
              >
                {tab.label}
              </button>
            ))}
          </div>
        </div>

        {/* Code Examples */}
        {codeExamples.map((tab) => (
          <div
            key={tab.id}
            class={`grid gap-6 md:grid-cols-2 ${
              activeTab.value === tab.id ? "block" : "hidden"
            }`}
          >
            {tab.examples.map((example) => (
              <div
                key={example.title}
                class="overflow-hidden rounded-xl bg-neutral-900"
              >
                {/* Header */}
                <div class="flex items-center justify-between border-b border-neutral-700 bg-neutral-800 px-4 py-3">
                  <span class="text-sm font-medium text-neutral-300">
                    {example.title}
                  </span>
                  <button
                    type="button"
                    class="rounded p-1.5 text-neutral-400 transition-colors hover:bg-neutral-700 hover:text-white"
                    aria-label="Copy code"
                  >
                    <svg
                      class="h-4 w-4"
                      fill="none"
                      stroke="currentColor"
                      viewBox="0 0 24 24"
                    >
                      <path
                        stroke-linecap="round"
                        stroke-linejoin="round"
                        stroke-width="2"
                        d="M8 16H6a2 2 0 01-2-2V6a2 2 0 012-2h8a2 2 0 012 2v2m-6 12h8a2 2 0 002-2v-8a2 2 0 00-2-2h-8a2 2 0 00-2 2v8a2 2 0 002 2z"
                      />
                    </svg>
                  </button>
                </div>
                {/* Code */}
                <div class="overflow-x-auto p-4">
                  <pre class="font-mono text-sm leading-relaxed">
                    <code
                      class="whitespace-pre text-neutral-100"
                      dangerouslySetInnerHTML={emailOff(example.code)}
                    />
                  </pre>
                </div>
              </div>
            ))}
          </div>
        ))}

        {/* CTA */}
        <div class="mt-12 text-center">
          <a
            href="/docs/api-reference/"
            class="text-primary-600 hover:text-primary-700 inline-flex items-center gap-2 font-medium"
          >
            View full API reference
            <svg
              class="h-4 w-4"
              fill="none"
              stroke="currentColor"
              viewBox="0 0 24 24"
            >
              <path
                stroke-linecap="round"
                stroke-linejoin="round"
                stroke-width="2"
                d="M13 7l5 5m0 0l-5 5m5-5H6"
              />
            </svg>
          </a>
        </div>
      </div>
    </section>
  );
});
