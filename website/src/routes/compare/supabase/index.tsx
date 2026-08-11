import { component$ } from "@builder.io/qwik";
import type { DocumentHead } from "@builder.io/qwik-city";
import { ComparisonPage } from "~/components/compare/comparison-page";
import { comparisons } from "~/data/comparisons";

const comparison = comparisons.find((c) => c.slug === "supabase");

export default component$(() => {
  if (!comparison) return null;
  return <ComparisonPage comparison={comparison} />;
});

const title = "Postrust vs Supabase: API server or platform";
const description =
  "How Postrust compares to Supabase: a single API server in front of your database versus a full backend platform, and why Supabase's REST layer is really a PostgREST comparison.";
const url = "https://postrust.org/compare/supabase";

export const head: DocumentHead = {
  title,
  links: [{ rel: "canonical", href: url }],
  meta: [
    { name: "description", content: description },
    {
      name: "keywords",
      content:
        "postrust vs supabase, supabase alternative, self-hosted supabase, postgresql api",
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
