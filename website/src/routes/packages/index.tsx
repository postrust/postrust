import { component$ } from "@builder.io/qwik";
import type { DocumentHead } from "@builder.io/qwik-city";
import { Link } from "@builder.io/qwik-city";

interface Pkg {
  name: string;
  role: string;
  description: string;
  highlights: string[];
  install: string;
  featured?: boolean;
}

// Kept in sync with each crate's Cargo.toml description and README.
const packages: Pkg[] = [
  {
    name: "postrust-server",
    role: "HTTP server",
    description:
      "PostgREST-compatible REST and GraphQL API server for PostgreSQL, built on Axum.",
    highlights: [
      "Ships the postrust binary — one executable, no runtime dependencies",
      "REST, GraphQL and an optional admin UI",
      "Configured entirely through environment variables",
    ],
    install: "cargo install postrust-server --features admin-ui",
    featured: true,
  },
  {
    name: "postrust-core",
    role: "Engine",
    description:
      "Request parsing, query planning and schema introspection for Postrust.",
    highlights: [
      "Parses select, limit, offset, order, filters and embedding",
      "Introspects your schema and plans reads, mutations and RPC calls",
      "Builds parameterised SQL cast to the right column types",
    ],
    install: "cargo add postrust-core",
  },
  {
    name: "postrust-sql",
    role: "SQL builder",
    description: "Type-safe SQL fragment and statement builder used by Postrust.",
    highlights: [
      "Composable SELECT, INSERT, UPDATE and DELETE builders",
      "Placeholder renumbering when fragments combine",
      "Identifier escaping and qualified names",
    ],
    install: "cargo add postrust-sql",
  },
  {
    name: "postrust-auth",
    role: "Authentication",
    description: "JWT authentication and role resolution for Postrust.",
    highlights: [
      "Verifies JWTs with plain or base64 secrets",
      "Resolves the PostgreSQL role per request",
      "Passes claims through as GUCs for row-level security",
    ],
    install: "cargo add postrust-auth",
  },
  {
    name: "postrust-response",
    role: "Response formatting",
    description: "Response formatting for Postrust: JSON, CSV, and OpenAPI output.",
    highlights: [
      "Content negotiation between JSON and CSV",
      "Content-Range headers for pagination and counts",
      "OpenAPI description of the exposed schema",
    ],
    install: "cargo add postrust-response",
  },
  {
    name: "postrust-graphql",
    role: "GraphQL",
    description:
      "GraphQL API and realtime subscriptions generated from a PostgreSQL schema.",
    highlights: [
      "Schema built from tables, views and relationships",
      "Queries, mutations, filtering and ordering",
      "Subscriptions over PostgreSQL LISTEN/NOTIFY",
    ],
    install: "cargo add postrust-graphql",
  },
  {
    name: "postrust-lambda",
    role: "AWS Lambda",
    description:
      "AWS Lambda adapter for Postrust, for serverless REST APIs over PostgreSQL.",
    highlights: [
      "Runs behind a function URL or API Gateway",
      "Reuses the schema cache across warm invocations",
    ],
    install: "cargo add postrust-lambda",
  },
  {
    name: "postrust-worker",
    role: "Cloudflare Workers",
    description:
      "Cloudflare Workers adapter for Postrust, for edge REST APIs over PostgreSQL.",
    highlights: [
      "Compiles to WebAssembly for Cloudflare Workers",
      "Connects via Hyperdrive or a TCP-capable proxy",
    ],
    install: "cargo add postrust-worker",
  },
  {
    name: "postrust-proxy",
    role: "Reverse proxy",
    description:
      "Reverse proxy for Postrust with load balancing and automatic TLS certificates.",
    highlights: [
      "Load balances across multiple Postrust backends",
      "Automatic certificate issuance and renewal via ACME",
      "Multi-tenant custom domains",
    ],
    install: "cargo add postrust-proxy",
  },
];

export default component$(() => {
  return (
    <div class="min-h-screen bg-white">
      <div class="bg-gradient-to-b from-neutral-50 to-white border-b border-neutral-200">
        <div class="container-wide py-16">
          <h1 class="text-4xl font-bold text-neutral-900 mb-4">Packages</h1>
          <p class="text-lg text-neutral-600 max-w-2xl">
            Postrust is published to crates.io as a set of crates, so you can run the
            whole server or depend on only the piece you need. All of them are released
            together and share a version.
          </p>
          <div class="mt-6 flex flex-wrap gap-3">
            <a
              href="https://crates.io/search?q=postrust"
              target="_blank"
              rel="noopener noreferrer"
              class="px-4 py-2 text-sm font-semibold text-white bg-neutral-900 hover:bg-neutral-800 rounded-lg transition-colors"
            >
              View on crates.io
            </a>
            <Link
              href="/docs"
              class="px-4 py-2 text-sm font-medium text-neutral-700 border border-neutral-300 hover:bg-neutral-50 rounded-lg transition-colors"
            >
              Documentation
            </Link>
          </div>
        </div>
      </div>

      <div class="container-wide py-12">
        {/* Overview table */}
        <div class="max-w-4xl mb-12 overflow-x-auto">
          <table class="w-full text-sm border border-neutral-200 rounded-lg">
            <thead class="bg-neutral-50">
              <tr>
                <th class="text-left px-4 py-3 font-medium text-neutral-900">Crate</th>
                <th class="text-left px-4 py-3 font-medium text-neutral-900">Role</th>
                <th class="text-left px-4 py-3 font-medium text-neutral-900">Links</th>
              </tr>
            </thead>
            <tbody>
              {packages.map((pkg) => (
                <tr key={pkg.name} class="border-t border-neutral-200">
                  <td class="px-4 py-3">
                    <code class="text-neutral-900 font-medium">{pkg.name}</code>
                  </td>
                  <td class="px-4 py-3 text-neutral-600">{pkg.role}</td>
                  <td class="px-4 py-3">
                    <div class="flex gap-3">
                      <a
                        href={`https://crates.io/crates/${pkg.name}`}
                        target="_blank"
                        rel="noopener noreferrer"
                        class="text-primary-600 hover:underline"
                      >
                        crates.io
                      </a>
                      <a
                        href={`https://docs.rs/${pkg.name}`}
                        target="_blank"
                        rel="noopener noreferrer"
                        class="text-primary-600 hover:underline"
                      >
                        docs.rs
                      </a>
                    </div>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>

        {/* Detail cards */}
        <div class="grid gap-6 md:grid-cols-2 max-w-6xl">
          {packages.map((pkg) => (
            <div
              key={pkg.name}
              class={`rounded-xl border p-6 ${
                pkg.featured
                  ? "border-primary-200 bg-primary-50/40"
                  : "border-neutral-200 bg-white"
              }`}
            >
              <div class="flex items-start justify-between gap-3 mb-3">
                <div>
                  <h2 class="text-lg font-bold text-neutral-900 font-mono">{pkg.name}</h2>
                  <span class="text-xs uppercase tracking-wide text-neutral-500">
                    {pkg.role}
                  </span>
                </div>
                {pkg.featured && (
                  <span class="shrink-0 px-2 py-1 text-xs font-medium text-primary-700 bg-primary-100 rounded">
                    Start here
                  </span>
                )}
              </div>

              <p class="text-neutral-600 mb-4">{pkg.description}</p>

              <ul class="space-y-2 mb-4">
                {pkg.highlights.map((item) => (
                  <li key={item} class="flex gap-2 text-sm text-neutral-600">
                    <svg
                      class="w-4 h-4 mt-0.5 shrink-0 text-primary-600"
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
                    <span>{item}</span>
                  </li>
                ))}
              </ul>

              <div class="bg-neutral-900 rounded-lg overflow-hidden mb-4">
                <pre class="p-3 text-xs overflow-x-auto">
                  <code class="text-neutral-100">{pkg.install}</code>
                </pre>
              </div>

              <div class="flex gap-4 text-sm">
                <a
                  href={`https://crates.io/crates/${pkg.name}`}
                  target="_blank"
                  rel="noopener noreferrer"
                  class="text-primary-600 hover:underline"
                >
                  crates.io
                </a>
                <a
                  href={`https://docs.rs/${pkg.name}`}
                  target="_blank"
                  rel="noopener noreferrer"
                  class="text-primary-600 hover:underline"
                >
                  API docs
                </a>
                <a
                  href={`https://github.com/postrust/postrust/tree/main/crates/${pkg.name}`}
                  target="_blank"
                  rel="noopener noreferrer"
                  class="text-primary-600 hover:underline"
                >
                  Source
                </a>
              </div>
            </div>
          ))}
        </div>

        <div class="max-w-4xl mt-12 p-6 bg-neutral-50 rounded-xl border border-neutral-200">
          <h2 class="text-lg font-bold text-neutral-900 mb-2">Versioning</h2>
          <p class="text-neutral-600 text-sm">
            All crates share the workspace version and are published together from a
            single <code class="px-1 py-0.5 bg-white border border-neutral-200 rounded">v*</code>{" "}
            tag, so any two Postrust crates at the same version are known to work
            together. Pin them to the same version in your{" "}
            <code class="px-1 py-0.5 bg-white border border-neutral-200 rounded">Cargo.toml</code>.
          </p>
        </div>
      </div>
    </div>
  );
});

export const head: DocumentHead = {
  title: "Packages - Postrust",
  meta: [
    {
      name: "description",
      content:
        "The Postrust crates published to crates.io: server, engine, SQL builder, auth, response formatting, GraphQL, Lambda, Workers and proxy.",
    },
  ],
};
