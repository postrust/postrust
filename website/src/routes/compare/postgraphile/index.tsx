import { component$ } from "@builder.io/qwik";
import type { DocumentHead } from "@builder.io/qwik-city";
import { ComparisonPage } from "~/components/compare/comparison-page";
import { comparisons } from "~/data/comparisons";

const comparison = comparisons.find((c) => c.slug === "postgraphile");

export default component$(() => {
  if (!comparison) return null;
  return <ComparisonPage comparison={comparison} />;
});

const title = "Postrust vs PostGraphile: PostgreSQL GraphQL compared";
const description =
  "How Postrust compares to PostGraphile V5 and its Grafast engine: measured GraphQL throughput on the same schema, REST alongside GraphQL, and where PostGraphile's extensibility wins.";
const url = "https://postrust.org/compare/postgraphile";

export const head: DocumentHead = {
  title,
  links: [{ rel: "canonical", href: url }],
  meta: [
    { name: "description", content: description },
    {
      name: "keywords",
      content:
        "postrust vs postgraphile, postgraphile alternative, postgresql graphql, grafast",
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
