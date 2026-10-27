import { component$ } from "@builder.io/qwik";
import type { DocumentHead } from "@builder.io/qwik-city";
import { Link } from "@builder.io/qwik-city";
import { GuidePage, guideHead } from "~/components/guides/guide-page";
import { guideBySlug } from "~/data/guides";
import { benchMeta, variantImages } from "~/data/measured";

const guide = guideBySlug("postgresql-api-server-languages");

const h2 =
  "mb-5 mt-14 text-2xl font-bold tracking-tight text-neutral-950 md:text-[1.75rem]";
const p = "mb-5 text-[1.0625rem] leading-8 text-neutral-700";
const a =
  "font-medium text-primary-700 underline decoration-primary-300 underline-offset-2 hover:decoration-primary-600";
const th = "px-4 py-3 text-left font-medium text-neutral-900 align-bottom";
const td = "px-4 py-3 text-neutral-700 align-top";

/**
 * As of October 2026. Language and licence for PostgREST, Hasura v2,
 * PostGraphile and Supabase come from ~/data/comparisons.ts; the rest link to
 * the repository they were checked against.
 */
const tools: {
  tool: string;
  href: string;
  language: string;
  surface: string;
  ships: string;
  licence: string;
}[] = [
  {
    tool: "PostgREST",
    href: "https://postgrest.org/",
    language: "Haskell",
    surface: "REST",
    ships: "Single binary, or a container image",
    licence: "MIT",
  },
  {
    tool: "Hasura v2",
    href: "https://hasura.io/",
    language: "Haskell",
    surface: "GraphQL",
    ships: "Container",
    licence: "Apache 2.0 (v2 core engine)",
  },
  {
    tool: "Hasura DDN (v3 engine)",
    href: "https://github.com/hasura/graphql-engine/tree/master/v3",
    language: "Rust",
    surface: "GraphQL",
    ships:
      "Engine plus a data connector per source; the PostgreSQL connector is also Rust",
    licence: "Apache 2.0 repository",
  },
  {
    tool: "PostGraphile V5",
    href: "https://postgraphile.org/",
    language: "TypeScript on Node.js",
    surface: "GraphQL",
    ships: "npm package on a Node.js runtime",
    licence: "MIT",
  },
  {
    tool: "pg_graphql",
    href: "https://github.com/supabase/pg_graphql",
    language: "Rust, built with pgrx",
    surface: "GraphQL",
    ships: "PostgreSQL extension, runs inside the database",
    licence: "Apache 2.0",
  },
  {
    tool: "GraphJin",
    href: "https://github.com/dosco/graphjin",
    language: "Go",
    surface: "GraphQL",
    ships: "Go binary or library",
    licence: "Apache 2.0",
  },
  {
    tool: "Supabase",
    href: "https://supabase.com/",
    language: "A platform of services (PostgREST, GoTrue, Realtime)",
    surface: "REST through PostgREST; GraphQL through a Postgres extension",
    ships: "Managed, or self-host the stack",
    licence: "Apache 2.0",
  },
  {
    tool: "Postrust",
    href: "https://github.com/postrust/postrust",
    language: "Rust",
    surface: "REST (PostgREST dialect) and GraphQL (Hasura dialect)",
    ships: "Single static binary, container, AWS Lambda, Cloudflare Workers",
    licence: "MIT",
  },
];

const mb = (kb: number) => `${(kb / 1024).toFixed(1)} MB`;

/** The image and memory figures from the published benchmark variant. */
const footprint = (() => {
  const images = variantImages[benchMeta.variant]?.images ?? {};
  const names: [string, string][] = [
    ["postrust", "Postrust"],
    ["postgrest", "PostgREST v16.1"],
    ["hasura", "Hasura v2.50.1"],
    ["postgraphile", "PostGraphile V5"],
  ];
  return names
    .filter(([key]) => images[key])
    .map(([key, name]) => ({
      name,
      onDisk: images[key].onDisk.replace(/(\d)([MG]B)$/, "$1 $2"),
      idle: mb(images[key].rssIdleKb),
      after: mb(images[key].rssAfterKb),
    }));
})();

export default component$(() => (
  <GuidePage
    guide={guide}
    answer="Most established PostgreSQL API servers are written in Haskell (PostgREST, Hasura v2) or TypeScript on Node.js (PostGraphile); newer ones are written in Rust (Postrust, the Hasura DDN v3 engine, and pg_graphql as a PostgreSQL extension) or Go (GraphJin). For per-request speed the language matters less than how the server turns a request into SQL and how many round trips it makes to the database. The runtime matters most for memory footprint, cold starts and how the server is deployed."
    faq={[
      {
        q: "Which PostgreSQL API servers are written in Rust?",
        a: "Postrust (REST and GraphQL from one binary), the Hasura DDN v3 engine, and pg_graphql, which is a PostgreSQL extension built with pgrx rather than a separate server. Hasura's PostgreSQL data connector for DDN is also written in Rust.",
      },
      {
        q: "What language is PostgREST written in?",
        a: "PostgREST is written in Haskell and released under the MIT licence. It ships as a single binary and as a container image.",
      },
      {
        q: "Is a Rust PostgreSQL API server automatically faster than a Haskell or Node.js one?",
        a: "Not automatically. Most of a request's time is usually spent on the database round trip and in the SQL the server generates, so query design and the number of round trips tend to matter more than the language. The language shows up most clearly in memory use, start-up time and how the server is packaged.",
      },
      {
        q: "Why does the runtime matter for AWS Lambda and Cloudflare Workers?",
        a: "Serverless platforms start new instances on demand and limit memory and CPU per request, so a small binary with no runtime to boot fits them well. Postrust runs on AWS Lambda through a native adapter and on Cloudflare Workers as WebAssembly, with the Worker bundle at 2.7 MB, 0.94 MB gzipped.",
      },
      {
        q: "Does Supabase use a Rust API server?",
        a: "Supabase's REST API is PostgREST, which is written in Haskell. Its GraphQL API comes from pg_graphql, a PostgreSQL extension written in Rust.",
      },
    ]}
  >
    <h2 class={h2}>
      Which languages are the established PostgreSQL API servers written in?
    </h2>
    <p class={p}>
      Haskell, TypeScript on Node.js, Rust and Go. The two longest-established,
      PostgREST and Hasura v2, are Haskell; PostGraphile is TypeScript; of the
      newer tools listed here, three are Rust and one is Go.
    </p>
    <div class="mb-6 overflow-x-auto rounded-xl border border-neutral-200 bg-white">
      <table class="w-full text-sm">
        <thead class="bg-neutral-50">
          <tr class="border-b border-neutral-200">
            <th class={th}>Tool</th>
            <th class={th}>Language / runtime</th>
            <th class={th}>API surface</th>
            <th class={th}>How it ships</th>
            <th class={th}>Licence</th>
          </tr>
        </thead>
        <tbody class="divide-y divide-neutral-200">
          {tools.map((t) => (
            <tr key={t.tool}>
              <td class={`${td} font-medium text-neutral-900`}>
                <a
                  href={t.href}
                  target="_blank"
                  rel="noopener noreferrer"
                  class={a}
                >
                  {t.tool}
                </a>
              </td>
              <td class={td}>{t.language}</td>
              <td class={td}>{t.surface}</td>
              <td class={td}>{t.ships}</td>
              <td class={td}>{t.licence}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
    <p class={p}>
      Versions are the ones the{" "}
      <Link href="/compare/" class={a}>
        comparison pages
      </Link>{" "}
      describe: PostgREST v16.1, Hasura v2.50.1 and PostGraphile V5. Hasura
      recommends DDN for new projects, and it is a different architecture from
      v2 rather than a rewrite of the same server. For how the GraphQL tools
      differ in features rather than language, see{" "}
      <Link href="/guides/postgresql-graphql-tools-compared/" class={a}>
        PostgreSQL GraphQL tools compared
      </Link>
      .
    </p>

    <h2 class={h2}>Which PostgreSQL API tools are written in Rust?</h2>
    <p class={p}>
      Postrust, the Hasura DDN v3 engine and pg_graphql. They use Rust in three
      quite different shapes.
    </p>
    <ul class="mb-5 list-disc space-y-3 pl-5 text-[1.0625rem] leading-8 text-neutral-700">
      <li>
        <strong class="text-neutral-900">Postrust</strong> is a standalone
        server built on Axum and SQLx. One binary serves a REST API in
        PostgREST&rsquo;s dialect and a GraphQL API in Hasura&rsquo;s, and the
        same code runs as a server, in a container, on AWS Lambda and on
        Cloudflare Workers.
      </li>
      <li>
        <strong class="text-neutral-900">Hasura DDN&rsquo;s v3 engine</strong>{" "}
        does not query the database itself. It sends an intermediate query to a
        data connector, and the PostgreSQL connector (
        <a
          href="https://github.com/hasura/ndc-postgres"
          target="_blank"
          rel="noopener noreferrer"
          class={a}
        >
          ndc-postgres
        </a>
        ) is Rust too.
      </li>
      <li>
        <strong class="text-neutral-900">pg_graphql</strong> is not a server at
        all. It is a PostgreSQL extension built with the pgrx framework, so the
        GraphQL query is resolved inside the database and something else has to
        put it on HTTP. On Supabase, that is PostgREST.
      </li>
    </ul>

    <h2 class={h2}>Where does the time in an API request actually go?</h2>
    <p class={p}>
      Mostly into the database: the network round trip to PostgreSQL and the
      query it runs. The server&rsquo;s own work &mdash; parsing the request,
      checking a token, building SQL, writing JSON &mdash; is usually the
      smaller part, whatever language it is written in.
    </p>
    <p class={p}>
      Postrust&rsquo;s{" "}
      <Link href="/docs/workers/" class={a}>
        Cloudflare Workers documentation
      </Link>{" "}
      measures this directly. With the round trip to the database set exactly,
      by delaying every packet the database sends, a read from the Postrust
      server took 1.0 ms with no added delay, 34 ms with 5 ms added, 124 ms with
      20 ms and 364 ms with 60 ms. The server did not get slower; it waited on
      the database once per statement. On Cloudflare through Hyperdrive, a read
      with the Worker and the database both in Singapore took 41 ms, and 255 ms
      with the database in Delhi, about 75 to 80 ms away.
    </p>
    <p class={p}>
      So the decisions that move latency most are architectural: how many
      statements a request needs, whether they are sent together, and what the
      generated SQL asks PostgreSQL to do. Hasura v2 compiles a GraphQL query
      into SQL rather than resolving it field by field, and PostGraphile
      V5&rsquo;s Grafast engine plans a query once and executes it with far
      fewer round trips than a naive resolver tree. A server that issues one
      query where another issues ten will usually win, in any language.
    </p>
    <p class={p}>
      The{" "}
      <Link href="/docs/benchmarks/" class={a}>
        benchmark page
      </Link>{" "}
      makes the same point about its own results. Postrust and PostgREST each
      issue exactly one query per request there, but Postrust sends a plain
      parameterised SELECT and renders the JSON itself, while PostgREST wraps
      the query in a CTE, computes a count and builds the JSON inside PostgreSQL
      on every request. Much of the measured difference is where each design
      does its work, not the speed of the HTTP layer. Connection pooling belongs
      in the same category: Postrust keeps a pool open (default size 10), and a
      server that opened a connection per request would pay for it in round
      trips.
    </p>

    <h2 class={h2}>How much memory and disk does each runtime use?</h2>
    <p class={p}>
      This is where the runtime shows most plainly. Haskell, Node.js and Rust
      programs carry different amounts of runtime with them, and that is visible
      in memory use and container image size.
    </p>
    <div class="mb-4 overflow-x-auto rounded-xl border border-neutral-200 bg-white">
      <table class="w-full text-sm">
        <thead class="bg-neutral-50">
          <tr class="border-b border-neutral-200">
            <th class={th}>Server</th>
            <th class={`${th} text-right`}>Container image, on disk</th>
            <th class={`${th} text-right`}>Memory before the first request</th>
            <th class={`${th} text-right`}>Memory after the benchmark</th>
          </tr>
        </thead>
        <tbody class="divide-y divide-neutral-200">
          {footprint.map((row) => (
            <tr key={row.name}>
              <td class={`${td} font-medium text-neutral-900`}>{row.name}</td>
              <td class={`${td} text-right tabular-nums`}>{row.onDisk}</td>
              <td class={`${td} text-right tabular-nums`}>{row.idle}</td>
              <td class={`${td} text-right tabular-nums`}>{row.after}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
    <p class="mb-5 text-sm text-neutral-500">
      Resident set size, read the same way for every server, from
      Postrust&rsquo;s benchmark harness on {benchMeta.cpu} with{" "}
      {benchMeta.postgres}, Debian-based images. The Alpine build of Postrust is
      a 34.9 MB image. Image size depends heavily on the base image:
      PostgREST&rsquo;s official image is the smallest here.
    </p>
    <p class={p}>
      The Postrust binary itself is 3,070,080 bytes (2.93 MiB) with default
      features and 5,220,864 bytes (4.98 MiB) with the admin UI, as shipped. A
      compiled language with no garbage-collected runtime starts small and stays
      close to what it is actively using; the figures above are one measurement
      of that, on one host, not a capacity plan.
    </p>

    <h2 class={h2}>
      Does the language matter for AWS Lambda and Cloudflare Workers?
    </h2>
    <p class={p}>
      More than anywhere else. Serverless platforms start instances on demand,
      cap memory and CPU per request, and limit what can be uploaded, so a small
      self-contained binary is a better fit than a server that needs a language
      runtime installed and warmed up.
    </p>
    <p class={p}>
      Postrust treats both as first-class{" "}
      <Link href="/docs/deployment/" class={a}>
        deployment targets
      </Link>
      . On AWS Lambda the function runs the server&rsquo;s own router, so REST,
      GraphQL, JWT authentication and row-level security behave identically; the
      schema cache loads once per sandbox, in the init phase, so warm
      invocations do not pay for it. On{" "}
      <Link href="/docs/workers/" class={a}>
        Cloudflare Workers
      </Link>{" "}
      it runs the server&rsquo;s own REST request pipeline compiled to
      WebAssembly, reaching PostgreSQL through Hyperdrive. That bundle is 2.7 MB
      of WebAssembly, 0.94 MB gzipped, within the free plan&rsquo;s 3 MB.
    </p>
    <p class={p}>
      Cold starts are still real, and again the database dominates them. The
      first request in a new Worker isolate took about 440 ms with the database
      in Singapore when the isolate had to read the catalog itself, and about
      230 ms with the schema cache in Workers KV; with the database in Delhi,
      about 2,000 ms against about 400 ms. That first request uses 60 to 250 ms
      of CPU, mostly starting the WebAssembly, against 4 to 11 ms for later
      ones.
    </p>

    <h2 class={h2}>How do the concurrency models differ?</h2>
    <p class={p}>
      All of these servers handle many requests at once without a thread per
      request, so for a database-bound API none of the models is a bottleneck by
      itself.
    </p>
    <p class={p}>
      Haskell&rsquo;s GHC runtime schedules lightweight threads across cores.
      Node.js runs JavaScript on a single-threaded event loop, so CPU-heavy work
      in one request delays the others unless the deployment runs several
      processes. Rust servers such as Postrust use async tasks on a
      multi-threaded runtime (Tokio, under Axum). Go uses goroutines. In every
      case the useful limit is usually the connection pool and the database
      behind it, not the scheduler.
    </p>

    <h2 class={h2}>
      How much should the language weigh when choosing a PostgreSQL API server?
    </h2>
    <p class={p}>
      Less than features, maturity and fit, unless footprint or serverless is a
      hard requirement. PostgREST and Hasura have years of production use behind
      them; PostGraphile is the most customisable; Supabase is a whole platform.
      Choose on those grounds first.
    </p>
    <p class={p}>
      The language becomes the deciding factor when the deployment shape is: a
      small container, many replicas, a function that must start quickly, or an
      edge runtime with a size limit. That is the case Postrust was built for.
      The{" "}
      <Link href="/compare/postgrest/" class={a}>
        PostgREST
      </Link>
      ,{" "}
      <Link href="/compare/hasura/" class={a}>
        Hasura
      </Link>
      ,{" "}
      <Link href="/compare/postgraphile/" class={a}>
        PostGraphile
      </Link>{" "}
      and{" "}
      <Link href="/compare/supabase/" class={a}>
        Supabase
      </Link>{" "}
      comparisons set out where each of those tools is the better choice.
    </p>
  </GuidePage>
));

export const head: DocumentHead = guideHead(guide);
