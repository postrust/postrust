import { component$ } from "@builder.io/qwik";

const features = [
  {
    icon: "bolt",
    title: "Blazing Fast",
    description: "Written in Rust for maximum performance. Sub-millisecond response times and minimal memory footprint.",
  },
  {
    icon: "cloud",
    title: "Serverless-First",
    description: "Native support for AWS Lambda. Single binary with ~50ms cold starts. No container overhead.",
  },
  {
    icon: "graphql",
    title: "GraphQL Built-in",
    description: "Full GraphQL API alongside REST. Queries, mutations, filtering, and nested relationships out of the box.",
  },
  {
    icon: "realtime",
    title: "Realtime Subscriptions",
    description: "Live updates via GraphQL subscriptions. Subscribe to any query or view changes with PostgreSQL LISTEN/NOTIFY.",
  },
  {
    icon: "vector",
    title: "pgvector Integration",
    description: "Native vector similarity search. Build AI-powered apps with embeddings, semantic search, and RAG pipelines.",
  },
  {
    icon: "routes",
    title: "Custom Routes in Rust",
    description: "Define custom endpoints in pure Rust. Full access to Axum handlers, middleware, and the database pool.",
  },
  {
    icon: "shield",
    title: "Secure by Design",
    description: "PostgreSQL Row-Level Security (RLS) enforcement. JWT authentication with customizable role claims.",
  },
  {
    icon: "code",
    title: "PostgREST Compatible",
    description: "Drop-in replacement for PostgREST in most use cases. Same query syntax, same filtering operators.",
  },
  {
    icon: "api",
    title: "Admin UI Included",
    description: "Built-in Swagger UI, Scalar API docs, and GraphQL Playground. OpenAPI 3.0 spec auto-generated.",
  },
  {
    icon: "package",
    title: "Single Binary",
    description: "~3.5MB binary with no runtime dependencies. Deploy anywhere - Docker, Lambda, bare metal.",
  },
];

const iconPaths: Record<string, string> = {
  bolt: "M13 2L3 14h9l-1 8 10-12h-9l1-8z",
  cloud: "M3 15a4 4 0 004 4h9a5 5 0 10-.1-9.999 5.002 5.002 0 10-9.78 2.096A4.001 4.001 0 003 15z",
  graphql: "M12 2L2 7l10 5 10-5-10-5zM2 17l10 5 10-5M2 12l10 5 10-5",
  realtime: "M13 10V3L4 14h7v7l9-11h-7z",
  vector: "M9 3v2m6-2v2M9 19v2m6-2v2M5 9H3m2 6H3m18-6h-2m2 6h-2M7 19h10a2 2 0 002-2V7a2 2 0 00-2-2H7a2 2 0 00-2 2v10a2 2 0 002 2zM9 9h6v6H9V9z",
  routes: "M13 7h8m0 0v8m0-8l-8 8-4-4-6 6",
  shield: "M9 12l2 2 4-4m5.618-4.016A11.955 11.955 0 0112 2.944a11.955 11.955 0 01-8.618 3.04A12.02 12.02 0 003 9c0 5.591 3.824 10.29 9 11.622 5.176-1.332 9-6.03 9-11.622 0-1.042-.133-2.052-.382-3.016z",
  code: "M10 20l4-16m4 4l4 4-4 4M6 16l-4-4 4-4",
  api: "M8 9l3 3-3 3m5 0h3M5 20h14a2 2 0 002-2V6a2 2 0 00-2-2H5a2 2 0 00-2 2v12a2 2 0 002 2z",
  package: "M20 7l-8-4-8 4m16 0l-8 4m8-4v10l-8 4m0-10L4 7m8 4v10M4 7v10l8 4",
};

export const FeaturesSection = component$(() => {
  return (
    <section class="section-padding bg-white">
      <div class="container-wide">
        {/* Section Header */}
        <div class="text-center max-w-3xl mx-auto mb-16">
          <h2 class="text-3xl md:text-4xl font-bold text-neutral-900 mb-4">
            Everything you need to build APIs
          </h2>
          <p class="text-lg text-neutral-600">
            Postrust provides a complete solution for exposing your PostgreSQL database
            as a secure, high-performance API. No boilerplate, no ORM, just your schema.
          </p>
        </div>

        {/* Features Grid */}
        <div class="grid md:grid-cols-2 lg:grid-cols-4 gap-6">
          {features.map((feature) => (
            <div
              key={feature.title}
              class="group p-6 bg-neutral-50 hover:bg-white rounded-xl border border-neutral-200 hover:border-primary-200 hover:shadow-lg transition-all duration-300"
            >
              <div class="w-12 h-12 bg-primary-100 group-hover:bg-primary-500 rounded-lg flex items-center justify-center mb-4 transition-colors">
                <svg
                  class="w-6 h-6 text-primary-600 group-hover:text-white transition-colors"
                  fill="none"
                  stroke="currentColor"
                  viewBox="0 0 24 24"
                  stroke-width="2"
                  stroke-linecap="round"
                  stroke-linejoin="round"
                >
                  <path d={iconPaths[feature.icon]} />
                </svg>
              </div>
              <h3 class="text-lg font-semibold text-neutral-900 mb-2">
                {feature.title}
              </h3>
              <p class="text-sm text-neutral-600 leading-relaxed">
                {feature.description}
              </p>
            </div>
          ))}
        </div>
      </div>
    </section>
  );
});
