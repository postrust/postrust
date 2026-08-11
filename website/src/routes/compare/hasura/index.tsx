import { component$ } from "@builder.io/qwik";
import type { DocumentHead } from "@builder.io/qwik-city";
import { ComparisonPage } from "~/components/compare/comparison-page";
import { comparisons } from "~/data/comparisons";

const comparison = comparisons.find((c) => c.slug === "hasura");

export default component$(() => {
  if (!comparison) return null;
  return <ComparisonPage comparison={comparison} />;
});

const title = "Postrust vs Hasura: GraphQL from PostgreSQL compared";
const description =
  "How Postrust compares to Hasura: permissions in the database rather than metadata, one binary rather than a platform, measured GraphQL throughput on the same schema, and where Hasura is the better choice.";
const url = "https://postrust.org/compare/hasura";

export const head: DocumentHead = {
  title,
  links: [{ rel: "canonical", href: url }],
  meta: [
    { name: "description", content: description },
    {
      name: "keywords",
      content:
        "postrust vs hasura, hasura alternative, postgresql graphql api, self-hosted graphql",
    },
    { property: "og:title", content: title },
    { property: "og:description", content: description },
    { property: "og:type", content: "article" },
    { property: "og:url", content: url },
    { name: "twitter:card", content: "summary_large_image" },
    { name: "twitter:title", content: title },
    { name: "twitter:description", content: description },
  ],
};
