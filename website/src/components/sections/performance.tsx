import { component$ } from "@builder.io/qwik";
import { Link } from "@builder.io/qwik-city";
import { benchMeta, measuredFor } from "~/data/measured";
import { measurementContext } from "~/data/comparisons";

/**
 * The measured comparison, on the homepage.
 *
 * Every number here is read from ~/data/measured.ts, which is generated from
 * the harness's results.json. Nothing on this page is typed in by hand, and
 * the bars are sized from the same values that label them -- so a bar cannot
 * disagree with its own figure.
 */

const TOOL = {
  postrust: {
    label: "Postrust",
    bar: "bg-primary-500",
    text: "text-primary-700",
  },
  postgrest: {
    label: "PostgREST",
    bar: "bg-neutral-400",
    text: "text-neutral-600",
  },
  hasura: { label: "Hasura", bar: "bg-neutral-400", text: "text-neutral-600" },
  postgraphile: {
    label: "PostGraphile",
    bar: "bg-neutral-300",
    text: "text-neutral-600",
  },
} as const;

type ToolKey = keyof typeof TOOL;

interface Series {
  scenario: string;
  bars: { tool: ToolKey; rps: number }[];
}

/** REST: Postrust against PostgREST, scenario by scenario. */
function restSeries(): Series[] {
  return measuredFor("postgrest", "rest")
    .filter((r) => r.postrust && r.other)
    .map((r) => ({
      scenario: r.scenario,
      bars: [
        { tool: "postrust" as ToolKey, rps: r.postrust!.rps },
        { tool: "postgrest" as ToolKey, rps: r.other!.rps },
      ],
    }));
}

/**
 * GraphQL: three tools, so the two comparison sets are joined on scenario
 * rather than a third list being maintained alongside them.
 */
function graphqlSeries(): Series[] {
  const hasura = measuredFor("hasura", "graphql");
  const postgraphile = measuredFor("postgraphile", "graphql");

  return hasura
    .filter((r) => r.postrust)
    .map((r) => {
      const pg = postgraphile.find((p) => p.scenario === r.scenario);
      const bars: { tool: ToolKey; rps: number }[] = [
        { tool: "postrust", rps: r.postrust!.rps },
      ];
      if (r.other) bars.push({ tool: "hasura", rps: r.other.rps });
      if (pg?.other) bars.push({ tool: "postgraphile", rps: pg.other.rps });
      return { scenario: r.scenario, bars };
    });
}

const Chart = component$<{ title: string; note: string; series: Series[] }>(
  ({ title, note, series }) => {
    // One scale across the whole chart, so bar lengths are comparable between
    // scenarios and not just within one.
    const max = Math.max(...series.flatMap((s) => s.bars.map((b) => b.rps)), 1);

    return (
      <div class="rounded-xl border border-neutral-200 bg-white p-6">
        <div class="mb-1 flex items-baseline justify-between gap-4">
          <h3 class="text-sm font-semibold tracking-wide text-neutral-900 uppercase">
            {title}
          </h3>
          <span class="text-xs text-neutral-500">requests/second</span>
        </div>
        <p class="mb-5 text-xs text-neutral-500">{note}</p>

        <div class="space-y-5">
          {series.map((s) => (
            <div key={s.scenario}>
              <div class="mb-1.5 text-sm font-medium text-neutral-700">
                {s.scenario}
              </div>
              <div class="space-y-1">
                {s.bars.map((b) => (
                  <div key={b.tool} class="flex items-center gap-3">
                    <span class="w-24 shrink-0 text-xs text-neutral-500">
                      {TOOL[b.tool].label}
                    </span>
                    <div class="h-4 min-w-0 flex-1 rounded bg-neutral-100">
                      <div
                        class={`h-4 rounded ${TOOL[b.tool].bar}`}
                        style={{
                          width: `${Math.max((b.rps / max) * 100, 1.5)}%`,
                        }}
                      />
                    </div>
                    <span
                      class={`w-16 shrink-0 text-right text-xs tabular-nums ${TOOL[b.tool].text}`}
                    >
                      {b.rps.toLocaleString("en-US")}
                    </span>
                  </div>
                ))}
              </div>
            </div>
          ))}
        </div>
      </div>
    );
  },
);

export const PerformanceSection = component$(() => {
  const rest = restSeries();
  const graphql = graphqlSeries();
  if (rest.length === 0 && graphql.length === 0) return null;

  const drift = benchMeta.selfConsistency?.drift_pct;

  return (
    <section class="section-padding bg-neutral-50">
      <div class="container-wide">
        <div class="mx-auto mb-12 max-w-3xl text-center">
          <h2 class="mb-4 text-3xl font-bold text-neutral-900 md:text-4xl">
            Measured, on a machine that held still
          </h2>
          <p class="text-lg text-neutral-600">
            Every server runs as a container against the same PostgreSQL
            instance and the same dataset, each pinned to its own CPU cores.
            These are the numbers the harness produced, not a summary of them.
          </p>
        </div>

        <div class="grid gap-6 lg:grid-cols-2">
          <Chart
            title="REST"
            note="Postrust against PostgREST, same request in each server's own dialect."
            series={rest}
          />
          <Chart
            title="GraphQL"
            note="Postrust and Hasura are sent the same query text, byte for byte."
            series={graphql}
          />
        </div>

        <div class="mt-6 rounded-xl border border-neutral-200 bg-white p-6">
          <p class="text-sm text-neutral-600">
            {benchMeta.cpu} · {benchMeta.cpuState} · PostgreSQL{" "}
            {benchMeta.postgres} · {benchMeta.requests.toLocaleString("en-US")}{" "}
            requests at concurrency {benchMeta.concurrency}, median of{" "}
            {benchMeta.repeats}.
            {drift != null && (
              <>
                {" "}
                The first measurement, repeated as the last action of the run,
                differed by {drift}%.
              </>
            )}
          </p>
          <p class="mt-3 text-sm text-neutral-500">
            Read these as ratios rather than as capacity, and as a comparison of
            whole designs rather than of HTTP layers — much of the REST gap is
            that Postrust renders JSON itself where PostgREST builds it inside
            PostgreSQL on every request. Everything shares one host, so absolute
            throughput depends on that machine — and running in a container
            costs about {measurementContext.dockerCostPct}% against the same
            binary run natively, charged to every server equally. PostgREST's
            own run-to-run spread is wider than the others' at up to{" "}
            {measurementContext.spreadPct.postgrest}%, so its multiples carry
            roughly ±{measurementContext.postgrestRatioTolerancePct}%.{" "}
            <Link
              href="/docs/benchmarks"
              class="text-primary-600 hover:text-primary-700"
            >
              Full method
            </Link>
          </p>
        </div>
      </div>
    </section>
  );
});
