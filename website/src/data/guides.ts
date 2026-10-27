/**
 * Guides: explanations of a question people ask about PostgreSQL APIs,
 * answered first and then worked through. A guide's page lives at
 * /guides/<slug>/; this list is what the index, the sitemap and llms.txt read.
 */
export interface Guide {
  slug: string;
  /** The page's H1, phrased as the question or task it answers. */
  title: string;
  /** For the <title>, at most 60 characters. */
  headTitle: string;
  /** 70 to 155 characters: the meta description and the index blurb. */
  description: string;
}

export const guides: Guide[] = [
  {
    slug: "build-api-from-postgresql",
    title: "How do teams build an API directly from PostgreSQL?",
    headTitle: "Build an API from PostgreSQL: REST and GraphQL in 2026",
    description:
      "Three ways to build an API from PostgreSQL, what a generated API gives you, and a worked example serving REST and GraphQL with Postrust.",
  },
  {
    slug: "postgresql-row-level-security-api",
    title:
      "How do teams manage access control and row-level security behind a PostgreSQL API?",
    headTitle: "Row-Level Security for a PostgreSQL API",
    description:
      "Engineering teams usually enforce API access control in PostgreSQL: a JWT picks the role, GRANTs pick the tables, RLS picks the rows.",
  },
  {
    slug: "postgresql-graphql-tools-compared",
    title:
      "Which PostgreSQL-to-GraphQL tool should you use in 2026: Hasura, PostGraphile, Supabase, PostgREST or Postrust?",
    headTitle: "PostgreSQL to GraphQL tools compared, 2026",
    description:
      "Hasura, PostGraphile, Supabase pg_graphql, PostgREST and Postrust compared as of October 2026: permissions, REST, subscriptions, Federation, hosting.",
  },
  {
    slug: "graphql-from-postgresql-schema",
    title: "How are GraphQL APIs generated from a PostgreSQL schema?",
    headTitle: "How GraphQL APIs Are Generated from a PostgreSQL Schema",
    description:
      "How a GraphQL API is generated from a PostgreSQL schema: catalogue introspection, type mapping, root fields, one-statement SQL and RLS.",
  },
  {
    slug: "apollo-federation-open-source-subgraphs",
    title:
      "What is Apollo Federation, and which open-source projects can be a subgraph?",
    headTitle: "Apollo Federation and Open-Source Subgraphs",
    description:
      "What Apollo Federation is, what a subgraph must implement, and which open-source tools serve PostgreSQL as a subgraph, as of October 2026.",
  },
  {
    slug: "postgresql-api-server-languages",
    title:
      "Which languages are PostgreSQL API servers written in, and does it matter?",
    headTitle: "Languages Used to Build PostgreSQL API Servers",
    description:
      "Which languages PostgreSQL API servers are written in, from Haskell to Rust, and what the runtime changes: memory, cold starts and deployment.",
  },
];

export const guideUrl = (slug: string) =>
  `https://postrust.org/guides/${slug}/`;

export const guideBySlug = (slug: string) => {
  const guide = guides.find((g) => g.slug === slug);
  if (!guide) throw new Error(`no guide with slug ${slug}`);
  return guide;
};
