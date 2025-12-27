import { component$ } from "@builder.io/qwik";

// Data sourced from project README and actual builds
const comparisons = [
  {
    metric: "Cold Start (Lambda)",
    postrust: "~50ms",
    postgrest: "N/A*",
    hasura: "N/A*",
    highlight: true,
  },
  {
    metric: "Binary Size",
    postrust: "~3.5 MB",
    postgrest: "~20 MB",
    hasura: "Container",
    highlight: false,
  },
  {
    metric: "Serverless",
    postrust: "Native",
    postgrest: "Via containers",
    hasura: "Cloud / containers",
    highlight: true,
  },
  {
    metric: "GraphQL",
    postrust: "Built-in",
    postgrest: "Not supported",
    hasura: "Built-in",
    highlight: true,
  },
  {
    metric: "Admin UI",
    postrust: "Built-in",
    postgrest: "Not included",
    hasura: "Cloud only",
    highlight: false,
  },
  {
    metric: "License",
    postrust: "MIT",
    postgrest: "MIT",
    hasura: "Apache 2.0",
    highlight: false,
  },
];

export const PerformanceSection = component$(() => {
  return (
    <section class="section-padding bg-neutral-50">
      <div class="container-wide">
        {/* Section Header */}
        <div class="text-center max-w-3xl mx-auto mb-16">
          <div class="inline-flex items-center gap-2 px-3 py-1 bg-green-100 text-green-700 rounded-full text-sm font-medium mb-4">
            <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13 10V3L4 14h7v7l9-11h-7z"/>
            </svg>
            Performance
          </div>
          <h2 class="text-3xl md:text-4xl font-bold text-neutral-900 mb-4">
            Built for speed and efficiency
          </h2>
          <p class="text-lg text-neutral-600">
            Rust's zero-cost abstractions and efficient memory management make
            Postrust ideal for serverless deployments where cold start matters.
          </p>
        </div>

        {/* Performance Visualization */}
        <div class="grid lg:grid-cols-2 gap-12 items-center mb-16">
          {/* Cold Start Comparison */}
          <div class="bg-white rounded-2xl p-8 shadow-sm border border-neutral-200">
            <h3 class="text-xl font-semibold text-neutral-900 mb-6">
              Why Serverless-First Matters
            </h3>
            <div class="space-y-4">
              {/* Native Lambda */}
              <div class="p-4 bg-primary-50 rounded-lg border border-primary-200">
                <div class="flex items-center gap-3 mb-2">
                  <svg class="w-5 h-5 text-primary-600" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 13l4 4L19 7"/>
                  </svg>
                  <span class="font-medium text-neutral-900">Native AWS Lambda Support</span>
                </div>
                <p class="text-sm text-neutral-600 ml-8">
                  Single binary compiled with cargo-lambda, ~50ms cold start
                </p>
              </div>
              {/* Binary Size */}
              <div class="p-4 bg-neutral-50 rounded-lg border border-neutral-200">
                <div class="flex items-center gap-3 mb-2">
                  <svg class="w-5 h-5 text-primary-600" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 13l4 4L19 7"/>
                  </svg>
                  <span class="font-medium text-neutral-900">3.5 MB Binary</span>
                </div>
                <p class="text-sm text-neutral-600 ml-8">
                  LTO-optimized release build, no runtime dependencies
                </p>
              </div>
              {/* No Container Needed */}
              <div class="p-4 bg-neutral-50 rounded-lg border border-neutral-200">
                <div class="flex items-center gap-3 mb-2">
                  <svg class="w-5 h-5 text-primary-600" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 13l4 4L19 7"/>
                  </svg>
                  <span class="font-medium text-neutral-900">No Container Required</span>
                </div>
                <p class="text-sm text-neutral-600 ml-8">
                  Deploy as native binary, skip container overhead
                </p>
              </div>
            </div>
            <p class="mt-6 text-sm text-neutral-500">
              Benchmarks from project README. Run your own tests for production decisions.
            </p>
          </div>

          {/* Key Stats */}
          <div class="grid grid-cols-2 gap-6">
            <div class="bg-white rounded-xl p-6 shadow-sm border border-neutral-200 text-center">
              <div class="text-4xl font-bold text-primary-600 mb-2">~50ms</div>
              <div class="text-sm text-neutral-600">Lambda cold start</div>
            </div>
            <div class="bg-white rounded-xl p-6 shadow-sm border border-neutral-200 text-center">
              <div class="text-4xl font-bold text-primary-600 mb-2">3.5 MB</div>
              <div class="text-sm text-neutral-600">Binary size</div>
            </div>
            <div class="bg-white rounded-xl p-6 shadow-sm border border-neutral-200 text-center">
              <div class="text-4xl font-bold text-primary-600 mb-2">REST+</div>
              <div class="text-sm text-neutral-600">GraphQL built-in</div>
            </div>
            <div class="bg-white rounded-xl p-6 shadow-sm border border-neutral-200 text-center">
              <div class="text-4xl font-bold text-primary-600 mb-2">0</div>
              <div class="text-sm text-neutral-600">Runtime dependencies</div>
            </div>
          </div>
        </div>

        {/* Comparison Table */}
        <div class="bg-white rounded-2xl shadow-sm border border-neutral-200 overflow-hidden">
          <div class="overflow-x-auto">
            <table class="w-full">
              <thead>
                <tr class="bg-neutral-50 border-b border-neutral-200">
                  <th class="px-6 py-4 text-left text-sm font-semibold text-neutral-900">Feature</th>
                  <th class="px-6 py-4 text-center text-sm font-semibold text-primary-600">
                    <div class="flex items-center justify-center gap-2">
                      <div class="w-6 h-6 bg-primary-100 rounded flex items-center justify-center">
                        <svg class="w-4 h-4 text-primary-600" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5">
                          <path d="M13 2L3 14h9l-1 8 10-12h-9l1-8z" stroke-linecap="round" stroke-linejoin="round"/>
                        </svg>
                      </div>
                      Postrust
                    </div>
                  </th>
                  <th class="px-6 py-4 text-center text-sm font-semibold text-neutral-600">PostgREST</th>
                  <th class="px-6 py-4 text-center text-sm font-semibold text-neutral-600">Hasura</th>
                </tr>
              </thead>
              <tbody class="divide-y divide-neutral-100">
                {comparisons.map((row) => (
                  <tr key={row.metric} class={row.highlight ? "bg-primary-50/50" : ""}>
                    <td class="px-6 py-4 text-sm font-medium text-neutral-900">{row.metric}</td>
                    <td class="px-6 py-4 text-center text-sm font-semibold text-primary-600">{row.postrust}</td>
                    <td class="px-6 py-4 text-center text-sm text-neutral-600">{row.postgrest}</td>
                    <td class="px-6 py-4 text-center text-sm text-neutral-600">{row.hasura}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          <div class="px-6 py-4 bg-neutral-50 border-t border-neutral-200">
            <p class="text-xs text-neutral-500">
              * PostgREST and Hasura typically run as containers, not native Lambda functions. Postrust can also run as a container if preferred.
            </p>
          </div>
        </div>
      </div>
    </section>
  );
});
