import { component$ } from "@builder.io/qwik";
import type { DocumentHead } from "@builder.io/qwik-city";
import { ComparisonPage } from "~/components/compare/comparison-page";
import { comparisons } from "~/data/comparisons";

const comparison = comparisons.find((c) => c.slug === "postgrest");

export default component$(() => {
  if (!comparison) return null;
  return <ComparisonPage comparison={comparison} />;
});

const title = "Postrust vs PostgREST: REST API from PostgreSQL compared";
const description =
  "How Postrust compares to PostgREST: shared query grammar, built-in GraphQL and subscriptions, conformance measured by replaying PostgREST's own test suite, and where PostgREST is the better choice.";
const url = "https://postrust.org/compare/postgrest";

export const head: DocumentHead = {
  title,
  links: [{ rel: "canonical", href: url }],
  meta: [
    { name: "description", content: description },
    {
      name: "keywords",
      content:
        "postrust vs postgrest, postgrest alternative, postgresql rest api, postgrest rust",
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
