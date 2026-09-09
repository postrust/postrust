import { component$ } from "@builder.io/qwik";
import type { DocumentHead } from "@builder.io/qwik-city";
import { Link } from "@builder.io/qwik-city";
import { benchMeta, measuredFor, type MeasuredRow } from "~/data/measured";
import { measurementContext as mc } from "~/data/comparisons";

/**
 * The benchmark method, and its results.
 *
 * Every figure comes from ~/data/measured.ts, which is generated from the
 * harness's results.json. Nothing here is typed in by hand, so this page
 * cannot drift from what was measured.
 *
 * This page carried a WITHDRAWN notice for as long as the figures were not
 * trustworthy. The investigation behind that is kept, further down, because it
 * is the reason the current setup looks the way it does.
 */

const ruledOut = [
  [
    "Thermal throttling and measurement order",
    "Each server was measured both first and second in an alternating design. No advantage follows the position.",
  ],
  [
    "Host load",
    "The harness scored higher at load average 18 than at load 5, while measuring by hand held steady at both.",
  ],
  [
    "A cold database",
    "The tables are 49 MB against 128 MB of shared_buffers, the fixtures ANALYZE, and the harness pre-warms with count(*).",
  ],
  [
    "The bulk load and checkpointing",
    "Reloading 400,000 rows and forcing a CHECKPOINT moved nothing.",
  ],
  [
    "Server and database start-up",
    "A restarted server reaches full throughput on its first sample; so does a restarted PostgreSQL.",
  ],
  [
    "Pausing the other containers",
    "A run with a single target, where there is nothing to pause, is equally depressed.",
  ],
  [
    "The harness's own measurement code",
    "Calling it directly on a settled stack returns the right answer, and replicating its full sequence by hand does too.",
  ],
  [
    "Docker's port forwarding",
    "The same load generator over the forwarded path and from inside the container network agree.",
  ],
];

const num = (n: number) => n.toLocaleString("en-US");

const ResultTable = component$<{ other: string; rows: MeasuredRow[] }>(
  ({ other, rows }) => (
    <div class="mb-8 overflow-x-auto">
      <table class="w-full min-w-[34rem] text-sm">
        <thead>
          <tr class="border-b border-neutral-300 text-left">
            <th class="py-2 pr-4 font-medium text-neutral-900">Scenario</th>
            <th class="text-primary-700 py-2 pr-4 text-right font-medium">
              Postrust
            </th>
            <th class="py-2 pr-4 text-right font-medium text-neutral-900">
              {other}
            </th>
            <th class="py-2 text-right font-medium text-neutral-500">Ratio</th>
          </tr>
        </thead>
        <tbody class="divide-y divide-neutral-200">
          {rows.map((r) => (
            <tr key={r.scenario}>
              <td class="py-2 pr-4 text-neutral-700">{r.scenario}</td>
              <td class="text-primary-700 py-2 pr-4 text-right tabular-nums">
                {r.postrust ? num(r.postrust.rps) : "—"}
                {r.postrust && (
                  <span class="ml-2 text-xs text-neutral-400">
                    {r.postrust.p50_ms} ms
                  </span>
                )}
              </td>
              <td class="py-2 pr-4 text-right text-neutral-700 tabular-nums">
                {r.other ? num(r.other.rps) : "—"}
                {r.other && (
                  <span class="ml-2 text-xs text-neutral-400">
                    {r.other.p50_ms} ms
                  </span>
                )}
              </td>
              <td class="py-2 text-right text-neutral-500 tabular-nums">
                {r.postrust && r.other
                  ? `${(r.postrust.rps / r.other.rps).toFixed(1)}x`
                  : "—"}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  ),
);

export default component$(() => {
  const rest = measuredFor("postgrest", "rest");
  const hasura = measuredFor("hasura", "graphql");
  const postgraphile = measuredFor("postgraphile", "graphql");
  const drift = benchMeta.selfConsistency?.drift_pct;

  return (
    <div class="min-h-screen bg-white">
      <div class="border-b border-neutral-200 bg-gradient-to-b from-neutral-50 to-white">
        <div class="container-wide py-12">
          <div class="mb-4 flex items-center gap-2 text-sm text-neutral-500">
            <Link href="/docs" class="hover:text-primary-600">
              Docs
            </Link>
            <span>/</span>
            <span class="text-neutral-900">Benchmarks</span>
          </div>
          <h1 class="mb-4 text-4xl font-bold text-neutral-900">Benchmarks</h1>
          <p class="max-w-2xl text-lg text-neutral-600">
            Every server runs as a container against the same PostgreSQL
            instance and the same dataset, each pinned to its own CPU cores on a
            dedicated bare-metal host. These are the numbers the harness
            produced.
          </p>
        </div>
      </div>

      <div class="container-wide py-12">
        <div class="max-w-3xl">
          <section class="mb-12">
            <h2 class="mb-1 text-2xl font-bold text-neutral-900">
              REST, against PostgREST
            </h2>
            <p class="mb-4 text-sm text-neutral-500">
              Requests per second, with median latency. Same request in each
              server's own dialect.
            </p>
            <ResultTable other="PostgREST" rows={rest} />

            <h2 class="mb-1 text-2xl font-bold text-neutral-900">
              GraphQL, against Hasura
            </h2>
            <p class="mb-4 text-sm text-neutral-500">
              Postrust and Hasura are sent the same query text, byte for byte.
            </p>
            <ResultTable other="Hasura" rows={hasura} />

            <h2 class="mb-1 text-2xl font-bold text-neutral-900">
              GraphQL, against PostGraphile
            </h2>
            <p class="mb-4 text-sm text-neutral-500">
              PostGraphile inflects field names its own way, so the query text
              differs. The request is the same.
            </p>
            <ResultTable other="PostGraphile" rows={postgraphile} />

            <div class="rounded-lg border border-neutral-200 bg-neutral-50 p-5 text-sm text-neutral-600">
              {benchMeta.cpu} · {benchMeta.cpuState} · PostgreSQL{" "}
              {benchMeta.postgres} · {num(benchMeta.requests)} requests at
              concurrency {benchMeta.concurrency}, median of {benchMeta.repeats}{" "}
              after {benchMeta.warmup} warm-up requests · {benchMeta.dataset}.
              {drift != null && (
                <>
                  {" "}
                  The first measurement, repeated as the last action of the run,
                  differed by {drift}%.
                </>
              )}
            </div>
          </section>

          <section class="mb-12">
            <h2 class="mb-4 text-2xl font-bold text-neutral-900">
              How much to trust each number
            </h2>
            <p class="mb-4 text-neutral-700">
              Repeated whole runs on this host agree to {mc.spreadPct.postrust}%
              for Postrust, {mc.spreadPct.postgraphile}% for PostGraphile and{" "}
              {mc.spreadPct.hasura}% for Hasura. PostgREST is the outlier at{" "}
              {mc.spreadPct.postgrest}%, so multiples quoted against it carry
              roughly ±{mc.postgrestRatioTolerancePct}%. The direction of the
              differences is far outside that margin; the precise multiples are
              not, and are not quoted to more precision than they carry.
            </p>
            <p class="text-neutral-700">
              Running in a container costs about {mc.dockerCostPct}% against the
              same binary run natively — {mc.dockerNetworkPct} of those points
              are Docker's network path rather than the container itself. It is
              charged to every server equally, so ratios are unaffected and
              absolute figures are understated by about that much.
            </p>
          </section>

          <section class="mb-12">
            <h2 class="mb-4 text-2xl font-bold text-neutral-900">
              Why the REST gap is as large as it is
            </h2>
            <p class="mb-4 text-neutral-700">
              Worth understanding before quoting the ratio. Three ways it could
              have been an artefact were tested and ruled out: nothing is
              cached, and both servers issue exactly one query per request
              (pg_stat_statements records 3,000 calls for 3,000 requests on
              each); the connection pools are 10 and 11; and the responses carry
              the same rows, the same Content-Range and within two bytes of each
              other.
            </p>
            <p class="mb-4 text-neutral-700">
              What differs is the SQL. Postrust sends a plain parameterised
              SELECT and renders the JSON itself. PostgREST wraps the query in a
              CTE, computes a count, builds the JSON inside PostgreSQL with
              json_agg and reads three GUCs — on every request, whether or not a
              count was asked for or a function set a response header.
            </p>
            <p class="text-neutral-700">
              So a large part of the ratio is{" "}
              <em>where each design does the work</em>, and PostgREST paying
              per-request for generality these scenarios do not use. That is a
              real architectural difference and a fair thing to measure. It is
              not evidence that Postrust's HTTP layer is four times faster, and
              the figures should not be read that way.
            </p>
          </section>

          <section class="mb-12">
            <h2 class="mb-4 text-2xl font-bold text-neutral-900">
              What stops a bad run being published
            </h2>
            <p class="mb-4 text-neutral-700">
              Two checks, both of which have caught something real.
            </p>
            <p class="mb-4 text-neutral-700">
              <strong>The targets must be answering the same question.</strong>{" "}
              Every scenario is fetched from every server before any of them is
              measured. Row counts must match — a difference there is never a
              dialect difference — and for REST the columns must match too.
              Until this existed the harness checked an HTTP status and nothing
              else, so one server returning a single row while another returned
              twenty-five would have passed and been measured as equal work.
            </p>
            <p class="text-neutral-700">
              <strong>The machine must have held still.</strong> The first
              measurement of a run is repeated as its last action, with
              everything else in between. If the two disagree by more than 3%,
              the run exits non-zero and its figures are not publishable. This
              matters more than it sounds: with a deliberately short
              2,000-request window the same comparison reads 13.1x instead of
              4.0x, from a run that otherwise looks perfectly healthy. The gate
              is what stands between that number and this page.
            </p>
          </section>

          <section class="mb-12">
            <h2 class="mb-4 text-2xl font-bold text-neutral-900">
              Why those checks exist
            </h2>
            <p class="mb-6 text-neutral-700">
              This page carried a <strong>withdrawn</strong> notice for a while,
              and no numbers at all. The figures that had been here were
              measured on a virtualised host where the same server, in the same
              container, measured seconds apart differed by about a factor of
              two — and whatever ran first was the most understated, which made
              run order a determinant of the published ratios. Everything below
              was ruled out at the time, each by direct test, and the mechanism
              was never identified.
            </p>
            <div class="mb-6 space-y-3">
              {ruledOut.map(([what, how]) => (
                <div key={what} class="border-l-2 border-neutral-200 pl-4">
                  <h3 class="mb-1 font-semibold text-neutral-900">{what}</h3>
                  <p class="text-sm text-neutral-600">{how}</p>
                </div>
              ))}
            </div>
            <p class="text-neutral-700">
              It does not reproduce on bare metal with each component pinned to
              its own CCD. It was escaped rather than explained, which is
              exactly why the self-consistency gate is now part of the harness
              rather than something done by hand — it is what would notice if
              the effect ever returned. The full record is in{" "}
              <code class="font-mono text-sm">scripts/BENCH-FINDINGS.md</code>,
              and the method in{" "}
              <code class="font-mono text-sm">docs/benchmarking.md</code>.
            </p>
          </section>

          <section class="mb-12">
            <div class="rounded-lg border border-neutral-200 bg-neutral-50 p-5">
              <h2 class="mb-2 text-lg font-bold text-neutral-900">
                Conformance is measured separately
              </h2>
              <p class="text-neutral-700">
                The conformance reports measure whether two servers give the
                same answer, not how fast they give it.{" "}
                <Link
                  href="/docs/conformance"
                  class="text-primary-600 font-medium hover:underline"
                >
                  Read the conformance reports
                </Link>
                .
              </p>
            </div>
          </section>

          <div class="flex items-center justify-between border-t border-neutral-200 pt-8">
            <Link
              href="/docs/conformance"
              class="hover:text-primary-600 text-neutral-600"
            >
              ← Conformance
            </Link>
            <Link
              href="/compare"
              class="hover:text-primary-600 text-neutral-600"
            >
              Compare →
            </Link>
          </div>
        </div>
      </div>
    </div>
  );
});

export const head: DocumentHead = {
  title: "Benchmarks - Postrust Documentation",
  links: [{ rel: "canonical", href: "https://postrust.org/docs/benchmarks" }],
  meta: [
    {
      name: "description",
      content:
        "Postrust measured against PostgREST, Hasura and PostGraphile on dedicated bare metal, with each component pinned to its own cores and a gate that rejects any run the machine drifted under.",
    },
  ],
};
