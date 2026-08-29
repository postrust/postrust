import { component$ } from "@builder.io/qwik";
import type { DocumentHead } from "@builder.io/qwik-city";
import { Link } from "@builder.io/qwik-city";
import {
  conformance,
  conformanceMeta,
  comparedHeaders,
  worstSpecs,
} from "~/data/conformance";

const groups = [
  { key: "all" as const, label: "All cases" },
  { key: "reads" as const, label: "Reads (GET, HEAD, OPTIONS)" },
  { key: "writes" as const, label: "Writes (POST, PATCH, PUT, DELETE)" },
];

const levels = [
  { key: "status" as const, label: "Status code" },
  { key: "statusAndBody" as const, label: "Status and body" },
  { key: "exceptContentRange" as const, label: "…and headers, except Content-Range" },
  { key: "fullContract" as const, label: "Full contract" },
];

const divergences = [
  {
    title: "PostgREST truncates a select at a stray )",
    body: "Probed against the reference directly: /clients?select=id)ZZ,nameQQ returns 200 and nameQQ never becomes a column. Everything after the paren is discarded silently. Postrust rejects it, because matching this means reintroducing a bug that was fixed on purpose — select=id, name, billing(address) used to return the id alone.",
  },
  {
    title: "Two upsert status codes",
    body: "POST with an empty body, and a PUT that replaced an existing row, return 201 where PostgREST returns 200. The evidence is one case each, against 58 that pass.",
  },
  {
    title: "Unspecified row order",
    body: "Two cases return the same rows in a different order, and neither request specifies order=. SQL guarantees nothing there, so both answers are correct and the measurement is over-reporting.",
  },
  {
    title: "Clock skew on nbf and iat, and none on exp",
    body: "PostgREST checks all three to the second. Postrust allows thirty seconds on the two that describe a token not yet valid, and none on the one that describes a token withdrawn — forgiving an expiry keeps a session alive past the moment its issuer ended it.",
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
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 5l7 7-7 7"/>
            </svg>
            <span class="text-neutral-900">PostgREST Conformance</span>
          </div>
          <h1 class="text-4xl font-bold text-neutral-900 mb-4">PostgREST conformance</h1>
          <p class="text-lg text-neutral-600 max-w-2xl">
            How closely the two agree, measured by replaying PostgREST&rsquo;s own test cases
            against both servers and diffing the live responses.
          </p>
        </div>
      </div>

      <div class="container-wide py-12">
        <div class="max-w-3xl">
          {/* Numbers */}
          <section class="mb-12">
            <div class="overflow-x-auto">
              <table class="w-full text-sm">
                <thead>
                  <tr class="border-b border-neutral-200">
                    <th class="text-left py-2 pr-4 font-semibold text-neutral-900">Compared on</th>
                    {groups.map((g) => (
                      <th key={g.key} class="text-right py-2 pl-4 font-semibold text-neutral-900">
                        {g.label}
                        <span class="block font-normal text-neutral-400">
                          {conformance[g.key].cases} cases
                        </span>
                      </th>
                    ))}
                  </tr>
                </thead>
                <tbody>
                  {levels.map((l) => (
                    <tr key={l.key} class="border-b border-neutral-100">
                      <td class="py-3 pr-4 text-neutral-600">{l.label}</td>
                      {groups.map((g) => (
                        <td key={g.key} class="py-3 pl-4 text-right">
                          <span class="font-mono font-semibold text-neutral-900">
                            {conformance[g.key][l.key].pct}%
                          </span>
                          <span class="block text-xs text-neutral-400">
                            {conformance[g.key][l.key].passed}/{conformance[g.key].cases}
                          </span>
                        </td>
                      ))}
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
            <p class="text-sm text-neutral-500 mt-4">
              Measured {conformanceMeta.measured} against PostgREST{" "}
              {conformanceMeta.postgrest}, on Postrust built with{" "}
              <code class="font-mono">{conformanceMeta.features}</code> and running in
              compatibility mode. Every figure is recomputed from the run&rsquo;s per-case output;
              nothing on this page is typed by hand.
            </p>
          </section>

          {/* What counts */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">What is compared</h2>
            <p class="text-neutral-600 mb-4">
              Agreement is reported at four strictness levels, because one systemic gap — a single
              header never emitted — would otherwise sink every case and hide the hundreds that
              differ in nothing else. The strictest is status, body, and these six headers:
            </p>
            <div class="flex flex-wrap gap-2 mb-4">
              {comparedHeaders.map((h) => (
                <code key={h} class="bg-neutral-100 text-neutral-800 px-2 py-1 rounded text-sm font-mono">
                  {h}
                </code>
              ))}
            </div>
            <p class="text-neutral-600">
              <code class="font-mono">Date</code>, <code class="font-mono">Server</code> and{" "}
              <code class="font-mono">Connection</code> differ between any two servers and say
              nothing about conformance, so they are not compared. Bodies are compared as parsed
              JSON, which means object key order does not survive the comparison — but it does
              survive into a CSV response, which puts its columns in key order.
            </p>
          </section>

          {/* Method */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">How it is measured</h2>
            <p class="text-neutral-600 mb-4">
              PostgREST&rsquo;s spec suite cannot be pointed at another server: it drives the WAI
              application in-process and imports its own config directly, so there is no HTTP
              boundary to intercept. Two parts of it are reusable — the fixture database, around
              280 tables of plain SQL, and the request literals inside the examples, each spelling
              out a method, path, headers and body.
            </p>
            <p class="text-neutral-600 mb-4">
              So the harness lifts those requests out of the Haskell and replays them over HTTP
              against both stock PostgREST and Postrust, each on an identically loaded fixture
              database, and diffs the live responses. The reference implementation is the oracle:
              no hspec expectation is ever interpreted, which means a mistake in the extractor
              shows up as a case both servers answer the same way rather than as a false failure.
            </p>
            <pre class="bg-neutral-900 text-neutral-100 rounded-lg p-4 text-sm overflow-x-auto"><code>{`scripts/conformance/conformance.sh`}</code></pre>
          </section>

          {/* Deliberate divergences */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">Where the two disagree on purpose</h2>
            <p class="text-neutral-600 mb-6">
              Some cases fail because PostgREST is wrong, or because neither answer is wrong. They
              are listed so nobody later &ldquo;fixes&rdquo; one without deciding to.
            </p>
            <div class="space-y-4">
              {divergences.map((d) => (
                <div key={d.title} class="p-4 bg-neutral-50 rounded-lg">
                  <h3 class="font-semibold text-neutral-900 mb-1">{d.title}</h3>
                  <p class="text-sm text-neutral-600">{d.body}</p>
                </div>
              ))}
            </div>
          </section>

          {/* Where the rest lives */}
          {worstSpecs.length > 0 && (
            <section class="mb-12">
              <h2 class="text-2xl font-bold text-neutral-900 mb-4">Where the remaining disagreement lives</h2>
              <p class="text-neutral-600 mb-4">
                By spec file, on status and body, worst first.
              </p>
              <div class="overflow-x-auto">
                <table class="w-full text-sm">
                  <thead>
                    <tr class="border-b border-neutral-200">
                      <th class="text-left py-2 pr-4 font-semibold text-neutral-900">Spec</th>
                      <th class="text-right py-2 font-semibold text-neutral-900">Agreement</th>
                    </tr>
                  </thead>
                  <tbody>
                    {worstSpecs.map((s) => (
                      <tr key={s.spec} class="border-b border-neutral-100">
                        <td class="py-2 pr-4"><code class="font-mono text-neutral-700">{s.spec}</code></td>
                        <td class="py-2 text-right">
                          <span class="font-mono text-neutral-900">{s.pct}%</span>
                          <span class="text-neutral-400"> ({s.passed}/{s.total})</span>
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </section>
          )}

          {/* Gaps */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">Known gaps</h2>
            <p class="text-neutral-600 mb-4">
              The largest is the OpenAPI document. PostgREST serves a 638 KB Swagger 2.0 document
              at <code class="font-mono">/</code> — 428 paths, 273 definitions, 1035 parameters.
              Postrust serves OpenAPI 3.0 for its own surface under{" "}
              <code class="font-mono">/admin</code> and does not yet generate PostgREST&rsquo;s.
              Bodies compare as exact JSON, so this is all-or-nothing rather than something to land
              incrementally.
            </p>
            <p class="text-neutral-600">
              Parse errors are the other: <code class="font-mono">?or=()</code> and some JSON-path
              failures answer generically where PostgREST names the offending character.{" "}
              <code class="font-mono">Prefer: tx=rollback</code> is not implemented, and is no
              longer reported as applied.
            </p>
          </section>

          <div class="flex items-center justify-between pt-8 border-t border-neutral-200">
            <Link href="/docs/benchmarks" class="flex items-center gap-2 text-neutral-600 hover:text-primary-600">
              <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 19l-7-7 7-7"/>
              </svg>
              Benchmarks
            </Link>
            <Link href="/compare/postgrest" class="flex items-center gap-2 text-neutral-600 hover:text-primary-600">
              Postrust vs PostgREST
              <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 5l7 7-7 7"/>
              </svg>
            </Link>
          </div>
        </div>
      </div>
    </div>
  );
});

export const head: DocumentHead = {
  title: "PostgREST Conformance - Postrust Documentation",
  links: [{ rel: "canonical", href: "https://postrust.org/docs/conformance" }],
  meta: [
    {
      name: "description",
      content:
        "How closely Postrust matches PostgREST, measured by replaying PostgREST's own test cases against both servers and diffing the live responses.",
    },
  ],
};
