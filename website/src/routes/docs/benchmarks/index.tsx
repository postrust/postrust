import { component$ } from "@builder.io/qwik";
import type { DocumentHead } from "@builder.io/qwik-city";
import { Link } from "@builder.io/qwik-city";

const scenarios = [
  { name: "Point lookup", request: "?id=eq.42", rps: "6065", p50: "8.0", p95: "10.0", p99: "22.0" },
  { name: "25-row page", request: "?select=id,name,price&limit=25", rps: "6065", p50: "8.0", p95: "11.0", p99: "13.0" },
  { name: "Filtered + ordered page", request: "?category=eq.cat-5&order=id.desc&limit=25", rps: "5025", p50: "9.0", p95: "14.0", p99: "23.0" },
  { name: "Page with exact count", request: "Prefer: count=exact", rps: "6373", p50: "7.0", p95: "11.0", p99: "19.0" },
  { name: "Range filter on numeric", request: "?price=gt.50&select=id,price&limit=25", rps: "6468", p50: "7.0", p95: "10.0", p99: "14.0" },
];

const binaryBuilds = [
  {
    build: "Default features",
    command: "cargo build --release -p postrust-server",
    bytes: "3,070,080",
    size: "2.93 MiB",
    note: "REST + GraphQL only",
  },
  {
    build: "With admin-ui",
    command: "cargo build --release -p postrust-server --features admin-ui",
    bytes: "5,220,864",
    size: "4.98 MiB",
    note: "What the Docker image and releases ship",
  },
];

const caveats = [
  {
    title: "Everything shares one machine",
    body: "PostgreSQL, Postrust and the load generator compete for the same cores over loopback. These numbers compare Postrust against itself across changes; they are not capacity planning figures.",
  },
  {
    title: "Latency is dominated by contention",
    body: "At concurrency 50 on a laptop, p50 sits around 8 ms. The same request against an idle server is roughly an order of magnitude faster.",
  },
  {
    title: "ab is the weakest load generator",
    body: "It is HTTP/1.0 only and its percentiles are coarse. Install oha for numbers worth comparing between runs.",
  },
  {
    title: "Cold start is measured elsewhere",
    body: "The ~50 ms Lambda cold-start figure comes from a different environment and is not produced by this harness.",
  },
  {
    title: "PostgREST is not measured here",
    body: "Figures for other projects in comparison tables come from their own documentation, not from this script.",
  },
];

export default component$(() => {
  return (
    <div class="min-h-screen bg-white">
      <div class="bg-gradient-to-b from-neutral-50 to-white border-b border-neutral-200">
        <div class="container-wide py-12">
          <div class="flex items-center gap-2 text-sm text-neutral-500 mb-4">
            <Link href="/docs" class="hover:text-primary-600">Docs</Link>
            <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 5l7 7-7 7" />
            </svg>
            <span class="text-neutral-900">Benchmarks</span>
          </div>
          <h1 class="text-4xl font-bold text-neutral-900 mb-4">Benchmarks</h1>
          <p class="text-lg text-neutral-600 max-w-2xl">
            Every performance number we publish comes from a script in the repository.
            Run it yourself and check.
          </p>
        </div>
      </div>

      <div class="container-wide py-12">
        <div class="max-w-3xl">
          {/* Reproduce */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">Reproduce it</h2>
            <p class="text-neutral-600 mb-4">
              The harness is self-contained: it builds a release binary, starts a throwaway
              PostgreSQL container, loads 100,000 rows, runs each scenario and prints a table.
              Everything it created is removed on exit.
            </p>
            <div class="bg-neutral-900 rounded-xl overflow-hidden mb-4">
              <pre class="p-4 text-sm overflow-x-auto">
                <code class="text-neutral-100">{`git clone https://github.com/postrust/postrust
cd postrust
./scripts/bench.sh

# Heavier load
REQUESTS=20000 CONCURRENCY=100 ./scripts/bench.sh

# Measure a minimal build instead of the shipped feature set
BENCH_FEATURES="" ./scripts/bench.sh`}</code>
              </pre>
            </div>
            <p class="text-sm text-neutral-500">
              Requires docker, cargo, curl, and one of oha, hey or ab as the load generator.
            </p>
          </section>

          {/* Binary size */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">Binary size</h2>
            <p class="text-neutral-600 mb-4">
              Size depends on the feature set, so quoting one number without the other
              is misleading. The published Docker image and release binaries are built
              with <code class="px-1 py-0.5 bg-neutral-100 rounded text-sm">admin-ui</code>,
              so ~5 MB describes what you actually download.
            </p>
            <div class="overflow-x-auto">
              <table class="w-full text-sm border border-neutral-200 rounded-lg">
                <thead class="bg-neutral-50">
                  <tr>
                    <th class="text-left px-4 py-3 font-medium text-neutral-900">Build</th>
                    <th class="text-right px-4 py-3 font-medium text-neutral-900">Bytes</th>
                    <th class="text-right px-4 py-3 font-medium text-neutral-900">Size</th>
                    <th class="text-left px-4 py-3 font-medium text-neutral-900">Contents</th>
                  </tr>
                </thead>
                <tbody>
                  {binaryBuilds.map((row) => (
                    <tr key={row.build} class="border-t border-neutral-200">
                      <td class="px-4 py-3 text-neutral-900 font-medium">{row.build}</td>
                      <td class="px-4 py-3 text-right text-neutral-600 tabular-nums">{row.bytes}</td>
                      <td class="px-4 py-3 text-right text-neutral-900 font-medium tabular-nums">{row.size}</td>
                      <td class="px-4 py-3 text-neutral-600">{row.note}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
            <p class="text-sm text-neutral-500 mt-3">
              Measured on macOS arm64, LTO release build, stripped. A Linux x86_64 build differs somewhat.
            </p>
          </section>

          {/* Throughput and latency */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">Throughput and latency</h2>
            <p class="text-neutral-600 mb-4">
              Against a 100,000-row table with indexes on{" "}
              <code class="px-1 py-0.5 bg-neutral-100 rounded text-sm">category</code> and{" "}
              <code class="px-1 py-0.5 bg-neutral-100 rounded text-sm">price</code>,
              3,000 requests per scenario at concurrency 50.
            </p>
            <div class="overflow-x-auto">
              <table class="w-full text-sm border border-neutral-200 rounded-lg">
                <thead class="bg-neutral-50">
                  <tr>
                    <th class="text-left px-4 py-3 font-medium text-neutral-900">Scenario</th>
                    <th class="text-right px-4 py-3 font-medium text-neutral-900">req/s</th>
                    <th class="text-right px-4 py-3 font-medium text-neutral-900">p50</th>
                    <th class="text-right px-4 py-3 font-medium text-neutral-900">p95</th>
                    <th class="text-right px-4 py-3 font-medium text-neutral-900">p99</th>
                  </tr>
                </thead>
                <tbody>
                  {scenarios.map((row) => (
                    <tr key={row.name} class="border-t border-neutral-200">
                      <td class="px-4 py-3">
                        <div class="text-neutral-900 font-medium">{row.name}</div>
                        <div class="text-neutral-500 text-xs font-mono mt-0.5">{row.request}</div>
                      </td>
                      <td class="px-4 py-3 text-right text-neutral-900 font-medium tabular-nums">{row.rps}</td>
                      <td class="px-4 py-3 text-right text-neutral-600 tabular-nums">{row.p50} ms</td>
                      <td class="px-4 py-3 text-right text-neutral-600 tabular-nums">{row.p95} ms</td>
                      <td class="px-4 py-3 text-right text-neutral-600 tabular-nums">{row.p99} ms</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
            <p class="text-sm text-neutral-500 mt-3">
              Apple M-series laptop, PostgreSQL 18 in Docker, ab as the load generator, all over loopback.
            </p>
          </section>

          {/* Memory */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">Memory</h2>
            <div class="grid grid-cols-2 gap-6 mb-4">
              <div class="bg-neutral-50 rounded-xl p-6 border border-neutral-200 text-center">
                <div class="text-4xl font-bold text-primary-600 mb-2">10.0 MB</div>
                <div class="text-sm text-neutral-600">RSS before serving any request</div>
              </div>
              <div class="bg-neutral-50 rounded-xl p-6 border border-neutral-200 text-center">
                <div class="text-4xl font-bold text-primary-600 mb-2">13.3 MB</div>
                <div class="text-sm text-neutral-600">RSS after 15,000 requests</div>
              </div>
            </div>
            <p class="text-neutral-600">
              Sustained load at concurrency 50 adds a few megabytes over idle and then
              levels off. Requests that return very large result sets are the exception:
              fetching all 100,000 rows in one response pushes RSS an order of magnitude
              higher, and the allocator does not immediately return it. Paginate.
            </p>
          </section>

          {/* Caveats */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">Caveats</h2>
            <p class="text-neutral-600 mb-4">
              Read these before quoting any throughput number, including ours.
            </p>
            <div class="space-y-4">
              {caveats.map((item) => (
                <div key={item.title} class="p-4 bg-amber-50 rounded-lg border border-amber-200">
                  <div class="font-medium text-neutral-900 mb-1">{item.title}</div>
                  <p class="text-sm text-neutral-600">{item.body}</p>
                </div>
              ))}
            </div>
          </section>

          {/* Correctness */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">Correctness first</h2>
            <p class="text-neutral-600 mb-4">
              A benchmark that measures a broken endpoint is worse than no benchmark, so
              the harness verifies every scenario returns a success status before timing
              it, and the query-parameter behaviour it depends on is covered by
              integration tests against a real database.
            </p>
            <div class="bg-neutral-900 rounded-xl overflow-hidden">
              <pre class="p-4 text-sm overflow-x-auto">
                <code class="text-neutral-100">{`DATABASE_URL="postgres://postgres:postgres@localhost:5432/postrust_test" \\
  cargo test -p postrust-server --test query_params -- --ignored`}</code>
              </pre>
            </div>
          </section>

          {/* Nav */}
          <div class="flex items-center justify-between pt-8 border-t border-neutral-200">
            <Link href="/docs/deployment" class="flex items-center gap-2 text-neutral-600 hover:text-primary-600">
              <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 19l-7-7 7-7" />
              </svg>
              Deployment
            </Link>
            <Link href="/docs/configuration" class="flex items-center gap-2 text-neutral-600 hover:text-primary-600">
              Configuration
              <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 5l7 7-7 7" />
              </svg>
            </Link>
          </div>
        </div>
      </div>
    </div>
  );
});

export const head: DocumentHead = {
  title: "Benchmarks - Postrust Documentation",
  meta: [
    {
      name: "description",
      content:
        "Measured binary size, request latency, throughput and memory for Postrust, with the script that reproduces every number.",
    },
  ],
};
