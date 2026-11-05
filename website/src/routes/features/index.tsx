import { component$ } from "@builder.io/qwik";
import type { DocumentHead } from "@builder.io/qwik-city";
import { Link } from "@builder.io/qwik-city";

const featureCategories = [
  {
    title: "REST API",
    description:
      "PostgREST-compatible REST API with advanced querying capabilities",
    links: [
      { href: "/docs/api-reference/", label: "REST API Reference" },
      {
        href: "/docs/conformance/postgrest/",
        label: "PostgREST Conformance Report",
      },
      {
        href: "/guides/build-api-from-postgresql/",
        label: "Guide: Build an API from PostgreSQL",
      },
    ],
    features: [
      {
        name: "CRUD Operations",
        description:
          "Full create, read, update, delete support on tables and views",
      },
      {
        name: "Advanced Filtering",
        description:
          "eq, neq, gt, lt, gte, lte, like, ilike, in, is, not operators",
      },
      {
        name: "Full-Text Search",
        description: "fts, plfts, phfts, wfts operators with language support",
      },
      {
        name: "Resource Embedding",
        description: "Automatic JOINs via foreign keys with nested resources",
      },
      {
        name: "Pagination",
        description: "Limit, offset, and Range header support",
      },
      {
        name: "Column Selection",
        description:
          "Select specific columns with aliasing and computed fields",
      },
    ],
  },
  {
    title: "GraphQL API",
    description:
      "Hasura's dialect, generated from your schema and measured against Hasura itself",
    links: [
      { href: "/docs/graphql/", label: "GraphQL & Federation Docs" },
      { href: "/docs/conformance/hasura/", label: "Hasura Conformance Report" },
      {
        href: "/guides/graphql-from-postgresql-schema/",
        label: "Guide: How GraphQL APIs Are Generated",
      },
      {
        href: "/guides/postgresql-graphql-tools-compared/",
        label: "Guide: PostgreSQL GraphQL Tools Compared",
      },
      {
        href: "/guides/apollo-federation-open-source-subgraphs/",
        label: "Guide: Apollo Federation Subgraphs",
      },
    ],
    features: [
      {
        name: "Dynamic Schema",
        description:
          "GraphQL schema automatically generated from your database",
      },
      {
        name: "Hasura Dialect",
        description:
          "A client generated against Hasura points at /v1/graphql unchanged",
      },
      {
        name: "Queries & Mutations",
        description:
          "author, author_by_pk, author_aggregate, insert_author_one, update_author_many, on_conflict upserts and nested writes",
      },
      {
        name: "Permissions",
        description:
          "A schema per role, row filters, separate read and write columns, presets and backend_only",
      },
      {
        name: "Real-time Subscriptions",
        description: "Live queries over WebSocket, mirroring the query root",
      },
      {
        name: "Apollo Federation",
        description:
          "Opt-in Federation v2 subgraph: _service, _entities and @key from the primary key, checked by Apollo's compatibility suite in CI",
      },
      {
        name: "Introspection",
        description: "Full schema introspection for tooling support",
      },
      {
        name: "GraphQL Playground",
        description: "Built-in interactive IDE at /v1/graphql",
      },
      {
        name: "Same RLS",
        description: "Same Row-Level Security enforcement as REST",
      },
    ],
  },
  {
    title: "Real-time & pgvector Search",
    description:
      "Live data synchronization powered by PostgreSQL LISTEN/NOTIFY and vector similarity search via pgvector",
    links: [
      { href: "/docs/realtime/", label: "Realtime Subscriptions Docs" },
      { href: "/docs/pgvector/", label: "pgvector Integration Docs" },
    ],
    features: [
      {
        name: "GraphQL Subscriptions",
        description: "Subscribe to any table changes via WebSocket",
      },
      {
        name: "PostgreSQL LISTEN/NOTIFY",
        description: "Native PostgreSQL pub/sub for efficient updates",
      },
      {
        name: "Automatic Triggers",
        description: "Changes broadcast automatically on INSERT/UPDATE/DELETE",
      },
      {
        name: "WebSocket Protocol",
        description:
          "Uses graphql-transport-ws and graphql-ws standard protocols",
      },
      {
        name: "pgvector Similarity Search",
        description:
          "Cosine, L2, and inner-product distance queries on pgvector embedding columns over REST and RPC",
      },
      {
        name: "Push, Not Poll",
        description:
          "Delivered on PostgreSQL's LISTEN/NOTIFY, not a polling loop",
      },
    ],
  },
  {
    title: "Security",
    description:
      "Enterprise-grade security with PostgreSQL Row-Level Security, asymmetric JWTs, and auto-rotating JWKS",
    links: [
      { href: "/docs/authentication/", label: "Authentication & JWKS Docs" },
      {
        href: "/guides/postgresql-row-level-security-api/",
        label: "Guide: Row-Level Security for PostgreSQL APIs",
      },
    ],
    features: [
      {
        name: "Asymmetric & Symmetric JWTs",
        description:
          "HS256, HS384, HS512, RS256, RS384, RS512, PS256, PS384, PS512, ES256, ES384, and EdDSA signature verification",
      },
      {
        name: "JWKS Auto-Rotation",
        description:
          "Auto-rotating JWKS key sets via PGRST_JWT_JWKS_URL for Auth0, Clerk, Supabase Auth, AWS Cognito, and Keycloak",
      },
      {
        name: "Row-Level Security",
        description:
          "PostgreSQL RLS policies enforced on every REST, GraphQL, and custom route request",
      },
      {
        name: "Role-Based Access",
        description:
          "Map JWT claims to PostgreSQL roles with NOINHERIT authenticator support",
      },
      {
        name: "Custom Claims",
        description:
          "Access verified JWT claims in SQL policies via request.jwt.claims",
      },
      {
        name: "Audience & Issuer Validation",
        description:
          "Configurable JWT aud and iss claim verification and anonymous role fallback",
      },
    ],
  },
  {
    title: "Developer Experience",
    description:
      "Built-in tools for development, configuration, and observability",
    links: [
      { href: "/docs/configuration/", label: "Configuration Reference" },
      { href: "/docs/benchmarks/", label: "Performance Benchmarks" },
    ],
    features: [
      {
        name: "Admin Dashboard",
        description: "Central hub with links to all dev tools",
      },
      {
        name: "Swagger UI",
        description: "Interactive API documentation and testing",
      },
      {
        name: "Scalar Docs",
        description: "Modern, beautiful API documentation",
      },
      {
        name: "OpenAPI Spec",
        description: "Auto-generated OpenAPI 3.0 specification",
      },
      {
        name: "OpenTelemetry & Logging",
        description: "Structured JSON logs and OpenTelemetry OTLP trace export",
      },
      {
        name: "Health Checks",
        description: "/_/health and /_/ready endpoints",
      },
    ],
  },
  {
    title: "Deployment",
    description:
      "A server, a container, Kubernetes, AWS Lambda, Cloudflare Workers, or multi-tenant SaaS proxy",
    links: [
      { href: "/docs/deployment/", label: "Server & Lambda Deployment Docs" },
      { href: "/docs/workers/", label: "Cloudflare Workers Docs" },
      { href: "/docs/saas-domains/", label: "SaaS Custom Domains & ACME Docs" },
      {
        href: "/guides/postgresql-api-server-languages/",
        label: "Guide: PostgreSQL API Server Languages",
      },
    ],
    features: [
      {
        name: "Docker",
        description: "Official container image ready to use",
      },
      {
        name: "AWS Lambda",
        description:
          "The server's REST and GraphQL router behind the Lambda runtime",
      },
      {
        name: "Cloudflare Workers",
        description:
          "The REST API in a Worker, over Hyperdrive, with the schema cache in KV",
      },
      {
        name: "Kubernetes",
        description: "Production-ready with health checks",
      },
      {
        name: "Single Binary",
        description:
          "2.93 MiB binary (4.98 MiB with Admin UI) that needs only libc",
      },
      {
        name: "SaaS Custom Domains",
        description:
          "Multi-tenant reverse proxy with automatic Let's Encrypt TLS certificates",
      },
    ],
  },
  {
    title: "Extensibility",
    description:
      "Extend Postrust with custom Rust code under the same RLS transaction",
    links: [
      { href: "/docs/custom-routes/", label: "Custom Routes in Rust Docs" },
      { href: "/packages/", label: "Rust Crates & Packages" },
    ],
    features: [
      {
        name: "Custom Routes",
        description:
          "Add your own endpoints in Rust with RlsConn enforcing the caller's JWT role",
      },
      {
        name: "Webhook Handlers",
        description: "Process Stripe, GitHub, and other webhooks",
      },
      {
        name: "RPC Functions",
        description: "Call PostgreSQL stored procedures",
      },
      {
        name: "Middleware",
        description: "Add custom request/response processing",
      },
      {
        name: "Type Safety",
        description: "Compile-time guarantees with Rust",
      },
      {
        name: "Async Runtime",
        description: "Built on Tokio for high concurrency",
      },
    ],
  },
];

export default component$(() => {
  return (
    <div class="min-h-screen">
      {/* Hero */}
      <section class="section-padding bg-gradient-to-b from-neutral-50 to-white">
        <div class="container-wide">
          <div class="mx-auto max-w-3xl text-center">
            <h1 class="mb-6 text-4xl font-bold text-neutral-900 md:text-5xl">
              Features: REST, GraphQL and realtime from PostgreSQL
            </h1>
            <p class="mb-8 text-lg text-neutral-600">
              Postrust provides a complete solution for exposing your PostgreSQL
              database as a secure, high-performance REST and GraphQL API.
            </p>
            <div class="flex flex-col items-center justify-center gap-4 sm:flex-row">
              <Link
                href="/docs/getting-started/"
                class="inline-flex items-center gap-2 rounded-lg bg-neutral-900 px-6 py-3 text-base font-semibold text-white transition-colors hover:bg-neutral-800"
              >
                Get Started
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
              </Link>
              <Link
                href="/compare/"
                class="inline-flex items-center gap-2 rounded-lg border border-neutral-300 bg-white px-6 py-3 text-base font-semibold text-neutral-700 transition-colors hover:bg-neutral-50"
              >
                Compare Alternatives
              </Link>
            </div>
          </div>
        </div>
      </section>

      {/* Feature Categories */}
      {featureCategories.map((category, index) => (
        <section
          key={category.title}
          class={`section-padding ${index % 2 === 0 ? "bg-white" : "bg-neutral-50"}`}
        >
          <div class="container-wide">
            <div class="mx-auto mb-12 max-w-3xl text-center">
              <h2 class="mb-4 text-3xl font-bold text-neutral-900">
                {category.title}
              </h2>
              <p class="mb-4 text-lg text-neutral-600">
                {category.description}
              </p>
              {category.links && (
                <div class="flex flex-wrap items-center justify-center gap-3">
                  {category.links.map((link) => (
                    <Link
                      key={link.href}
                      href={link.href}
                      class="text-primary-700 bg-primary-50 hover:bg-primary-100 border-primary-200 inline-flex items-center rounded-full border px-3.5 py-1.5 text-xs font-semibold transition-colors"
                    >
                      {link.label} &rarr;
                    </Link>
                  ))}
                </div>
              )}
            </div>

            <div class="grid gap-6 md:grid-cols-2 lg:grid-cols-3">
              {category.features.map((feature) => (
                <div
                  key={feature.name}
                  class="hover:border-primary-200 rounded-xl border border-neutral-200 bg-white p-6 transition-all hover:shadow-md"
                >
                  <h3 class="mb-2 text-lg font-semibold text-neutral-900">
                    {feature.name}
                  </h3>
                  <p class="text-sm text-neutral-600">{feature.description}</p>
                </div>
              ))}
            </div>
          </div>
        </section>
      ))}

      {/* CTA */}
      <section class="section-padding bg-neutral-900">
        <div class="container-wide text-center">
          <h2 class="mb-4 text-3xl font-bold text-white">
            Ready to get started?
          </h2>
          <p class="mx-auto mb-8 max-w-2xl text-lg text-neutral-300">
            Set up your PostgreSQL API in under 5 minutes.
          </p>
          <Link
            href="/docs/getting-started/"
            class="inline-flex items-center gap-2 rounded-lg bg-white px-8 py-4 text-base font-semibold text-neutral-900 transition-colors hover:bg-neutral-100"
          >
            Read the Documentation
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
          </Link>
        </div>
      </section>
    </div>
  );
});

export const head: DocumentHead = {
  title: "Postrust Features: PostgreSQL REST, GraphQL, Realtime & pgvector",
  meta: [
    {
      name: "description",
      content:
        "Explore Postrust features: PostgREST REST API, Hasura-dialect GraphQL, Apollo Federation v2 subgraphs, LISTEN/NOTIFY subscriptions, pgvector search, and RLS security.",
    },
  ],
};
