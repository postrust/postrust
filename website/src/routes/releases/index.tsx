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

const LATEST = "2.3.1";
const VERSION = "2.3.0";

const minorAdded = [
  {
    title: "The Cloudflare Workers adapter works",
    body: "postrust-worker 0.7.0 serves the REST API in a Worker and reaches PostgreSQL through Hyperdrive. Every request runs in a transaction that switches to the token's role and sets its claims, as on the server; the schema cache lives in Workers KV and is refreshed by a Cron Trigger or an authenticated POST /_/reload. It scores 1422 of PostgREST's 1499 conformance cases in workerd. REST only.",
  },
  {
    title: "postrust-engine, the request pipeline with no web framework",
    body: "The REST pipeline came out of the server: handle(&state, request) takes an http::Request and returns an http::Response. The server is an axum wrapper around it, and the Worker runs the same code. Server throughput is unchanged, measured on pinned cores, and the binary is 1% larger.",
  },
  {
    title: "A driver-neutral database interface, and statement pipelining",
    body: "postrust_core::db's Db and Tx traits are implemented by sqlx's pool and by PgSocketDb, which speaks the wire protocol through tokio-postgres over any socket. A differential test suite holds the two to identical answers. Over the socket driver a plain read is one flight to the database after connecting, where it was five, and a write two: from Singapore to a database in Delhi, through Hyperdrive, a read went from 380 to 240 ms.",
  },
];

const patch = [
  {
    title: "A write the response refuses is rolled back",
    body: "Refusals that can only be made after the statement has run — more rows than Prefer: max-affected allowed, several rows where one object was asked for, a PUT whose body names another row, a function setting an invalid response.status or response.headers — were made after the transaction had been committed, so the client was told the request failed while every row stayed changed. PostgREST rolls these back, and now so does Postrust, on either driver.",
  },
  {
    title: "A schema reload no longer deadlocks a request in flight",
    body: "A request held the schema cache's read lock from planning to the end of its query and took it again along the way. A reload (NOTIFY pgrst) queued in between waited on the request while the request waited on the reload, and every request after them queued behind both until the server was restarted. The lock is now released before the query runs.",
  },
  {
    title: "A write refused at COMMIT is reported as what it is",
    body: "A deferred constraint fails at COMMIT, and that was answered 500 Service temporarily unavailable, as if the pool had failed. It is now the database's error with its SQLSTATE: a deferred foreign key violation is a 409, as an immediate one is.",
  },
  {
    title: "A header a pre-request function sets reaches a read",
    body: "db-pre-request can set response headers and status on any request, and PostgREST honours them on a GET — a Cache-Control, say. They were read back only after a function call or a write.",
  },
  {
    title: "__typename inside a relationship or an aggregate",
    body: 'Asking for __typename anywhere below the root — on an object or array relationship, the rows a mutation returned, or inside an aggregate\'s functions such as max { __typename id } — failed the whole query with column "__typename" does not exist; only the root and aggregate nodes answered it. Apollo Client adds __typename to every selection set by default, and a Federation router asks for it on every entity reference it follows, so a query that crossed from one subgraph to another through a relationship failed. It is now answered by the schema, as it is at the root.',
  },
  {
    title: "A GraphQL token that does not verify is refused",
    body: "A token signed with the wrong key, expired, or not a bearer token at all was served as the anonymous role over GraphQL, where REST refuses it with 401. The client kept getting answers, as someone else, with nothing to say why its rows had gone. It is now refused as invalid-jwt, as Hasura refuses it, and a verified token naming no role on a server with no anonymous role as jwt-missing-role-claims. With no JWT secret configured the Authorization header is still ignored, so a router forwarding it to every subgraph is unaffected.",
  },
];

const added = [
  {
    title: "postrust_server::build, for serving the server's router elsewhere",
    body: "Everything between a loaded configuration and a router answering requests — the pool, the schema cache, REST, GraphQL and the layers around them — is a library function, with a Host saying what the host can do beyond answering requests. The binary and the Lambda adapter both serve what it returns. A graphql feature on postrust-server serves GraphQL without the admin UI, which admin-ui still implies.",
  },
  {
    title: "The Lambda adapter is tested where it runs",
    body: "A CI job builds the function with cargo-lambda and runs it in AWS's own provided:al2023 image, which carries the Lambda runtime emulator, against PostgreSQL with row-level security and a NOINHERIT authenticator: filters, writes allowed and refused, per-user rows over REST and GraphQL, functions seeing the token's role and claims, and the table state the writes leave behind.",
  },
];

const fixed = [
  {
    title: "The Lambda adapter serves the server's router",
    body: "It carried a request path of its own, and nothing ran it. That path never bound a query's parameters, so every filter and every write failed with bind message supplies 0 parameters; and it never switched to the token's role or set its claims, so a function connected as a superuser answered every caller with every row, row-level security or not, including callers whose token named a role that does not exist. It now hands the server's router to the Lambda runtime: REST, GraphQL with Federation, JWT, the role switch and the claims behave exactly as they do in the binary.",
  },
  {
    title: "A NOINHERIT authenticator sees the schema",
    body: "The authenticator PostgREST recommends logs in with no privileges of its own and becomes the token's role for each request. The schema cache read information_schema, whose views show a connection only what it can use without switching role — through that authenticator, nothing — so every table was not found, on the server and on Lambda alike. Tables, columns, write permissions, data representations and GraphQL enum tables are now read for the roles the connection can become. Under a superuser or an inheriting authenticator the schema cache is unchanged, compared table by table and column by column over PostgREST's fixture schema.",
  },
  {
    title: "Enum labels and table comments come from the right object",
    body: "Enum labels were looked up by type name alone, so same-named enums in two schemas could swap labels, and PostgREST's own fixture — which declares an enum named bit — had its bit(5) column reported as an enum of one and two. A table's description was looked up by OID alone, so a function that happened to share the table's OID lent it its comment. Both are now looked up on the object itself.",
  },
  {
    title: "The documentation stops promising a Workers adapter",
    body: "The docs and website described Cloudflare Workers support with Hyperdrive that compiles to WebAssembly. postrust-worker is a stub that answers that it is a stub, and does not currently build for WebAssembly; the pages now say so, and what it will take.",
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
              PATCH
            </span>
            <span class="text-sm text-neutral-500">postrust-worker 1.0.0</span>
          </div>
          <h1 class="mb-4 text-4xl font-bold text-neutral-900">
            Postrust {LATEST}
          </h1>
          <p class="mb-4 max-w-2xl text-lg text-neutral-600">
            The{" "}
            <Link
              href="/docs/workers/"
              class="text-primary-600 hover:underline"
            >
              Cloudflare Workers adapter
            </Link>{" "}
            is released as <code class="text-base">postrust-worker</code> 1.0.0,
            and keeps semver from here, on a version of its own. Its promise
            covers what a deployment sees rather than a Rust API: the variables
            it reads, the bindings it looks for, its endpoints and defaults.
            Nothing else changed; the other crates and the images carry{" "}
            {VERSION}&rsquo;s code as {LATEST}.
          </p>
          <p class="max-w-2xl text-lg text-neutral-600">
            {VERSION}: the{" "}
            <Link
              href="/docs/workers/"
              class="text-primary-600 hover:underline"
            >
              Cloudflare Workers adapter
            </Link>{" "}
            serves the REST API over Hyperdrive, on the server&rsquo;s own
            request pipeline. A write the response refuses is rolled back, as
            PostgREST rolls it back. And two GraphQL fixes, found by running two
            Postrust subgraphs behind Apollo Router for{" "}
            <Link
              href="/blog/postgresql-apollo-federation-subgraph/"
              class="text-primary-600 hover:underline"
            >
              a post on Federation
            </Link>
            , first cut as 2.2.1 and released here instead. No breaking change:{" "}
            <code class="text-base">cargo semver-checks</code> against 2.2.0
            passes for every stable crate. 2.2.0, below, is the release that
            made AWS Lambda and a <code class="text-base">NOINHERIT</code>{" "}
            authenticator work.
          </p>
        </div>
      </div>

      <div class="container-wide py-12">
        <div class="max-w-3xl">
          <h2 class="mb-8 text-3xl font-bold text-neutral-900">
            Postrust {VERSION}
          </h2>

          {/* The minor release */}
          <section class="mb-12">
            <h2 class="mb-4 text-2xl font-bold text-neutral-900">
              Added in {VERSION}
            </h2>
            <div class="space-y-4">
              {minorAdded.map((c) => (
                <div key={c.title} class="rounded-lg bg-neutral-50 p-4">
                  <h3 class="mb-1 font-semibold text-neutral-900">{c.title}</h3>
                  <p class="text-sm text-neutral-600">{c.body}</p>
                </div>
              ))}
            </div>
          </section>

          <section class="mb-12">
            <h2 class="mb-4 text-2xl font-bold text-neutral-900">
              Fixed in {VERSION}
            </h2>
            <div class="space-y-4">
              {patch.map((c) => (
                <div key={c.title} class="rounded-lg bg-neutral-50 p-4">
                  <h3 class="mb-1 font-semibold text-neutral-900">{c.title}</h3>
                  <p class="text-sm text-neutral-600">{c.body}</p>
                </div>
              ))}
            </div>
            <p class="mt-4 text-neutral-600">
              <strong>Upgrading:</strong> nothing to change, unless a GraphQL
              client relied on an expired or invalid token being answered
              anonymously; it now gets an{" "}
              <code class="font-mono text-sm">invalid-jwt</code> error and
              should drop the token or refresh it. A{" "}
              <code class="font-mono text-sm">db-pre-request</code> function
              that sets response headers now has them applied to reads too, as
              PostgREST does. The public Rust API gains a crate,{" "}
              <code class="font-mono text-sm">postrust-engine</code>, and{" "}
              <code class="font-mono text-sm">postrust_core::db</code>, and
              loses nothing. Conformance is unchanged: PostgREST 1424/1499,
              Hasura 454/468. The README, docs and site also stop saying what
              nobody measured &mdash; &ldquo;serverless-first&rdquo;,
              &ldquo;5&ndash;20 ms cold starts&rdquo;, a ~5 MB binary.
            </p>
          </section>

          {/* The numbers */}
          <section class="mb-12">
            <h2 class="mb-4 text-2xl font-bold text-neutral-900">
              Measured, not asserted
            </h2>
            <div class="mb-4 grid gap-4 sm:grid-cols-2">
              <Link
                href="/docs/conformance/postgrest/"
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
                href="/docs/conformance/hasura/"
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
              move when the suites are re-run and not otherwise. Both were run
              on this release&rsquo;s code, and score what 2.2.0 scored. Through
              the socket driver the Worker uses, PostgREST&rsquo;s suite agrees
              on every case but one, a function that asks whether it runs as a
              named prepared statement; see{" "}
              <Link
                href="/docs/conformance/postgrest/"
                class="text-primary-600 hover:underline"
              >
                the conformance notes
              </Link>
              .
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

          <h2 class="mb-8 border-t border-neutral-200 pt-12 text-3xl font-bold text-neutral-900">
            Postrust 2.2.0
          </h2>

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
                What 2.2.0 means
              </h2>
              <p class="mb-2 text-neutral-700">
                A minor release. The public Rust API gains{" "}
                <code class="text-sm">postrust_server::build</code>,{" "}
                <code class="text-sm">App</code> and{" "}
                <code class="text-sm">Host</code>, a{" "}
                <code class="text-sm">graphql</code> feature and{" "}
                <code class="text-sm">schema_cache::enum_values</code>, and
                loses nothing: <code class="text-sm">cargo semver-checks</code>{" "}
                against 2.1.0 passes for every stable crate. Everything else
                here is behaviour you see by running Postrust.
              </p>
              <p class="text-neutral-700">
                <code class="text-sm">postrust-proxy</code> and{" "}
                <code class="text-sm">postrust-worker</code> stay at 0.6.0:
                neither changed.
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
              Both came out of checking what the website said about serverless
              before a post repeated it:{" "}
              <a
                href="https://github.com/postrust/postrust/pull/48"
                class="text-primary-600 hover:underline"
              >
                #48
              </a>{" "}
              for Lambda and{" "}
              <a
                href="https://github.com/postrust/postrust/pull/49"
                class="text-primary-600 hover:underline"
              >
                #49
              </a>{" "}
              for the authenticator. How to set the authenticator up is in{" "}
              <Link
                href="/docs/authentication/"
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
              <strong>If you run the server</strong>, nothing is required. If
              you switched your authenticator to{" "}
              <code class="font-mono text-sm">INHERIT</code> to get past
              &ldquo;Could not find the table&rdquo;, you can switch it back:
            </p>
            <div class="mb-4 overflow-hidden rounded-xl bg-neutral-900">
              <div class="border-b border-neutral-700 bg-neutral-800 px-4 py-2">
                <span class="text-sm text-neutral-400">sql</span>
              </div>
              <pre class="overflow-x-auto p-4 text-sm">
                <code class="text-neutral-100">{`CREATE ROLE authenticator LOGIN NOINHERIT PASSWORD '...';
GRANT web_anon, web_user TO authenticator;`}</code>
              </pre>
            </div>
            <p class="mb-4 text-neutral-600">
              <strong>If you run the Lambda adapter</strong>, its paths move to
              where the server has them: tables under{" "}
              <code class="font-mono text-sm">/api</code>, GraphQL at{" "}
              <code class="font-mono text-sm">/v1/graphql</code>. Set{" "}
              <code class="font-mono text-sm">PGRST_COMPAT_MODE=true</code> to
              serve the REST surface at the root as well; the old root paths
              only ever answered unfiltered reads. The pool is two connections
              per sandbox unless{" "}
              <code class="font-mono text-sm">PGRST_DB_POOL_SIZE</code> says
              otherwise, and the function must not connect as a superuser.
            </p>
            <p class="text-neutral-600">
              <strong>If you depend on the crates</strong>, nothing changes;
              there is only more to call.
            </p>
          </section>

          {/* Gaps */}
          <section class="mb-12">
            <h2 class="mb-4 text-2xl font-bold text-neutral-900">Known gaps</h2>
            <p class="mb-4 text-neutral-600">
              On Lambda: no GraphQL subscriptions, since a function cannot hold
              a WebSocket, and no{" "}
              <code class="font-mono text-sm">PGRST_DB_CHANNEL_ENABLED</code>{" "}
              schema reload, since a frozen sandbox cannot listen; a new
              deployment or a cold start loads the schema afresh. Cloudflare
              Workers is not supported yet.
            </p>
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
                <h3 class="mb-1 font-semibold text-neutral-900">2.1.0</h3>
                <p class="mb-2 text-sm text-neutral-600">
                  <strong>
                    Row-level security sees the same identity over GraphQL as
                    over REST, subscriptions included.
                  </strong>{" "}
                  The verified JWT claims reach GraphQL transactions, and a
                  WebSocket is authenticated from its{" "}
                  <code class="font-mono">connection_init</code> payload as an
                  HTTP request is. Subscriptions over a WebSocket had not worked
                  on the server as shipped; they do. Thanks to @CBeardSafire,{" "}
                  <a
                    href="https://github.com/postrust/postrust/pull/45"
                    class="text-primary-600 hover:underline"
                  >
                    #45
                  </a>
                  .
                </p>
                <p class="text-sm text-neutral-600">
                  The documented per-claim policies were rewritten to hold on
                  pooled connections, where a setting a connection once held
                  reads as an empty string rather than NULL.
                </p>
              </div>
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
                href="/docs/authentication/"
                class="text-primary-600 hover:underline"
              >
                the authentication docs
              </Link>
              .
            </p>
          </section>

          <div class="flex items-center justify-between border-t border-neutral-200 pt-8">
            <Link
              href="/docs/getting-started/"
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
              href="/docs/conformance/"
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
  title: `Postrust ${VERSION} — the REST API on Cloudflare Workers, and refused writes rolled back`,
  links: [{ rel: "canonical", href: "https://postrust.org/releases/" }],
  meta: [
    {
      name: "description",
      content: `Postrust ${VERSION}: the REST API in a Cloudflare Worker over Hyperdrive, on the server's own request pipeline; a write the response refuses is rolled back; __typename inside a GraphQL relationship works, and a GraphQL token that does not verify is refused.`,
    },
  ],
};
