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
            <span class="text-neutral-900">API Reference</span>
          </div>
          <h1 class="text-4xl font-bold text-neutral-900 mb-4">API Reference</h1>
          <p class="text-lg text-neutral-600 max-w-2xl">
            Complete reference for Postrust REST API endpoints, operators, and headers.
          </p>
        </div>
      </div>

      <div class="container-wide py-12">
        <div class="max-w-3xl">
          {/* Endpoints */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-6">Endpoints</h2>
            <div class="mb-6 p-4 bg-neutral-50 border border-neutral-200 rounded-lg text-sm text-neutral-600">
              <strong class="text-neutral-900">Note on paths.</strong> By default the REST API
              is served under <code class="font-mono">/api</code> (e.g.{" "}
              <code class="font-mono">GET /api/users</code>). The root-level paths below work
              as-is when <Link href="/docs/configuration" class="text-primary-600 hover:underline">
              compatibility mode</Link> (<code class="font-mono">PGRST_COMPAT_MODE=true</code>) is
              enabled; otherwise prefix them with <code class="font-mono">/api</code>.
            </div>
            <div class="overflow-x-auto">
              <table class="w-full text-sm">
                <thead>
                  <tr class="border-b border-neutral-200">
                    <th class="text-left py-3 px-4 font-semibold text-neutral-900">Method</th>
                    <th class="text-left py-3 px-4 font-semibold text-neutral-900">Endpoint</th>
                    <th class="text-left py-3 px-4 font-semibold text-neutral-900">Description</th>
                  </tr>
                </thead>
                <tbody class="divide-y divide-neutral-100">
                  <tr>
                    <td class="py-3 px-4"><code class="bg-green-100 text-green-700 px-2 py-0.5 rounded">GET</code></td>
                    <td class="py-3 px-4 font-mono text-neutral-700">/{"{table}"}</td>
                    <td class="py-3 px-4 text-neutral-600">Read rows from table</td>
                  </tr>
                  <tr>
                    <td class="py-3 px-4"><code class="bg-blue-100 text-blue-700 px-2 py-0.5 rounded">POST</code></td>
                    <td class="py-3 px-4 font-mono text-neutral-700">/{"{table}"}</td>
                    <td class="py-3 px-4 text-neutral-600">Create new rows</td>
                  </tr>
                  <tr>
                    <td class="py-3 px-4"><code class="bg-yellow-100 text-yellow-700 px-2 py-0.5 rounded">PATCH</code></td>
                    <td class="py-3 px-4 font-mono text-neutral-700">/{"{table}"}</td>
                    <td class="py-3 px-4 text-neutral-600">Update existing rows</td>
                  </tr>
                  <tr>
                    <td class="py-3 px-4"><code class="bg-purple-100 text-purple-700 px-2 py-0.5 rounded">PUT</code></td>
                    <td class="py-3 px-4 font-mono text-neutral-700">/{"{table}"}</td>
                    <td class="py-3 px-4 text-neutral-600">Upsert rows</td>
                  </tr>
                  <tr>
                    <td class="py-3 px-4"><code class="bg-red-100 text-red-700 px-2 py-0.5 rounded">DELETE</code></td>
                    <td class="py-3 px-4 font-mono text-neutral-700">/{"{table}"}</td>
                    <td class="py-3 px-4 text-neutral-600">Delete rows</td>
                  </tr>
                  <tr>
                    <td class="py-3 px-4"><code class="bg-blue-100 text-blue-700 px-2 py-0.5 rounded">POST</code></td>
                    <td class="py-3 px-4 font-mono text-neutral-700">/rpc/{"{function}"}</td>
                    <td class="py-3 px-4 text-neutral-600">Call stored procedure</td>
                  </tr>
                </tbody>
              </table>
            </div>
          </section>

          {/* Filtering Operators */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-6">Filtering Operators</h2>
            <div class="overflow-x-auto">
              <table class="w-full text-sm">
                <thead>
                  <tr class="border-b border-neutral-200">
                    <th class="text-left py-3 px-4 font-semibold text-neutral-900">Operator</th>
                    <th class="text-left py-3 px-4 font-semibold text-neutral-900">Description</th>
                    <th class="text-left py-3 px-4 font-semibold text-neutral-900">Example</th>
                  </tr>
                </thead>
                <tbody class="divide-y divide-neutral-100">
                  {[
                    { op: "eq", desc: "Equals", ex: "?status=eq.active" },
                    { op: "neq", desc: "Not equals", ex: "?status=neq.deleted" },
                    { op: "gt", desc: "Greater than", ex: "?price=gt.100" },
                    { op: "gte", desc: "Greater than or equal", ex: "?price=gte.100" },
                    { op: "lt", desc: "Less than", ex: "?price=lt.50" },
                    { op: "lte", desc: "Less than or equal", ex: "?price=lte.50" },
                    { op: "like", desc: "Pattern match (case-sensitive)", ex: "?name=like.*Widget*" },
                    { op: "ilike", desc: "Pattern match (case-insensitive)", ex: "?name=ilike.*widget*" },
                    { op: "in", desc: "In list", ex: "?id=in.(1,2,3)" },
                    { op: "is", desc: "Is null/true/false", ex: "?deleted_at=is.null" },
                  ].map((row) => (
                    <tr key={row.op}>
                      <td class="py-3 px-4 font-mono text-primary-600">{row.op}</td>
                      <td class="py-3 px-4 text-neutral-700">{row.desc}</td>
                      <td class="py-3 px-4 font-mono text-neutral-500 text-xs">{row.ex}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </section>

          {/* Resource Embedding */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">Resource Embedding</h2>
            <p class="text-neutral-600 mb-4">
              Embed related resources using foreign key relationships:
            </p>
            <div class="bg-neutral-900 rounded-xl overflow-hidden">
              <pre class="p-4 text-sm overflow-x-auto">
                <code class="text-neutral-100">{`# Embed customer in orders
GET /orders?select=*,customer(name,email)

# Nested embedding
GET /orders?select=*,items(product(name,price))

# Filter on embedded resource
GET /orders?select=*,customer!inner(*)&customer.country=eq.US`}</code>
              </pre>
            </div>
          </section>

          {/* Prefer Headers */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">Prefer Headers</h2>
            <div class="space-y-4">
              {[
                { header: "return=representation", desc: "Return created/updated records in response" },
                { header: "return=headers-only", desc: "No body. The only case that gets a Location header naming the created row." },
                { header: "count=exact", desc: "Include exact row count in response" },
                { header: "resolution=merge-duplicates", desc: "Upsert (insert or update)" },
              ].map((item) => (
                <div key={item.header} class="flex items-start gap-4 p-4 bg-neutral-50 rounded-lg">
                  <code class="bg-neutral-200 text-neutral-800 px-2 py-1 rounded text-sm font-mono">
                    {item.header}
                  </code>
                  <span class="text-neutral-600">{item.desc}</span>
                </div>
              ))}
            </div>
          </section>

          {/* Response headers */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">Response Headers</h2>
            <div class="overflow-x-auto">
              <table class="w-full text-sm">
                <thead>
                  <tr class="border-b border-neutral-200">
                    <th class="text-left py-2 pr-4 font-semibold text-neutral-900">Header</th>
                    <th class="text-left py-2 pr-4 font-semibold text-neutral-900">When</th>
                    <th class="text-left py-2 font-semibold text-neutral-900">Description</th>
                  </tr>
                </thead>
                <tbody>
                  {[
                    ["Content-Range", "Reads, and writes reporting a count", "The window returned and the total: 0-24/100"],
                    ["Range-Unit", "With Content-Range", "Always items"],
                    ["Location", "POST with Prefer: return=headers-only", "The created row's key: /users?id=eq.42"],
                    ["Preference-Applied", "When a Prefer was honoured", "The preferences actually applied"],
                    ["Allow", "OPTIONS", "The methods this resource answers"],
                    ["WWW-Authenticate", "401", "Bearer, plus what was wrong with a token that was read and refused"],
                  ].map(([header, when, desc]) => (
                    <tr key={header} class="border-b border-neutral-100">
                      <td class="py-2 pr-4"><code class="font-mono text-primary-700">{header}</code></td>
                      <td class="py-2 pr-4 text-neutral-500">{when}</td>
                      <td class="py-2 text-neutral-600">{desc}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </section>

          {/* OPTIONS */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">OPTIONS and Allow</h2>
            <p class="text-neutral-600 mb-4">
              <code class="font-mono">OPTIONS</code> reports what a resource actually answers,
              derived from the schema rather than fixed. A table offers what its grants allow; a
              view offers what PostgreSQL can genuinely write through it, which is not the same as
              what it has been granted. <code class="font-mono">PUT</code> replaces one row named
              by its key, so a relation without a primary key does not offer it however writable
              it otherwise is. A <code class="font-mono">VOLATILE</code> function is{" "}
              <code class="font-mono">OPTIONS,POST</code>; a stable one adds{" "}
              <code class="font-mono">GET,HEAD</code>.
            </p>
            <pre class="bg-neutral-900 text-neutral-100 rounded-lg p-4 text-sm overflow-x-auto"><code>{`curl -i -X OPTIONS http://localhost:3000/api/users
# Allow: OPTIONS,GET,HEAD,POST,PUT,PATCH,DELETE`}</code></pre>
          </section>

          {/* Next */}
          <div class="flex items-center justify-between pt-8 border-t border-neutral-200">
            <Link
              href="/docs/getting-started"
              class="flex items-center gap-2 text-neutral-600 hover:text-primary-600"
            >
              <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 19l-7-7 7-7"/>
              </svg>
              Getting Started
            </Link>
            <Link
              href="/docs/authentication"
              class="flex items-center gap-2 text-neutral-600 hover:text-primary-600"
            >
              Authentication
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
  title: "API Reference - Postrust Documentation",
  meta: [
    {
      name: "description",
      content: "Complete API reference for Postrust REST endpoints, filtering operators, and headers.",
    },
  ],
};
