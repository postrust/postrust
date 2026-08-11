import { component$ } from "@builder.io/qwik";
import { Link } from "@builder.io/qwik-city";
import { comparisons } from "~/data/comparisons";

/**
 * Links to the per-tool comparison pages.
 *
 * Each card leads with what the other tool is good at, because someone landing
 * here is choosing between options and a page that only flatters itself is not
 * useful to them.
 */
export const ComparisonSection = component$(() => {
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
