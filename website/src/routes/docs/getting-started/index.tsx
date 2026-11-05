import { component$ } from "@builder.io/qwik";
import type { DocumentHead } from "@builder.io/qwik-city";
import { Link } from "@builder.io/qwik-city";

export default component$(() => {
  return (
    <div class="min-h-screen bg-white">
      {/* Header */}
      <div class="border-b border-neutral-200 bg-gradient-to-b from-neutral-50 to-white">
        <div class="container-wide py-12">
          <div class="mb-4 flex items-center gap-2 text-sm text-neutral-500">
            <Link href="/docs/" class="hover:text-primary-600">
              Docs
            </Link>
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
                d="M9 5l7 7-7 7"
              />
            </svg>
            <span class="text-neutral-900">Getting Started</span>
          </div>
          <h1 class="mb-4 text-4xl font-bold text-neutral-900">
            Getting Started
          </h1>
          <p class="max-w-2xl text-lg text-neutral-600">
            Get Postrust up and running in minutes. This guide will walk you
            through installation, creating your first API, and making your first
            requests.
          </p>
        </div>
      </div>

      {/* Content */}
      <div class="container-wide py-12">
        <div class="max-w-3xl">
          {/* Prerequisites */}
          <section class="mb-12">
            <h2 class="mb-4 text-2xl font-bold text-neutral-900">
              Prerequisites
            </h2>
            <ul class="space-y-2 text-neutral-700">
              <li class="flex items-start gap-3">
                <svg
                  class="text-primary-500 mt-0.5 h-5 w-5 flex-shrink-0"
                  fill="none"
                  stroke="currentColor"
                  viewBox="0 0 24 24"
                >
                  <path
                    stroke-linecap="round"
                    stroke-linejoin="round"
                    stroke-width="2"
                    d="M5 13l4 4L19 7"
                  />
                </svg>
                <span>PostgreSQL 12 or later</span>
              </li>
              <li class="flex items-start gap-3">
                <svg
                  class="text-primary-500 mt-0.5 h-5 w-5 flex-shrink-0"
                  fill="none"
                  stroke="currentColor"
                  viewBox="0 0 24 24"
                >
                  <path
                    stroke-linecap="round"
                    stroke-linejoin="round"
                    stroke-width="2"
                    d="M5 13l4 4L19 7"
                  />
                </svg>
                <span>Rust 1.94+ (for building from source) or Docker</span>
              </li>
            </ul>
          </section>

          {/* Installation */}
          <section class="mb-12">
            <h2 class="mb-6 text-2xl font-bold text-neutral-900">
              Installation
            </h2>

            {/* Docker */}
            <div class="mb-8">
              <h3 class="mb-3 flex items-center gap-2 text-lg font-semibold text-neutral-900">
                <span class="bg-primary-100 text-primary-600 flex h-6 w-6 items-center justify-center rounded-full text-sm font-bold">
                  1
                </span>
                Using Docker (Recommended)
              </h3>
              <p class="mb-4 text-neutral-600">
                The fastest way to get started:
              </p>
              <div class="overflow-hidden rounded-xl bg-neutral-900">
                <div class="flex items-center justify-between border-b border-neutral-700 bg-neutral-800 px-4 py-2">
                  <span class="text-sm text-neutral-400">Terminal</span>
                </div>
                <pre class="overflow-x-auto p-4 text-sm">
                  <code class="text-neutral-100">{`# Clone the repository
git clone https://github.com/postrust/postrust.git
cd postrust

# Start PostgreSQL and Postrust
docker-compose up -d

# API is available at http://localhost:3000
curl http://localhost:3000/`}</code>
                </pre>
              </div>
              <div class="mt-4 rounded-lg border border-neutral-200 bg-neutral-50 p-4 text-sm text-neutral-600">
                <strong class="text-neutral-900">Note on paths.</strong> By
                default the REST API is served under{" "}
                <code class="font-mono">/api</code> (e.g.{" "}
                <code class="font-mono">GET /api/products</code>). The examples
                in this guide use root-level paths like{" "}
                <code class="font-mono">/products</code> — these work as-is with{" "}
                <Link
                  href="/docs/configuration/"
                  class="text-primary-600 hover:underline"
                >
                  compatibility mode
                </Link>{" "}
                (<code class="font-mono">PGRST_COMPAT_MODE=true</code>) enabled;
                otherwise prefix them with <code class="font-mono">/api</code>.
              </div>
            </div>

            {/* From Source */}
            <div class="mb-8">
              <h3 class="mb-3 flex items-center gap-2 text-lg font-semibold text-neutral-900">
                <span class="flex h-6 w-6 items-center justify-center rounded-full bg-neutral-100 text-sm font-bold text-neutral-600">
                  2
                </span>
                Building from Source
              </h3>
              <div class="overflow-hidden rounded-xl bg-neutral-900">
                <div class="flex items-center justify-between border-b border-neutral-700 bg-neutral-800 px-4 py-2">
                  <span class="text-sm text-neutral-400">Terminal</span>
                </div>
                <pre class="overflow-x-auto p-4 text-sm">
                  <code class="text-neutral-100">{`# Clone the repository
git clone https://github.com/postrust/postrust.git
cd postrust

# Build in release mode
cargo build --release

# Binary is at target/release/postrust
./target/release/postrust --help`}</code>
                </pre>
              </div>
            </div>

            {/* Pre-built */}
            <div>
              <h3 class="mb-3 flex items-center gap-2 text-lg font-semibold text-neutral-900">
                <span class="flex h-6 w-6 items-center justify-center rounded-full bg-neutral-100 text-sm font-bold text-neutral-600">
                  3
                </span>
                Pre-built Binaries
              </h3>
              <p class="text-neutral-600">
                Download pre-built binaries from the{" "}
                <a
                  href="https://github.com/postrust/postrust/releases"
                  target="_blank"
                  rel="noopener noreferrer"
                  class="text-primary-600 hover:text-primary-700 underline"
                >
                  Releases page
                </a>
                .
              </p>
            </div>
          </section>

          {/* Your First API */}
          <section class="mb-12">
            <h2 class="mb-6 text-2xl font-bold text-neutral-900">
              Your First API
            </h2>

            {/* Step 1 */}
            <div class="mb-8">
              <h3 class="mb-3 text-lg font-semibold text-neutral-900">
                1. Create a Database Table
              </h3>
              <p class="mb-4 text-neutral-600">
                Connect to your PostgreSQL database and create a simple table:
              </p>
              <div class="overflow-hidden rounded-xl bg-neutral-900">
                <div class="flex items-center justify-between border-b border-neutral-700 bg-neutral-800 px-4 py-2">
                  <span class="text-sm text-neutral-400">SQL</span>
                </div>
                <pre class="overflow-x-auto p-4 text-sm">
                  <code class="text-neutral-100">{`CREATE TABLE products (
    id SERIAL PRIMARY KEY,
    name VARCHAR(255) NOT NULL,
    price DECIMAL(10, 2) NOT NULL,
    in_stock BOOLEAN DEFAULT true
);

INSERT INTO products (name, price) VALUES
    ('Widget', 29.99),
    ('Gadget', 49.99),
    ('Gizmo', 19.99);`}</code>
                </pre>
              </div>
            </div>

            {/* Step 2 */}
            <div class="mb-8">
              <h3 class="mb-3 text-lg font-semibold text-neutral-900">
                2. Create a Database Role
              </h3>
              <p class="mb-4 text-neutral-600">
                Postrust uses PostgreSQL roles for access control:
              </p>
              <div class="overflow-hidden rounded-xl bg-neutral-900">
                <div class="flex items-center justify-between border-b border-neutral-700 bg-neutral-800 px-4 py-2">
                  <span class="text-sm text-neutral-400">SQL</span>
                </div>
                <pre class="overflow-x-auto p-4 text-sm">
                  <code class="text-neutral-100">{`-- Create an anonymous role
CREATE ROLE web_anon NOLOGIN;

-- Grant access to the products table
GRANT USAGE ON SCHEMA public TO web_anon;
GRANT SELECT ON public.products TO web_anon;`}</code>
                </pre>
              </div>
            </div>

            {/* Step 3 */}
            <div class="mb-8">
              <h3 class="mb-3 text-lg font-semibold text-neutral-900">
                3. Start Postrust
              </h3>
              <div class="overflow-hidden rounded-xl bg-neutral-900">
                <div class="flex items-center justify-between border-b border-neutral-700 bg-neutral-800 px-4 py-2">
                  <span class="text-sm text-neutral-400">Terminal</span>
                </div>
                <pre class="overflow-x-auto p-4 text-sm">
                  <code class="text-neutral-100">{`# Set required environment variables
export DATABASE_URL="postgres://user:password@localhost:5432/mydb"
export PGRST_DB_ANON_ROLE="web_anon"

# Start the server
./target/release/postrust`}</code>
                </pre>
              </div>
            </div>

            {/* Step 4 */}
            <div>
              <h3 class="mb-3 text-lg font-semibold text-neutral-900">
                4. Make Your First Request
              </h3>
              <p class="mb-4 text-neutral-600">
                By default, Postrust serves REST tables under{" "}
                <code class="font-mono text-sm">/api/</code> (set{" "}
                <code class="font-mono text-sm">PGRST_COMPAT_MODE=true</code> to
                also serve them at <code class="font-mono text-sm">/</code>):
              </p>
              <div class="overflow-hidden rounded-xl bg-neutral-900">
                <div class="flex items-center justify-between border-b border-neutral-700 bg-neutral-800 px-4 py-2">
                  <span class="text-sm text-neutral-400">Terminal</span>
                </div>
                <pre class="overflow-x-auto p-4 text-sm">
                  <code class="text-neutral-100">{`# Get all products
curl http://localhost:3000/api/products

# Response:
# [
#   {"id": 1, "name": "Widget", "price": 29.99, "in_stock": true},
#   {"id": 2, "name": "Gadget", "price": 49.99, "in_stock": true},
#   {"id": 3, "name": "Gizmo", "price": 19.99, "in_stock": true}
# ]`}</code>
                </pre>
              </div>
            </div>
          </section>

          {/* Basic Operations */}
          <section class="mb-12">
            <h2 class="mb-6 text-2xl font-bold text-neutral-900">
              Basic Operations
            </h2>

            <div class="space-y-6">
              <div>
                <h3 class="mb-3 text-lg font-semibold text-neutral-900">
                  Filtering
                </h3>
                <div class="overflow-hidden rounded-xl bg-neutral-900">
                  <pre class="overflow-x-auto p-4 text-sm">
                    <code class="text-neutral-100">{`# Products under $30
curl "http://localhost:3000/api/products?price=lt.30"

# Products in stock
curl "http://localhost:3000/api/products?in_stock=eq.true"

# Products matching name pattern
curl "http://localhost:3000/api/products?name=like.G*"`}</code>
                  </pre>
                </div>
              </div>

              <div>
                <h3 class="mb-3 text-lg font-semibold text-neutral-900">
                  Selecting Columns
                </h3>
                <div class="overflow-hidden rounded-xl bg-neutral-900">
                  <pre class="overflow-x-auto p-4 text-sm">
                    <code class="text-neutral-100">{`# Only get name and price
curl "http://localhost:3000/api/products?select=name,price"`}</code>
                  </pre>
                </div>
              </div>

              <div>
                <h3 class="mb-3 text-lg font-semibold text-neutral-900">
                  Ordering
                </h3>
                <div class="overflow-hidden rounded-xl bg-neutral-900">
                  <pre class="overflow-x-auto p-4 text-sm">
                    <code class="text-neutral-100">{`# Order by price descending
curl "http://localhost:3000/api/products?order=price.desc"`}</code>
                  </pre>
                </div>
              </div>

              <div>
                <h3 class="mb-3 text-lg font-semibold text-neutral-900">
                  Pagination
                </h3>
                <div class="overflow-hidden rounded-xl bg-neutral-900">
                  <pre class="overflow-x-auto p-4 text-sm">
                    <code class="text-neutral-100">{`# Get first 10 products
curl "http://localhost:3000/api/products?limit=10"

# Get products 11-20
curl "http://localhost:3000/api/products?limit=10&offset=10"`}</code>
                  </pre>
                </div>
              </div>
            </div>
          </section>

          {/* Adding Authentication */}
          <section class="mb-12">
            <h2 class="mb-6 text-2xl font-bold text-neutral-900">
              Adding Authentication
            </h2>

            <div class="space-y-6">
              <div>
                <h3 class="mb-3 text-lg font-semibold text-neutral-900">
                  1. Set a JWT Secret
                </h3>
                <div class="overflow-hidden rounded-xl bg-neutral-900">
                  <pre class="overflow-x-auto p-4 text-sm">
                    <code class="text-neutral-100">{`export PGRST_JWT_SECRET="your-super-secret-key-at-least-32-chars"`}</code>
                  </pre>
                </div>
              </div>

              <div>
                <h3 class="mb-3 text-lg font-semibold text-neutral-900">
                  2. Create an Authenticated Role
                </h3>
                <div class="overflow-hidden rounded-xl bg-neutral-900">
                  <div class="flex items-center justify-between border-b border-neutral-700 bg-neutral-800 px-4 py-2">
                    <span class="text-sm text-neutral-400">SQL</span>
                  </div>
                  <pre class="overflow-x-auto p-4 text-sm">
                    <code class="text-neutral-100">{`CREATE ROLE web_user NOLOGIN;
GRANT USAGE ON SCHEMA public TO web_user;
GRANT ALL ON public.products TO web_user;`}</code>
                  </pre>
                </div>
              </div>

              <div>
                <h3 class="mb-3 text-lg font-semibold text-neutral-900">
                  3. Make Authenticated Requests
                </h3>
                <div class="overflow-hidden rounded-xl bg-neutral-900">
                  <pre class="overflow-x-auto p-4 text-sm">
                    <code class="text-neutral-100">{`# Create a JWT token (use your preferred method)
# Token payload: {"role": "web_user", "sub": "user123"}

curl http://localhost:3000/api/products \\
  -H "Authorization: Bearer eyJhbGciOiJIUzI1NiIs..."`}</code>
                  </pre>
                </div>
              </div>
            </div>
          </section>

          {/* Next Steps */}
          <section class="mb-12">
            <h2 class="mb-6 text-2xl font-bold text-neutral-900">Next Steps</h2>
            <div class="grid gap-4 sm:grid-cols-2">
              <Link
                href="/docs/configuration/"
                class="group hover:bg-primary-50 hover:border-primary-200 rounded-xl border border-neutral-200 bg-neutral-50 p-4 transition-colors"
              >
                <h3 class="group-hover:text-primary-600 mb-1 font-semibold text-neutral-900">
                  Configuration Reference
                </h3>
                <p class="text-sm text-neutral-600">
                  All configuration options
                </p>
              </Link>
              <Link
                href="/docs/api-reference/"
                class="group hover:bg-primary-50 hover:border-primary-200 rounded-xl border border-neutral-200 bg-neutral-50 p-4 transition-colors"
              >
                <h3 class="group-hover:text-primary-600 mb-1 font-semibold text-neutral-900">
                  API Reference
                </h3>
                <p class="text-sm text-neutral-600">
                  Complete API documentation
                </p>
              </Link>
              <Link
                href="/docs/authentication/"
                class="group hover:bg-primary-50 hover:border-primary-200 rounded-xl border border-neutral-200 bg-neutral-50 p-4 transition-colors"
              >
                <h3 class="group-hover:text-primary-600 mb-1 font-semibold text-neutral-900">
                  Authentication
                </h3>
                <p class="text-sm text-neutral-600">
                  JWT and Row-Level Security
                </p>
              </Link>
              <Link
                href="/docs/deployment/"
                class="group hover:bg-primary-50 hover:border-primary-200 rounded-xl border border-neutral-200 bg-neutral-50 p-4 transition-colors"
              >
                <h3 class="group-hover:text-primary-600 mb-1 font-semibold text-neutral-900">
                  Deployment
                </h3>
                <p class="text-sm text-neutral-600">Deploy to production</p>
              </Link>
            </div>
          </section>

          {/* Help */}
          <section class="rounded-2xl bg-neutral-900 p-8 text-center">
            <h2 class="mb-2 text-xl font-bold text-white">Need help?</h2>
            <p class="mb-4 text-neutral-300">
              Join our community for support and discussions.
            </p>
            <div class="flex items-center justify-center gap-4">
              <a
                href="https://github.com/postrust/postrust/issues"
                target="_blank"
                rel="noopener noreferrer"
                class="inline-flex items-center gap-2 rounded-lg bg-white/10 px-4 py-2 text-sm font-medium text-white transition-colors hover:bg-white/20"
              >
                <svg
                  class="h-5 w-5"
                  fill="none"
                  stroke="currentColor"
                  viewBox="0 0 24 24"
                >
                  <path
                    stroke-linecap="round"
                    stroke-linejoin="round"
                    stroke-width="2"
                    d="M12 8v4m0 4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"
                  />
                </svg>
                Report Issues
              </a>
              <a
                href="https://github.com/postrust/postrust"
                target="_blank"
                rel="noopener noreferrer"
                class="inline-flex items-center gap-2 rounded-lg bg-white/10 px-4 py-2 text-sm font-medium text-white transition-colors hover:bg-white/20"
              >
                <svg class="h-5 w-5" fill="currentColor" viewBox="0 0 24 24">
                  <path
                    fill-rule="evenodd"
                    d="M12 2C6.477 2 2 6.484 2 12.017c0 4.425 2.865 8.18 6.839 9.504.5.092.682-.217.682-.483 0-.237-.008-.868-.013-1.703-2.782.605-3.369-1.343-3.369-1.343-.454-1.158-1.11-1.466-1.11-1.466-.908-.62.069-.608.069-.608 1.003.07 1.531 1.032 1.531 1.032.892 1.53 2.341 1.088 2.91.832.092-.647.35-1.088.636-1.338-2.22-.253-4.555-1.113-4.555-4.951 0-1.093.39-1.988 1.029-2.688-.103-.253-.446-1.272.098-2.65 0 0 .84-.27 2.75 1.026A9.564 9.564 0 0112 6.844c.85.004 1.705.115 2.504.337 1.909-1.296 2.747-1.027 2.747-1.027.546 1.379.202 2.398.1 2.651.64.7 1.028 1.595 1.028 2.688 0 3.848-2.339 4.695-4.566 4.943.359.309.678.92.678 1.855 0 1.338-.012 2.419-.012 2.747 0 .268.18.58.688.482A10.019 10.019 0 0022 12.017C22 6.484 17.522 2 12 2z"
                    clip-rule="evenodd"
                  />
                </svg>
                GitHub
              </a>
            </div>
          </section>
        </div>
      </div>
    </div>
  );
});

export const head: DocumentHead = {
  title: "Getting Started - Postrust Documentation",
  meta: [
    {
      name: "description",
      content:
        "Get Postrust up and running in minutes. Installation guide, first API setup, and basic operations.",
    },
  ],
};
