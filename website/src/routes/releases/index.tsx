import { component$ } from "@builder.io/qwik";
import type { DocumentHead } from "@builder.io/qwik-city";
import { Link } from "@builder.io/qwik-city";
import { conformance, conformanceMeta } from "~/data/conformance";
import {
  hasuraConformance,
  hasuraConformanceMeta,
  hasuraAgreement,
} from "~/data/hasura-conformance";

const VERSION = "1.0.0-alpha.1";

const added = [
  {
    title: "Hasura's GraphQL dialect",
    body: "A client generated against Hasura points at /v1/graphql unchanged: the root fields, the generated _bool_exp filter types, order_by as a list, aggregates, nested writes, on_conflict upserts, update_many, live subscriptions, and Hasura's error envelope — including the path that names a place in the request rather than in the response.",
  },
  {
    title: "The permission model that comes with it",
    body: "A schema per role, built from a schema cache already reduced to what that role can see, so nothing in the builders needs to know a permission exists. Row filters in the same language a where is written in. Reading and writing as two column sets, because a role may write a column it cannot read. Presets, ceilings, backend_only and _exists.",
  },
  {
    title: "Hasura's authentication contract",
    body: "An admin secret, x-hasura-* headers as session variables a policy can read, and a token that may select among the roles its x-hasura-allowed-roles lists. One deliberate difference: with no secret configured, headers carry no weight at all — a policy reading a value the caller chose is not a policy.",
  },
  {
    title: "More of PostgREST's surface",
    body: "Embedding through junction tables, spreads in the parent query, filtering and ordering an embedded list, computed relationships, Prefer: missing=default and max-affected, custom media types, and verbatim database errors in compatibility mode.",
  },
];

const fixed = [
  {
    title: "Range headers were ignored unless they began 0-",
    body: "Every other range silently returned the whole relation — the request succeeded, so nothing looked wrong. Range: 5-9 now means rows 5 to 9, and an inverted range is refused with 416 rather than quietly widened.",
  },
  {
    title: "OPTIONS never reached the handler",
    body: "The CORS layer answers every OPTIONS itself and never calls what it wraps, so no response carried Allow and the body was empty because nothing built one.",
  },
  {
    title: "An expiry was honoured with 30 seconds of slack",
    body: "exp is now checked to the second. The slack remains on nbf and iat, which describe a token not yet valid rather than one its issuer withdrew.",
  },
  {
    title: "Two error codes were documented the wrong way round",
    body: "PGRST301 and PGRST302 had been swapped in the documentation — so anyone branching on them from the docs branched wrongly.",
  },
];

export default component$(() => {
  const pg = conformance.all;
  const hg = hasuraConformance.all;

  return (
    <div class="min-h-screen bg-white">
      <div class="bg-gradient-to-b from-neutral-50 to-white border-b border-neutral-200">
        <div class="container-wide py-12">
          <div class="flex items-center gap-3 mb-4">
            <span class="px-2 py-0.5 text-xs font-semibold rounded bg-amber-100 text-amber-800 border border-amber-200">
              PRERELEASE
            </span>
            <span class="text-sm text-neutral-500">{hasuraConformanceMeta.measured}</span>
          </div>
          <h1 class="text-4xl font-bold text-neutral-900 mb-4">Postrust {VERSION}</h1>
          <p class="text-lg text-neutral-600 max-w-2xl">
            Both surfaces stop being asserted and start being measured. Postrust answers
            PostgREST&rsquo;s REST dialect and Hasura&rsquo;s GraphQL dialect, and how closely it
            answers each is now a number produced by replaying the other server&rsquo;s own test
            suite against both and diffing the live responses.
          </p>
        </div>
      </div>

      <div class="container-wide py-12">
        <div class="max-w-3xl">
          {/* The numbers */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">Measured, not asserted</h2>
            <div class="grid sm:grid-cols-2 gap-4 mb-4">
              <Link
                href="/docs/conformance/postgrest"
                class="block p-5 rounded-lg border border-neutral-200 hover:border-primary-400 transition-colors"
              >
                <div class="text-sm text-neutral-500 mb-1">
                  vs PostgREST {conformanceMeta.postgrest}
                </div>
                <div class="text-3xl font-bold text-neutral-900 font-mono">
                  {pg.statusAndBody.pct}%
                </div>
                <div class="text-sm text-neutral-600 mt-1">
                  status and body, over {pg.cases} replayed cases
                </div>
                <div class="text-sm text-neutral-500 mt-2">
                  {pg.fullContract.pct}% on the full contract, headers included
                </div>
              </Link>
              <Link
                href="/docs/conformance/hasura"
                class="block p-5 rounded-lg border border-neutral-200 hover:border-primary-400 transition-colors"
              >
                <div class="text-sm text-neutral-500 mb-1">
                  vs Hasura {hasuraConformanceMeta.hasura}
                </div>
                <div class="text-3xl font-bold text-neutral-900 font-mono">
                  {hg.sameData.pct}%
                </div>
                <div class="text-sm text-neutral-600 mt-1">
                  same data, over {hg.cases} cases in {hasuraConformanceMeta.groups} groups
                </div>
                <div class="text-sm text-neutral-500 mt-2">
                  {hg.status.pct}% agree on status; {hg.fullBody.pct}% on the whole body
                </div>
              </Link>
            </div>
            <p class="text-neutral-600 mb-4">
              Neither harness interprets a test expectation. The reference implementation&rsquo;s
              live response is the oracle, so a mistake in the extractor shows up as a case both
              servers answer the same way rather than as a false failure. Of the{" "}
              {hasuraAgreement.sameData + hasuraAgreement.bothRefuse} Hasura cases counted at that
              level, {hasuraAgreement.sameData} agree about data and {hasuraAgreement.bothRefuse}{" "}
              agree because both servers refuse — a distinction worth keeping, since counting only
              the first would score a case where Hasura itself raises an error as a failure to
              match it.
            </p>
            <p class="text-neutral-600">
              Both numbers carry their provenance. Each harness builds its own candidate, because
              which features it was built with is part of what is measured and cannot be read off
              the binary, and records the reference version, the features, the commit and whether
              the reference was replayed or a recording reused. The generators that put these
              figures on this page read that record and refuse a run that cannot account for
              itself. Nothing here is typed by hand.
            </p>
          </section>

          {/* Alpha */}
          <section class="mb-12">
            <div class="p-5 rounded-lg bg-amber-50 border border-amber-200">
              <h2 class="text-lg font-bold text-neutral-900 mb-2">What the alpha means</h2>
              <p class="text-neutral-700 mb-2">
                The surfaces are measured. The public Rust API has not been lived with by anyone
                outside the repository, and a prerelease carries no stability promise — expect it
                to move before 1.0.0.
              </p>
              <p class="text-neutral-700">
                The HTTP and GraphQL surfaces are the part meant to be stable. If you are pointing
                a PostgREST or Hasura client at this, that is the contract the conformance reports
                describe.
              </p>
            </div>
          </section>

          {/* Added */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">Added</h2>
            <div class="space-y-4">
              {added.map((a) => (
                <div key={a.title} class="p-4 bg-neutral-50 rounded-lg">
                  <h3 class="font-semibold text-neutral-900 mb-1">{a.title}</h3>
                  <p class="text-sm text-neutral-600">{a.body}</p>
                </div>
              ))}
            </div>
          </section>

          {/* Fixed */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">Fixed</h2>
            <p class="text-neutral-600 mb-4">
              Every one of these was found by measurement rather than by a report, and each had
              the same shape: the request succeeded, so nothing looked wrong.
            </p>
            <div class="space-y-4">
              {fixed.map((f) => (
                <div key={f.title} class="p-4 bg-neutral-50 rounded-lg">
                  <h3 class="font-semibold text-neutral-900 mb-1">{f.title}</h3>
                  <p class="text-sm text-neutral-600">{f.body}</p>
                </div>
              ))}
            </div>
          </section>

          {/* Breaking */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">Breaking changes</h2>
            <p class="text-neutral-600 mb-4">
              These are why the version is 1.0.0 rather than 0.4.1. They affect Rust code that
              depends on the crates; the HTTP and GraphQL surfaces are unaffected.
            </p>
            <ul class="space-y-2 text-neutral-600">
              <li>
                <code class="font-mono text-sm">JwtError</code> lost six variants and gained four.
              </li>
              <li>
                <code class="font-mono text-sm">Range</code>,{" "}
                <code class="font-mono text-sm">QueryResult</code> and{" "}
                <code class="font-mono text-sm">Table</code> each gained public fields. None is{" "}
                <code class="font-mono text-sm">#[non_exhaustive]</code> yet, so a struct literal
                downstream needs updating; marking them is planned during the alpha series.
              </li>
            </ul>
          </section>

          {/* Gaps */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">Known gaps</h2>
            <p class="text-neutral-600 mb-4">
              The largest is <strong>introspection</strong>, and it is not reachable from here:
              async-graphql builds its own registry and keeps it private, so the directives it
              installs and the order it lists types in cannot be changed from outside the library.
              Eight of the sixteen remaining Hasura divergences are that one thing.
            </p>
            <p class="text-neutral-600">
              Beside it: <code class="font-mono text-sm">_stream</code> subscriptions, the
              cursor-based half of Hasura&rsquo;s subscription surface; and the OpenAPI document
              PostgREST serves at <code class="font-mono text-sm">/</code>. Actions and Apollo
              federation are subsystems rather than gaps. The two{" "}
              <code class="font-mono text-sm">FINDINGS.md</code> files record the rest, including
              four faults found in the Hasura harness itself — one of which invalidated eleven
              runs.
            </p>
          </section>

          {/* Install */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">Try it</h2>
            <div class="bg-neutral-900 rounded-xl overflow-hidden">
              <div class="px-4 py-2 bg-neutral-800 border-b border-neutral-700">
                <span class="text-sm text-neutral-400">bash</span>
              </div>
              <pre class="p-4 text-sm overflow-x-auto">
                <code class="text-neutral-100">{`docker pull ghcr.io/postrust/postrust:v${VERSION}`}</code>
              </pre>
            </div>
            <p class="text-sm text-neutral-500 mt-3">
              A prerelease is not tagged <code class="font-mono">latest</code>, so it has to be
              asked for by name.
            </p>
          </section>

          <div class="flex items-center justify-between pt-8 border-t border-neutral-200">
            <Link href="/docs/getting-started" class="flex items-center gap-2 text-neutral-600 hover:text-primary-600">
              <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 19l-7-7 7-7"/>
              </svg>
              Getting Started
            </Link>
            <Link href="/docs/conformance" class="flex items-center gap-2 text-neutral-600 hover:text-primary-600">
              Conformance
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
  title: `Postrust ${VERSION} — measured against PostgREST and Hasura`,
  links: [{ rel: "canonical", href: "https://postrust.org/releases" }],
  meta: [
    {
      name: "description",
      content:
        "Postrust 1.0.0-alpha.1: a PostgREST-compatible REST API and a Hasura-dialect GraphQL API from your PostgreSQL schema, each measured by replaying the other server's own test suite and diffing the live responses.",
    },
  ],
};
