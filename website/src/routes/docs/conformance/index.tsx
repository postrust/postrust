import { component$ } from "@builder.io/qwik";
import type { DocumentHead } from "@builder.io/qwik-city";
import { Link } from "@builder.io/qwik-city";
import { conformance, conformanceMeta } from "~/data/conformance";
import {
  hasuraConformance,
  hasuraConformanceMeta,
} from "~/data/hasura-conformance";

export default component$(() => {
  const pg = conformance.all;
  const hg = hasuraConformance.all;

  const reports = [
    {
      href: "/docs/conformance/postgrest",
      surface: "REST",
      against: `PostgREST ${conformanceMeta.postgrest}`,
      headline: pg.statusAndBody.pct,
      headlineLabel: "agree on status and body",
      cases: `${pg.cases} replayed cases`,
      second: `${pg.fullContract.pct}% on the full contract, including the six headers that are part of an answer`,
      measured: conformanceMeta.measured,
      blurb:
        "PostgREST's own spec suite cannot be pointed at another server, so the harness lifts the request literals out of the Haskell and replays them over HTTP against both.",
    },
    {
      href: "/docs/conformance/hasura",
      surface: "GraphQL",
      against: `Hasura ${hasuraConformanceMeta.hasura}`,
      headline: hg.sameData.pct,
      headlineLabel: "return the same data",
      cases: `${hg.cases} cases in ${hasuraConformanceMeta.groups} groups`,
      second: `${hg.status.pct}% agree on status; ${hg.fullBody.pct}% match the whole body, error wording included`,
      measured: hasuraConformanceMeta.measured,
      blurb:
        "Hasura's corpus is YAML, so the harness extracts each case and replays it against both servers, converting the names and permissions Hasura keeps in metadata.",
    },
  ];

  return (
    <div class="min-h-screen bg-white">
      <div class="bg-gradient-to-b from-neutral-50 to-white border-b border-neutral-200">
        <div class="container-wide py-12">
          <div class="flex items-center gap-2 text-sm text-neutral-500 mb-4">
            <Link href="/docs" class="hover:text-primary-600">Docs</Link>
            <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 5l7 7-7 7"/>
            </svg>
            <span class="text-neutral-900">Conformance</span>
          </div>
          <h1 class="text-4xl font-bold text-neutral-900 mb-4">Conformance</h1>
          <p class="text-lg text-neutral-600 max-w-2xl">
            Postrust answers two dialects it did not invent. How closely it answers each is not a
            matter of opinion here — it is measured, against the server it replaces, by replaying
            that server&rsquo;s own test suite against both and diffing the live responses.
          </p>
        </div>
      </div>

      <div class="container-wide py-12">
        <div class="max-w-3xl">
          <section class="mb-12">
            <div class="grid sm:grid-cols-2 gap-4">
              {reports.map((r) => (
                <Link
                  key={r.href}
                  href={r.href}
                  class="block p-5 rounded-lg border border-neutral-200 hover:border-primary-400 transition-colors"
                >
                  <div class="flex items-center justify-between mb-2">
                    <span class="text-xs font-semibold uppercase tracking-wide text-neutral-400">
                      {r.surface}
                    </span>
                    <span class="text-xs text-neutral-400">{r.measured}</span>
                  </div>
                  <div class="text-sm text-neutral-500 mb-1">vs {r.against}</div>
                  <div class="text-3xl font-bold text-neutral-900 font-mono">{r.headline}%</div>
                  <div class="text-sm text-neutral-600 mt-1">{r.headlineLabel}</div>
                  <div class="text-xs text-neutral-400 mt-1">{r.cases}</div>
                  <p class="text-sm text-neutral-500 mt-3">{r.second}</p>
                  <p class="text-sm text-neutral-600 mt-3">{r.blurb}</p>
                  <span class="inline-flex items-center gap-1 text-sm text-primary-600 mt-3 font-medium">
                    Read the report
                    <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                      <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 5l7 7-7 7"/>
                    </svg>
                  </span>
                </Link>
              ))}
            </div>
          </section>

          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">Why it is done this way</h2>
            <p class="text-neutral-600 mb-4">
              Neither harness interprets a test expectation. The reference implementation&rsquo;s
              live response is the oracle, which means a mistake in the extractor shows up as a
              case both servers answer the same way rather than as a false failure. A harness that
              scored itself against a suite&rsquo;s written expectations would be measuring the
              extractor as much as the server.
            </p>
            <p class="text-neutral-600 mb-4">
              Agreement is reported at four strictness levels rather than one, because a single
              systemic gap — one header never emitted — would otherwise sink every case and hide
              the hundreds that differ in nothing else.
            </p>
            <p class="text-neutral-600">
              Both numbers carry their provenance. Each harness builds its own candidate, because
              which features it was built with is part of what is being measured and cannot be read
              off the binary, and writes a record of the reference version, the features, the
              commit, and whether the reference was replayed or a recording reused. The generators
              that put these figures on the site read that record and refuse a run that cannot
              account for itself — a run measured with the wrong binary produces a number that
              looks exactly like a good one.
            </p>
          </section>

          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">What is not measured</h2>
            <p class="text-neutral-600">
              Both harnesses run one configuration each, and say so. The PostgREST harness measures
              compatibility mode, so the default mount under{" "}
              <code class="font-mono">/api</code> is not covered by it. The Hasura harness
              authenticates with the admin secret, as Hasura&rsquo;s own suite does, so role
              selection by a bearer token is implemented and unmeasured. Each{" "}
              <code class="font-mono">FINDINGS.md</code> records the gaps a case cannot reach,
              alongside the faults found in the harness itself.
            </p>
          </section>

          <div class="flex items-center justify-between pt-8 border-t border-neutral-200">
            <Link href="/docs/benchmarks" class="flex items-center gap-2 text-neutral-600 hover:text-primary-600">
              <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 19l-7-7 7-7"/>
              </svg>
              Benchmarks
            </Link>
            <Link href="/compare" class="flex items-center gap-2 text-neutral-600 hover:text-primary-600">
              Compare
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
  title: "Conformance - Postrust Documentation",
  links: [{ rel: "canonical", href: "https://postrust.org/docs/conformance" }],
  meta: [
    {
      name: "description",
      content:
        "How closely Postrust matches PostgREST and Hasura, measured by replaying each server's own test suite against both and diffing the live responses.",
    },
  ],
};
