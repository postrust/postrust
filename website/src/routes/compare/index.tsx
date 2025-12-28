import { component$ } from "@builder.io/qwik";
import type { DocumentHead } from "@builder.io/qwik-city";
import { Link } from "@builder.io/qwik-city";

const comparisons = [
  {
    name: "PostgREST",
    slug: "postgrest",
    description: "The original PostgreSQL REST API server written in Haskell",
    pros: ["Mature ecosystem", "Wide adoption", "Excellent documentation"],
    cons: ["No custom routes", "No GraphQL", "No realtime subscriptions"],
  },
  {
    name: "Hasura",
    slug: "hasura",
    description: "GraphQL engine with real-time subscriptions",
    pros: ["Feature-rich", "Real-time subscriptions", "Cloud offering"],
    cons: ["No native custom routes", "Resource intensive", "Complex licensing"],
  },
  {
    name: "Supabase",
    slug: "supabase",
    description: "Full Firebase alternative with managed PostgreSQL",
    pros: ["Full platform", "Great DX", "Generous free tier"],
    cons: ["Managed only", "Less control", "PostgREST under the hood"],
  },
];

const detailedComparison = [
  { feature: "Language", postrust: "Rust", postgrest: "Haskell", hasura: "Haskell", supabase: "Elixir + PostgREST" },
  { feature: "Cold Start (Lambda)", postrust: "~50ms", postgrest: "N/A*", hasura: "N/A*", supabase: "N/A (managed)" },
  { feature: "Binary Size", postrust: "~3.5 MB", postgrest: "~20 MB", hasura: "Container", supabase: "N/A (managed)" },
  { feature: "REST API", postrust: "Yes", postgrest: "Yes", hasura: "Yes", supabase: "Yes" },
  { feature: "GraphQL", postrust: "Built-in", postgrest: "No", hasura: "Built-in", supabase: "via pg_graphql" },
  { feature: "Realtime Subscriptions", postrust: "Built-in", postgrest: "No", hasura: "Built-in", supabase: "Via Realtime" },
  { feature: "Custom Routes (Rust)", postrust: "Native Axum", postgrest: "No", hasura: "Actions only", supabase: "Edge Functions" },
  { feature: "pgvector Support", postrust: "Native", postgrest: "Limited", hasura: "Via Remote Schema", supabase: "Native" },
  { feature: "Admin UI", postrust: "Built-in", postgrest: "No", hasura: "Cloud only", supabase: "Built-in" },
  { feature: "Self-Hosted", postrust: "Yes", postgrest: "Yes", hasura: "Yes (OSS)", supabase: "Yes (complex)" },
  { feature: "Serverless Native", postrust: "Yes", postgrest: "Via container", hasura: "Via container", supabase: "No" },
  { feature: "License", postrust: "MIT", postgrest: "MIT", hasura: "Apache 2.0", supabase: "Apache 2.0" },
];

export default component$(() => {
  return (
    <div class="min-h-screen bg-white">
      {/* Hero */}
      <section class="section-padding bg-gradient-to-b from-neutral-50 to-white">
        <div class="container-wide">
          <div class="max-w-3xl mx-auto text-center">
            <h1 class="text-4xl md:text-5xl font-bold text-neutral-900 mb-6">
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
          <div class="grid md:grid-cols-3 gap-8 mb-16">
            {comparisons.map((item) => (
              <div key={item.slug} class="bg-white rounded-2xl p-8 border border-neutral-200 hover:shadow-lg transition-shadow">
                <h2 class="text-2xl font-bold text-neutral-900 mb-2">
                  vs {item.name}
                </h2>
                <p class="text-neutral-600 mb-6">
                  {item.description}
                </p>

                <div class="mb-6">
                  <h3 class="text-sm font-semibold text-neutral-500 uppercase tracking-wide mb-3">
                    Their Strengths
                  </h3>
                  <ul class="space-y-2">
                    {item.pros.map((pro) => (
                      <li key={pro} class="flex items-start gap-2 text-sm text-neutral-700">
                        <svg class="w-4 h-4 text-green-500 mt-0.5 flex-shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 13l4 4L19 7"/>
                        </svg>
                        {pro}
                      </li>
                    ))}
                  </ul>
                </div>

                <div class="mb-6">
                  <h3 class="text-sm font-semibold text-neutral-500 uppercase tracking-wide mb-3">
                    Their Limitations
                  </h3>
                  <ul class="space-y-2">
                    {item.cons.map((con) => (
                      <li key={con} class="flex items-start gap-2 text-sm text-neutral-700">
                        <svg class="w-4 h-4 text-red-500 mt-0.5 flex-shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12"/>
                        </svg>
                        {con}
                      </li>
                    ))}
                  </ul>
                </div>

                <Link
                  href={`/compare/${item.slug}`}
                  class="inline-flex items-center text-sm font-medium text-primary-600 hover:text-primary-700"
                >
                  Detailed comparison
                  <svg class="w-4 h-4 ml-1" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13 7l5 5m0 0l-5 5m5-5H6"/>
                  </svg>
                </Link>
              </div>
            ))}
          </div>

          {/* Detailed Table */}
          <div class="bg-white rounded-2xl border border-neutral-200 overflow-hidden">
            <div class="p-6 border-b border-neutral-200">
              <h2 class="text-2xl font-bold text-neutral-900">
                Feature Comparison
              </h2>
            </div>
            <div class="overflow-x-auto">
              <table class="w-full">
                <thead>
                  <tr class="bg-neutral-50 border-b border-neutral-200">
                    <th class="px-6 py-4 text-left text-sm font-semibold text-neutral-900">Feature</th>
                    <th class="px-6 py-4 text-center text-sm font-semibold text-primary-600">
                      <div class="flex items-center justify-center gap-2">
                        <div class="w-5 h-5 bg-primary-100 rounded flex items-center justify-center">
                          <svg class="w-3 h-3 text-primary-600" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3">
                            <path d="M13 2L3 14h9l-1 8 10-12h-9l1-8z" stroke-linecap="round" stroke-linejoin="round"/>
                          </svg>
                        </div>
                        Postrust
                      </div>
                    </th>
                    <th class="px-6 py-4 text-center text-sm font-semibold text-neutral-600">PostgREST</th>
                    <th class="px-6 py-4 text-center text-sm font-semibold text-neutral-600">Hasura</th>
                    <th class="px-6 py-4 text-center text-sm font-semibold text-neutral-600">Supabase</th>
                  </tr>
                </thead>
                <tbody class="divide-y divide-neutral-100">
                  {detailedComparison.map((row) => (
                    <tr key={row.feature}>
                      <td class="px-6 py-4 text-sm font-medium text-neutral-900">{row.feature}</td>
                      <td class="px-6 py-4 text-center text-sm font-medium text-primary-600">{row.postrust}</td>
                      <td class="px-6 py-4 text-center text-sm text-neutral-600">{row.postgrest}</td>
                      <td class="px-6 py-4 text-center text-sm text-neutral-600">{row.hasura}</td>
                      <td class="px-6 py-4 text-center text-sm text-neutral-600">{row.supabase}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
            <div class="p-4 bg-neutral-50 border-t border-neutral-200">
              <p class="text-xs text-neutral-500">
                * PostgREST and Hasura typically run as containers, not native Lambda functions. Cold start times are from project README; run your own benchmarks for production decisions.
              </p>
            </div>
          </div>
        </div>
      </section>

      {/* CTA */}
      <section class="section-padding bg-neutral-50">
        <div class="container-wide text-center">
          <h2 class="text-3xl font-bold text-neutral-900 mb-4">
            Ready to try Postrust?
          </h2>
          <p class="text-lg text-neutral-600 mb-8 max-w-2xl mx-auto">
            Get started in minutes with our quick start guide.
          </p>
          <Link
            href="/docs/getting-started"
            class="inline-flex items-center gap-2 px-8 py-4 text-base font-semibold text-white bg-neutral-900 hover:bg-neutral-800 rounded-lg transition-colors"
          >
            Get Started
            <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13 7l5 5m0 0l-5 5m5-5H6"/>
            </svg>
          </Link>
        </div>
      </section>
    </div>
  );
});

export const head: DocumentHead = {
  title: "Compare - Postrust vs PostgREST, Hasura, Supabase",
  meta: [
    {
      name: "description",
      content: "Compare Postrust with PostgREST, Hasura, and Supabase. See why Postrust is the best choice for serverless PostgreSQL APIs.",
    },
  ],
};
