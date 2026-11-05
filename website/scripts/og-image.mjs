// Renders public/og.png and page-specific 1200x630 OpenGraph cards in
// public/og/<slug>.png. Run with `node scripts/og-image.mjs` after changing
// page titles or adding routes.
import { mkdirSync } from "node:fs";
import { createRequire } from "node:module";
const require = createRequire(import.meta.url);
const sharp = require("sharp");

const esc = (s) =>
  s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");

function renderSvg({ badge, line1, line2, subtitle, path }) {
  return `<svg xmlns="http://www.w3.org/2000/svg" width="1200" height="630" viewBox="0 0 1200 630">
  <defs>
    <linearGradient id="mark" x1="0%" y1="0%" x2="100%" y2="100%">
      <stop offset="0%" stop-color="#f5912c"/><stop offset="100%" stop-color="#d56a10"/>
    </linearGradient>
  </defs>
  <rect width="1200" height="630" fill="#171717"/>
  <g transform="translate(80 76) scale(2.75)">
    <rect width="32" height="32" rx="6" fill="url(#mark)"/>
    <path d="M17.5 6L9 18h7l-1 8 9-12h-7l1-8z" fill="white" stroke="white" stroke-width="1" stroke-linejoin="round"/>
  </g>
  <text x="192" y="140" font-family="DejaVu Sans, Arial, sans-serif" font-size="48" font-weight="700" fill="#ffffff">Postrust</text>
  ${
    badge
      ? `<rect x="415" y="102" width="${Math.max(120, badge.length * 16 + 36)}" height="46" rx="23" fill="#262626" stroke="#404040" stroke-width="1.5"/>
  <text x="433" y="132" font-family="DejaVu Sans, Arial, sans-serif" font-size="22" font-weight="600" fill="#f5912c">${esc(badge)}</text>`
      : ""
  }
  <text x="80" y="305" font-family="DejaVu Sans, Arial, sans-serif" font-size="56" font-weight="700" fill="#ffffff">${esc(line1)}</text>
  <text x="80" y="382" font-family="DejaVu Sans, Arial, sans-serif" font-size="56" font-weight="700" fill="#f5912c">${esc(line2)}</text>
  <text x="80" y="485" font-family="DejaVu Sans, Arial, sans-serif" font-size="28" fill="#a3a3a3">${esc(subtitle)}</text>
  <text x="80" y="555" font-family="DejaVu Sans, Arial, sans-serif" font-size="24" fill="#737373">postrust.org${esc(path)} · open source (MIT)</text>
</svg>`;
}

const cards = [
  {
    slug: "",
    path: "",
    badge: "3.1 · Rust",
    line1: "PostgreSQL as REST and",
    line2: "GraphQL APIs, in Rust",
    subtitle:
      "One 3 MB binary · Docker · Kubernetes · AWS Lambda · Cloudflare Workers",
  },
  {
    slug: "features",
    path: "/features/",
    badge: "Features",
    line1: "REST, GraphQL, Realtime",
    line2: "& pgvector Search",
    subtitle:
      "PostgREST REST · Hasura GraphQL · Apollo Federation v2 · Asymmetric JWT & JWKS",
  },
  {
    slug: "pricing",
    path: "/pricing/",
    badge: "Pricing",
    line1: "100% Open Source (MIT)",
    line2: "& Enterprise Support",
    subtitle:
      "Free forever in production · Optional SLA, consulting & custom engineering",
  },
  {
    slug: "enterprise",
    path: "/enterprise/",
    badge: "Enterprise",
    line1: "Production Support, SLAs",
    line2: "& Architecture Consulting",
    subtitle:
      "Priority response · Security reviews · Migration from PostgREST or Hasura",
  },
  {
    slug: "packages",
    path: "/packages/",
    badge: "Crates",
    line1: "Modular Rust Crates on",
    line2: "crates.io & docs.rs",
    subtitle:
      "postrust-server · postrust-engine · postrust-graphql · postrust-worker · postrust-lambda",
  },
  {
    slug: "community",
    path: "/community/",
    badge: "Community",
    line1: "Open-Source Community",
    line2: "& Contributors",
    subtitle:
      "GitHub Discussions · Issue tracker · Benchmarks & conformance suites",
  },
  {
    slug: "releases",
    path: "/releases/",
    badge: "Changelog",
    line1: "Postrust 3.1 Release",
    line2: "Notes & Changelog",
    subtitle:
      "Verbatim PostgreSQL errors · JWKS rotation · Custom Rust routes · Workers over Hyperdrive",
  },
  {
    slug: "compare",
    path: "/compare/",
    badge: "Comparisons",
    line1: "Postrust vs PostgREST,",
    line2: "Hasura, PostGraphile & Supabase",
    subtitle:
      "Measured throughput, memory footprint, conformance & architectural trade-offs",
  },
  {
    slug: "compare-postgrest",
    path: "/compare/postgrest/",
    badge: "Comparison",
    line1: "Postrust vs PostgREST",
    line2: "95% Conformance + GraphQL",
    subtitle:
      "1,424 / 1,499 PostgREST test cases · 3.3x to 8.4x throughput · Built-in GraphQL",
  },
  {
    slug: "compare-hasura",
    path: "/compare/hasura/",
    badge: "Comparison",
    line1: "Postrust vs Hasura",
    line2: "97% Conformance in 3 MB",
    subtitle:
      "454 / 468 Hasura test cases · Same GraphQL dialect · 25 MB vs 623 MB RAM",
  },
  {
    slug: "compare-postgraphile",
    path: "/compare/postgraphile/",
    badge: "Comparison",
    line1: "Postrust vs PostGraphile",
    line2: "Rust Binary vs Node.js",
    subtitle:
      "REST + GraphQL + Federation v2 from one native binary without a Node runtime",
  },
  {
    slug: "compare-supabase",
    path: "/compare/supabase/",
    badge: "Comparison",
    line1: "Postrust vs Supabase",
    line2: "Self-Hosted Single Binary",
    subtitle:
      "PostgREST-compatible REST + Hasura-dialect GraphQL + Realtime in one binary",
  },
  {
    slug: "docs",
    path: "/docs/",
    badge: "Documentation",
    line1: "Postrust Documentation",
    line2: "Setup, REST, GraphQL & Auth",
    subtitle:
      "Quick start · REST API · GraphQL & Federation · RLS · AWS Lambda · Cloudflare Workers",
  },
  {
    slug: "docs-getting-started",
    path: "/docs/getting-started/",
    badge: "Docs · Quick Start",
    line1: "Getting Started with",
    line2: "Postrust in 5 Minutes",
    subtitle:
      "Expose your PostgreSQL tables over /api/ and /v1/graphql via Docker or binary",
  },
  {
    slug: "docs-api-reference",
    path: "/docs/api-reference/",
    badge: "Docs · REST API",
    line1: "REST API Reference",
    line2: "Filters, Joins & Mutations",
    subtitle:
      "PostgREST-compatible operators · Resource embedding · Upserts · Stored procedures",
  },
  {
    slug: "docs-graphql",
    path: "/docs/graphql/",
    badge: "Docs · GraphQL",
    line1: "GraphQL API & Apollo",
    line2: "Federation v2 Subgraphs",
    subtitle:
      "Hasura-compatible queries, mutations, aggregations, permissions & _entities",
  },
  {
    slug: "docs-realtime",
    path: "/docs/realtime/",
    badge: "Docs · Realtime",
    line1: "Realtime GraphQL",
    line2: "Subscriptions on Postgres",
    subtitle:
      "Push updates over WebSocket powered by PostgreSQL LISTEN / NOTIFY",
  },
  {
    slug: "docs-pgvector",
    path: "/docs/pgvector/",
    badge: "Docs · AI & Vector",
    line1: "pgvector Integration",
    line2: "& Similarity Search",
    subtitle:
      "Cosine, L2, and inner-product vector queries for embeddings and RAG pipelines",
  },
  {
    slug: "docs-authentication",
    path: "/docs/authentication/",
    badge: "Docs · Security",
    line1: "JWT Authentication, JWKS",
    line2: "& Row-Level Security",
    subtitle:
      "HS256, RS256, PS256, ES256, EdDSA · Auto-rotating JWKS · PostgreSQL RLS",
  },
  {
    slug: "docs-configuration",
    path: "/docs/configuration/",
    badge: "Docs · Reference",
    line1: "Configuration &",
    line2: "Environment Variables",
    subtitle:
      "Database pool · Schemas · JWT & JWKS · GraphQL Federation · OpenTelemetry",
  },
  {
    slug: "docs-custom-routes",
    path: "/docs/custom-routes/",
    badge: "Docs · Extensibility",
    line1: "Custom Axum Routes",
    line2: "Under PostgreSQL RLS",
    subtitle:
      "Mount custom Rust handlers sharing the server's JWT auth and RlsConn transaction",
  },
  {
    slug: "docs-saas-domains",
    path: "/docs/saas-domains/",
    badge: "Docs · Proxy",
    line1: "Multi-Tenant SaaS Domains",
    line2: "& Automatic ACME TLS",
    subtitle:
      "HTTP/2 reverse proxy · Let's Encrypt HTTP-01 · Host routing & health checks",
  },
  {
    slug: "docs-deployment",
    path: "/docs/deployment/",
    badge: "Docs · Deployment",
    line1: "Deploy on Docker, K8s",
    line2: "& AWS Lambda",
    subtitle:
      "Single 3 MB binary · Container images · cargo-lambda adapter tested in AL2023",
  },
  {
    slug: "docs-workers",
    path: "/docs/workers/",
    badge: "Docs · Serverless",
    line1: "Postrust on Cloudflare",
    line2: "Workers & Hyperdrive",
    subtitle:
      "WebAssembly REST pipeline · PostgreSQL wire protocol over Workers sockets · KV cache",
  },
  {
    slug: "docs-conformance",
    path: "/docs/conformance/",
    badge: "Docs · Conformance",
    line1: "Measured API Conformance",
    line2: "PostgREST & Hasura Suites",
    subtitle:
      "Replayed against live reference servers in CI · Every passing & failing case published",
  },
  {
    slug: "docs-conformance-postgrest",
    path: "/docs/conformance/postgrest/",
    badge: "Docs · Conformance",
    line1: "PostgREST Conformance",
    line2: "95.0% Status & Body Match",
    subtitle:
      "1,424 / 1,499 replayed cases from PostgREST v13.0.4's own test suite",
  },
  {
    slug: "docs-conformance-hasura",
    path: "/docs/conformance/hasura/",
    badge: "Docs · Conformance",
    line1: "Hasura GraphQL Conformance",
    line2: "97.0% Data Agreement",
    subtitle:
      "454 / 468 cases across 30 groups replayed against live Hasura v2.48.10",
  },
  {
    slug: "docs-benchmarks",
    path: "/docs/benchmarks/",
    badge: "Docs · Benchmarks",
    line1: "PostgreSQL API Server",
    line2: "Benchmarks & Latency",
    subtitle:
      "44,324 req/s (1.1 ms p50) · Measured against PostgREST, Hasura & PostGraphile",
  },
  {
    slug: "guides",
    path: "/guides/",
    badge: "Engineering Guides",
    line1: "Guides to PostgreSQL",
    line2: "REST & GraphQL APIs",
    subtitle:
      "Schema generation · Row-Level Security · Apollo Federation · Tool comparisons",
  },
  {
    slug: "guides-build-api-from-postgresql",
    path: "/guides/build-api-from-postgresql/",
    badge: "Guide",
    line1: "How to Build a REST &",
    line2: "GraphQL API from Postgres",
    subtitle:
      "Hand-written code vs ORM vs schema-generated APIs, with a worked SQL walkthrough",
  },
  {
    slug: "guides-postgresql-row-level-security-api",
    path: "/guides/postgresql-row-level-security-api/",
    badge: "Guide",
    line1: "Row-Level Security (RLS)",
    line2: "for PostgreSQL APIs",
    subtitle:
      "How JWT claims select database roles, table GRANTs, and per-row RLS policies",
  },
  {
    slug: "guides-postgresql-graphql-tools-compared",
    path: "/guides/postgresql-graphql-tools-compared/",
    badge: "Guide · 2026",
    line1: "PostgreSQL-to-GraphQL",
    line2: "Tools Compared (2026)",
    subtitle:
      "Hasura vs PostGraphile vs Supabase pg_graphql vs PostgREST vs Postrust",
  },
  {
    slug: "guides-graphql-from-postgresql-schema",
    path: "/guides/graphql-from-postgresql-schema/",
    badge: "Guide",
    line1: "How GraphQL APIs Are",
    line2: "Generated from PostgreSQL",
    subtitle:
      "pg_catalog introspection · Type mapping · Single-statement SQL compilation & RLS",
  },
  {
    slug: "guides-apollo-federation-open-source-subgraphs",
    path: "/guides/apollo-federation-open-source-subgraphs/",
    badge: "Guide · Federation",
    line1: "Apollo Federation v2 &",
    line2: "Open-Source Subgraphs",
    subtitle:
      "How _service, _entities, @key & @shareable turn PostgreSQL into a subgraph",
  },
  {
    slug: "guides-postgresql-api-server-languages",
    path: "/guides/postgresql-api-server-languages/",
    badge: "Guide · Runtime",
    line1: "Languages Used to Build",
    line2: "PostgreSQL API Servers",
    subtitle:
      "Haskell, Node.js, Go, C# and Rust compared on memory, cold starts & deployment",
  },
  {
    slug: "blog",
    path: "/blog/",
    badge: "Engineering Blog",
    line1: "Postrust Engineering",
    line2: "Notes & Deep Dives",
    subtitle:
      "Architecture, SQL compilation, Apollo Federation, and benchmark methodology",
  },
  {
    slug: "blog-postgresql-apollo-federation-subgraph",
    path: "/blog/postgresql-apollo-federation-subgraph/",
    badge: "Blog · Federation",
    line1: "PostgreSQL as an Apollo",
    line2: "Federation v2 Subgraph",
    subtitle:
      "Serving _service and batched _entities from primary keys without a resolver layer",
  },
];

const outDir = new URL("../public/og/", import.meta.url).pathname;
mkdirSync(outDir, { recursive: true });

for (const card of cards) {
  const svg = renderSvg(card);
  const dest =
    card.slug === ""
      ? new URL("../public/og.png", import.meta.url).pathname
      : `${outDir}${card.slug}.png`;
  await sharp(Buffer.from(svg)).png().toFile(dest);
}
console.log(`Generated ${cards.length} OpenGraph images.`);
