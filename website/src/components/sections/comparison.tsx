import { component$ } from "@builder.io/qwik";
import { Link } from "@builder.io/qwik-city";
import { comparisons } from "~/data/comparisons";
import { measuredFor, benchMeta } from "~/data/measured";

/**
 * Links to the per-tool comparison pages.
 *
 * Each card leads with what the other tool is good at, because someone landing
 * here is choosing between options and a page that only flatters itself is not
 * useful to them.
 */
export const ComparisonSection = component$(() => {
  // The embed scenario across every tool that supports the surface, read out of
  // the generated measurements rather than written down here.
  const restEmbed = measuredFor("postgrest", "rest").find((r) =>
    r.scenario.includes("embed"),
  );
  const hasuraEmbed = measuredFor("hasura", "graphql").find((r) =>
    r.scenario.includes("embed"),
  );
  const graphileEmbed = measuredFor("postgraphile", "graphql").find((r) =>
    r.scenario.includes("embed"),
  );

  const headline = [
    restEmbed?.postrust && {
      name: "Postrust",
      surface: "REST",
      ours: true,
      rps: restEmbed.postrust.rps,
      p95: restEmbed.postrust.p95_ms,
    },
    hasuraEmbed?.postrust && {
      name: "Postrust",
      surface: "GraphQL",
      ours: true,
      rps: hasuraEmbed.postrust.rps,
      p95: hasuraEmbed.postrust.p95_ms,
    },
    restEmbed?.other && {
      name: "PostgREST",
      surface: "REST",
      ours: false,
      rps: restEmbed.other.rps,
      p95: restEmbed.other.p95_ms,
    },
    hasuraEmbed?.other && {
      name: "Hasura",
      surface: "GraphQL",
      ours: false,
      rps: hasuraEmbed.other.rps,
      p95: hasuraEmbed.other.p95_ms,
    },
    graphileEmbed?.other && {
      name: "PostGraphile",
      surface: "GraphQL",
      ours: false,
      rps: graphileEmbed.other.rps,
      p95: graphileEmbed.other.p95_ms,
    },
  ].filter(Boolean) as Array<{
    name: string;
    surface: string;
    ours: boolean;
    rps: number;
    p95: number;
  }>;

  return (
    <section class="section-padding bg-neutral-50">
      <div class="container-wide">
        <div class="mb-12 max-w-3xl">
          <h2 class="mb-4 text-3xl font-bold text-neutral-900 md:text-4xl">
            How it compares
          </h2>
          <p class="text-lg text-neutral-600">
            Postrust is not the only way to get an API out of a PostgreSQL
            schema, and it is not the right answer for every project. Each
            comparison lists measured throughput on the same database and the
            cases where the other tool is the better choice.
          </p>
        </div>

        {/* One measured scenario, rather than a wall of them: the embed is the
            interesting case, since it is where these tools differ most. */}
        <div class="mb-10 overflow-x-auto rounded-xl border border-neutral-200 bg-white">
          <table class="w-full text-sm">
            <thead class="bg-neutral-50">
              <tr>
                <th class="px-6 py-3 text-left font-medium text-neutral-900">
                  A page of 25 rows with a related collection embedded
                </th>
                <th class="px-6 py-3 text-right font-medium text-neutral-900">
                  req/s
                </th>
                <th class="px-6 py-3 text-right font-medium text-neutral-900">
                  p95
                </th>
              </tr>
            </thead>
            <tbody class="divide-y divide-neutral-200">
              {headline.map((row) => (
                <tr key={row.name} class={row.ours ? "bg-primary-50/40" : ""}>
                  <td
                    class={`px-6 py-3 ${row.ours ? "text-primary-700 font-semibold" : "text-neutral-700"}`}
                  >
                    {row.name}
                    <span class="ml-2 text-xs text-neutral-500">
                      {row.surface}
                    </span>
                  </td>
                  <td
                    class={`px-6 py-3 text-right tabular-nums ${row.ours ? "text-primary-700 font-semibold" : "text-neutral-700"}`}
                  >
                    {row.rps.toLocaleString()}
                  </td>
                  <td class="px-6 py-3 text-right text-neutral-600 tabular-nums">
                    {row.p95} ms
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          <div class="border-t border-neutral-200 bg-neutral-50 px-6 py-3">
            <p class="text-xs text-neutral-500">
              Same database, same dataset, every server in a container, each
              tool on its own defaults. Median of {benchMeta.repeats} runs on{" "}
              {benchMeta.host}.{" "}
              <Link
                href="/docs/benchmarks"
                class="text-primary-600 hover:text-primary-700"
              >
                Method and full results
              </Link>
            </p>
          </div>
        </div>

        <div class="grid gap-6 md:grid-cols-2 lg:grid-cols-4">
          {comparisons.map((c) => (
            <Link
              key={c.slug}
              href={`/compare/${c.slug}`}
              class="group rounded-xl border border-neutral-200 bg-white p-6 transition-all hover:border-neutral-300 hover:shadow-sm"
            >
              <h3 class="mb-2 text-lg font-semibold text-neutral-900">
                vs {c.name}
              </h3>
              <p class="mb-4 text-sm text-neutral-600">
                {c.description ?? c.language}
              </p>
              <span class="text-primary-600 group-hover:text-primary-700 text-sm font-medium">
                Read the comparison →
              </span>
            </Link>
          ))}
        </div>
      </div>
    </section>
  );
});
