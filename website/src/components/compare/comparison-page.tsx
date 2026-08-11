import { component$ } from "@builder.io/qwik";
import { Link } from "@builder.io/qwik-city";
import type { Comparison } from "~/data/comparisons";
import { perfCaveats } from "~/data/comparisons";
import { measuredFor, benchMeta } from "~/data/measured";
import type { MeasuredRow } from "~/data/measured";

interface Props {
  comparison: Comparison;
}

export const ComparisonPage = component$<Props>(({ comparison: c }) => {
  const rest = measuredFor(c.slug, "rest");
  const graphql = measuredFor(c.slug, "graphql");
  const hasNumbers = rest.length > 0 || graphql.length > 0;

  // FAQ structured data. The questions on the page and the questions in the
  // markup are the same list, so they cannot drift apart.
  const faqJsonLd = JSON.stringify({
    "@context": "https://schema.org",
    "@type": "FAQPage",
    mainEntity: c.faq.map((entry) => ({
      "@type": "Question",
      name: entry.q,
      acceptedAnswer: { "@type": "Answer", text: entry.a },
    })),
  });

  return (
    <div class="min-h-screen bg-white">
      <script type="application/ld+json" dangerouslySetInnerHTML={faqJsonLd} />

      {/* Hero */}
      <section class="section-padding bg-gradient-to-b from-neutral-50 to-white">
        <div class="container-wide">
          <div class="max-w-3xl">
            <nav class="mb-6 text-sm text-neutral-500">
              <Link href="/compare" class="hover:text-neutral-900">
                Compare
              </Link>
              <span class="mx-2">/</span>
              <span class="text-neutral-900">{c.name}</span>
            </nav>

            <h1 class="mb-6 text-4xl font-bold text-neutral-900 md:text-5xl">
              {c.tagline}
            </h1>

            {c.intro.map((para) => (
              <p key={para.slice(0, 24)} class="mb-4 text-lg text-neutral-600">
                {para}
              </p>
            ))}

            <dl class="mt-8 flex flex-wrap gap-x-8 gap-y-2 text-sm">
              <div>
                <dt class="text-neutral-500">{c.name} is written in</dt>
                <dd class="font-medium text-neutral-900">{c.language}</dd>
              </div>
              <div>
                <dt class="text-neutral-500">Licence</dt>
                <dd class="font-medium text-neutral-900">{c.license}</dd>
              </div>
              <div>
                <dt class="text-neutral-500">Version compared</dt>
                <dd class="font-medium text-neutral-900">{c.versionTested}</dd>
              </div>
              <div>
                <dt class="text-neutral-500">Their site</dt>
                <dd class="font-medium">
                  <a
                    href={c.url}
                    rel="noopener"
                    class="text-primary-600 hover:text-primary-700"
                  >
                    {c.url.replace(/^https?:\/\//, "").replace(/\/$/, "")}
                  </a>
                </dd>
              </div>
            </dl>
          </div>
        </div>
      </section>

      {/* Feature comparison */}
      <section class="section-padding">
        <div class="container-wide">
          <div class="max-w-4xl">
            <h2 class="mb-2 text-3xl font-bold text-neutral-900">
              Feature comparison
            </h2>
            <p class="mb-8 text-neutral-600">
              Where a capability has a condition attached, the condition is in
              the cell.
            </p>

            <div class="overflow-x-auto rounded-xl border border-neutral-200">
              <table class="w-full text-sm">
                <thead class="bg-neutral-50">
                  <tr>
                    <th class="px-6 py-4 text-left font-medium text-neutral-900">
                      Feature
                    </th>
                    <th class="text-primary-600 px-6 py-4 text-left font-medium">
                      Postrust
                    </th>
                    <th class="px-6 py-4 text-left font-medium text-neutral-900">
                      {c.name}
                    </th>
                  </tr>
                </thead>
                <tbody class="divide-y divide-neutral-200">
                  {c.features.map((row) => (
                    <tr key={row.feature}>
                      <td class="px-6 py-4 font-medium text-neutral-900">
                        {row.feature}
                      </td>
                      <td class="px-6 py-4 text-neutral-700">{row.postrust}</td>
                      <td class="px-6 py-4 text-neutral-700">{row.other}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </div>
        </div>
      </section>

      {/* Measured performance */}
      {hasNumbers && (
        <section class="section-padding bg-neutral-50">
          <div class="container-wide">
            <div class="max-w-4xl">
              <h2 class="mb-2 text-3xl font-bold text-neutral-900">
                Measured performance
              </h2>
              <p class="mb-8 text-neutral-600">
                Produced by{" "}
                <code class="rounded border border-neutral-200 bg-white px-1 py-0.5 text-sm">
                  scripts/bench-compare.sh
                </code>
                , which starts both servers as containers against the same
                database and runs the same load against each. Where {c.name} is
                faster, that is what the table says.
              </p>

              {rest.length > 0 && (
                <div class="mb-10">
                  <h3 class="mb-3 text-lg font-semibold text-neutral-900">
                    REST
                  </h3>
                  <MeasuredTable rows={rest} otherName={c.name} />
                </div>
              )}

              {graphql.length > 0 && (
                <div class="mb-10">
                  <h3 class="mb-3 text-lg font-semibold text-neutral-900">
                    GraphQL
                  </h3>
                  <MeasuredTable rows={graphql} otherName={c.name} />
                </div>
              )}

              <div class="rounded-xl border border-neutral-200 bg-white p-6">
                <h3 class="mb-3 text-sm font-semibold tracking-wide text-neutral-900 uppercase">
                  How this was measured
                </h3>
                <ul class="space-y-2 text-sm text-neutral-600">
                  {perfCaveats.map((caveat) => (
                    <li key={caveat.slice(0, 24)}>{caveat}</li>
                  ))}
                </ul>
                <p class="mt-4 text-sm text-neutral-500">
                  {benchMeta.host} · PostgreSQL {benchMeta.postgres} ·{" "}
                  {benchMeta.dataset} · {benchMeta.requests} requests at
                  concurrency {benchMeta.concurrency}.{" "}
                  <Link
                    href="/docs/benchmarks"
                    class="text-primary-600 hover:text-primary-700"
                  >
                    Full benchmark method
                  </Link>
                </p>
              </div>
            </div>
          </div>
        </section>
      )}

      {/* Honest trade-offs */}
      <section class="section-padding">
        <div class="container-wide">
          <div class="grid max-w-4xl gap-8 md:grid-cols-2">
            <div class="rounded-xl border border-neutral-200 bg-white p-8">
              <h2 class="mb-4 text-xl font-bold text-neutral-900">
                When {c.name} is the better choice
              </h2>
              <ul class="space-y-3">
                {c.whenTheirs.map((item) => (
                  <li
                    key={item.slice(0, 24)}
                    class="text-sm leading-relaxed text-neutral-700"
                  >
                    {item}
                  </li>
                ))}
              </ul>
            </div>

            <div class="rounded-xl border border-neutral-200 bg-white p-8">
              <h2 class="mb-4 text-xl font-bold text-neutral-900">
                When Postrust fits better
              </h2>
              <ul class="space-y-3">
                {c.whenOurs.map((item) => (
                  <li
                    key={item.slice(0, 24)}
                    class="text-sm leading-relaxed text-neutral-700"
                  >
                    {item}
                  </li>
                ))}
              </ul>
            </div>
          </div>
        </div>
      </section>

      {/* Migration */}
      {c.migration && c.migration.length > 0 && (
        <section class="section-padding bg-neutral-50">
          <div class="container-wide">
            <div class="max-w-3xl">
              <h2 class="mb-6 text-3xl font-bold text-neutral-900">
                Moving from {c.name}
              </h2>
              <ul class="space-y-4">
                {c.migration.map((step) => (
                  <li key={step.slice(0, 24)} class="text-neutral-700">
                    {step}
                  </li>
                ))}
              </ul>
            </div>
          </div>
        </section>
      )}

      {/* FAQ */}
      <section class="section-padding">
        <div class="container-wide">
          <div class="max-w-3xl">
            <h2 class="mb-8 text-3xl font-bold text-neutral-900">Questions</h2>
            <div class="space-y-8">
              {c.faq.map((entry) => (
                <div key={entry.q}>
                  <h3 class="mb-2 text-lg font-semibold text-neutral-900">
                    {entry.q}
                  </h3>
                  <p class="text-neutral-600">{entry.a}</p>
                </div>
              ))}
            </div>
          </div>
        </div>
      </section>

      {/* CTA */}
      <section class="section-padding bg-neutral-50">
        <div class="container-wide text-center">
          <h2 class="mb-4 text-3xl font-bold text-neutral-900">
            Try it against your own schema
          </h2>
          <p class="mx-auto mb-8 max-w-2xl text-lg text-neutral-600">
            Point it at a database and see what it generates. That is a shorter
            path than reading a comparison table.
          </p>
          <div class="flex flex-wrap justify-center gap-4">
            <Link
              href="/docs/getting-started"
              class="inline-flex items-center gap-2 rounded-lg bg-neutral-900 px-8 py-4 text-base font-semibold text-white transition-colors hover:bg-neutral-800"
            >
              Get started
            </Link>
            <Link
              href="/compare"
              class="inline-flex items-center gap-2 rounded-lg border border-neutral-300 bg-white px-8 py-4 text-base font-semibold text-neutral-900 transition-colors hover:border-neutral-400"
            >
              All comparisons
            </Link>
          </div>
        </div>
      </section>
    </div>
  );
});

interface TableProps {
  rows: MeasuredRow[];
  otherName: string;
}

const MeasuredTable = component$<TableProps>(({ rows, otherName }) => (
  <div class="overflow-x-auto rounded-xl border border-neutral-200 bg-white">
    <table class="w-full text-sm">
      <thead class="bg-neutral-50">
        <tr>
          <th class="px-6 py-3 text-left font-medium text-neutral-900">
            Scenario
          </th>
          <th class="text-primary-600 px-6 py-3 text-right font-medium">
            Postrust req/s
          </th>
          <th class="px-6 py-3 text-right font-medium text-neutral-900">
            {otherName} req/s
          </th>
          <th class="text-primary-600 px-6 py-3 text-right font-medium">
            Postrust p95
          </th>
          <th class="px-6 py-3 text-right font-medium text-neutral-900">
            {otherName} p95
          </th>
        </tr>
      </thead>
      <tbody class="divide-y divide-neutral-200">
        {rows.map((row) => (
          <tr key={row.scenario}>
            <td class="px-6 py-3 font-medium text-neutral-900">
              {row.scenario}
            </td>
            <td class="px-6 py-3 text-right text-neutral-700 tabular-nums">
              {row.postrust
                ? row.postrust.rps.toLocaleString()
                : "not supported"}
            </td>
            <td class="px-6 py-3 text-right text-neutral-700 tabular-nums">
              {row.other ? row.other.rps.toLocaleString() : "not supported"}
            </td>
            <td class="px-6 py-3 text-right text-neutral-700 tabular-nums">
              {row.postrust ? `${row.postrust.p95_ms} ms` : "—"}
            </td>
            <td class="px-6 py-3 text-right text-neutral-700 tabular-nums">
              {row.other ? `${row.other.p95_ms} ms` : "—"}
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  </div>
));
