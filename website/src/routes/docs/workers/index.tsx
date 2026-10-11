import { component$ } from "@builder.io/qwik";
import type { DocumentHead } from "@builder.io/qwik-city";
import { Link } from "@builder.io/qwik-city";

const GUIDE = "https://github.com/postrust/postrust/blob/main/docs/workers.md";

/** What each host offers. Only what has been checked on both is listed. */
const surface: { what: string; server: string; worker: string }[] = [
  { what: "REST API (PostgREST dialect)", server: "Yes", worker: "Yes" },
  { what: "Where REST answers", server: "/api, or the root in compatibility mode", worker: "The root, always" },
  { what: "GraphQL (Hasura dialect)", server: "Yes", worker: "No" },
  { what: "Apollo Federation subgraph", server: "Yes", worker: "No" },
  { what: "GraphQL subscriptions", server: "Yes, over WebSockets", worker: "No" },
  { what: "JWT, roles and row-level security", server: "Yes", worker: "Yes, the same pipeline" },
  { what: "OpenAPI document at the REST root", server: "Yes", worker: "Yes" },
  { what: "Admin UI, Swagger, custom routes", server: "Yes, with the admin-ui feature", worker: "No" },
  { what: "Database connections", server: "A pool, kept open", worker: "One per request, to Hyperdrive" },
  { what: "Schema cache", server: "Loaded at start-up; reloaded on NOTIFY when the channel is enabled", worker: "Kept in Workers KV; reloaded by cron or POST /_/reload" },
  { what: "Row ceiling when PGRST_DB_MAX_ROWS is unset", server: "None", worker: "1000 (unlimited lifts it)" },
  { what: "Database driver", server: "sqlx", worker: "tokio-postgres over a Workers socket" },
];

/** PostgREST's own suite, replayed against each candidate. */
const conformance: { candidate: string; full: string; body: string; note: string }[] = [
  { candidate: "Server (sqlx)", full: "1424 / 1499", body: "1451", note: "The reference figure" },
  {
    candidate: "Server, REST through the Worker's driver",
    full: "1423 / 1499",
    body: "1450",
    note: "Named prepared statements",
  },
  {
    candidate: "Cloudflare Worker in workerd",
    full: "1422 / 1499",
    body: "1449",
    note: "Named prepared statements; a gzip'd empty preflight body",
  },
];

/** Measured side by side on one machine, with the same database and load client. */
const latency: { what: string; one: string; eight: string }[] = [
  { what: "Server, point lookup", one: "1.1 ms", eight: "2,981 req/s" },
  { what: "Worker, point lookup, PostgreSQL asking for SCRAM", one: "54 ms", eight: "22 req/s" },
  { what: "Worker, point lookup, no password authentication", one: "11 ms", eight: "254 req/s" },
  { what: "Worker, 25-row page, no password authentication", one: "11 ms", eight: "239 req/s" },
];

/** What pipelining would save per read, by round-trip time to the database. */
const pipelining: { rtt: string; read: string; write: string }[] = [
  { rtt: "~1 ms (database in the user's region)", read: "~3-4 ms", write: "~4 ms" },
  { rtt: "~20 ms (another region)", read: "~60-80 ms", write: "~80 ms" },
  { rtt: "~60 ms (another continent)", read: "~180-240 ms", write: "~240 ms" },
];

const th = "text-left py-3 px-4 font-semibold text-neutral-900";
const td = "py-3 px-4 text-neutral-600 align-top";
const h2 = "text-2xl font-bold text-neutral-900 mb-4";
const p = "text-neutral-600 mb-4";
const code = "font-mono text-sm";
const a = "text-primary-600 hover:text-primary-700 underline";

export default component$(() => {
  return (
    <div class="min-h-screen bg-white">
      <div class="bg-gradient-to-b from-neutral-50 to-white border-b border-neutral-200">
        <div class="container-wide py-12">
          <div class="flex items-center gap-2 text-sm text-neutral-500 mb-4">
            <Link href="/docs/" class="hover:text-primary-600">
              Docs
            </Link>
            <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 5l7 7-7 7" />
            </svg>
            <span class="text-neutral-900">Cloudflare Workers</span>
          </div>
          <h1 class="text-4xl font-bold text-neutral-900 mb-4">Cloudflare Workers</h1>
          <p class="text-lg text-neutral-600 max-w-2xl">
            The REST API in a Worker, reaching PostgreSQL through Hyperdrive, and every way it
            differs from the server: what it serves, how it answers, what it costs, and what it does
            not do.
          </p>
        </div>
      </div>

      <div class="container-wide py-12">
        <div class="max-w-4xl">
          <section class="mb-12">
            <h2 class={h2}>What is the same</h2>
            <p class={p}>
              The Worker runs the server&rsquo;s own REST request pipeline (
              <code class={code}>postrust-engine</code>), unchanged. Parsing, planning, embedding,
              counts, media types and errors are the same code. Every request runs in a transaction
              that switches to the token&rsquo;s role and sets its claims before anything else, so
              row-level security applies exactly as on the server. Connect through a{" "}
              <code class={code}>NOINHERIT</code> authenticator that holds no privileges of its own,
              as on the server.
            </p>
            <p class={p}>
              Only the database driver differs. It speaks the PostgreSQL wire protocol over a Workers
              socket, and it is tested to answer as the server&rsquo;s does. The same statements go
              through both drivers, and the answers must match exactly: every column type, how
              parameters are typed, and errors down to the SQLSTATE, detail and constraint.
            </p>
          </section>

          <section class="mb-12">
            <h2 class={h2}>What each serves</h2>
            <div class="overflow-x-auto rounded-xl border border-neutral-200">
              <table class="w-full text-sm">
                <thead class="bg-neutral-50">
                  <tr class="border-b border-neutral-200">
                    <th class={th}>&nbsp;</th>
                    <th class={th}>Server</th>
                    <th class={th}>Cloudflare Worker</th>
                  </tr>
                </thead>
                <tbody class="divide-y divide-neutral-100">
                  {surface.map((row) => (
                    <tr key={row.what}>
                      <td class={`${td} font-medium text-neutral-900`}>{row.what}</td>
                      <td class={td}>{row.server}</td>
                      <td class={td}>{row.worker}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
            <p class="text-neutral-500 text-sm mt-3">
              For GraphQL, Federation or subscriptions serverless, use the{" "}
              <Link href="/docs/deployment/" class={a}>
                AWS Lambda adapter
              </Link>
              , which serves REST and GraphQL, Federation included, but not subscriptions.
            </p>
          </section>

          <section class="mb-12">
            <h2 class={h2}>Where the answers differ</h2>
            <p class={p}>
              Asked the same thing, the Worker answers as the server does, with these exceptions:
            </p>
            <ul class="list-disc pl-5 text-neutral-600 space-y-3 mb-4">
              <li>
                <strong class="text-neutral-900">A row ceiling by default.</strong> An isolate has 128
                MB, so a read is capped at 1000 rows unless{" "}
                <code class={code}>PGRST_DB_MAX_ROWS</code> says otherwise. The server applies no
                ceiling unless one is set. <code class={code}>PGRST_DB_MAX_ROWS=unlimited</code> makes
                the Worker match.
              </li>
              <li>
                <strong class="text-neutral-900">The REST API is at the root.</strong>{" "}
                <code class={code}>/todos</code>, not <code class={code}>/api/todos</code>, as
                PostgREST serves it.
              </li>
              <li>
                <strong class="text-neutral-900">No named prepared statements.</strong> The server
                caches prepared statements on its pooled connections. The Worker sends each statement
                unnamed, in one round trip, because its connection never outlives the request. A
                function that inspects <code class={code}>pg_prepared_statements</code> sees the
                difference; nothing else does.
              </li>
              <li>
                <strong class="text-neutral-900">Compression is Cloudflare&rsquo;s.</strong> workerd
                may gzip a response, an empty one included, when the client accepts it. Once decoded,
                the body is the server&rsquo;s.
              </li>
              <li>
                <strong class="text-neutral-900">Key order is a build choice, as on the server.</strong>{" "}
                Object keys come out alphabetically unless the Worker is built with{" "}
                <code class={code}>--features compat-key-order</code>, which keeps select order as
                PostgREST does. The values are identical either way.
              </li>
            </ul>
          </section>

          <section class="mb-12">
            <h2 class={h2}>Conformance</h2>
            <p class={p}>
              PostgREST&rsquo;s own spec suite, replayed over HTTP against PostgREST and each
              candidate in turn, all on identical fixture databases (see{" "}
              <Link href="/docs/conformance/postgrest/" class={a}>
                PostgREST conformance
              </Link>
              ). The Worker ran in workerd, the Workers runtime, through a Hyperdrive binding:
            </p>
            <div class="overflow-x-auto rounded-xl border border-neutral-200 mb-4">
              <table class="w-full text-sm">
                <thead class="bg-neutral-50">
                  <tr class="border-b border-neutral-200">
                    <th class={th}>Candidate</th>
                    <th class={th}>Full contract</th>
                    <th class={th}>Status and body</th>
                    <th class={th}>What differs from the server</th>
                  </tr>
                </thead>
                <tbody class="divide-y divide-neutral-100">
                  {conformance.map((row) => (
                    <tr key={row.candidate}>
                      <td class={`${td} font-medium text-neutral-900`}>{row.candidate}</td>
                      <td class={`${td} font-mono`}>{row.full}</td>
                      <td class={`${td} font-mono`}>{row.body}</td>
                      <td class={td}>{row.note}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
            <p class={p}>
              The Worker&rsquo;s first run scored 1405, and the gap was a real bug. When a write was
              refused for its token, its request body was never read. The unread stream then threw
              after the response had gone, which restarted the isolate and failed whatever request
              it served next. The Worker now reads the body itself first. Its test suite sends
              exactly that sequence, and also checks the Worker&rsquo;s log for uncaught exceptions.
            </p>
          </section>

          <section class="mb-12">
            <h2 class={h2}>What it costs</h2>
            <p class={p}>
              Measured on one machine: the server, and the Worker under{" "}
              <code class={code}>wrangler dev</code>, against the same PostgreSQL with 100,000 rows,
              with the same requests and the same keep-alive load client. Local workerd is not
              Cloudflare&rsquo;s edge, and there is no real Hyperdrive in it. These figures show
              what the Worker&rsquo;s design costs, not production latency.
            </p>
            <div class="overflow-x-auto rounded-xl border border-neutral-200 mb-4">
              <table class="w-full text-sm">
                <thead class="bg-neutral-50">
                  <tr class="border-b border-neutral-200">
                    <th class={th}>&nbsp;</th>
                    <th class={th}>One at a time, p50</th>
                    <th class={th}>8 concurrent</th>
                  </tr>
                </thead>
                <tbody class="divide-y divide-neutral-100">
                  {latency.map((row) => (
                    <tr key={row.what}>
                      <td class={`${td} font-medium text-neutral-900`}>{row.what}</td>
                      <td class={`${td} font-mono`}>{row.one}</td>
                      <td class={`${td} font-mono`}>{row.eight}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
            <ul class="list-disc pl-5 text-neutral-600 space-y-3 mb-4">
              <li>
                <strong class="text-neutral-900">The authentication handshake is most of it.</strong>{" "}
                Every request opens a connection. With <code class={code}>scram-sha-256</code>, the
                client derives its key with 4,096 rounds of PBKDF2-SHA-256 in WebAssembly, about 43
                ms of CPU per request locally. Whether Hyperdrive asks a Worker for SCRAM can only be
                measured on a real deployment. Measure it against your plan&rsquo;s CPU-time limit
                before relying on these numbers.
              </li>
              <li>
                <strong class="text-neutral-900">Without SCRAM, about 11 ms a request.</strong> That
                covers WebAssembly, a fresh connection, and the statements a request sends one after
                another (below).
              </li>
              <li>
                <strong class="text-neutral-900">The bundle</strong> is 2.7 MB of WebAssembly, 0.93
                MB gzipped, within the free plan&rsquo;s 3 MB.
              </li>
              <li>
                <strong class="text-neutral-900">The server is unaffected.</strong> Separating the
                pipeline from its driver was measured against 2.2.1 on pinned cores, six alternating
                rounds per build: throughput within the noise of the host on every scenario, and the
                binary 1.0% larger.
              </li>
            </ul>
          </section>

          <section class="mb-12">
            <h2 class={h2}>Round trips, and statement pipelining</h2>
            <p class={p}>
              Each request sends its statements one at a time and waits for each reply before the
              next. On the server that wait is a local pool, measured in microseconds. From a
              Worker, each one is a round trip to the database through Hyperdrive.
            </p>
            <ul class="list-disc pl-5 text-neutral-600 space-y-2 mb-4">
              <li>
                <strong class="text-neutral-900">A read: five.</strong>{" "}
                <code class={code}>BEGIN</code>, <code class={code}>SET TRANSACTION READ ONLY</code>,
                the settings (role, claims, request headers), the query,{" "}
                <code class={code}>COMMIT</code>.
              </li>
              <li>
                <strong class="text-neutral-900">A write or function call: five or six.</strong>{" "}
                <code class={code}>BEGIN</code>, the settings, the statement, reading back the
                response headers and status, <code class={code}>COMMIT</code>.
              </li>
              <li>
                <strong class="text-neutral-900">One more each</strong> for a non-default isolation
                level, <code class={code}>db-pre-request</code>, or{" "}
                <code class={code}>Prefer: count</code>.
              </li>
            </ul>
            <p class={p}>
              <strong class="text-neutral-900">Pipelining is planned, not done.</strong> It would send
              the same statements, in the same order, without waiting between them:
            </p>
            <ul class="list-disc pl-5 text-neutral-600 space-y-2 mb-4">
              <li>
                <strong class="text-neutral-900">Reads</strong> would take two flights, or one, since
                a read-only transaction has nothing to commit.
              </li>
              <li>
                <strong class="text-neutral-900">Writes</strong> would take two. The commit still has
                to wait for the result to be checked, because a singular response that matched
                several rows must roll back.
              </li>
            </ul>
            <p class={p}>
              What that saves depends only on the distance to the database. These are projected
              savings, not measurements:
            </p>
            <div class="overflow-x-auto rounded-xl border border-neutral-200 mb-4">
              <table class="w-full text-sm">
                <thead class="bg-neutral-50">
                  <tr class="border-b border-neutral-200">
                    <th class={th}>Round trip to the database</th>
                    <th class={th}>Saved per read</th>
                    <th class={th}>Saved per write</th>
                  </tr>
                </thead>
                <tbody class="divide-y divide-neutral-100">
                  {pipelining.map((row) => (
                    <tr key={row.rtt}>
                      <td class={`${td} font-medium text-neutral-900`}>{row.rtt}</td>
                      <td class={`${td} font-mono`}>{row.read}</td>
                      <td class={`${td} font-mono`}>{row.write}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
            <p class={p}>
              Pipelining does not shorten the connection handshake, which every request still pays.
              It is tracked in{" "}
              <a
                href="https://github.com/postrust/postrust/issues/50"
                target="_blank"
                rel="noopener noreferrer"
                class={a}
              >
                issue #50
              </a>
              .
            </p>
          </section>

          <section class="mb-12">
            <h2 class={h2}>Settings only the Worker has</h2>
            <ul class="list-disc pl-5 text-neutral-600 space-y-3 mb-4">
              <li>
                <code class={code}>PGRST_WORKER_SCHEMA_TTL</code> (default 60): how many seconds an
                isolate trusts its copy of the schema cache before checking KV for a newer one,
                and how long the KV copy lives (at least 60 seconds, KV&rsquo;s minimum). Once it
                expires, the next isolate to miss reloads the catalog, so a migration is seen
                without a reload.
              </li>
              <li>
                <code class={code}>POSTRUST_RELOAD_SECRET</code>, a secret: enables{" "}
                <code class={code}>POST /_/reload</code>, which reloads the schema cache from the
                catalog and writes it to KV for every isolate.
              </li>
              <li>
                The <code class={code}>POSTRUST_SCHEMA_CACHE</code> KV binding: optional. Without it,
                each new isolate reads the catalog itself, one query per table for its columns,
                and again every TTL.
              </li>
            </ul>
            <p class={p}>
              Everything else is the server&rsquo;s own variables, set as Worker variables or
              secrets (see{" "}
              <Link href="/docs/configuration/" class={a}>
                Configuration
              </Link>
              ). Settings that only a long-running server can use are ignored: the pool size, the
              listen address, <code class={code}>PGRST_DB_CHANNEL</code>.
            </p>
          </section>

          <section class="mb-12">
            <h2 class={h2}>Deploying</h2>
            <p class={p}>
              Create a Hyperdrive configuration and a KV namespace, copy{" "}
              <code class={code}>crates/postrust-worker/wrangler.example.toml</code> to{" "}
              <code class={code}>wrangler.toml</code>, put the JWT secret in with{" "}
              <code class={code}>wrangler secret put</code>, and run{" "}
              <code class={code}>wrangler deploy</code>. The full guide, with the database role to
              create, is{" "}
              <a href={GUIDE} target="_blank" rel="noopener noreferrer" class={a}>
                docs/workers.md
              </a>
              .
            </p>
            <p class={p}>
              <code class={code}>postrust-worker</code> is on its own <code class={code}>0.x</code>{" "}
              line. The request pipeline underneath it is the stable one; what is not settled yet is
              the Worker&rsquo;s own surface: the KV record, the reload endpoint, its defaults.
            </p>
          </section>

          <div class="flex flex-wrap gap-6 pt-8 border-t border-neutral-200">
            <Link href="/docs/deployment/" class="flex items-center gap-2 text-neutral-600 hover:text-primary-600">
              Deployment
              <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 5l7 7-7 7" />
              </svg>
            </Link>
            <Link href="/docs/conformance/postgrest/" class="flex items-center gap-2 text-neutral-600 hover:text-primary-600">
              PostgREST conformance
              <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 5l7 7-7 7" />
              </svg>
            </Link>
          </div>
        </div>
      </div>
    </div>
  );
});

export const head: DocumentHead = {
  title: "Cloudflare Workers - Postrust Documentation",
  meta: [
    {
      name: "description",
      content:
        "Postrust's REST API in a Cloudflare Worker over Hyperdrive: what it serves, where its answers differ from the server, conformance, measured costs and what it does not do.",
    },
  ],
};
