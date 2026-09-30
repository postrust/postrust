import { component$ } from "@builder.io/qwik";
import type { DocumentHead } from "@builder.io/qwik-city";
import { Link } from "@builder.io/qwik-city";
import { conformance, conformanceMeta } from "~/data/conformance";
import { h2spec, autobahn } from "~/data/transport-conformance";
import {
  hasuraConformance,
  hasuraConformanceMeta,
  hasuraAgreement,
} from "~/data/hasura-conformance";

const VERSION = "2.0.0";

const added = [
  {
    title: "Postrust can be an Apollo Federation v2 subgraph",
    body: "Set PGRST_GRAPHQL_FEDERATION=true and the GraphQL API adds _service { sdl }, _entities(representations:) and the Federation directives, so an Apollo router or gateway can compose it with other services. It is opt-in: with the variable unset, Federation metadata is inert and adds nothing to the SDL, and a test holds it there.",
  },
  {
    title: "A table's primary key is its @key, and a table can be shared",
    body: 'A table marked "federation": {"shared": true} in PGRST_GRAPHQL_METADATA keeps its unprefixed type name, so two subgraphs can both contribute to it, and its non-key columns, computed fields and relationships are marked @shareable. Key columns are shareable through @key already.',
  },
  {
    title: "A prefix, so several Postrust subgraphs can be composed",
    body: "PGRST_GRAPHQL_TYPE_PREFIX prefixes generated types and root fields, so subgraphs over different databases do not collide when composed. A shared entity's object type stays unprefixed; its root fields and helper types still take the prefix.",
  },
  {
    title: "Root types a router expects",
    body: "With Federation on, the root types are Query, Mutation and Subscription rather than Hasura's query_root family. With it off, nothing about the names changes.",
  },
  {
    title: "_entities lets PostgreSQL decide what matches",
    body: "Each type's representations are resolved in one query. A key that PostgreSQL considers equal but spells differently — an uppercase UUID, a citext value in another case, a timestamptz with a different offset — finds its row, and a key that matches nothing is a null in its position rather than an error for the whole list. Select permissions apply to _entities exactly as to any other read.",
  },
  {
    title: "A column can be exposed as GraphQL ID",
    body: 'PostgreSQL has no ID type, so a key that a router or client expects as ID could not be one. A column_types entry in PGRST_GRAPHQL_METADATA — {"id": "ID"} — changes the GraphQL type of a text, integer or UUID column without changing how it is read, bound or cast.',
  },
  {
    title: "Apollo's own compatibility suite, in CI",
    body: "Apollo's subgraph compatibility suite runs on every pull request that touches the GraphQL path, against a fixture pinned to Apollo's schema and data. A required-capability failure fails the build; an optional one is reported without failing it.",
  },
];

const changed = [
  {
    title: "A boolean setting that is not a boolean now stops the server",
    body: "Every PGRST_* true/false variable used to read anything other than an exact lowercase true as false, so PGRST_DB_AGGREGATES_ENABLED=TRUE, or =yes, or a typo silently disabled the feature. They now accept true/false, 1/0, yes/no and on/off in any case, and anything else fails startup with the variable's name.",
  },
  {
    title: "Configuration and schema structs are #[non_exhaustive]",
    body: "Federation added public fields to AppConfig, SchemaConfig, TableNames, GeneratedSchema and eight other public structs, and adding a field breaks any code that builds one with an exhaustive struct literal. So that the next feature is not another major release, those structs and the related configuration, schema and metadata types are now #[non_exhaustive].",
  },
  {
    title: "postrust-proxy and postrust-worker go to 0.6.0",
    body: "They depend on postrust-core, which is now 2.0.0. Neither changed otherwise.",
  },
];

export default component$(() => {
  const pg = conformance.all;
  const hg = hasuraConformance.all;

  return (
    <div class="min-h-screen bg-white">
      <div class="border-b border-neutral-200 bg-gradient-to-b from-neutral-50 to-white">
        <div class="container-wide py-12">
          <div class="mb-4 flex items-center gap-3">
            <span class="rounded border border-neutral-200 bg-neutral-100 px-2 py-0.5 text-xs font-semibold text-neutral-800">
              MAJOR
            </span>
            <span class="text-sm text-neutral-500">
              A major version for the Rust API; additive for the server
            </span>
          </div>
          <h1 class="mb-4 text-4xl font-bold text-neutral-900">
            Postrust {VERSION}
          </h1>
          <p class="max-w-2xl text-lg text-neutral-600">
            Postrust can now be an Apollo Federation v2 subgraph, so a router
            can compose its GraphQL API with other services. It is opt-in, and
            with it off the generated schema is what it was. The version is
            2.0.0 because of the Rust API, not the server: if you run the binary
            or the image, the one change you can notice is that a malformed
            boolean setting now stops startup instead of reading as false.
          </p>
        </div>
      </div>

      <div class="container-wide py-12">
        <div class="max-w-3xl">
          {/* The numbers */}
          <section class="mb-12">
            <h2 class="mb-4 text-2xl font-bold text-neutral-900">
              Measured, not asserted
            </h2>
            <div class="mb-4 grid gap-4 sm:grid-cols-2">
              <Link
                href="/docs/conformance/postgrest"
                class="hover:border-primary-400 block rounded-lg border border-neutral-200 p-5 transition-colors"
              >
                <div class="mb-1 text-sm text-neutral-500">
                  vs PostgREST {conformanceMeta.postgrest}
                </div>
                <div class="font-mono text-3xl font-bold text-neutral-900">
                  {pg.statusAndBody.pct}%
                </div>
                <div class="mt-1 text-sm text-neutral-600">
                  status and body, over {pg.cases} replayed cases
                </div>
                <div class="mt-2 text-sm text-neutral-500">
                  {pg.fullContract.pct}% on the full contract, headers included
                </div>
              </Link>
              <Link
                href="/docs/conformance/hasura"
                class="hover:border-primary-400 block rounded-lg border border-neutral-200 p-5 transition-colors"
              >
                <div class="mb-1 text-sm text-neutral-500">
                  vs Hasura {hasuraConformanceMeta.hasura}
                </div>
                <div class="font-mono text-3xl font-bold text-neutral-900">
                  {hg.sameData.pct}%
                </div>
                <div class="mt-1 text-sm text-neutral-600">
                  same data, over {hg.cases} cases in{" "}
                  {hasuraConformanceMeta.groups} groups
                </div>
                <div class="mt-2 text-sm text-neutral-500">
                  {hg.status.pct}% agree on status; {hg.fullBody.pct}% on the
                  whole body
                </div>
              </Link>
            </div>
            <p class="mb-4 text-neutral-600">
              These are the figures from the most recent recorded runs:
              PostgREST on {conformanceMeta.measured} at commit{" "}
              <code class="font-mono text-sm">
                {conformanceMeta.commit.slice(0, 7)}
              </code>
              , Hasura on {hasuraConformanceMeta.measured} at commit{" "}
              <code class="font-mono text-sm">
                {hasuraConformanceMeta.commit.slice(0, 7)}
              </code>
              . This page reads them from the run&rsquo;s own record, so they
              move when the suites are re-run and not otherwise. With Federation
              off, this release adds nothing to the GraphQL schema the Hasura
              suite measures.
            </p>
            <p class="mb-4 text-neutral-600">
              Neither harness interprets a test expectation. The reference
              implementation&rsquo;s live response is the oracle, so a mistake
              in the extractor shows up as a case both servers answer the same
              way rather than as a false failure. Of the{" "}
              {hasuraAgreement.sameData + hasuraAgreement.bothRefuse} Hasura
              cases counted at that level, {hasuraAgreement.sameData} agree
              about data and {hasuraAgreement.bothRefuse} agree because both
              servers refuse — a distinction worth keeping, since counting only
              the first would score a case where Hasura itself raises an error
              as a failure to match it.
            </p>
            <p class="text-neutral-600">
              Federation is measured by a third suite that is not ours:
              Apollo&rsquo;s subgraph compatibility suite, run against a fixture
              pinned to Apollo&rsquo;s own schema and data. Its required
              capabilities gate the build. The optional ones Postrust does not
              yet support are listed under Known gaps below rather than left for
              someone else to find.
            </p>
          </section>

          {/* Transports */}
          <section class="mb-12">
            <h2 class="mb-4 text-2xl font-bold text-neutral-900">
              The front door, measured
            </h2>
            <div class="mb-4 grid gap-4 sm:grid-cols-2">
              <div class="rounded-lg border border-neutral-200 p-5">
                <div class="mb-1 text-sm text-neutral-500">h2spec</div>
                <div class="font-mono text-3xl font-bold text-neutral-900">
                  {h2spec.passed}/{h2spec.tests}
                </div>
                <div class="mt-1 text-sm text-neutral-600">
                  passed, {h2spec.failed} failed
                </div>
                <div class="mt-2 text-sm text-neutral-500">
                  {h2spec.skipped} skipped, against the HTTP/2-only listener
                </div>
              </div>
              <div class="rounded-lg border border-neutral-200 p-5">
                <div class="mb-1 text-sm text-neutral-500">Autobahn</div>
                <div class="font-mono text-3xl font-bold text-neutral-900">
                  {autobahn.regressions.length}
                </div>
                <div class="mt-1 text-sm text-neutral-600">
                  cases the tunnel made worse than no tunnel
                </div>
                <div class="mt-2 text-sm text-neutral-500">
                  over {autobahn.cases} cases, {autobahn.ok} of{" "}
                  {autobahn.settledCases} OK
                </div>
              </div>
            </div>
            <p class="mb-4 text-neutral-600">
              Three suites, none of them ours. <strong>HTTP Garden</strong>, a
              differential fuzzer that sends a payload through a proxy and shows
              how a set of origin servers parsed what came out, is what found
              the hop-by-hop defect. <strong>h2spec</strong> speaks HTTP/2 at
              the listener. <strong>Autobahn</strong> is the reference WebSocket
              suite.
            </p>
            <p class="mb-4 text-neutral-600">
              The Autobahn figure needs its method stated, because the obvious
              number would be misleading. Postrust splices two upgraded byte
              streams and never parses a WebSocket frame, so most of what the
              suite scores belongs to the origin behind it, not to the proxy.
              Every run therefore has a twin that bypasses the proxy entirely,
              and the figure above is the difference: cases that are worse
              through the tunnel than without it. Of {autobahn.cases} cases,{" "}
              {autobahn.failed} fails &mdash; and fails identically with no
              proxy in the path, so it is the origin&rsquo;s.
            </p>
            <p class="text-neutral-600">
              One family of cases is excluded from that comparison and named on
              every run rather than dropped quietly: those that send a valid
              message, then an invalid frame, and expect the echo of the first.
              The origin fails the connection without flushing that echo when
              both arrive in one read, and any relay coalesces what a client
              chopped &mdash; measured directly, with no proxy involved, the
              same bytes sent octet-wise echo and sent as one write do not.
              Which member of the family trips varies between runs, so the{" "}
              {autobahn.segmentationSensitive} of them are left out of the OK
              count above rather than making it move between runs. On this run{" "}
              {autobahn.intermittentCount === 0
                ? "none of them was worse than the baseline"
                : `${autobahn.intermittentCount} of them was worse than the baseline`}
              .
            </p>
          </section>

          {/* What the version means */}
          <section class="mb-12">
            <div class="rounded-lg border border-amber-200 bg-amber-50 p-5">
              <h2 class="mb-2 text-lg font-bold text-neutral-900">
                What 2.0.0 means
              </h2>
              <p class="mb-2 text-neutral-700">
                The features are additive for anyone running the server. The
                major bump is for the Rust API: Federation added public fields
                to <code class="text-sm">AppConfig</code>,{" "}
                <code class="text-sm">SchemaConfig</code>,{" "}
                <code class="text-sm">TableNames</code>,{" "}
                <code class="text-sm">GeneratedSchema</code> and eight other
                public structs, and adding a field breaks code that builds one
                with an exhaustive struct literal. The stability policy covers
                public struct fields, and{" "}
                <code class="text-sm">cargo semver-checks</code> against 1.0.1
                agrees.
              </p>
              <p class="text-neutral-700">
                Those structs are now{" "}
                <code class="text-sm">#[non_exhaustive]</code>, so a new field
                or variant on one of them is a minor release from here, and a
                feature that adds a setting does not need 3.0.0.
              </p>
            </div>
          </section>

          {/* Added */}
          <section class="mb-12">
            <h2 class="mb-4 text-2xl font-bold text-neutral-900">Added</h2>
            <div class="space-y-4">
              {added.map((a) => (
                <div key={a.title} class="rounded-lg bg-neutral-50 p-4">
                  <h3 class="mb-1 font-semibold text-neutral-900">{a.title}</h3>
                  <p class="text-sm text-neutral-600">{a.body}</p>
                </div>
              ))}
            </div>
            <p class="mt-4 text-neutral-600">
              Thanks to @CBeardSafire, who asked for Federation in{" "}
              <a
                href="https://github.com/postrust/postrust/issues/40"
                class="text-primary-600 hover:underline"
              >
                #40
              </a>{" "}
              and built it in{" "}
              <a
                href="https://github.com/postrust/postrust/pull/42"
                class="text-primary-600 hover:underline"
              >
                #42
              </a>
              . How to set it up is in{" "}
              <Link
                href="/docs/graphql"
                class="text-primary-600 hover:underline"
              >
                the GraphQL docs
              </Link>
              .
            </p>
          </section>

          {/* Changed */}
          <section class="mb-12">
            <h2 class="mb-4 text-2xl font-bold text-neutral-900">Changed</h2>
            <div class="space-y-4">
              {changed.map((c) => (
                <div key={c.title} class="rounded-lg bg-neutral-50 p-4">
                  <h3 class="mb-1 font-semibold text-neutral-900">{c.title}</h3>
                  <p class="text-sm text-neutral-600">{c.body}</p>
                </div>
              ))}
            </div>
          </section>

          {/* Upgrading */}
          <section class="mb-12">
            <h2 class="mb-4 text-2xl font-bold text-neutral-900">Upgrading</h2>
            <p class="mb-4 text-neutral-600">
              <strong>If you run the server</strong>, check your boolean
              settings. A deployment that was relying on a malformed value
              reading as false will not start after upgrading. That is the point
              &mdash; the value never meant what it looked like it meant &mdash;
              and the error names the variable to fix. Nothing else changes: the
              HTTP and GraphQL surfaces are the same, and Federation stays off
              until you turn it on.
            </p>
            <p class="mb-4 text-neutral-600">
              <strong>If you depend on the crates</strong>, build the affected
              types from their default and assign the fields you need. A struct
              literal &mdash; including one ending in{" "}
              <code class="font-mono text-sm">..Default::default()</code>{" "}
              &mdash; no longer compiles for them from another crate:
            </p>
            <div class="mb-4 overflow-hidden rounded-xl bg-neutral-900">
              <div class="border-b border-neutral-700 bg-neutral-800 px-4 py-2">
                <span class="text-sm text-neutral-400">rust</span>
              </div>
              <pre class="overflow-x-auto p-4 text-sm">
                <code class="text-neutral-100">{`let mut config = SchemaConfig::default();
config.exposed_schemas = vec!["public".into()];
config.enable_mutations = true;`}</code>
              </pre>
            </div>
            <p class="mb-4 text-neutral-600">
              An exhaustive <code class="font-mono text-sm">match</code> on{" "}
              <code class="font-mono text-sm">ColumnTypeOverride</code> needs a
              wildcard arm. The affected types are{" "}
              <code class="font-mono text-sm">AppConfig</code> and{" "}
              <code class="font-mono text-sm">RoleSettings</code> in{" "}
              <code class="font-mono text-sm">postrust-core</code>; the schema
              and field types in{" "}
              <code class="font-mono text-sm">postrust_graphql::schema</code>;
              and the metadata types in{" "}
              <code class="font-mono text-sm">postrust_graphql::names</code>.
              The changelog lists every one. Nothing else in the public API
              changed.
            </p>
            <p class="text-neutral-600">
              <code class="font-mono text-sm">AppConfig::try_from_env</code>{" "}
              returns the boolean error;{" "}
              <code class="font-mono text-sm">AppConfig::from_env</code> panics
              with it.
            </p>
          </section>

          {/* Gaps */}
          <section class="mb-12">
            <h2 class="mb-4 text-2xl font-bold text-neutral-900">Known gaps</h2>
            <p class="mb-4 text-neutral-600">
              In Federation: the primary key is the only{" "}
              <code class="font-mono text-sm">@key</code>, so repeatable keys
              and nested keys are not expressible, and neither are extended or
              external fields, <code class="font-mono text-sm">@requires</code>,{" "}
              <code class="font-mono text-sm">@provides</code>,{" "}
              <code class="font-mono text-sm">@override</code>,{" "}
              <code class="font-mono text-sm">@tag</code>,{" "}
              <code class="font-mono text-sm">@inaccessible</code>,{" "}
              <code class="font-mono text-sm">@composeDirective</code>,{" "}
              <code class="font-mono text-sm">@interfaceObject</code> or federated
              tracing. Apollo counts these as optional capabilities; the
              compatibility fixture keeps them in its target schema on purpose,
              so they show in every report rather than being dropped from it.
              The suite&rsquo;s <code class="font-mono text-sm">@shareable</code>{" "}
              check also reports a failure: it looks for the directive on the
              type, and Postrust puts it on each non-key field, which composes
              the same way.
            </p>
            <p class="mb-4 text-neutral-600">
              In the dialects, unchanged from 1.0: the largest gap is{" "}
              <strong>introspection</strong>, and it is not reachable from here
              &mdash; async-graphql builds its own registry and keeps it
              private, so the directives it installs and the order it lists
              types in cannot be changed from outside the library. Beside it:{" "}
              <code class="font-mono text-sm">_stream</code> subscriptions, the
              cursor-based half of Hasura&rsquo;s subscription surface; and the
              OpenAPI document PostgREST serves at{" "}
              <code class="font-mono text-sm">/</code>.
            </p>
            <p class="text-neutral-600">
              In the proxy, also unchanged: no HTTP/3, no upstream HTTP/2 over
              TLS, no response cache, no retry or circuit-breaking, no
              configuration reload, and automatic certificates are HTTP-01 only,
              so no wildcards. The{" "}
              <code class="font-mono text-sm">FINDINGS.md</code> files record
              the rest.
            </p>
          </section>

          {/* Previous releases */}
          <section class="mb-12">
            <h2 class="mb-4 text-2xl font-bold text-neutral-900">
              Previous releases
            </h2>
            <div class="space-y-4">
              <div class="rounded-lg bg-neutral-50 p-4">
                <h3 class="mb-1 font-semibold text-neutral-900">1.0.1</h3>
                <p class="mb-2 text-sm text-neutral-600">
                  <strong>
                    The server could not talk to a hosted database over TLS, and
                    did not say so.
                  </strong>{" "}
                  <code class="font-mono">sqlx</code> was built without any TLS
                  feature, so <code class="font-mono">sslmode=require</code>{" "}
                  failed outright, and the default{" "}
                  <code class="font-mono">sslmode=prefer</code> silently fell
                  back to cleartext &mdash; every 1.0.0 deployment against a
                  hosted database was unencrypted on the wire unless it had
                  asked for <code class="font-mono">require</code> and noticed
                  the error. Fixed with rustls verifying against the host trust
                  store, so a private CA works without rebuilding, and a test
                  keeps the feature enabled.
                </p>
                <p class="text-sm text-neutral-600">
                  rustls moved to 0.23.45 for RUSTSEC-2026-0285, which enabling
                  TLS in sqlx had made reachable from the stable line. Thanks to
                  @CBeardSafire, who found the TLS problem and sent the fix in{" "}
                  <a
                    href="https://github.com/postrust/postrust/pull/38"
                    class="text-primary-600 hover:underline"
                  >
                    #38
                  </a>
                  .
                </p>
              </div>
              <div class="rounded-lg bg-neutral-50 p-4">
                <h3 class="mb-1 font-semibold text-neutral-900">1.0.0</h3>
                <p class="mb-2 text-sm text-neutral-600">
                  Seven crates took a semver promise: a breaking change to the
                  public Rust API needs a major bump, which is why this release
                  is 2.0.0. <code class="font-mono">postrust-proxy</code> and{" "}
                  <code class="font-mono">postrust-worker</code> moved to their
                  own 0.x lines, and nothing in the stable line depends on
                  either.
                </p>
                <p class="text-sm text-neutral-600">
                  No code changed from the beta. The throughput figures were
                  restored, measured on dedicated bare metal behind a gate that
                  rejects a run the machine drifted under, and both conformance
                  suites were re-run from a clean tree. The work before it made
                  the HTTP proxy runnable and testable &mdash; automatic
                  certificates, host routing over HTTP/2, domain verification
                  that proves something, and a declared minimum Rust version of
                  1.88.
                </p>
              </div>
            </div>
            <p class="mt-4 text-sm text-neutral-500">
              The full history is in{" "}
              <a
                href="https://github.com/postrust/postrust/blob/main/CHANGELOG.md"
                class="text-primary-600 hover:underline"
              >
                CHANGELOG.md
              </a>
              .
            </p>
          </section>

          {/* Install */}
          <section class="mb-12">
            <h2 class="mb-4 text-2xl font-bold text-neutral-900">Try it</h2>
            <div class="overflow-hidden rounded-xl bg-neutral-900">
              <div class="border-b border-neutral-700 bg-neutral-800 px-4 py-2">
                <span class="text-sm text-neutral-400">bash</span>
              </div>
              <pre class="overflow-x-auto p-4 text-sm">
                <code class="text-neutral-100">{`docker pull postrust/postrust:${VERSION}`}</code>
              </pre>
            </div>
            <p class="mt-3 text-sm text-neutral-500">
              Federation is off until{" "}
              <code class="font-mono">PGRST_GRAPHQL_FEDERATION=true</code>. A
              working subgraph fixture, and how to run Apollo&rsquo;s suite
              against it, is in{" "}
              <a
                href="https://github.com/postrust/postrust/tree/main/scripts/apollo-federation"
                class="text-primary-600 hover:underline"
              >
                scripts/apollo-federation
              </a>
              .
            </p>
          </section>

          <div class="flex items-center justify-between border-t border-neutral-200 pt-8">
            <Link
              href="/docs/getting-started"
              class="hover:text-primary-600 flex items-center gap-2 text-neutral-600"
            >
              <svg
                class="h-4 w-4"
                fill="none"
                stroke="currentColor"
                viewBox="0 0 24 24"
              >
                <path
                  stroke-linecap="round"
                  stroke-linejoin="round"
                  stroke-width="2"
                  d="M15 19l-7-7 7-7"
                />
              </svg>
              Getting Started
            </Link>
            <Link
              href="/docs/conformance"
              class="hover:text-primary-600 flex items-center gap-2 text-neutral-600"
            >
              Conformance
              <svg
                class="h-4 w-4"
                fill="none"
                stroke="currentColor"
                viewBox="0 0 24 24"
              >
                <path
                  stroke-linecap="round"
                  stroke-linejoin="round"
                  stroke-width="2"
                  d="M9 5l7 7-7 7"
                />
              </svg>
            </Link>
          </div>
        </div>
      </div>
    </div>
  );
});

export const head: DocumentHead = {
  title: `Postrust ${VERSION} — an Apollo Federation subgraph, opt-in`,
  links: [{ rel: "canonical", href: "https://postrust.org/releases" }],
  meta: [
    {
      name: "description",
      content: `Postrust ${VERSION}: opt-in Apollo Federation v2 subgraph support, columns exposed as GraphQL ID, and boolean settings that fail startup when malformed. A major version for Rust API changes only; nothing else changes for the server.`,
    },
  ],
};
