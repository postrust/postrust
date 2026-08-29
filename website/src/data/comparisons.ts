// Data for the /compare/* pages.
//
// Two rules for anything in this file:
//
//   1. Feature claims describe what the code does today, not what is planned.
//      Where a capability has a condition attached, the condition is in the
//      cell rather than in a footnote.
//   2. Performance figures live in ./measured.ts and come from
//      scripts/bench-compare.sh. Nothing here is estimated.

export interface FeatureRow {
  feature: string;
  postrust: string;
  other: string;
}

export interface FaqEntry {
  q: string;
  a: string;
}

export interface Comparison {
  slug: string;
  name: string;
  /** Used in <title> and H1. */
  tagline: string;
  /** One line, for cards on the homepage and the compare index. */
  description: string;
  /** Their homepage. Linked, because a comparison that hides the alternative is an advert. */
  url: string;
  language: string;
  license: string;
  /** The exact version measured or described, so the page can be dated. */
  versionTested: string;
  /** Lead paragraphs. Kept short. */
  intro: string[];
  features: FeatureRow[];
  /** Honest cases for choosing them instead. Not a rhetorical device. */
  whenTheirs: string[];
  /** Cases where Postrust is the better fit. */
  whenOurs: string[];
  migration?: string[];
  faq: FaqEntry[];
}

/** Shared caveats shown under every performance table. */
export const perfCaveats = [
  "Every server runs as a container on one docker network against the same PostgreSQL instance and the same dataset. No tool gets to skip container overhead.",
  "Each tool keeps its own default connection pool and worker settings. Tuning one and not the others measures the tuning, not the tool.",
  "Requests are expressed in each tool's own dialect, because the dialects differ. The work asked of PostgreSQL is the same.",
  "Every target is warmed before any of them is measured, and the database cache is populated first, so no tool pays to warm the cache for the ones measured after it.",
  "Each figure is the median of several runs rather than the best of them, because a best-of-N reports whichever tool got the quietest moment on the machine.",
  "These are single-machine numbers from a laptop. They are useful for comparing the tools against each other, not for capacity planning.",
];

export const comparisons: Comparison[] = [
  {
    slug: "postgrest",
    name: "PostgREST",
    tagline: "Postrust vs PostgREST",
    description: "The project that established the idea. Haskell, REST only.",
    url: "https://postgrest.org/",
    language: "Haskell",
    license: "MIT",
    versionTested: "v16.1",
    intro: [
      "PostgREST is the project that established this idea: point a server at a PostgreSQL schema and get a REST API, with permissions left to the database. Postrust follows its URL grammar deliberately, so `?select=`, `?order=`, the filter operators and the `Prefer` headers mean the same thing in both.",
      "How closely is measured rather than asserted: replaying PostgREST's own test cases against both servers gives 96.7% agreement on status and body, and 94.9% once every header that is part of the answer is compared too. The [conformance report](/docs/conformance) says what that covers and where the two differ on purpose.",
      "The differences are not in the query language. They are in what ships in the process and what the deployment looks like.",
    ],
    features: [
      { feature: "REST API from the schema", postrust: "Yes", other: "Yes" },
      {
        feature: "Query grammar",
        postrust: "PostgREST-compatible",
        other: "The reference",
      },
      {
        feature: "GraphQL",
        postrust: "Built in, with the admin-ui build feature",
        other: "Not supported",
      },
      {
        feature: "Subscriptions",
        postrust: "GraphQL subscriptions over LISTEN/NOTIFY",
        other: "Not supported",
      },
      {
        feature: "Custom endpoints",
        postrust: "Axum handlers in the same binary, needs a rebuild",
        other: "SQL functions via /rpc",
      },
      {
        feature: "Business logic",
        postrust: "SQL functions, or Rust in-process",
        other: "SQL functions",
      },
      {
        feature: "Runtime",
        postrust: "Single static binary",
        other: "Single binary",
      },
      {
        feature: "AWS Lambda",
        postrust: "Native, via the postrust-lambda crate",
        other: "Container images",
      },
      { feature: "License", postrust: "MIT", other: "MIT" },
      {
        feature: "Maturity",
        postrust: "Young. Smaller surface, fewer users.",
        other: "Years of production use, large community",
      },
    ],
    whenTheirs: [
      "You need the full PostgREST surface. Postrust implements the parts of it that are exercised by its test suite, and PostgREST has had years to grow behaviours that Postrust has not reimplemented.",
      "You want the answer to an obscure question to already exist. PostgREST's documentation and issue history cover ground a young project has not.",
      "REST is all you need, and a second API surface is weight rather than value.",
      "You would rather run software that many organisations have already run in production.",
    ],
    whenOurs: [
      "You want REST and GraphQL from one process rather than two deployments.",
      "You are deploying to Lambda and cold start matters.",
      "You want to add an endpoint in Rust next to the generated API instead of pushing everything into SQL.",
    ],
    migration: [
      "The URL grammar is the same, so existing query strings generally work unchanged.",
      "Configuration uses the same `PGRST_*` environment variable names, including `PGRST_DB_ANON_ROLE` and `PGRST_JWT_SECRET`.",
      "Tables are mounted under `/api` by default rather than at the root. Check your base URL before assuming a 404 is something worse. CORS preflight and the `Allow` header work at both mounts.",
      "NUMERIC columns come back as JSON numbers, as they do from PostgREST, though the scale can differ: 4.2000 where PostgREST gives 4.20. Both parse to the same value.",
      "Object keys come back alphabetically rather than in select order. Build with the compat-key-order feature to match PostgREST; it is off by default because it costs up to 15% on wide rows.",
      "`Location` is sent only for `Prefer: return=headers-only`, as PostgREST sends it. A caller taking the row back reads the key out of the body.",
      "`Prefer: tx=rollback` is not implemented. It is no longer reported as applied either, which it briefly was while committing the write.",
      "Postrust allows 30 seconds of clock skew on a token's `nbf` and `iat`, and none on its `exp`. PostgREST checks all three to the second.",
      "Verify the specific PostgREST features you depend on against Postrust's test suite before switching anything that matters. The conformance report says what is measured and where the two disagree on purpose.",
    ],
    faq: [
      {
        q: "Is Postrust a drop-in replacement for PostgREST?",
        a: "For the query grammar and configuration, largely yes. For the whole feature surface, no. PostgREST has been developed for years and Postrust implements a subset. Test the endpoints you actually use.",
      },
      {
        q: "Why does the same request need a different URL?",
        a: "Postrust mounts generated routes under /api so custom routes and the admin UI can live alongside them without colliding with a table name.",
      },
      {
        q: "Does Postrust support PostgREST's /rpc functions?",
        a: "Yes. Functions in the exposed schema are callable, and that is also how vector similarity search works: the ordering happens inside a SQL function rather than through a query parameter.",
      },
    ],
  },

  {
    slug: "hasura",
    name: "Hasura",
    tagline: "Postrust vs Hasura",
    description:
      "A GraphQL platform. Permissions as metadata, not database roles.",
    url: "https://hasura.io/",
    language: "Haskell",
    license: "Apache 2.0 (v2 core engine)",
    versionTested: "v2.44.0",
    intro: [
      "Hasura generates a GraphQL API from a PostgreSQL schema and models permissions as metadata rather than as database roles. It is a platform: a console, event triggers, remote schemas, and joins across more than one data source.",
      "Postrust is a smaller thing on purpose. Permissions stay in PostgreSQL as roles and row-level security, and configuration stays as environment variables rather than as state a server owns.",
    ],
    features: [
      { feature: "GraphQL from the schema", postrust: "Yes", other: "Yes" },
      {
        feature: "REST API",
        postrust: "Yes, PostgREST-compatible",
        other: "RESTified GraphQL endpoints",
      },
      {
        feature: "Subscriptions",
        postrust: "Over LISTEN/NOTIFY",
        other: "Yes, a core feature",
      },
      {
        feature: "Permissions",
        postrust: "PostgreSQL roles and RLS",
        other: "Metadata, per role and per field",
      },
      {
        feature: "Setup before first query",
        postrust: "None. Tables are exposed on connect.",
        other: "Tables must be tracked and permissions declared",
      },
      {
        feature: "Multiple data sources",
        postrust: "PostgreSQL only",
        other: "Yes, plus remote schemas and joins",
      },
      {
        feature: "Event triggers / webhooks",
        postrust: "Not built in",
        other: "Yes",
      },
      {
        feature: "Admin console",
        postrust: "Admin UI with the admin-ui feature",
        other: "Yes, extensive",
      },
      {
        feature: "Runtime",
        postrust: "Single static binary",
        other: "Container",
      },
      {
        feature: "License",
        postrust: "MIT",
        other: "Apache 2.0 for the v2 core engine",
      },
    ],
    whenTheirs: [
      "You need permissions expressed per role and per field without modelling them as database roles and RLS policies.",
      "Your API spans more than PostgreSQL. Remote schemas and cross-source joins have no equivalent here.",
      "You want event triggers, a console for exploring and editing, and the rest of a platform rather than a server.",
      "You want a GraphQL implementation with a large user base behind it. Note that Hasura's own recommendation for new projects is v3 / DDN, which is a different product from the v2 engine compared here.",
    ],
    whenOurs: [
      "You want permissions to live in the database, where every other client is already subject to them.",
      "You would rather deploy a binary than operate a platform and its metadata.",
      "You need REST and GraphQL over the same tables without a translation layer between them.",
    ],
    faq: [
      {
        q: "Why does the benchmark compare against Hasura v2 rather than v3?",
        a: "v2 is the self-hostable engine that runs as a single container against one PostgreSQL database, which is the closest comparison to Postrust. v3 / DDN is a different architecture and Hasura recommends it for new projects.",
      },
      {
        q: "Does Postrust need tables tracked before they appear?",
        a: "No. It reads the schema on connect, and what the connecting role is granted is what the API exposes. That is a smaller feature set than Hasura's metadata model, and it is also less to keep in sync.",
      },
    ],
  },

  {
    slug: "postgraphile",
    name: "PostGraphile",
    tagline: "Postrust vs PostGraphile",
    description: "GraphQL built to be reshaped. V5 plans with Gra*fast*.",
    url: "https://postgraphile.org/",
    language: "TypeScript / Node.js",
    license: "MIT",
    versionTested: "V5",
    intro: [
      "PostGraphile generates a GraphQL API from a PostgreSQL schema and puts most of its effort into the quality of that schema and the plan behind it. V5's Gra*fast* engine plans a query once and executes it with far fewer round trips than a naive resolver tree.",
      "It is also the most customisable of these tools. If you want to reshape the generated schema, PostGraphile expects that and has a plugin system built for it.",
    ],
    features: [
      { feature: "GraphQL from the schema", postrust: "Yes", other: "Yes" },
      {
        feature: "REST API",
        postrust: "Yes, PostgREST-compatible",
        other: "Not supported",
      },
      {
        feature: "Subscriptions",
        postrust: "Over LISTEN/NOTIFY",
        other: "Built in over LISTEN/NOTIFY; you define the fields",
      },
      {
        feature: "Schema customisation",
        postrust: "Not extensible at runtime",
        other: "Plugin system, the main design goal",
      },
      { feature: "Relay support", postrust: "No", other: "Yes" },
      {
        feature: "Permissions",
        postrust: "PostgreSQL roles and RLS",
        other: "PostgreSQL roles and RLS",
      },
      {
        feature: "Runtime",
        postrust: "Single static binary",
        other: "Node.js",
      },
      {
        feature: "Custom logic",
        postrust: "Rust handlers, or SQL functions",
        other: "TypeScript plugins, or SQL functions",
      },
      { feature: "License", postrust: "MIT", other: "MIT" },
    ],
    whenTheirs: [
      "You want to shape the GraphQL schema rather than accept what was generated. This is PostGraphile's central strength and Postrust has no answer to it.",
      "Your team writes TypeScript and wants API extensions in the same language as the rest of the stack.",
      "You want a Relay-compliant schema, or the connection and node-id conventions that come with it.",
      "GraphQL is the API and REST is not needed.",
    ],
    whenOurs: [
      "You want one process serving both REST and GraphQL.",
      "You would rather deploy a static binary than a Node runtime.",
      "The generated schema is fine as generated and extensibility is not what you are paying for.",
    ],
    faq: [
      {
        q: "Why do the GraphQL queries differ between the two in the benchmark?",
        a: "Field names are inflected differently: Postrust exposes bench_items where PostGraphile exposes allBenchItems with a nodes wrapper. The request asks PostgreSQL for the same rows either way, and both queries are in the benchmark script.",
      },
      {
        q: "Is Gra*fast* faster than what Postrust does?",
        a: "Look at the measured table on this page rather than taking either project's word for it. Gra*fast* is a serious piece of engineering and the numbers are the numbers.",
      },
    ],
  },

  {
    slug: "supabase",
    name: "Supabase",
    tagline: "Postrust vs Supabase",
    description: "A whole backend platform. Its REST layer is PostgREST.",
    url: "https://supabase.com/",
    language: "Platform (PostgREST, GoTrue, Realtime)",
    license: "Apache 2.0",
    versionTested: "Hosted platform",
    intro: [
      "Supabase is not really the same kind of thing. It is a platform — Postgres, auth, storage, realtime, a dashboard — and its REST API is PostgREST, which Supabase documents plainly.",
      "So a comparison is mostly about scope. If the question is whether to run one small server or adopt a platform, that is a different decision from choosing between API generators.",
    ],
    features: [
      {
        feature: "REST API",
        postrust: "Yes, PostgREST-compatible",
        other: "Yes, PostgREST",
      },
      {
        feature: "GraphQL",
        postrust: "Built in",
        other: "Via a Postgres extension",
      },
      {
        feature: "Auth",
        postrust: "JWT verification; bring your own issuer",
        other: "Full auth service with providers",
      },
      { feature: "Storage", postrust: "Not in scope", other: "Yes" },
      {
        feature: "Realtime",
        postrust: "GraphQL subscriptions over LISTEN/NOTIFY",
        other: "Realtime service",
      },
      {
        feature: "Dashboard",
        postrust: "Admin UI with the admin-ui feature",
        other: "Yes, extensive",
      },
      {
        feature: "Hosting",
        postrust: "You run it, anywhere",
        other: "Managed, or self-host the stack",
      },
      {
        feature: "Scope",
        postrust: "One server in front of your database",
        other: "A backend platform",
      },
    ],
    whenTheirs: [
      "You want auth, storage, realtime and a dashboard without assembling them.",
      "You would rather not operate anything.",
      "You are early and want to move quickly with defaults that are already wired together.",
    ],
    whenOurs: [
      "You already have a PostgreSQL database and want an API in front of it, not a platform around it.",
      "You need to run inside your own infrastructure, on your own terms.",
      "You want a single binary you can deploy to Lambda or a small container.",
    ],
    faq: [
      {
        q: "Supabase uses PostgREST, so is Postrust comparable to Supabase's API?",
        a: "For the REST layer, the comparison is really Postrust vs PostgREST, which has its own page. Supabase's value is the rest of the platform.",
      },
    ],
  },
];

export function comparisonBySlug(slug: string): Comparison | undefined {
  return comparisons.find((c) => c.slug === slug);
}
