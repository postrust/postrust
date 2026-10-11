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

/** On Cloudflare through a real Hyperdrive: each request's wall time inside the Worker. */
const cloudflare: { where: string; read: string; write: string }[] = [
  { where: "Singapore, database in Singapore", read: "41 ms", write: "45 ms" },
  { where: "Singapore, database in Delhi (80 ms away)", read: "255 ms", write: "318 ms" },
  { where: "The same, Worker placed in Delhi", read: "170 ms", write: "190 ms" },
  { where: "London, database in Delhi", read: "428 ms", write: "574 ms" },
  { where: "Amsterdam, database in Delhi", read: "586 ms", write: "813 ms" },
];

/** Statement pipelining, before and after, on Cloudflare. */
const pipelined: { where: string; rb: string; ra: string; wb: string; wa: string }[] = [
  { where: "Singapore, database in Singapore", rb: "69 ms", ra: "48 ms", wb: "60 ms", wa: "53 ms" },
  { where: "Singapore, database in Delhi", rb: "380 ms", ra: "240 ms", wb: "424 ms", wa: "306 ms" },
  { where: "London, database in Delhi", rb: "869 ms", ra: "428 ms", wb: "836 ms", wa: "574 ms" },
  { where: "Amsterdam, database in Delhi", rb: "1283 ms", ra: "586 ms", wb: "1266 ms", wa: "813 ms" },
];

/** The round trip set exactly with tc netem, in local workerd, beside the server. */
const netem: { rtt: string; rb: string; ra: string; wb: string; wa: string; server: string }[] = [
  { rtt: "0 ms", rb: "11.5 ms", ra: "10.4 ms", wb: "11.5 ms", wa: "10.7 ms", server: "1.0 ms" },
  { rtt: "5 ms", rb: "49 ms", ra: "26 ms", wb: "50 ms", wa: "32 ms", server: "34 ms" },
  { rtt: "20 ms", rb: "155 ms", ra: "72 ms", wb: "156 ms", wa: "93 ms", server: "124 ms" },
  { rtt: "60 ms", rb: "436 ms", ra: "192 ms", wb: "436 ms", wa: "253 ms", server: "364 ms" },
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
              Measured on Cloudflare through a real Hyperdrive, with its query caching off so every
              request reached the database, from a client in Singapore. Each figure is the
              request&rsquo;s wall time inside the Worker, as <code class={code}>wrangler tail</code>{" "}
              reports it, so the client&rsquo;s own distance to Cloudflare is not in it: the median
              of 64 to 150 point lookups and one-row inserts on the benchmark dataset.
            </p>
            <div class="overflow-x-auto rounded-xl border border-neutral-200 mb-4">
              <table class="w-full text-sm">
                <thead class="bg-neutral-50">
                  <tr class="border-b border-neutral-200">
                    <th class={th}>Where the Worker ran</th>
                    <th class={th}>Read</th>
                    <th class={th}>Write</th>
                  </tr>
                </thead>
                <tbody class="divide-y divide-neutral-100">
                  {cloudflare.map((row) => (
                    <tr key={row.where}>
                      <td class={`${td} font-medium text-neutral-900`}>{row.where}</td>
                      <td class={`${td} font-mono`}>{row.read}</td>
                      <td class={`${td} font-mono`}>{row.write}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
            <ul class="list-disc pl-5 text-neutral-600 space-y-3 mb-4">
              <li>
                <strong class="text-neutral-900">The distance to the database is the cost.</strong>{" "}
                Connecting to Hyperdrive takes 3 to 6 ms, because Hyperdrive answers the connection
                near the Worker. Every wait on a statement is a round trip to the database: about 75
                to 80 ms from Singapore to Delhi.
              </li>
              <li>
                <strong class="text-neutral-900">
                  Put the Worker near the database when it is far from your users.
                </strong>{" "}
                A request waits on the database several times and on the client once.{" "}
                <code class={code}>[placement]</code> in <code class={code}>wrangler.toml</code>, a
                region or <code class={code}>mode = "smart"</code>, moves it: placed in Delhi, a read
                from Singapore went from 255 ms to 170 ms.
              </li>
              <li>
                <strong class="text-neutral-900">CPU is 4 to 11 ms a request.</strong> Hyperdrive
                does not ask the Worker for a SCRAM exchange, which costs about 43 ms of CPU against a
                PostgreSQL in local workerd. Workers Free allows 10 ms of CPU a request, so production
                belongs on Workers Paid.
              </li>
              <li>
                <strong class="text-neutral-900">The bundle</strong> is 2.7 MB of WebAssembly, 0.94
                MB gzipped, within the free plan&rsquo;s 3 MB.
              </li>
              <li>
                <strong class="text-neutral-900">The server is unaffected.</strong> Separating the
                pipeline from its driver was measured against the tree it started from on pinned
                cores, six alternating rounds per build: throughput within the noise of the host on
                every scenario, and the binary 1.0% larger.
              </li>
            </ul>
          </section>

          <section class="mb-12">
            <h2 class={h2}>Statement pipelining</h2>
            <p class={p}>
              A request sends its statements in as few flights as it can. A plain read sends{" "}
              <code class={code}>BEGIN</code>, <code class={code}>SET TRANSACTION READ ONLY</code>,
              the settings (role, claims, request headers), the query and{" "}
              <code class={code}>COMMIT</code> in one &mdash; a <code class={code}>GET</code> to a
              function too. A read with a count, or a media type the schema renders, takes a flight or
              two more. A write or a <code class={code}>POST</code> to a function sends everything up
              to reading back its response settings in one, and <code class={code}>COMMIT</code> in a
              second, once the response is known to be one it can send. Each statement keeps its own
              sync point, so a failure is reported as if they had been sent one at a time.
            </p>
            <p class={p}>
              Before and after were measured as an A/B pair on one hostname, in a run of their own,
              so an &ldquo;after&rdquo; figure need not match the table above: from Singapore it
              reads 240 and 306 ms here against 255 and 318 ms there, and 48 and 53 ms against 41
              and 45 ms in the database&rsquo;s own region. That is the spread between runs.
            </p>
            <div class="overflow-x-auto rounded-xl border border-neutral-200 mb-4">
              <table class="w-full text-sm">
                <thead class="bg-neutral-50">
                  <tr class="border-b border-neutral-200">
                    <th class={th}>On Cloudflare</th>
                    <th class={th}>Read, before</th>
                    <th class={th}>Read, after</th>
                    <th class={th}>Write, before</th>
                    <th class={th}>Write, after</th>
                  </tr>
                </thead>
                <tbody class="divide-y divide-neutral-100">
                  {pipelined.map((row) => (
                    <tr key={row.where}>
                      <td class={`${td} font-medium text-neutral-900`}>{row.where}</td>
                      <td class={`${td} font-mono`}>{row.rb}</td>
                      <td class={`${td} font-mono`}>{row.ra}</td>
                      <td class={`${td} font-mono`}>{row.wb}</td>
                      <td class={`${td} font-mono`}>{row.wa}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
            <p class={p}>
              Hyperdrive does not pass a pipeline on to the database whole: from Singapore, five
              statements sent together took about 218 ms against 78 ms for one. With the database in
              another region a read still came out 37&ndash;54% faster and a write 28&ndash;36%
              faster; in its own region, 30% for a read and 12% for a write.
            </p>
            <p class={p}>
              With the round trip set exactly, by delaying every packet the database sends, in local
              workerd beside the server:
            </p>
            <div class="overflow-x-auto rounded-xl border border-neutral-200 mb-4">
              <table class="w-full text-sm">
                <thead class="bg-neutral-50">
                  <tr class="border-b border-neutral-200">
                    <th class={th}>Added round trip</th>
                    <th class={th}>Read, before</th>
                    <th class={th}>Read, after</th>
                    <th class={th}>Write, before</th>
                    <th class={th}>Write, after</th>
                    <th class={th}>Server</th>
                  </tr>
                </thead>
                <tbody class="divide-y divide-neutral-100">
                  {netem.map((row) => (
                    <tr key={row.rtt}>
                      <td class={`${td} font-medium text-neutral-900`}>{row.rtt}</td>
                      <td class={`${td} font-mono`}>{row.rb}</td>
                      <td class={`${td} font-mono`}>{row.ra}</td>
                      <td class={`${td} font-mono`}>{row.wb}</td>
                      <td class={`${td} font-mono`}>{row.wa}</td>
                      <td class={`${td} font-mono`}>{row.server}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
            <p class={p}>
              Over a plain connection the pipeline is whole: a plain read is three round trips (two to
              connect, one flight) where it was seven. Past a few milliseconds of distance the Worker
              answers sooner than the server, whose driver waits on every statement.
            </p>
          </section>

          <section class="mb-12">
            <h2 class={h2}>Cold starts</h2>
            <p class={p}>
              The first request in a new isolate, on Cloudflare, timed inside the Worker: a handful
              of trials each from Singapore, so these are sizes rather than precise figures.
            </p>
            <div class="overflow-x-auto rounded-xl border border-neutral-200 mb-4">
              <table class="w-full text-sm">
                <thead class="bg-neutral-50">
                  <tr class="border-b border-neutral-200">
                    <th class={th}>&nbsp;</th>
                    <th class={th}>Without KV</th>
                    <th class={th}>Schema cache in KV</th>
                  </tr>
                </thead>
                <tbody class="divide-y divide-neutral-100">
                  <tr>
                    <td class={`${td} font-medium text-neutral-900`}>Database in Singapore</td>
                    <td class={`${td} font-mono`}>~440 ms</td>
                    <td class={`${td} font-mono`}>~230 ms</td>
                  </tr>
                  <tr>
                    <td class={`${td} font-medium text-neutral-900`}>Database in Delhi</td>
                    <td class={`${td} font-mono`}>~2,000 ms</td>
                    <td class={`${td} font-mono`}>~400 ms</td>
                  </tr>
                </tbody>
              </table>
            </div>
            <ul class="list-disc pl-5 text-neutral-600 space-y-3 mb-4">
              <li>
                <strong class="text-neutral-900">Keep the KV binding.</strong> Without it, a new
                isolate reads the catalog itself, one query per table, each a round trip to the
                database. With it, the isolate reads one KV value.
              </li>
              <li>
                <strong class="text-neutral-900">
                  An isolate&rsquo;s first request uses 60 to 250 ms of CPU
                </strong>
                , against 4 to 11 ms after it, mostly starting the WebAssembly. Workers Free allows 10
                ms a request.
              </li>
              <li>
                <strong class="text-neutral-900">A reload spreads at KV&rsquo;s pace.</strong> KV is
                eventually consistent, and a location can serve the previous value for a minute or
                more after a write.
              </li>
            </ul>
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
