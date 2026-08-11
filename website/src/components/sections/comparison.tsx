import { component$ } from "@builder.io/qwik";
import { Link } from "@builder.io/qwik-city";
import { comparisons } from "~/data/comparisons";
import { measuredFor, benchMeta } from "~/data/measured";

interface Entry {
  name: string;
  ours: boolean;
  rps: number;
  p95: number;
}

interface Group {
  surface: string;
  entries: Entry[];
}

/**
 * Links to the per-tool comparison pages, over one measured scenario.
 *
 * Grouped by surface rather than ranked together: a REST figure and a GraphQL
 * figure are not the same measurement, and one combined list invites a
 * comparison that is not being made.
 */
export const ComparisonSection = component$(() => {
  const restEmbed = measuredFor("postgrest", "rest").find((r) =>
    r.scenario.includes("embed"),
  );
  const hasuraEmbed = measuredFor("hasura", "graphql").find((r) =>
    r.scenario.includes("embed"),
  );
  const graphileEmbed = measuredFor("postgraphile", "graphql").find((r) =>
    r.scenario.includes("embed"),
  );

  const groups: Group[] = (
    [
      {
        surface: "REST",
        entries: [
          restEmbed?.postrust && {
            name: "Postrust",
            ours: true,
            rps: restEmbed.postrust.rps,
            p95: restEmbed.postrust.p95_ms,
          },
          restEmbed?.other && {
            name: "PostgREST",
            ours: false,
            rps: restEmbed.other.rps,
            p95: restEmbed.other.p95_ms,
          },
        ].filter(Boolean) as Entry[],
      },
      {
        surface: "GraphQL",
        entries: [
          hasuraEmbed?.postrust && {
            name: "Postrust",
            ours: true,
            rps: hasuraEmbed.postrust.rps,
            p95: hasuraEmbed.postrust.p95_ms,
          },
          hasuraEmbed?.other && {
            name: "Hasura",
            ours: false,
            rps: hasuraEmbed.other.rps,
            p95: hasuraEmbed.other.p95_ms,
          },
          graphileEmbed?.other && {
            name: "PostGraphile",
            ours: false,
            rps: graphileEmbed.other.rps,
            p95: graphileEmbed.other.p95_ms,
          },
        ].filter(Boolean) as Entry[],
      },
    ] as Group[]
  ).filter((group) => group.entries.length > 1);

  return (
    <section class="section-padding bg-neutral-50">
      <div class="container-wide">
        <div class="mb-10 max-w-3xl">
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

        <div class="mb-10 overflow-hidden rounded-xl border border-neutral-200 bg-white">
          <div class="border-b border-neutral-200 px-6 py-4">
            <h3 class="font-semibold text-neutral-900">
              A page of 25 rows with a related collection embedded
            </h3>
            <p class="mt-1 text-sm text-neutral-500">
              Requests per second, higher is better. Each surface is compared
              only against the tools that expose it.
            </p>
          </div>

          {groups.map((group) => {
            const fastest = Math.max(...group.entries.map((e) => e.rps));
            const ourRps = group.entries.find((e) => e.ours)?.rps ?? fastest;

            return (
              <div
                key={group.surface}
                class="border-b border-neutral-200 last:border-b-0"
              >
                <div class="bg-neutral-50/60 px-6 py-2">
                  <span class="text-xs font-semibold tracking-wide text-neutral-500 uppercase">
                    {group.surface}
                  </span>
                </div>

                {group.entries.map((entry) => {
                  // Against Postrust: at or above 1 means they are that many
                  // times slower, below 1 means they are faster.
                  const ratio = ourRps / entry.rps;
                  const label = entry.ours
                    ? null
                    : ratio >= 1
                      ? `${ratio.toFixed(1)}× slower`
                      : `${(1 / ratio).toFixed(1)}× faster`;

                  return (
                    <div
                      key={entry.name}
                      class={`flex items-center gap-4 px-6 py-3 ${
                        entry.ours ? "bg-primary-50/40" : ""
                      }`}
                    >
                      <div class="w-28 shrink-0 sm:w-32">
                        <span
                          class={
                            entry.ours
                              ? "text-primary-700 font-semibold"
                              : "text-neutral-800"
                          }
                        >
                          {entry.name}
                        </span>
                      </div>

                      <div class="hidden h-2.5 flex-1 overflow-hidden rounded-full bg-neutral-100 sm:block">
                        <div
                          class={`h-full rounded-full ${
                            entry.ours ? "bg-primary-500" : "bg-neutral-400"
                          }`}
                          style={{ width: `${(entry.rps / fastest) * 100}%` }}
                        />
                      </div>

                      <div class="w-20 shrink-0 text-right">
                        <span
                          class={`tabular-nums ${
                            entry.ours
                              ? "text-primary-700 font-semibold"
                              : "text-neutral-700"
                          }`}
                        >
                          {entry.rps.toLocaleString()}
                        </span>
                      </div>

                      <div class="w-16 shrink-0 text-right text-sm text-neutral-500 tabular-nums">
                        {entry.p95} ms
                      </div>

                      <div class="w-24 shrink-0 text-right text-sm">
                        {label ? (
                          <span
                            class={
                              ratio >= 1
                                ? "text-neutral-500"
                                : "font-medium text-neutral-900"
                            }
                          >
                            {label}
                          </span>
                        ) : (
                          <span class="text-neutral-400">baseline</span>
                        )}
                      </div>
                    </div>
                  );
                })}
              </div>
            );
          })}

          <div class="border-t border-neutral-200 bg-neutral-50 px-6 py-3">
            <p class="text-xs text-neutral-500">
              Columns: requests per second, p95 latency, and the difference
              against Postrust. Same database, same dataset, every server in a
              container, each tool on its own defaults. Median of{" "}
              {benchMeta.repeats} runs on {benchMeta.host}.{" "}
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
              <p class="mb-4 text-sm text-neutral-600">{c.description}</p>
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
