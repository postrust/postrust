import { component$, useSignal } from "@builder.io/qwik";

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
        title: "Rust Handler Definition",
        code: `// crates/postrust-server/src/custom.rs
use axum::{extract::State, Json, routing::post};

pub fn custom_router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/webhooks/stripe", post(handle_stripe))
        .route("/ai/embed", post(generate_embedding))
}

async fn handle_stripe(
    State(state): State<Arc<AppState>>,
    body: String,
) -> impl IntoResponse {
    // Full access to database pool, config, JWT
    let result = sqlx::query("UPDATE subscriptions...")
        .execute(&state.pool).await;
    Json(json!({ "received": true }))
}`,
      },
      {
        title: "Custom Middleware",
        code: `// Add rate limiting, logging, or custom auth
use axum::middleware;

pub fn custom_router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/expensive", post(expensive_operation))
        .layer(middleware::from_fn(rate_limit))
        .layer(middleware::from_fn(custom_auth))
}

async fn rate_limit<B>(req: Request<B>, next: Next<B>) -> Response {
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
        <div class="text-center max-w-3xl mx-auto mb-12">
          <div class="inline-flex items-center gap-2 px-3 py-1 bg-accent-100 text-accent-700 rounded-full text-sm font-medium mb-4">
            <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M10 20l4-16m4 4l4 4-4 4M6 16l-4-4 4-4"/>
            </svg>
            Developer Experience
          </div>
          <h2 class="text-3xl md:text-4xl font-bold text-neutral-900 mb-4">
            Intuitive API, powerful results
          </h2>
          <p class="text-lg text-neutral-600">
            Whether you prefer REST or GraphQL, Postrust provides a clean,
            consistent API that's easy to use and powerful enough for production.
          </p>
        </div>

        {/* Tab Navigation */}
        <div class="flex justify-center mb-8">
          <div class="inline-flex p-1 bg-neutral-100 rounded-lg">
            {codeExamples.map((tab) => (
              <button
                key={tab.id}
                onClick$={() => activeTab.value = tab.id}
                class={`px-6 py-2.5 text-sm font-medium rounded-md transition-all ${
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
            class={`grid md:grid-cols-2 gap-6 ${
              activeTab.value === tab.id ? "block" : "hidden"
            }`}
          >
            {tab.examples.map((example) => (
              <div
                key={example.title}
                class="bg-neutral-900 rounded-xl overflow-hidden"
              >
                {/* Header */}
                <div class="flex items-center justify-between px-4 py-3 bg-neutral-800 border-b border-neutral-700">
                  <span class="text-sm font-medium text-neutral-300">
                    {example.title}
                  </span>
                  <button
                    type="button"
                    class="p-1.5 text-neutral-400 hover:text-white hover:bg-neutral-700 rounded transition-colors"
                    aria-label="Copy code"
                  >
                    <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                      <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M8 16H6a2 2 0 01-2-2V6a2 2 0 012-2h8a2 2 0 012 2v2m-6 12h8a2 2 0 002-2v-8a2 2 0 00-2-2h-8a2 2 0 00-2 2v8a2 2 0 002 2z"/>
                    </svg>
                  </button>
                </div>
                {/* Code */}
                <div class="p-4 overflow-x-auto">
                  <pre class="text-sm font-mono leading-relaxed">
                    <code class="text-neutral-100 whitespace-pre">
                      {example.code}
                    </code>
                  </pre>
                </div>
              </div>
            ))}
          </div>
        ))}

        {/* CTA */}
        <div class="mt-12 text-center">
          <a
            href="/docs/api-reference"
            class="inline-flex items-center gap-2 text-primary-600 hover:text-primary-700 font-medium"
          >
            View full API reference
            <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13 7l5 5m0 0l-5 5m5-5H6"/>
            </svg>
          </a>
        </div>
      </div>
    </section>
  );
});
