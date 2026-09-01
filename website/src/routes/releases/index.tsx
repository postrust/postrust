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

const VERSION = "1.0.0-beta.1";

const added = [
  {
    title: "Automatic SSL, actually implemented",
    body: "A verified domain asking for ACME is queued, and a background worker places the order, answers the HTTP-01 challenge and renews the certificate 30 days before it expires. The documentation had advertised this as a feature and the schema had tracked an ssl_status the whole time, while nothing in the crate had ever spoken to a certificate authority. Tested end to end against Pebble — Let's Encrypt's test CA, which deliberately misbehaves — through the challenge endpoint that ships.",
  },
  {
    title: "The endpoints the documentation promised",
    body: "Updating a domain, uploading your own certificate, and asking for issuance were all documented and none was in the router. An uploaded certificate is checked before it is stored: the key must match the chain, it must not be expired, and it must cover the domain, wildcards counting for exactly one label. Skipping any of those gives a listener that accepts the upload and then fails every handshake.",
  },
  {
    title: "Database-backed configuration that reads and writes",
    body: "Loading routes from the database returned an empty set behind a TODO, so a proxy pointed at one started, logged nothing wrong and answered every request with 503. The admin API replied 201 and persisted nothing. Both are real now, over three new tables — one of which the certificate store had been querying since it was written and which nothing had ever created.",
  },
  {
    title: "A declared and enforced minimum Rust version",
    body: "It was declared nowhere, while the README said 1.78 and the docs said 1.75. The floor is 1.88, established by building on it and by 1.87 being refused, and a CI job now checks it against the locked dependency set.",
  },
  {
    title: "Conformance on a schedule, and a security policy",
    body: "The suites that produce every figure on this site run nightly and weekly, and each regenerates the published data and fails if it has drifted. Nothing had been re-running them. SECURITY.md says how to report privately and what is in scope, and cargo audit runs on every pull request.",
  },
];

const fixed = [
  {
    title: "HTTP domain verification proved nothing",
    body: "The endpoint serving the ownership challenge computed the expected content from whatever token was in the path and returned it, with no database lookup and no host check — so every token verified, for every domain whose DNS pointed at the proxy. It now answers only for a challenge that exists, is unexpired, is unresolved, and whose domain matches the request.",
  },
  {
    title: "Host-based routing did not work over HTTP/2",
    body: "Route selection read only the Host header, and HTTP/2 has none — the authority arrives on the URI instead. The same host-matched route answered 200 over HTTP/1.1 and 404 over HTTP/2. Found by running it, not by reading it.",
  },
  {
    title: "Every parameterised route would panic when first used",
    body: "Twenty-three routes across the admin and multi-tenant APIs used a path syntax the web framework had stopped accepting. Neither router is mounted anywhere, so none had ever been called and nothing had noticed.",
  },
  {
    title: "Route matching honoured half of what you could declare",
    body: "Path-match type, methods and header criteria were all declarable and all ignored, and each silent omission widened a route past what its author wrote: an exact match on /health also caught /health-internal, and a route restricted to GET accepted DELETE.",
  },
  {
    title: "Twelve dependency advisories, five of them in certificate validation",
    body: "An X.509 name-constraint bypass, two PKCS7 validation bypasses, a CRL scope error and a timing side channel — all in the path of a proxy that terminates TLS — plus four in the certificate-chain verifier and unbounded empty frames in the HTTP/2 library.",
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
            A release about the front door. The HTTP proxy was a library with no entry point and,
            it turned out, had never routed a request from a file config. Making it runnable made
            it testable, and testing it found the rest &mdash; a smuggling vector, a framing
            ambiguity, and three transports that were missing rather than broken.
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

          {/* Transports */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">The front door, measured</h2>
            <div class="grid sm:grid-cols-2 gap-4 mb-4">
              <div class="p-5 rounded-lg border border-neutral-200">
                <div class="text-sm text-neutral-500 mb-1">h2spec</div>
                <div class="text-3xl font-bold text-neutral-900 font-mono">
                  {h2spec.passed}/{h2spec.tests}
                </div>
                <div class="text-sm text-neutral-600 mt-1">
                  passed, {h2spec.failed} failed
                </div>
                <div class="text-sm text-neutral-500 mt-2">
                  {h2spec.skipped} skipped, against the HTTP/2-only listener
                </div>
              </div>
              <div class="p-5 rounded-lg border border-neutral-200">
                <div class="text-sm text-neutral-500 mb-1">Autobahn</div>
                <div class="text-3xl font-bold text-neutral-900 font-mono">
                  {autobahn.regressions.length}
                </div>
                <div class="text-sm text-neutral-600 mt-1">
                  cases the tunnel made worse than no tunnel
                </div>
                <div class="text-sm text-neutral-500 mt-2">
                  over {autobahn.cases} cases, {autobahn.ok} of {autobahn.settledCases} OK
                </div>
              </div>
            </div>
            <p class="text-neutral-600 mb-4">
              Three suites, none of them ours. <strong>HTTP Garden</strong>, a differential fuzzer
              that sends a payload through a proxy and shows how a set of origin servers parsed
              what came out, is what found the hop-by-hop defect. <strong>h2spec</strong> speaks
              HTTP/2 at the listener. <strong>Autobahn</strong> is the reference WebSocket suite.
            </p>
            <p class="text-neutral-600 mb-4">
              The Autobahn figure needs its method stated, because the obvious number would be
              misleading. Postrust splices two upgraded byte streams and never parses a WebSocket
              frame, so most of what the suite scores belongs to the origin behind it, not to the
              proxy. Every run therefore has a twin that bypasses the proxy entirely, and the
              figure above is the difference: cases that are worse through the tunnel than without
              it. Of {autobahn.cases} cases, {autobahn.failed} fails &mdash; and fails identically
              with no proxy in the path, so it is the origin&rsquo;s.
            </p>
            <p class="text-neutral-600">
              One family of cases is excluded from that comparison and named on every run rather
              than dropped quietly: those that send a valid message, then an invalid frame, and
              expect the echo of the first. The origin fails the connection without flushing that
              echo when both arrive in one read, and any relay coalesces what a client chopped
              &mdash; measured directly, with no proxy involved, the same bytes sent octet-wise
              echo and sent as one write do not. Which member of the family trips varies between
              runs, so the {autobahn.segmentationSensitive} of them are left out of the OK count
              above rather than making it move between runs. On this run{" "}
              {autobahn.intermittentCount === 0
                ? "none of them was worse than the baseline"
                : `${autobahn.intermittentCount} of them was worse than the baseline`}
              .
            </p>
          </section>

          {/* Alpha */}
          <section class="mb-12">
            <div class="p-5 rounded-lg bg-amber-50 border border-amber-200">
              <h2 class="text-lg font-bold text-neutral-900 mb-2">What the beta means</h2>
              <p class="text-neutral-700 mb-2">
                Everything on the checklist for a stable release is done except the one thing that
                cannot be done from inside the repository: being used by somebody outside it. That
                is what this beta is for.
              </p>
              <p class="text-neutral-700 mb-2">
                Seven crates share this version and will carry a semver promise at 1.0.0.{" "}
                <code class="text-sm">postrust-proxy</code> and{" "}
                <code class="text-sm">postrust-worker</code> are on their own 0.x lines and carry
                none — the proxy because its surface is wide and nobody outside this repository has
                depended on it yet, the worker because it is a stub. Nothing in the stable line
                depends on either.
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
              These affect Rust code that depends on the crates. The HTTP and GraphQL surfaces are
              unaffected, and a proxy configuration that worked before still works &mdash; every
              new field has a default.
            </p>
            <ul class="space-y-2 text-neutral-600">
              <li>
                <strong>The minimum Rust version is 1.88.</strong> It was previously undeclared,
                and the README claimed 1.78. The floor moved as a direct cost of the security
                updates below, which was a trade worth making.
              </li>
              <li>
                <strong>
                  <code class="font-mono text-sm">postrust-proxy</code> and{" "}
                  <code class="font-mono text-sm">postrust-worker</code> left the shared version.
                </strong>{" "}
                They are on their own 0.x lines. Both still release on the same tags; nothing in
                the stable line depends on either.
              </li>
              <li>
                <code class="font-mono text-sm">POST /config/reload</code> is gone. It answered
                &ldquo;Configuration reload requested&rdquo; and reloaded nothing, sending on a
                channel nobody read. Changing configuration needs a restart, which is now what the
                documentation says.
              </li>
              <li>
                A route declaring a path-match type, methods or header criteria now has them
                enforced. They were previously ignored, so such a route matched{" "}
                <em>more</em> traffic than it asked for &mdash; narrowing it is the fix, but it is
                a behaviour change.
              </li>
              <li>
                The proxy&rsquo;s database-access layer and its row types are no longer public.
                They mirror the schema, and publishing them would have frozen it.
              </li>
            </ul>
          </section>

          {/* Gaps */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">Known gaps</h2>
            <p class="text-neutral-600 mb-4">
              In the proxy: <strong>HTTP/3</strong> is not implemented, and neither is upstream
              HTTP/2 over TLS &mdash; a backend can be told to speak h2c in cleartext, but there
              is no ALPN on the upstream leg. There is no response cache and no retry or
              circuit-breaking. One h2spec case, an invalid connection preface answered without
              GOAWAY, fails on the shared HTTP/1.1-and-h2c port and passes on the HTTP/2-only one:
              a port that sniffs its protocol cannot tell a corrupted preface from a malformed
              HTTP/1 request. That is why the dedicated port exists, and why it is optional rather
              than the default.
            </p>
            <p class="text-neutral-600 mb-4">
              Also in the proxy: <code class="font-mono text-sm">retry_count</code> on a route is
              declarable and unread, and there is no configuration reload. Manual certificate
              upload works; automatic issuance is HTTP-01 only, so no wildcards.
            </p>
            <p class="text-neutral-600 mb-4">
              In the dialects, unchanged from the previous alpha: the largest gap is{" "}
              <strong>introspection</strong>, and it is not reachable from here &mdash;
              async-graphql builds its own registry and keeps it private, so the directives it
              installs and the order it lists types in cannot be changed from outside the library.
              Eight of the sixteen remaining Hasura divergences are that one thing.
            </p>
            <p class="text-neutral-600">
              Beside it: <code class="font-mono text-sm">_stream</code> subscriptions, the
              cursor-based half of Hasura&rsquo;s subscription surface; and the OpenAPI document
              PostgREST serves at <code class="font-mono text-sm">/</code>. Actions and Apollo
              federation are subsystems rather than gaps. The{" "}
              <code class="font-mono text-sm">FINDINGS.md</code> files record the rest.
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
  title: `Postrust ${VERSION} — measured, and honest about the rest`,
  links: [{ rel: "canonical", href: "https://postrust.org/releases" }],
  meta: [
    {
      name: "description",
      content:
        "Postrust 1.0.0-beta.1: ACME certificate issuance tested against a real CA, database-backed proxy configuration, a declared and enforced MSRV, and twelve dependency advisories closed.",
    },
  ],
};
