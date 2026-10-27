import type { RequestHandler } from "@builder.io/qwik-city";
import { comparisons } from "~/data/comparisons";
import { docSections } from "~/data/docs";
import { guides } from "~/data/guides";

const SITE = "https://postrust.org";

/**
 * The site summarised for language models (https://llmstxt.org/), from the
 * lists the docs index, the guides index and the comparison pages already
 * read, so a page added to any of them is listed here too.
 */
export const onGet: RequestHandler = ({ send, cacheControl }) => {
  cacheControl({ maxAge: 3600, staleWhileRevalidate: 60 * 60 * 24 });
  const lines = [
    "# Postrust",
    "",
    "> Postrust is an open-source (MIT) server, written in Rust, that serves a PostgreSQL schema as REST and GraphQL APIs from one binary. Its REST API speaks PostgREST's dialect and its GraphQL API speaks Hasura's, and how closely each matches is measured by replaying that project's own test suite. Access control is PostgreSQL's own: each request runs as the role in its JWT, under row-level security. It runs on servers and serverless alike: as a binary, in Docker, on Kubernetes (Helm chart), on Fly.io or Railway, on AWS Lambda and on Cloudflare Workers.",
    "",
    "Source: https://github.com/postrust/postrust. Crates: https://crates.io/crates/postrust-server.",
    "",
    "## Docs",
    "",
    ...docSections
      .filter((d) => d.href.startsWith("/docs/"))
      .map((d) => `- [${d.title}](${SITE}${d.href}): ${d.description}`),
    "",
    "## Guides",
    "",
    ...guides.map(
      (g) => `- [${g.title}](${SITE}/guides/${g.slug}/): ${g.description}`,
    ),
    "",
    "## Comparisons",
    "",
    ...comparisons.map(
      (c) =>
        `- [Postrust vs ${c.name}](${SITE}/compare/${c.slug}/): ${c.description}`,
    ),
    "",
    "## Optional",
    "",
    `- [Releases](${SITE}/releases/): what changed in each version.`,
    `- [Apollo Federation with PostgreSQL, worked example](${SITE}/blog/postgresql-apollo-federation-subgraph/)`,
    `- [Enterprise support](${SITE}/enterprise/): support agreements, and team management and billing through the SaaS starter kit.`,
    `- [Maintainer](${SITE}/community/#maintainer)`,
    "",
  ];
  send(
    new Response(lines.join("\n"), {
      headers: { "Content-Type": "text/plain; charset=utf-8" },
    }),
  );
};
