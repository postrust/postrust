import { component$ } from "@builder.io/qwik";
import type { DocumentHead } from "@builder.io/qwik-city";
import { Link } from "@builder.io/qwik-city";
import { measuredFor, benchMeta } from "~/data/measured";

const scenarios = [
  {
    name: "Point lookup",
    request: "?id=eq.42",
    rps: "6065",
    p50: "8.0",
    p95: "10.0",
    p99: "22.0",
  },
  {
    name: "25-row page",
    request: "?select=id,name,price&limit=25",
    rps: "6065",
    p50: "8.0",
    p95: "11.0",
    p99: "13.0",
  },
  {
    name: "Filtered + ordered page",
    request: "?category=eq.cat-5&order=id.desc&limit=25",
    rps: "5025",
    p50: "9.0",
    p95: "14.0",
    p99: "23.0",
  },
  {
    name: "Page with exact count",
    request: "Prefer: count=exact",
    rps: "6373",
    p50: "7.0",
    p95: "11.0",
    p99: "19.0",
  },
  {
    name: "Range filter on numeric",
    request: "?price=gt.50&select=id,price&limit=25",
    rps: "6468",
    p50: "7.0",
    p95: "10.0",
    p99: "14.0",
  },
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
      <div class="border-b border-neutral-200 bg-gradient-to-b from-neutral-50 to-white">
        <div class="container-wide py-12">
          <div class="mb-4 flex items-center gap-2 text-sm text-neutral-500">
            <Link href="/docs" class="hover:text-primary-600">
              Docs
            </Link>
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
                d="M9 5l7 7-7 7"
              />
            </svg>
            <span class="text-neutral-900">Benchmarks</span>
          </div>
          <h1 class="mb-4 text-4xl font-bold text-neutral-900">Benchmarks</h1>
          <p class="max-w-2xl text-lg text-neutral-600">
            Every performance number we publish comes from a script in the
            repository. Run it yourself and check.
          </p>
        </div>
      </div>

      <div class="container-wide py-12">
        <div class="max-w-3xl">
          {/* Reproduce */}
          <section class="mb-12">
            <h2 class="mb-4 text-2xl font-bold text-neutral-900">
              Reproduce it
            </h2>
            <p class="mb-4 text-neutral-600">
              The harness is self-contained: it builds a release binary, starts
              a throwaway PostgreSQL container, loads 100,000 rows, runs each
              scenario and prints a table. Everything it created is removed on
              exit.
            </p>
            <div class="mb-4 overflow-hidden rounded-xl bg-neutral-900">
              <pre class="overflow-x-auto p-4 text-sm">
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
              Requires docker, cargo, curl, jq, and either oha or ab as the load
              generator.
            </p>
          </section>

          {/* Comparison harness */}
          <section class="mb-12">
            <h2 class="mb-4 text-2xl font-bold text-neutral-900">
              Comparing against other tools
            </h2>
            <p class="mb-4 text-neutral-600">
              A second harness measures Postrust against PostgREST, Hasura and
              PostGraphile. Every server runs as a container on one docker
              network against the same PostgreSQL instance and the same dataset,
              memory is read the same way for all of them, and each keeps its
              own default pool and worker settings. Tuning one and not the
              others measures the tuning rather than the tool.
            </p>
            <div class="mb-4 overflow-hidden rounded-xl bg-neutral-900">
              <div class="border-b border-neutral-700 bg-neutral-800 px-4 py-2">
                <span class="text-sm text-neutral-400">Terminal</span>
              </div>
              <pre class="overflow-x-auto p-4 text-sm">
                <code class="text-neutral-100">{`./scripts/bench-compare.sh

# Alpine base images instead of Debian
VARIANT=alpine ./scripts/bench-compare.sh

# One tool at a time
ONLY=postrust,postgrest ./scripts/bench-compare.sh`}</code>
              </pre>
            </div>
            <p class="mb-4 text-neutral-600">
              Two matrices come out of it, because Hasura and PostGraphile
              expose no REST surface and PostgREST exposes no GraphQL. Results
              are written as{" "}
              <code class="rounded bg-neutral-100 px-1 py-0.5 text-sm">
                results.json
              </code>
              , and the figures published on the comparison pages are copied
              from that file rather than retyped.
            </p>
            <h3 class="mt-8 mb-3 text-lg font-semibold text-neutral-900">
              REST, requests per second
            </h3>
            <div class="mb-6 overflow-x-auto rounded-lg border border-neutral-200">
              <table class="w-full text-sm">
                <thead class="bg-neutral-50">
                  <tr>
                    <th class="px-4 py-3 text-left font-medium text-neutral-900">
                      Scenario
                    </th>
                    <th class="text-primary-600 px-4 py-3 text-right font-medium">
                      Postrust
                    </th>
                    <th class="px-4 py-3 text-right font-medium text-neutral-900">
                      PostgREST
                    </th>
                  </tr>
                </thead>
                <tbody class="divide-y divide-neutral-200">
                  {measuredFor("postgrest", "rest").map((row) => (
                    <tr key={row.scenario}>
                      <td class="px-4 py-3 text-neutral-900">{row.scenario}</td>
                      <td class="px-4 py-3 text-right text-neutral-700 tabular-nums">
                        {row.postrust
                          ? row.postrust.rps.toLocaleString()
                          : "n/a"}
                      </td>
                      <td class="px-4 py-3 text-right text-neutral-700 tabular-nums">
                        {row.other ? row.other.rps.toLocaleString() : "n/a"}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>

            <h3 class="mb-3 text-lg font-semibold text-neutral-900">
              GraphQL, requests per second
            </h3>
            <div class="mb-6 overflow-x-auto rounded-lg border border-neutral-200">
              <table class="w-full text-sm">
                <thead class="bg-neutral-50">
                  <tr>
                    <th class="px-4 py-3 text-left font-medium text-neutral-900">
                      Scenario
                    </th>
                    <th class="text-primary-600 px-4 py-3 text-right font-medium">
                      Postrust
                    </th>
                    <th class="px-4 py-3 text-right font-medium text-neutral-900">
                      Hasura
                    </th>
                    <th class="px-4 py-3 text-right font-medium text-neutral-900">
                      PostGraphile
                    </th>
                  </tr>
                </thead>
                <tbody class="divide-y divide-neutral-200">
                  {measuredFor("hasura", "graphql").map((row, index) => {
                    const pg = measuredFor("postgraphile", "graphql")[index];
                    return (
                      <tr key={row.scenario}>
                        <td class="px-4 py-3 text-neutral-900">
                          {row.scenario}
                        </td>
                        <td class="px-4 py-3 text-right text-neutral-700 tabular-nums">
                          {row.postrust
                            ? row.postrust.rps.toLocaleString()
                            : "n/a"}
                        </td>
                        <td class="px-4 py-3 text-right text-neutral-700 tabular-nums">
                          {row.other ? row.other.rps.toLocaleString() : "n/a"}
                        </td>
                        <td class="px-4 py-3 text-right text-neutral-700 tabular-nums">
                          {pg?.other ? pg.other.rps.toLocaleString() : "n/a"}
                        </td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            </div>

            <p class="mb-6 text-sm text-neutral-500">
              PostgREST appears only in the REST table and the two GraphQL
              engines only in the GraphQL one, because neither exposes the other
              surface. {benchMeta.requests} requests at concurrency{" "}
              {benchMeta.concurrency}, median of {benchMeta.repeats} runs after{" "}
              {benchMeta.warmup} warm-up requests, on {benchMeta.host} against{" "}
              {benchMeta.postgres}. Figures from one run are comparable with
              each other; figures from different runs on a busy machine are not,
              which is why comparing two versions of the same code means running
              both side by side rather than one after the other.
            </p>

            <p class="text-neutral-600">
              Per-tool numbers, including the scenarios where another tool is
              faster, are on the{" "}
              <Link
                href="/compare"
                class="text-primary-600 hover:text-primary-700"
              >
                comparison pages
              </Link>
              .
            </p>
          </section>

          {/* Binary size */}
          <section class="mb-12">
            <h2 class="mb-4 text-2xl font-bold text-neutral-900">
              Binary size
            </h2>
            <p class="mb-4 text-neutral-600">
              Size depends on the feature set, so quoting one number without the
              other is misleading. The published Docker image and release
              binaries are built with{" "}
              <code class="rounded bg-neutral-100 px-1 py-0.5 text-sm">
                admin-ui
              </code>
              , so ~5 MB describes what you actually download.
            </p>
            <div class="overflow-x-auto">
              <table class="w-full rounded-lg border border-neutral-200 text-sm">
                <thead class="bg-neutral-50">
                  <tr>
                    <th class="px-4 py-3 text-left font-medium text-neutral-900">
                      Build
                    </th>
                    <th class="px-4 py-3 text-right font-medium text-neutral-900">
                      Bytes
                    </th>
                    <th class="px-4 py-3 text-right font-medium text-neutral-900">
                      Size
                    </th>
                    <th class="px-4 py-3 text-left font-medium text-neutral-900">
                      Contents
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {binaryBuilds.map((row) => (
                    <tr key={row.build} class="border-t border-neutral-200">
                      <td class="px-4 py-3 font-medium text-neutral-900">
                        {row.build}
                      </td>
                      <td class="px-4 py-3 text-right text-neutral-600 tabular-nums">
                        {row.bytes}
                      </td>
                      <td class="px-4 py-3 text-right font-medium text-neutral-900 tabular-nums">
                        {row.size}
                      </td>
                      <td class="px-4 py-3 text-neutral-600">{row.note}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
            <p class="mt-3 text-sm text-neutral-500">
              Measured on macOS arm64, LTO release build, stripped. A Linux
              x86_64 build differs somewhat.
            </p>
          </section>

          {/* Throughput and latency */}
          <section class="mb-12">
            <h2 class="mb-4 text-2xl font-bold text-neutral-900">
              Throughput and latency
            </h2>
            <p class="mb-4 text-neutral-600">
              Against a 100,000-row table with indexes on{" "}
              <code class="rounded bg-neutral-100 px-1 py-0.5 text-sm">
                category
              </code>{" "}
              and{" "}
              <code class="rounded bg-neutral-100 px-1 py-0.5 text-sm">
                price
              </code>
              , 3,000 requests per scenario at concurrency 50.
            </p>
            <div class="overflow-x-auto">
              <table class="w-full rounded-lg border border-neutral-200 text-sm">
                <thead class="bg-neutral-50">
                  <tr>
                    <th class="px-4 py-3 text-left font-medium text-neutral-900">
                      Scenario
                    </th>
                    <th class="px-4 py-3 text-right font-medium text-neutral-900">
                      req/s
                    </th>
                    <th class="px-4 py-3 text-right font-medium text-neutral-900">
                      p50
                    </th>
                    <th class="px-4 py-3 text-right font-medium text-neutral-900">
                      p95
                    </th>
                    <th class="px-4 py-3 text-right font-medium text-neutral-900">
                      p99
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {scenarios.map((row) => (
                    <tr key={row.name} class="border-t border-neutral-200">
                      <td class="px-4 py-3">
                        <div class="font-medium text-neutral-900">
                          {row.name}
                        </div>
                        <div class="mt-0.5 font-mono text-xs text-neutral-500">
                          {row.request}
                        </div>
                      </td>
                      <td class="px-4 py-3 text-right font-medium text-neutral-900 tabular-nums">
                        {row.rps}
                      </td>
                      <td class="px-4 py-3 text-right text-neutral-600 tabular-nums">
                        {row.p50} ms
                      </td>
                      <td class="px-4 py-3 text-right text-neutral-600 tabular-nums">
                        {row.p95} ms
                      </td>
                      <td class="px-4 py-3 text-right text-neutral-600 tabular-nums">
                        {row.p99} ms
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
            <p class="mt-3 text-sm text-neutral-500">
              Apple M-series laptop, PostgreSQL 18 in Docker, ab as the load
              generator, all over loopback.
            </p>
          </section>

          {/* Memory */}
          <section class="mb-12">
            <h2 class="mb-4 text-2xl font-bold text-neutral-900">Memory</h2>
            <div class="mb-4 grid grid-cols-2 gap-6">
              <div class="rounded-xl border border-neutral-200 bg-neutral-50 p-6 text-center">
                <div class="text-primary-600 mb-2 text-4xl font-bold">
                  10.0 MB
                </div>
                <div class="text-sm text-neutral-600">
                  RSS before serving any request
                </div>
              </div>
              <div class="rounded-xl border border-neutral-200 bg-neutral-50 p-6 text-center">
                <div class="text-primary-600 mb-2 text-4xl font-bold">
                  13.3 MB
                </div>
                <div class="text-sm text-neutral-600">
                  RSS after 15,000 requests
                </div>
              </div>
            </div>
            <p class="text-neutral-600">
              Sustained load at concurrency 50 adds a few megabytes over idle
              and then levels off. Requests that return very large result sets
              are the exception: fetching all 100,000 rows in one response
              pushes RSS an order of magnitude higher, and the allocator does
              not immediately return it. Paginate.
            </p>
          </section>

          {/* Caveats */}
          <section class="mb-12">
            <h2 class="mb-4 text-2xl font-bold text-neutral-900">Caveats</h2>
            <p class="mb-4 text-neutral-600">
              Read these before quoting any throughput number, including ours.
            </p>
            <div class="space-y-4">
              {caveats.map((item) => (
                <div
                  key={item.title}
                  class="rounded-lg border border-amber-200 bg-amber-50 p-4"
                >
                  <div class="mb-1 font-medium text-neutral-900">
                    {item.title}
                  </div>
                  <p class="text-sm text-neutral-600">{item.body}</p>
                </div>
              ))}
            </div>
          </section>

          {/* Correctness */}
          <section class="mb-12">
            <h2 class="mb-4 text-2xl font-bold text-neutral-900">
              Correctness first
            </h2>
            <p class="mb-4 text-neutral-600">
              A benchmark that measures a broken endpoint is worse than no
              benchmark, so the harness verifies every scenario returns a
              success status before timing it, and the query-parameter behaviour
              it depends on is covered by integration tests against a real
              database.
            </p>
            <div class="overflow-hidden rounded-xl bg-neutral-900">
              <pre class="overflow-x-auto p-4 text-sm">
                <code class="text-neutral-100">{`DATABASE_URL="postgres://postgres:postgres@localhost:5432/postrust_test" \\
  cargo test -p postrust-server --test query_params -- --ignored`}</code>
              </pre>
            </div>
          </section>

          {/* Nav */}
          <div class="flex items-center justify-between border-t border-neutral-200 pt-8">
            <Link
              href="/docs/deployment"
              class="hover:text-primary-600 flex items-center gap-2 text-neutral-600"
            >
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
                  d="M15 19l-7-7 7-7"
                />
              </svg>
              Deployment
            </Link>
            <Link
              href="/docs/configuration"
              class="hover:text-primary-600 flex items-center gap-2 text-neutral-600"
            >
              Configuration
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
                  d="M9 5l7 7-7 7"
                />
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
