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
            <span class="text-neutral-900">GraphQL</span>
          </div>
          <h1 class="text-4xl font-bold text-neutral-900 mb-4">GraphQL API</h1>
          <p class="text-lg text-neutral-600 max-w-2xl">
            Full GraphQL support with automatic schema generation from your PostgreSQL database.
          </p>
        </div>
      </div>

      <div class="container-wide py-12">
        <div class="max-w-3xl">
          {/* Endpoints */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">Endpoints</h2>
            <div class="space-y-3">
              <div class="p-4 bg-neutral-50 rounded-lg">
                <code class="font-mono text-primary-600">POST /graphql</code>
                <p class="text-neutral-600 text-sm mt-1">Execute GraphQL queries and mutations</p>
              </div>
              <div class="p-4 bg-neutral-50 rounded-lg">
                <code class="font-mono text-primary-600">GET /graphql</code>
                <p class="text-neutral-600 text-sm mt-1">GraphQL Playground (interactive IDE)</p>
              </div>
              <div class="p-4 bg-neutral-50 rounded-lg">
                <code class="font-mono text-primary-600">GET /admin/graphql</code>
                <p class="text-neutral-600 text-sm mt-1">GraphQL Playground (admin UI)</p>
              </div>
            </div>
          </section>

          {/* Queries */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">Queries</h2>
            <div class="bg-neutral-900 rounded-xl overflow-hidden">
              <div class="px-4 py-2 bg-neutral-800 border-b border-neutral-700">
                <span class="text-sm text-neutral-400">GraphQL</span>
              </div>
              <pre class="p-4 text-sm overflow-x-auto">
                <code class="text-neutral-100">{`# Basic query
query {
  users {
    id
    name
    email
  }
}

# With filtering and pagination
query {
  users(
    filter: { status: { eq: "active" } }
    limit: 10
    offset: 0
    orderBy: { createdAt: DESC }
  ) {
    id
    name
    email
    createdAt
  }
}

# Nested relationships
query {
  orders {
    id
    total
    customer {
      name
      email
    }
    items {
      quantity
      product {
        name
        price
      }
    }
  }
}`}</code>
              </pre>
            </div>
          </section>

          {/* Mutations */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">Mutations</h2>
            <div class="bg-neutral-900 rounded-xl overflow-hidden">
              <div class="px-4 py-2 bg-neutral-800 border-b border-neutral-700">
                <span class="text-sm text-neutral-400">GraphQL</span>
              </div>
              <pre class="p-4 text-sm overflow-x-auto">
                <code class="text-neutral-100">{`# Insert
mutation {
  insertUsers(objects: [
    { name: "Alice", email: "alice@example.com" }
  ]) {
    id
    name
    createdAt
  }
}

# Update
mutation {
  updateUsers(
    filter: { id: { eq: 1 } }
    set: { name: "Alice Smith" }
  ) {
    id
    name
  }
}

# Delete
mutation {
  deleteUsers(filter: { id: { eq: 1 } }) {
    id
  }
}`}</code>
              </pre>
            </div>
          </section>

          {/* Filtering */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">Filter Operators</h2>
            <div class="overflow-x-auto">
              <table class="w-full text-sm">
                <thead>
                  <tr class="border-b border-neutral-200">
                    <th class="text-left py-3 px-4 font-semibold text-neutral-900">Operator</th>
                    <th class="text-left py-3 px-4 font-semibold text-neutral-900">Description</th>
                  </tr>
                </thead>
                <tbody class="divide-y divide-neutral-100">
                  {[
                    { op: "eq", desc: "Equals" },
                    { op: "neq", desc: "Not equals" },
                    { op: "gt / gte", desc: "Greater than / or equal" },
                    { op: "lt / lte", desc: "Less than / or equal" },
                    { op: "like / ilike", desc: "Pattern match (case sensitive/insensitive)" },
                    { op: "in", desc: "In list of values" },
                    { op: "isNull", desc: "Is null check" },
                  ].map((row) => (
                    <tr key={row.op}>
                      <td class="py-3 px-4 font-mono text-primary-600">{row.op}</td>
                      <td class="py-3 px-4 text-neutral-600">{row.desc}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </section>

          {/* Example Request */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">Example Request</h2>
            <div class="bg-neutral-900 rounded-xl overflow-hidden">
              <div class="px-4 py-2 bg-neutral-800 border-b border-neutral-700">
                <span class="text-sm text-neutral-400">cURL</span>
              </div>
              <pre class="p-4 text-sm overflow-x-auto">
                <code class="text-neutral-100">{`curl -X POST http://localhost:3000/graphql \\
  -H "Content-Type: application/json" \\
  -H "Authorization: Bearer eyJhbGci..." \\
  -d '{
    "query": "{ users { id name email } }"
  }'`}</code>
              </pre>
            </div>
          </section>

          {/* Next */}
          <div class="flex items-center justify-between pt-8 border-t border-neutral-200">
            <Link
              href="/docs/deployment"
              class="flex items-center gap-2 text-neutral-600 hover:text-primary-600"
            >
              <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 19l-7-7 7-7"/>
              </svg>
              Deployment
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
  title: "GraphQL - Postrust Documentation",
  meta: [
    {
      name: "description",
      content: "Full GraphQL API with automatic schema generation, queries, mutations, and filtering.",
    },
  ],
};
