import { component$ } from "@builder.io/qwik";
import type { DocumentHead } from "@builder.io/qwik-city";
import { Link } from "@builder.io/qwik-city";
import {
  hasuraConformance,
  hasuraConformanceMeta,
  hasuraAgreement,
  worstGroups,
} from "~/data/hasura-conformance";

const groups = [
  { key: "all" as const, label: "All cases" },
  { key: "reads" as const, label: "Reads (query)" },
  { key: "writes" as const, label: "Writes (mutation)" },
];

const levels = [
  { key: "status" as const, label: "HTTP status" },
  { key: "sameOutcome" as const, label: "…and the same outcome" },
  { key: "sameData" as const, label: "…and the same data" },
  { key: "fullBody" as const, label: "Whole body, wording included" },
];

const divergences = [
  {
    title: "An unsecured server trusts no header",
    body: "Hasura with no admin secret treats every caller as an administrator, which also lets any caller name its own role and its own identity. Here, with no secret configured, x-hasura-* headers carry no weight and session variables come only from a verified token. A policy reading a value the caller chose is not a policy, and the failure is silent — the query succeeds, against the wrong rows. It costs nothing measured: every case in the corpus that names a role sends the secret beside it.",
  },
  {
    title: "Relationship names are derived, not configured",
    body: "Every relationship in the corpus is named by a metadata command a human wrote; here they come from foreign keys. Where a fixture chose something other than the convention, the field is not there under that name. This is structural to reflecting a database instead of configuring one.",
  },
  {
    title: "Two databases naming different constraints",
    body: "pg_dump restores constraints in a different order than the fixture created them, so PostgreSQL reports a different constraint name for the same violation. Two cases, proven against one PostgreSQL — neither server is answering wrongly.",
  },
  {
    title: "Introspection belongs to an administrator",
    body: "The corpus expects __schema to be refused for a role that has it disabled. A v2.50.1 reference does not refuse it: asked with the admin secret beside X-Hasura-Role, it answers from that role's own restricted schema, so the permissions apply and the introspection rule does not. This server was built to the corpus's text first and measured against the reference second, which is the right way round to find that.",
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
            <span class="text-neutral-900">Hasura Conformance</span>
          </div>
          <h1 class="text-4xl font-bold text-neutral-900 mb-4">Hasura conformance</h1>
          <p class="text-lg text-neutral-600 max-w-2xl">
            How closely the GraphQL dialects agree, measured by replaying Hasura&rsquo;s own test
            corpus against both servers and diffing the live responses.
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
                          {hasuraConformance[g.key].cases} cases
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
                            {hasuraConformance[g.key][l.key].pct}%
                          </span>
                          <span class="block text-xs text-neutral-400">
                            {hasuraConformance[g.key][l.key].passed}/{hasuraConformance[g.key].cases}
                          </span>
                        </td>
                      ))}
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
            <p class="text-sm text-neutral-500 mt-4">
              Measured {hasuraConformanceMeta.measured} against{" "}
              <code class="font-mono">hasura/graphql-engine:{hasuraConformanceMeta.hasura}</code>,
              on Postrust built with{" "}
              <code class="font-mono">{hasuraConformanceMeta.features}</code>, over{" "}
              {hasuraConformanceMeta.cases} cases in {hasuraConformanceMeta.groups} groups. Every
              figure is recomputed from the run&rsquo;s per-case output; nothing on this page is
              typed by hand.
            </p>
          </section>

          {/* What counts */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">What is compared</h2>
            <p class="text-neutral-600 mb-4">
              Agreement is reported at four strictness levels, because one systemic gap would
              otherwise sink every case and hide the hundreds that differ in nothing else.
            </p>
            <p class="text-neutral-600 mb-4">
              The third is the one that matters, and the one that needs explaining. Two servers
              agree about data when they return the same rows — and they also agree when both
              refuse. Counting only the first would score a case where Hasura itself raises an
              error as a failure of this server to match it, which is backwards. Of the{" "}
              {hasuraAgreement.sameData + hasuraAgreement.bothRefuse} cases that level counts,{" "}
              {hasuraAgreement.sameData} agree about data and {hasuraAgreement.bothRefuse} agree
              because both servers refuse.
            </p>
            <p class="text-neutral-600">
              The strictest level compares the whole body, errors included — wording and all.
              Bodies are compared as parsed JSON.
            </p>
          </section>

          {/* Method */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">How it is measured</h2>
            <p class="text-neutral-600 mb-4">
              The harness extracts cases from Hasura&rsquo;s own test corpus and replays each one
              over HTTP against both <code class="font-mono">hasura/graphql-engine</code> and
              Postrust, on identically loaded fixture databases, then diffs the live responses.
              Hasura is the oracle: no expectation written in the corpus is ever interpreted, so a
              mistake in the extractor shows up as a case both servers answer the same way rather
              than as a false failure.
            </p>
            <p class="text-neutral-600 mb-4">
              What Hasura keeps in metadata rather than in the database — relationship names, what
              each role may do — is converted from each group&rsquo;s own metadata and given to the
              candidate. This measures a configured server, which is what migrating involves, not a
              bare one.
            </p>
            <pre class="bg-neutral-900 text-neutral-100 rounded-lg p-4 text-sm overflow-x-auto"><code>{`scripts/hasura-conformance/conformance.sh
node scripts/gen-hasura-conformance.mjs scripts/hasura-conformance/.work/diff.json`}</code></pre>
            <p class="text-sm text-neutral-500 mt-4">
              The harness builds its own candidate and records what it built beside the results.
              The generator reads that record rather than its own arguments and refuses to publish
              a run that cannot account for itself — a run measured with the wrong binary produces
              a number that looks exactly like a good one.
            </p>
          </section>

          {/* Deliberate divergences */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">Where the two disagree on purpose</h2>
            <p class="text-neutral-600 mb-6">
              Some cases fail because neither answer is wrong, and some because matching Hasura
              would mean matching something worth not matching. They are listed so nobody later
              &ldquo;fixes&rdquo; one without deciding to.
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
          {worstGroups.length > 0 && (
            <section class="mb-12">
              <h2 class="text-2xl font-bold text-neutral-900 mb-4">Where the remaining disagreement lives</h2>
              <p class="text-neutral-600 mb-4">
                By corpus group, on the same-data level, worst first. Groups that agree completely
                are not listed.
              </p>
              <div class="overflow-x-auto">
                <table class="w-full text-sm">
                  <thead>
                    <tr class="border-b border-neutral-200">
                      <th class="text-left py-2 pr-4 font-semibold text-neutral-900">Group</th>
                      <th class="text-right py-2 font-semibold text-neutral-900">Agreement</th>
                    </tr>
                  </thead>
                  <tbody>
                    {worstGroups.map((g) => (
                      <tr key={g.group} class="border-b border-neutral-100">
                        <td class="py-2 pr-4"><code class="font-mono text-neutral-700">{g.group}</code></td>
                        <td class="py-2 text-right">
                          <span class="font-mono text-neutral-900">{g.pct}%</span>
                          <span class="text-neutral-400"> ({g.passed}/{g.total})</span>
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
              The largest is introspection, and it is not reachable from here: async-graphql builds
              its own registry and keeps it private, so the directives it installs, the types it
              adds, and the order it lists them in cannot be changed from outside the library.
              Every large introspection case needs at least one of those.
            </p>
            <p class="text-neutral-600 mb-4">
              Beside it: <code class="font-mono">_stream</code> subscriptions, the cursor-based
              half of the subscription surface; a function returning a single row as a root field;
              and generated descriptions, which are this server&rsquo;s wording rather than
              Hasura&rsquo;s. Actions and Apollo federation are subsystems rather than gaps.
            </p>
            <p class="text-neutral-600">
              The run history — including the runs whose numbers are not publishable, and why — is
              in <code class="font-mono">scripts/hasura-conformance/FINDINGS.md</code>, which also
              records four faults found in the harness itself. One of them invalidated eleven runs.
            </p>
          </section>

          <div class="flex items-center justify-between pt-8 border-t border-neutral-200">
            <Link href="/docs/conformance" class="flex items-center gap-2 text-neutral-600 hover:text-primary-600">
              <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 19l-7-7 7-7"/>
              </svg>
              PostgREST conformance
            </Link>
            <Link href="/compare/hasura" class="flex items-center gap-2 text-neutral-600 hover:text-primary-600">
              Postrust vs Hasura
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
  title: "Hasura Conformance - Postrust Documentation",
  links: [{ rel: "canonical", href: "https://postrust.org/docs/conformance/hasura" }],
  meta: [
    {
      name: "description",
      content:
        "How closely Postrust matches Hasura's GraphQL dialect, measured by replaying Hasura's own test corpus against both servers and diffing the live responses.",
    },
  ],
};
