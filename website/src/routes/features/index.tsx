import { component$ } from "@builder.io/qwik";
import type { DocumentHead } from "@builder.io/qwik-city";
import { Link } from "@builder.io/qwik-city";

const featureCategories = [
  {
    title: "REST API",
    description: "PostgREST-compatible REST API with advanced querying capabilities",
    features: [
      {
        name: "CRUD Operations",
        description: "Full create, read, update, delete support on tables and views",
      },
      {
        name: "Advanced Filtering",
        description: "eq, neq, gt, lt, gte, lte, like, ilike, in, is, not operators",
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
        description: "Select specific columns with aliasing and computed fields",
      },
    ],
  },
  {
    title: "GraphQL API",
    description: "Full GraphQL support with automatic schema generation",
    features: [
      {
        name: "Dynamic Schema",
        description: "GraphQL schema automatically generated from your database",
      },
      {
        name: "Queries & Mutations",
        description: "Full CRUD with insertOne, insert, update, delete operations",
      },
      {
        name: "Real-time Subscriptions",
        description: "WebSocket subscriptions for live data updates",
      },
      {
        name: "Introspection",
        description: "Full schema introspection for tooling support",
      },
      {
        name: "GraphQL Playground",
        description: "Built-in interactive IDE at /api/graphql",
      },
      {
        name: "Same RLS",
        description: "Same Row-Level Security enforcement as REST",
      },
    ],
  },
  {
    title: "Real-time",
    description: "Live data synchronization powered by PostgreSQL LISTEN/NOTIFY",
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
        description: "Uses graphql-transport-ws standard protocol",
      },
      {
        name: "Connection Management",
        description: "Automatic reconnection and subscription recovery",
      },
      {
        name: "Low Latency",
        description: "Sub-millisecond notification delivery",
      },
    ],
  },
  {
    title: "Security",
    description: "Enterprise-grade security with PostgreSQL Row-Level Security",
    features: [
      {
        name: "JWT Authentication",
        description: "HS256, HS384, HS512 signature verification",
      },
      {
        name: "Row-Level Security",
        description: "PostgreSQL RLS policies enforced on every request",
      },
      {
        name: "Role-Based Access",
        description: "Map JWT claims to PostgreSQL roles",
      },
      {
        name: "Custom Claims",
        description: "Access JWT claims in your SQL policies",
      },
      {
        name: "Anonymous Role",
        description: "Configurable unauthenticated access",
      },
      {
        name: "Audience Validation",
        description: "Optional JWT audience claim verification",
      },
    ],
  },
  {
    title: "Developer Experience",
    description: "Built-in tools for development and debugging",
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
        name: "Structured Logging",
        description: "JSON logs with request tracing",
      },
      {
        name: "Health Checks",
        description: "/_/health and /_/ready endpoints",
      },
    ],
  },
  {
    title: "Deployment",
    description: "Deploy anywhere with native serverless support",
    features: [
      {
        name: "Docker",
        description: "Official container image ready to use",
      },
      {
        name: "AWS Lambda",
        description: "Native Lambda adapter with connection pooling",
      },
      {
        name: "Cloudflare Workers",
        description: "Edge deployment with Hyperdrive support",
      },
      {
        name: "Kubernetes",
        description: "Production-ready with health checks",
      },
      {
        name: "Single Binary",
        description: "~5MB binary with no dependencies",
      },
      {
        name: "Any Platform",
        description: "Fly.io, Railway, Render, DigitalOcean, etc.",
      },
    ],
  },
  {
    title: "Extensibility",
    description: "Extend Postrust with custom Rust code",
    features: [
      {
        name: "Custom Routes",
        description: "Add your own endpoints in Rust",
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
          <div class="max-w-3xl mx-auto text-center">
            <h1 class="text-4xl md:text-5xl font-bold text-neutral-900 mb-6">
              Everything you need to build APIs
            </h1>
            <p class="text-lg text-neutral-600 mb-8">
              Postrust provides a complete solution for exposing your PostgreSQL database
              as a secure, high-performance REST and GraphQL API.
            </p>
            <div class="flex flex-col sm:flex-row items-center justify-center gap-4">
              <Link
                href="/docs/getting-started"
                class="inline-flex items-center gap-2 px-6 py-3 text-base font-semibold text-white bg-neutral-900 hover:bg-neutral-800 rounded-lg transition-colors"
              >
                Get Started
                <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13 7l5 5m0 0l-5 5m5-5H6"/>
                </svg>
              </Link>
              <Link
                href="/compare"
                class="inline-flex items-center gap-2 px-6 py-3 text-base font-semibold text-neutral-700 bg-white border border-neutral-300 hover:bg-neutral-50 rounded-lg transition-colors"
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
            <div class="max-w-3xl mx-auto text-center mb-12">
              <h2 class="text-3xl font-bold text-neutral-900 mb-4">
                {category.title}
              </h2>
              <p class="text-lg text-neutral-600">
                {category.description}
              </p>
            </div>

            <div class="grid md:grid-cols-2 lg:grid-cols-3 gap-6">
              {category.features.map((feature) => (
                <div
                  key={feature.name}
                  class="bg-white rounded-xl p-6 border border-neutral-200 hover:border-primary-200 hover:shadow-md transition-all"
                >
                  <h3 class="text-lg font-semibold text-neutral-900 mb-2">
                    {feature.name}
                  </h3>
                  <p class="text-sm text-neutral-600">
                    {feature.description}
                  </p>
                </div>
              ))}
            </div>
          </div>
        </section>
      ))}

      {/* CTA */}
      <section class="section-padding bg-neutral-900">
        <div class="container-wide text-center">
          <h2 class="text-3xl font-bold text-white mb-4">
            Ready to get started?
          </h2>
          <p class="text-lg text-neutral-300 mb-8 max-w-2xl mx-auto">
            Set up your PostgreSQL API in under 5 minutes.
          </p>
          <Link
            href="/docs/getting-started"
            class="inline-flex items-center gap-2 px-8 py-4 text-base font-semibold text-neutral-900 bg-white hover:bg-neutral-100 rounded-lg transition-colors"
          >
            Read the Documentation
            <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13 7l5 5m0 0l-5 5m5-5H6"/>
            </svg>
          </Link>
        </div>
      </section>
    </div>
  );
});

export const head: DocumentHead = {
  title: "Features - Postrust",
  meta: [
    {
      name: "description",
      content: "Explore all features of Postrust - REST API, GraphQL, JWT authentication, Row-Level Security, and more.",
    },
  ],
};
