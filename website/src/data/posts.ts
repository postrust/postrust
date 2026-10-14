/**
 * The blog, newest first. A post's page lives at /blog/<slug>; this list is
 * what the index, the sitemap and each page's metadata read from.
 */
export interface Post {
  slug: string;
  title: string;
  description: string;
  /** ISO date the post was published. */
  date: string;
  author: string;
  /** The author's own site, linked from the byline and the BlogPosting data. */
  authorUrl: string;
  /** Rounded up, prose only: code is read at a different pace. */
  readingMinutes: number;
  tags: string[];
}

export const posts: Post[] = [
  {
    slug: "postgresql-apollo-federation-subgraph",
    title:
      "PostgreSQL as an Apollo Federation subgraph, without writing resolvers",
    description:
      "Two PostgreSQL databases, two generated subgraphs, one Apollo Router: what a resolver-less subgraph has to do, a worked example with row-level security carried through the router, and how the other tools that do this compare as of October 2026.",
    date: "2026-10-10",
    author: "Bhanu Pratap Chaudhary",
    authorUrl: "https://bhanu.io/",
    readingMinutes: 8,
    tags: ["Apollo Federation", "PostgreSQL", "GraphQL"],
  },
];

export const postUrl = (slug: string) => `https://postrust.org/blog/${slug}/`;

export const formatDate = (iso: string) =>
  new Date(`${iso}T00:00:00Z`).toLocaleDateString("en-GB", {
    day: "numeric",
    month: "long",
    year: "numeric",
    timeZone: "UTC",
  });
