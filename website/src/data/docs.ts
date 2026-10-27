/**
 * The documentation's sections, in the order the docs index lists them. The
 * index and llms.txt both read this list.
 */
export const docSections = [
  {
    title: "Getting Started",
    description: "Quick start guide to get your first API running in minutes",
    href: "/docs/getting-started/",
    icon: "rocket",
  },
  {
    title: "API Reference",
    description:
      "Complete reference for REST endpoints, operators, and headers",
    href: "/docs/api-reference/",
    icon: "code",
  },
  {
    title: "GraphQL",
    description:
      "Full GraphQL API with queries, mutations, subscriptions, and filtering",
    href: "/docs/graphql/",
    icon: "graphql",
  },
  {
    title: "Realtime Subscriptions",
    description:
      "Live data updates via GraphQL subscriptions and PostgreSQL LISTEN/NOTIFY",
    href: "/docs/realtime/",
    icon: "realtime",
  },
  {
    title: "pgvector Integration",
    description:
      "Vector similarity search for AI apps, semantic search, and RAG pipelines",
    href: "/docs/pgvector/",
    icon: "vector",
  },
  {
    title: "Custom Routes",
    description:
      "Define custom Rust handlers for webhooks, business logic, and more",
    href: "/docs/custom-routes/",
    icon: "puzzle",
  },
  {
    title: "SaaS Domains (Beta)",
    description:
      "Multi-tenant custom domains with automatic SSL and dynamic routing",
    href: "/docs/saas-domains/",
    icon: "globe",
  },
  {
    title: "Authentication",
    description: "JWT authentication and PostgreSQL Row-Level Security",
    href: "/docs/authentication/",
    icon: "shield",
  },
  {
    title: "Configuration",
    description: "Environment variables and runtime configuration options",
    href: "/docs/configuration/",
    icon: "settings",
  },
  {
    title: "Deployment",
    description: "Deploy to Docker, AWS Lambda, Kubernetes, and more",
    href: "/docs/deployment/",
    icon: "cloud",
  },
  {
    title: "Cloudflare Workers",
    description:
      "The REST API in a Worker over Hyperdrive, and every way it differs from the server",
    href: "/docs/workers/",
    icon: "cloud",
  },
  {
    title: "Benchmarks",
    description:
      "Measured binary size, latency and memory, and the script that reproduces them",
    href: "/docs/benchmarks/",
    icon: "chart",
  },
  {
    title: "Conformance",
    description:
      "How closely Postrust matches PostgREST and Hasura, measured against each by replaying its own test suite",
    href: "/docs/conformance/",
    icon: "chart",
  },
  {
    title: "PostgREST Conformance",
    description:
      "The REST report: what agrees, and where the two differ on purpose",
    href: "/docs/conformance/postgrest/",
    icon: "chart",
  },
  {
    title: "Hasura Conformance",
    description:
      "The GraphQL report: what agrees, and where the two differ on purpose",
    href: "/docs/conformance/hasura/",
    icon: "graphql",
  },
  {
    title: "Guides",
    description:
      "Row-level security behind an API, GraphQL from a schema, and how the tools compare",
    href: "/guides/",
    icon: "rocket",
  },
];
