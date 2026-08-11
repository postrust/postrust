import { component$ } from "@builder.io/qwik";
import type { DocumentHead } from "@builder.io/qwik-city";
import { Link } from "@builder.io/qwik-city";

const comparisons = [
  {
    name: "PostgREST",
    slug: "postgrest",
    description:
      "The project that established the idea. Written in Haskell, REST only.",
    pros: [
      "Years of production use",
      "Large community",
      "Excellent documentation",
    ],
    cons: ["No GraphQL", "No subscriptions", "Logic lives in SQL"],
  },
  {
    name: "Hasura",
    slug: "hasura",
    description: "A GraphQL platform with permissions modelled as metadata.",
    pros: [
      "Per-field permissions",
      "Multiple data sources",
      "Event triggers and console",
    ],
    cons: [
      "Tables must be tracked first",
      "A platform to operate",
      "Heavier footprint",
    ],
  },
  {
    name: "PostGraphile",
    slug: "postgraphile",
    description:
      "GraphQL from your schema, built to be reshaped. V5 plans with Gra*fast*.",
    pros: ["Deeply extensible", "Relay support", "TypeScript plugins"],
    cons: ["No REST surface", "Needs a Node runtime"],
  },
  {
    name: "Supabase",
    slug: "supabase",
    description: "A backend platform. Its REST layer is PostgREST.",
    pros: [
      "Auth, storage and realtime included",
      "Managed hosting",
      "Generous free tier",
    ],
    cons: ["A platform, not a server", "Less control when self-hosted"],
  },
];

// Feature rows only. Throughput and image size are measured per tool by
// scripts/bench-compare.sh and shown on each /compare/<tool> page, rather than
// summarised into a single cell here.
const detailedComparison = [
  {
    feature: "Language",
    postrust: "Rust",
    postgrest: "Haskell",
    hasura: "Haskell",
    postgraphile: "TypeScript",
    supabase: "Platform",
  },
  {
    feature: "REST API",
    postrust: "Yes",
    postgrest: "Yes",
    hasura: "RESTified endpoints",
    postgraphile: "No",
    supabase: "Yes (PostgREST)",
  },
  {
    feature: "GraphQL",
    postrust: "Built in",
    postgrest: "No",
    hasura: "Built in",
    postgraphile: "Built in",
    supabase: "Via extension",
  },
  {
    feature: "Subscriptions",
    postrust: "LISTEN/NOTIFY",
    postgrest: "No",
    hasura: "Yes",
    postgraphile: "LISTEN/NOTIFY",
    supabase: "Realtime service",
  },
  {
    feature: "Permissions",
    postrust: "Roles + RLS",
    postgrest: "Roles + RLS",
    hasura: "Metadata",
    postgraphile: "Roles + RLS",
    supabase: "Roles + RLS",
  },
  {
    feature: "Setup before first query",
    postrust: "None",
    postgrest: "None",
    hasura: "Track tables",
    postgraphile: "None",
    supabase: "None",
  },
  {
    feature: "Custom routes in-process",
    postrust: "Axum, needs rebuild",
    postgrest: "No",
    hasura: "Actions",
    postgraphile: "Plugins",
    supabase: "Edge Functions",
  },
  {
    feature: "Schema customisation",
    postrust: "No",
    postgrest: "No",
    hasura: "Metadata",
    postgraphile: "Plugin system",
    supabase: "No",
  },
  {
    feature: "Runtime",
    postrust: "Static binary",
    postgrest: "Binary",
    hasura: "Container",
    postgraphile: "Node.js",
    supabase: "Managed / stack",
  },
  {
    feature: "AWS Lambda",
    postrust: "Native crate",
    postgrest: "Container",
    hasura: "Container",
    postgraphile: "Eject to serverless",
    supabase: "No",
  },
  {
    feature: "Self-hosted",
    postrust: "Yes",
    postgrest: "Yes",
    hasura: "Yes (v2 OSS)",
    postgraphile: "Yes",
    supabase: "Yes (full stack)",
  },
  {
    feature: "License",
    postrust: "MIT",
    postgrest: "MIT",
    hasura: "Apache 2.0 (v2)",
    postgraphile: "MIT",
    supabase: "Apache 2.0",
  },
];

export default component$(() => {
  return (
    <div class="min-h-screen bg-white">
      {/* Hero */}
      <section class="section-padding bg-gradient-to-b from-neutral-50 to-white">
        <div class="container-wide">
          <div class="mx-auto max-w-3xl text-center">
            <h1 class="mb-6 text-4xl font-bold text-neutral-900 md:text-5xl">
              How Postrust Compares
            </h1>
            <p class="text-lg text-neutral-600">
              See how Postrust stacks up against other PostgreSQL API solutions.
              Choose the right tool for your needs.
            </p>
          </div>
        </div>
      </section>

      {/* Comparison Cards */}
      <section class="section-padding">
        <div class="container-wide">
          <div class="mb-16 grid gap-8 md:grid-cols-2 lg:grid-cols-4">
            {comparisons.map((item) => (
              <div
                key={item.slug}
                class="rounded-2xl border border-neutral-200 bg-white p-8 transition-shadow hover:shadow-lg"
              >
                <h2 class="mb-2 text-2xl font-bold text-neutral-900">
                  vs {item.name}
                </h2>
                <p class="mb-6 text-neutral-600">{item.description}</p>

                <div class="mb-6">
                  <h3 class="mb-3 text-sm font-semibold tracking-wide text-neutral-500 uppercase">
                    Their Strengths
                  </h3>
                  <ul class="space-y-2">
                    {item.pros.map((pro) => (
                      <li
                        key={pro}
                        class="flex items-start gap-2 text-sm text-neutral-700"
                      >
                        <svg
                          class="mt-0.5 h-4 w-4 flex-shrink-0 text-green-500"
                          fill="none"
                          stroke="currentColor"
                          viewBox="0 0 24 24"
                        >
                          <path
                            stroke-linecap="round"
                            stroke-linejoin="round"
                            stroke-width="2"
                            d="M5 13l4 4L19 7"
                          />
                        </svg>
                        {pro}
                      </li>
                    ))}
                  </ul>
                </div>

                <div class="mb-6">
                  <h3 class="mb-3 text-sm font-semibold tracking-wide text-neutral-500 uppercase">
                    Their Limitations
                  </h3>
                  <ul class="space-y-2">
                    {item.cons.map((con) => (
                      <li
                        key={con}
                        class="flex items-start gap-2 text-sm text-neutral-700"
                      >
                        <svg
                          class="mt-0.5 h-4 w-4 flex-shrink-0 text-red-500"
                          fill="none"
                          stroke="currentColor"
                          viewBox="0 0 24 24"
                        >
                          <path
                            stroke-linecap="round"
                            stroke-linejoin="round"
                            stroke-width="2"
                            d="M6 18L18 6M6 6l12 12"
                          />
                        </svg>
                        {con}
                      </li>
                    ))}
                  </ul>
                </div>

                <Link
                  href={`/compare/${item.slug}`}
                  class="text-primary-600 hover:text-primary-700 inline-flex items-center text-sm font-medium"
                >
                  Detailed comparison
                  <svg
                    class="ml-1 h-4 w-4"
                    fill="none"
                    stroke="currentColor"
                    viewBox="0 0 24 24"
                  >
                    <path
                      stroke-linecap="round"
                      stroke-linejoin="round"
                      stroke-width="2"
                      d="M13 7l5 5m0 0l-5 5m5-5H6"
                    />
                  </svg>
                </Link>
              </div>
            ))}
          </div>

          {/* Detailed Table */}
          <div class="overflow-hidden rounded-2xl border border-neutral-200 bg-white">
            <div class="border-b border-neutral-200 p-6">
              <h2 class="text-2xl font-bold text-neutral-900">
                Feature Comparison
              </h2>
            </div>
            <div class="overflow-x-auto">
              <table class="w-full">
                <thead>
                  <tr class="border-b border-neutral-200 bg-neutral-50">
                    <th class="px-6 py-4 text-left text-sm font-semibold text-neutral-900">
                      Feature
                    </th>
                    <th class="text-primary-600 px-6 py-4 text-center text-sm font-semibold">
                      <div class="flex items-center justify-center gap-2">
                        <div class="bg-primary-100 flex h-5 w-5 items-center justify-center rounded">
                          <svg
                            class="text-primary-600 h-3 w-3"
                            viewBox="0 0 24 24"
                            fill="none"
                            stroke="currentColor"
                            stroke-width="3"
                          >
                            <path
                              d="M13 2L3 14h9l-1 8 10-12h-9l1-8z"
                              stroke-linecap="round"
                              stroke-linejoin="round"
                            />
                          </svg>
                        </div>
                        Postrust
                      </div>
                    </th>
                    <th class="px-6 py-4 text-center text-sm font-semibold text-neutral-600">
                      PostgREST
                    </th>
                    <th class="px-6 py-4 text-center text-sm font-semibold text-neutral-600">
                      Hasura
                    </th>
                    <th class="px-6 py-4 text-center text-sm font-semibold text-neutral-600">
                      PostGraphile
                    </th>
                    <th class="px-6 py-4 text-center text-sm font-semibold text-neutral-600">
                      Supabase
                    </th>
                  </tr>
                </thead>
                <tbody class="divide-y divide-neutral-100">
                  {detailedComparison.map((row) => (
                    <tr key={row.feature}>
                      <td class="px-6 py-4 text-sm font-medium text-neutral-900">
                        {row.feature}
                      </td>
                      <td class="text-primary-600 px-6 py-4 text-center text-sm font-medium">
                        {row.postrust}
                      </td>
                      <td class="px-6 py-4 text-center text-sm text-neutral-600">
                        {row.postgrest}
                      </td>
                      <td class="px-6 py-4 text-center text-sm text-neutral-600">
                        {row.hasura}
                      </td>
                      <td class="px-6 py-4 text-center text-sm text-neutral-600">
                        {row.postgraphile}
                      </td>
                      <td class="px-6 py-4 text-center text-sm text-neutral-600">
                        {row.supabase}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
            <div class="border-t border-neutral-200 bg-neutral-50 p-4">
              <p class="text-xs text-neutral-500">
                Feature rows describe what each tool does today, with the
                condition in the cell where one applies. Throughput, memory and
                image size are measured per tool by scripts/bench-compare.sh and
                reported on each comparison page, including the scenarios where
                the other tool is faster.
              </p>
            </div>
          </div>
        </div>
      </section>

      {/* CTA */}
      <section class="section-padding bg-neutral-50">
        <div class="container-wide text-center">
          <h2 class="mb-4 text-3xl font-bold text-neutral-900">
            Ready to try Postrust?
          </h2>
          <p class="mx-auto mb-8 max-w-2xl text-lg text-neutral-600">
            Get started in minutes with our quick start guide.
          </p>
          <Link
            href="/docs/getting-started"
            class="inline-flex items-center gap-2 rounded-lg bg-neutral-900 px-8 py-4 text-base font-semibold text-white transition-colors hover:bg-neutral-800"
          >
            Get Started
            <svg
              class="h-4 w-4"
              fill="none"
              stroke="currentColor"
              viewBox="0 0 24 24"
            >
              <path
                stroke-linecap="round"
                stroke-linejoin="round"
                stroke-width="2"
                d="M13 7l5 5m0 0l-5 5m5-5H6"
              />
            </svg>
          </Link>
        </div>
      </section>
    </div>
  );
});

export const head: DocumentHead = {
  title: "Compare Postrust with PostgREST, Hasura, PostGraphile and Supabase",
  meta: [
    {
      name: "description",
      content:
        "Feature and measured performance comparisons between Postrust and PostgREST, Hasura, PostGraphile and Supabase, including where each of them is the better choice.",
    },
  ],
};
