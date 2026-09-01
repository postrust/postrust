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

const VERSION = "1.0.0-alpha.2";

const added = [
  {
    title: "TLS, and with it HTTP/2 as clients actually use it",
    body: "tls.cert_file and tls.key_file start an HTTPS listener that offers h2 and http/1.1 by ALPN and dispatches on what was negotiated rather than sniffing for it. Before this, https_host and https_port were configuration nothing listened on, which left HTTP/2 reachable only in cleartext and WebSocket only as ws://.",
  },
  {
    title: "HTTP/2",
    body: "h2c alongside HTTP/1.1 on the cleartext port, h2 over TLS by ALPN, an optional HTTP/2-only port, and a per-backend upstream protocol — h2c has no ALPN to negotiate with, so a backend that speaks it has to say so. HTTP/2 is per-hop: the version a client arrives with never carries onto the upstream connection.",
  },
  {
    title: "WebSocket, including over HTTP/2",
    body: "Over HTTP/1.1, over TLS as wss://, and over HTTP/2 by extended CONNECT (RFC 8441) translated into an HTTP/1.1 upgrade for the origin. The two handshakes are not the same conversation: HTTP/2 sends no Sec-WebSocket-Key, so the proxy generates one for the origin, and success is 200 rather than a 101 that means nothing on a multiplexed stream.",
  },
  {
    title: "A runnable proxy",
    body: "postrust-proxy <config.toml>. The crate was a library with no entry point, so nothing external could be pointed at it — not a conformance suite, not a load generator, not a browser. Most of what follows was only findable once something could be.",
  },
];

const fixed = [
  {
    title: "A file-configured proxy answered every request with 503",
    body: "Route and upstream registration both guarded on an identifier that only the database path ever sets, so a proxy started from a TOML file logged the routes it had loaded and then matched none of them.",
  },
  {
    title: "Hop-by-hop headers were forwarded to origins",
    body: "Connection is itself hop-by-hop and the headers it names must be removed before forwarding; neither happened. A client could name any header in Connection and have it arrive at the origin unchanged — a request-smuggling and cache-poisoning vector, and the first thing the differential fuzzer found.",
  },
  {
    title: "Content-Length and Transfer-Encoding together were accepted",
    body: "The spec gives Transfer-Encoding precedence, but a proxy that quietly resolves the disagreement is how a smuggling chain starts. Such a request is now refused with 400, which is how the HTTP library already treats a duplicate Content-Length.",
  },
  {
    title: "TCP_NODELAY was set on no socket at all",
    body: "Not the accepted connection, not the upgrade connection, not the pooled upstream connector. Nagle batched small writes and, with delayed acknowledgement at the far end, added latency to every small forwarded response.",
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
              These affect Rust code that depends on the crates. The HTTP and GraphQL surfaces are
              unaffected, and a proxy configuration that worked before still works &mdash; every
              new field has a default.
            </p>
            <ul class="space-y-2 text-neutral-600">
              <li>
                <code class="font-mono text-sm">Backend</code> gained{" "}
                <code class="font-mono text-sm">http_version</code>,{" "}
                <code class="font-mono text-sm">ServerConfig</code> gained{" "}
                <code class="font-mono text-sm">http2_port</code>, and{" "}
                <code class="font-mono text-sm">TlsConfig</code> gained{" "}
                <code class="font-mono text-sm">cert_file</code> and{" "}
                <code class="font-mono text-sm">key_file</code>. None is{" "}
                <code class="font-mono text-sm">#[non_exhaustive]</code> yet, so a struct literal
                downstream needs updating; marking them is still planned during the alpha series.
              </li>
              <li>
                A proxy that relied on <code class="font-mono text-sm">Connection</code> or the
                headers it names reaching the origin will no longer see them. That was the
                vulnerability, not a feature, but it is a behaviour change.
              </li>
              <li>
                A request carrying both <code class="font-mono text-sm">Content-Length</code> and{" "}
                <code class="font-mono text-sm">Transfer-Encoding</code> is now refused rather
                than forwarded.
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
  title: `Postrust ${VERSION} — the front door, measured`,
  links: [{ rel: "canonical", href: "https://postrust.org/releases" }],
  meta: [
    {
      name: "description",
      content:
        "Postrust 1.0.0-alpha.2: TLS with ALPN, HTTP/2 and WebSocket in the proxy, and the hop-by-hop and framing defects found by wiring up three HTTP conformance suites.",
    },
  ],
};
