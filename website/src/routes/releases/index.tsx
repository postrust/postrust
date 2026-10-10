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

const VERSION = "2.1.0";

const added = [
  {
    title: "GraphQL transactions carry the verified JWT claims",
    body: "REST set request.jwt.claims for a row-level policy to read; GraphQL set only the role and Hasura's session variables, so a policy telling apart users who share a database role worked over one surface and not the other. Both now set the claims document, with role set to the role the request runs as, and each top-level claim whose name can be a setting as its own request.jwt.claims.<name>.",
  },
  {
    title: "Subscriptions are authenticated as HTTP requests are",
    body: 'A WebSocket connection is identified once, from its connection_init payload — {"headers": {"Authorization": "Bearer <token>"}}, as Hasura\'s clients send it, since a browser cannot set headers on a WebSocket — and the upgrade request\'s headers. It goes through the same code an HTTP request does: the admin secret, X-Hasura-Role against the token\'s allowed roles, the role\'s schema and its introspection setting. A connection HTTP would refuse is refused at connection_init, and every live query re-reads as the connection\'s identity, so one row-level policy covers queries and subscriptions alike.',
  },
];

const fixed = [
  {
    title: "GraphQL subscriptions over a WebSocket work",
    body: "They had not, on the server as shipped: the handshake answered 101 and then nothing, not even connection_ack. The adapter that percent-encodes the > in select=data->>id before hyper parses a request line followed HTTP/1.1 framing and did not know about Upgrade, so after the handshake it read every WebSocket frame as the next request line — holding it back until a newline byte happened to arrive, and percent-encoding the high-bit bytes masked frames are full of. It now stops rewriting after a request that asks to switch protocols.",
  },
  {
    title: "A subscription that fails says why",
    body: "A live query whose read failed — a permission refused, a cast that cannot be made — ended with a bare complete, the error dropped on the way out, so a refused subscription looked exactly like one that had finished. The error is now sent, and the subscription ends after it.",
  },
  {
    title: "The documented policies work on pooled connections",
    body: "A per-claim setting a connection has ever held reads as '' rather than NULL once the transaction that set it ends. The documentation cast them straight to integer, so a token without the claim was answered on some connections and failed with invalid input syntax for type integer on others, depending only on what the connection had served before. The examples now read the claims document, which is set on every request; the per-claim settings are documented with NULLIF(..., '') and with what they cannot reach.",
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
              MINOR
            </span>
            <span class="text-sm text-neutral-500">
              Additive; nothing breaks
            </span>
          </div>
          <h1 class="mb-4 text-4xl font-bold text-neutral-900">
            Postrust {VERSION}
          </h1>
          <p class="max-w-2xl text-lg text-neutral-600">
            Row-level security now sees the same identity over GraphQL as over
            REST, including in subscriptions &mdash; which, it turned out, did
            not work over a WebSocket at all until this release. One policy on
            the verified JWT claims now covers a REST read, a GraphQL query and
            a live query alike.
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
              move when the suites are re-run and not otherwise. Both suites
              were re-run on this release&rsquo;s branch and reproduce these
              figures case for case: the claims and the WebSocket fix change
              nothing either of them measures.
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
                What 2.1.0 means
              </h2>
              <p class="mb-2 text-neutral-700">
                A minor release. The public Rust API gains one function,{" "}
                <code class="text-sm">AuthResult::claim_settings</code>, and
                loses nothing: <code class="text-sm">cargo semver-checks</code>{" "}
                against 2.0.0 passes for every stable crate. Everything else
                here is behaviour you see by running the server.
              </p>
              <p class="text-neutral-700">
                <code class="text-sm">postrust-proxy</code> and{" "}
                <code class="text-sm">postrust-worker</code> stay at 0.6.0:
                neither uses anything that changed.
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
              Thanks to @CBeardSafire, who exposed the claims to GraphQL in{" "}
              <a
                href="https://github.com/postrust/postrust/pull/45"
                class="text-primary-600 hover:underline"
              >
                #45
              </a>
              ; testing that end to end is what found the WebSocket and the
              documentation problems below, fixed in{" "}
              <a
                href="https://github.com/postrust/postrust/pull/47"
                class="text-primary-600 hover:underline"
              >
                #47
              </a>
              . How to write the policies is in{" "}
              <Link
                href="/docs/authentication"
                class="text-primary-600 hover:underline"
              >
                the authentication docs
              </Link>
              .
            </p>
          </section>

          {/* Fixed */}
          <section class="mb-12">
            <h2 class="mb-4 text-2xl font-bold text-neutral-900">Fixed</h2>
            <div class="space-y-4">
              {fixed.map((c) => (
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
              <strong>If you run the server</strong>, nothing is required. Two
              things are worth checking.
            </p>
            <p class="mb-4 text-neutral-600">
              <strong>Policies on per-claim settings.</strong> If you wrote one
              the way the documentation used to show, casting{" "}
              <code class="font-mono text-sm">
                current_setting(&apos;request.jwt.claims.&lt;name&gt;&apos;,
                true)
              </code>{" "}
              straight to a type, it will now fail for a token without that
              claim on a connection that has served one with it &mdash; the
              settings were never set before, so it never read anything. Read
              the document instead, or wrap the setting in{" "}
              <code class="font-mono text-sm">NULLIF(..., &apos;&apos;)</code>:
            </p>
            <div class="mb-4 overflow-hidden rounded-xl bg-neutral-900">
              <div class="border-b border-neutral-700 bg-neutral-800 px-4 py-2">
                <span class="text-sm text-neutral-400">sql</span>
              </div>
              <pre class="overflow-x-auto p-4 text-sm">
                <code class="text-neutral-100">{`-- always defined, and reaches nested claims
(current_setting('request.jwt.claims', true)::json->>'organization_id')::integer

-- the per-claim form, safe on a reused connection
NULLIF(current_setting('request.jwt.claims.organization_id', true), '')::integer`}</code>
              </pre>
            </div>
            <p class="mb-4 text-neutral-600">
              <strong>Subscription clients.</strong> Send the token in the{" "}
              <code class="font-mono text-sm">connection_init</code> payload, as
              Hasura&rsquo;s clients already do. A connection with no token now
              runs as the anonymous role, and where an admin secret is
              configured with no unauthorized role it is refused &mdash; as the
              same request over HTTP would be.
            </p>
            <p class="text-neutral-600">
              <strong>If you depend on the crates</strong>, nothing changes.{" "}
              <code class="font-mono text-sm">
                postrust_graphql::handler::graphql_ws_handler
              </code>{" "}
              is unchanged and now documented as authenticating nothing; the
              server serves its WebSocket routes with its own handler.
            </p>
          </section>

          {/* Gaps */}
          <section class="mb-12">
            <h2 class="mb-4 text-2xl font-bold text-neutral-900">Known gaps</h2>
            <p class="mb-4 text-neutral-600">
              In subscriptions: a connection keeps the identity it was opened
              with, so a token that expires while a subscription is open does
              not end it &mdash; a client that needs that should reconnect with
              a fresh token. A live query on a table without a trigger re-reads
              on{" "}
              <code class="font-mono text-sm">PGRST_SUBSCRIPTION_REFRESH</code>{" "}
              (30 seconds by default) rather than when a row changes.
            </p>
            <p class="mb-4 text-neutral-600">
              In Federation, unchanged from 2.0.0: the primary key is the only{" "}
              <code class="font-mono text-sm">@key</code>, so repeatable keys
              and nested keys are not expressible, and neither are extended or
              external fields, <code class="font-mono text-sm">@requires</code>,{" "}
              <code class="font-mono text-sm">@provides</code>,{" "}
              <code class="font-mono text-sm">@override</code>,{" "}
              <code class="font-mono text-sm">@tag</code>,{" "}
              <code class="font-mono text-sm">@inaccessible</code>,{" "}
              <code class="font-mono text-sm">@composeDirective</code>,{" "}
              <code class="font-mono text-sm">@interfaceObject</code> or
              federated tracing. Apollo counts these as optional capabilities;
              the compatibility fixture keeps them in its target schema on
              purpose, so they show in every report rather than being dropped
              from it. The suite&rsquo;s{" "}
              <code class="font-mono text-sm">@shareable</code> check also
              reports a failure: it looks for the directive on the type, and
              Postrust puts it on each non-key field, which composes the same
              way.
            </p>
            <p class="mb-4 text-neutral-600">
              In the dialects, unchanged since 1.0: the largest gap is{" "}
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
                <h3 class="mb-1 font-semibold text-neutral-900">2.0.0</h3>
                <p class="mb-2 text-sm text-neutral-600">
                  <strong>
                    Postrust can be an Apollo Federation v2 subgraph.
                  </strong>{" "}
                  Opt-in with{" "}
                  <code class="font-mono">PGRST_GRAPHQL_FEDERATION=true</code>:{" "}
                  <code class="font-mono">_service</code> and{" "}
                  <code class="font-mono">_entities</code>, a table&rsquo;s
                  primary key as its <code class="font-mono">@key</code>, shared
                  entities, a type prefix for composing several Postrust
                  subgraphs, and columns exposed as GraphQL{" "}
                  <code class="font-mono">ID</code>. Apollo&rsquo;s
                  compatibility suite gates the build. Thanks to @CBeardSafire,{" "}
                  <a
                    href="https://github.com/postrust/postrust/pull/42"
                    class="text-primary-600 hover:underline"
                  >
                    #42
                  </a>
                  .
                </p>
                <p class="text-sm text-neutral-600">
                  A major version for the Rust API only: Federation added public
                  fields to configuration and schema structs, which are now{" "}
                  <code class="font-mono">#[non_exhaustive]</code> so the next
                  such feature is a minor release. For the server, the one
                  change to notice was that a malformed boolean setting stopped
                  startup instead of reading as false.
                </p>
              </div>
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
                  public Rust API needs a major bump, which is why the
                  Federation release was 2.0.0.{" "}
                  <code class="font-mono">postrust-proxy</code> and{" "}
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
              Subscriptions are at{" "}
              <code class="font-mono">ws://&lt;host&gt;/v1/graphql/ws</code>,
              over <code class="font-mono">graphql-transport-ws</code> or the
              older <code class="font-mono">graphql-ws</code>, with the token in
              the <code class="font-mono">connection_init</code> payload. The
              policies that read the claims are in{" "}
              <Link
                href="/docs/authentication"
                class="text-primary-600 hover:underline"
              >
                the authentication docs
              </Link>
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
  title: `Postrust ${VERSION} — the same identity over GraphQL, subscriptions included`,
  links: [{ rel: "canonical", href: "https://postrust.org/releases" }],
  meta: [
    {
      name: "description",
      content: `Postrust ${VERSION}: verified JWT claims reach GraphQL transactions as they do REST ones, GraphQL subscriptions over a WebSocket work and are authenticated as HTTP requests are, and the documented row-level policies hold on pooled connections. A minor release; nothing breaks.`,
    },
  ],
};
