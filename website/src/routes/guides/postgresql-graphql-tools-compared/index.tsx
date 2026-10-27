import { component$ } from "@builder.io/qwik";
import type { DocumentHead } from "@builder.io/qwik-city";
import { Link } from "@builder.io/qwik-city";
import { GuidePage, guideHead } from "~/components/guides/guide-page";
import { guideBySlug } from "~/data/guides";
import { LATEST } from "~/data/version";
import { conformance, conformanceMeta } from "~/data/conformance";
import {
  hasuraConformance,
  hasuraConformanceMeta,
} from "~/data/hasura-conformance";

const guide = guideBySlug("postgresql-graphql-tools-compared");

const h2 =
  "mb-5 mt-16 text-2xl font-bold tracking-tight text-neutral-950 md:text-[1.75rem]";
const h3 = "mb-3 mt-8 text-lg font-semibold text-neutral-900";
const p = "mb-5 text-[1.0625rem] leading-8 text-neutral-700";
const a =
  "font-medium text-primary-700 underline decoration-primary-300 underline-offset-2 hover:decoration-primary-600";
const ul =
  "marker:text-primary-500 mb-5 list-disc space-y-2 pl-6 text-[1.0625rem] leading-8 text-neutral-700";

const tools = [
  "Postrust",
  "Hasura v2",
  "PostGraphile V5",
  "Supabase (pg_graphql)",
  "PostgREST",
] as const;

/**
 * As of October 2026. Every cell restates src/data/comparisons.ts, the
 * /compare/ index table, or the Federation post's sourced "others" table.
 */
const rows: {
  feature: string;
  cells: [string, string, string, string, string];
}[] = [
  {
    feature: "Written in",
    cells: [
      "Rust",
      "Haskell",
      "TypeScript / Node.js",
      "A platform (PostgREST, GoTrue, Realtime)",
      "Haskell",
    ],
  },
  {
    feature: "Licence",
    cells: ["MIT", "Apache 2.0 (v2 core engine)", "MIT", "Apache 2.0", "MIT"],
  },
  {
    feature: "GraphQL",
    cells: [
      "Built in, Hasura's dialect",
      "Built in, its own dialect",
      "Built in",
      "Via a Postgres extension",
      "Not supported",
    ],
  },
  {
    feature: "REST alongside GraphQL",
    cells: [
      "Yes, PostgREST-compatible",
      "RESTified GraphQL endpoints",
      "Not supported",
      "Yes, PostgREST",
      "REST only",
    ],
  },
  {
    feature: "Permissions",
    cells: [
      "PostgreSQL roles and RLS, or Hasura's model from a JSON document",
      "Metadata, per role and per field",
      "PostgreSQL roles and RLS",
      "PostgreSQL roles and RLS",
      "PostgreSQL roles and RLS",
    ],
  },
  {
    feature: "Setup before first query",
    cells: [
      "None",
      "Tables must be tracked and permissions declared",
      "None",
      "None",
      "None",
    ],
  },
  {
    feature: "Subscriptions",
    cells: [
      "Live queries over LISTEN/NOTIFY; no _stream",
      "Yes, a core feature, including _stream",
      "Built in over LISTEN/NOTIFY; you define the fields",
      "Realtime service",
      "Not supported",
    ],
  },
  {
    feature: "Apollo Federation",
    cells: [
      "Opt-in v2 subgraph; @key is the primary key",
      "Subgraph, Federation v1 only",
      "No maintained support found for V5",
      "No; not planned",
      "Not applicable (no GraphQL)",
    ],
  },
  {
    feature: "Schema customisation",
    cells: ["No", "Metadata", "Plugin system", "No", "No"],
  },
  {
    feature: "Deployment shape",
    cells: [
      "Single static binary; native AWS Lambda adapter",
      "Container",
      "Node.js",
      "Managed platform, or the self-hosted stack",
      "Single binary",
    ],
  },
  {
    feature: "Self-host or managed",
    cells: [
      "You run it, anywhere",
      "Self-host (v2 open source)",
      "Self-host",
      "Managed, or self-host the stack",
      "Self-host",
    ],
  },
  {
    feature: "Version described",
    cells: [LATEST, "v2.50.1", "V5", "Hosted platform", "v16.1"],
  },
];

export default component$(() => (
  <GuidePage
    guide={guide}
    answer="As of October 2026, the main tools that generate a GraphQL API from PostgreSQL are Hasura (a platform with permissions kept in its own metadata), PostGraphile V5 (Node.js, built to have its schema reshaped by plugins), Supabase's pg_graphql (an extension inside the Supabase platform) and Postrust (one Rust binary serving Hasura-dialect GraphQL and PostgREST-style REST over the same tables, with permissions in PostgreSQL). PostgREST is often listed beside them but serves REST only. Choose by where you want permissions to live, whether you need REST as well, whether you will reshape the schema, and whether you want a platform or a server."
    faq={[
      {
        q: "Does PostgREST support GraphQL?",
        a: "No. PostgREST serves a REST API only. Supabase adds GraphQL through the pg_graphql Postgres extension, and Postrust serves a PostgREST-compatible REST API and a Hasura-dialect GraphQL API from one process.",
      },
      {
        q: "Which PostgreSQL GraphQL tools keep permissions in the database?",
        a: "PostGraphile, Supabase and Postrust rely on PostgreSQL roles and row-level security, as PostgREST does for REST. Hasura models permissions as metadata, per role and per field; Postrust can also read Hasura's permission model from a JSON document.",
      },
      {
        q: "Can I point a Hasura client at Postrust?",
        a: "Postrust speaks Hasura's GraphQL dialect at /v1/graphql, and how closely it agrees with Hasura v2.50.1 is measured by replaying Hasura's own test corpus against both servers. Introspection is the largest known gap, and _stream subscriptions are not implemented.",
      },
      {
        q: "Which of these tools can be an Apollo Federation subgraph?",
        a: "As of October 2026: Postrust as an opt-in Federation v2 subgraph, Hasura v2 as a Federation v1 subgraph, and Hasura DDN, which documents subgraph compliance without naming a version. Supabase pg_graphql has no Federation support and PostGraphile has no maintained support for V5.",
      },
      {
        q: "Is Hasura v2 or Hasura DDN the one to compare?",
        a: "This comparison describes the self-hostable v2 engine, which runs as a single container against one PostgreSQL. Hasura recommends v3 / DDN for new projects, and it is a different architecture and product.",
      },
      {
        q: "Which is fastest?",
        a: "This site does not currently publish throughput comparisons: the comparison benchmark is being re-measured because its results did not reproduce reliably. The method and caveats are on the benchmarks page.",
      },
    ]}
  >
    <div class="mb-8 overflow-x-auto rounded-xl border border-neutral-200">
      <table class="w-full text-sm">
        <thead class="bg-neutral-50">
          <tr>
            <th class="px-4 py-3 text-left font-medium text-neutral-900">
              As of October 2026
            </th>
            {tools.map((t) => (
              <th
                key={t}
                class={`px-4 py-3 text-left font-medium ${t === "Postrust" ? "text-primary-600" : "text-neutral-900"}`}
              >
                {t}
              </th>
            ))}
          </tr>
        </thead>
        <tbody class="divide-y divide-neutral-200">
          {rows.map((row) => (
            <tr key={row.feature} class="align-top">
              <td class="px-4 py-3 font-medium text-neutral-900">
                {row.feature}
              </td>
              {row.cells.map((cell, i) => (
                <td key={tools[i]} class="min-w-36 px-4 py-3 text-neutral-700">
                  {cell}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
    <p class={p}>
      Where a capability has a condition attached, the condition is in the cell.
      We make Postrust, so read its column with that in mind; each tool has a
      detailed page &mdash;{" "}
      <Link href="/compare/hasura/" class={a}>
        Hasura
      </Link>
      ,{" "}
      <Link href="/compare/postgraphile/" class={a}>
        PostGraphile
      </Link>
      ,{" "}
      <Link href="/compare/supabase/" class={a}>
        Supabase
      </Link>{" "}
      and{" "}
      <Link href="/compare/postgrest/" class={a}>
        PostgREST
      </Link>{" "}
      &mdash; including where each is the better choice.
    </p>

    <h2 class={h2}>Which of these tools actually serve GraphQL?</h2>
    <p class={p}>
      Four of the five do. PostgREST serves REST only, and appears here because
      it is often named in the same breath and because two of the others build
      on its URL grammar.
    </p>
    <p class={p}>
      Hasura, PostGraphile and Postrust are servers that read the schema and
      serve GraphQL themselves. Supabase is a platform &mdash; Postgres, auth,
      storage, realtime and a dashboard &mdash; whose GraphQL comes from the
      pg_graphql extension inside the database and whose REST API is PostgREST.
      Choosing Supabase is therefore less a choice of GraphQL generator than a
      choice of platform.
    </p>
    <p class={p}>
      The GraphQL dialects are not interchangeable. Postrust deliberately speaks
      Hasura&rsquo;s: a client generated against Hasura &mdash; its queries, its
      codegen output, its endpoint at <code>/v1/graphql</code> &mdash; points at
      Postrust unchanged. PostGraphile inflects names its own way and supports
      Relay conventions, which Postrust does not.
    </p>

    <h2 class={h2}>
      Where do permissions live: in the database or in metadata?
    </h2>
    <p class={p}>
      Hasura keeps permissions in its own metadata, per role and per field.
      PostGraphile, Supabase, PostgREST and Postrust leave them to PostgreSQL
      roles and row-level security, so every other client of the database is
      subject to the same rules.
    </p>
    <p class={p}>
      The trade-off is real in both directions. Metadata permissions give Hasura
      a console to manage them and features such as inherited roles and
      remote-schema permissions. Database permissions mean there is no second
      copy of the rules to keep in sync, and a tool reading the schema needs no
      setup before the first query: what the connecting role is granted is what
      the API exposes. Hasura instead requires tables to be tracked and
      permissions declared.
    </p>
    <p class={p}>
      Postrust supports both. It reads PostgreSQL roles and RLS, and it can also
      read Hasura&rsquo;s permission model &mdash; a schema per role, row
      filters, separate read and write columns, presets and{" "}
      <code>backend_only</code> &mdash; from a JSON document you supply. It does
      not reach inherited roles or remote-schema permissions.
    </p>

    <h2 class={h2}>Which tools serve REST and GraphQL from the same tables?</h2>
    <p class={p}>
      Postrust serves both from one process: a PostgREST-compatible REST API and
      a Hasura-dialect GraphQL API. Supabase offers both as separate services of
      one platform; Hasura offers RESTified GraphQL endpoints; PostGraphile has
      no REST surface; PostgREST has no GraphQL.
    </p>
    <p class={p}>
      For a SaaS team this matters when some clients want REST (webhooks,
      partners, simple scripts) and the front end wants GraphQL. Running
      PostgREST plus a GraphQL server is two deployments that must agree on
      permissions; RLS makes that agreement automatic, but the operations cost
      remains.
    </p>

    <h2 class={h2}>
      Which can be an Apollo Federation subgraph in October 2026?
    </h2>
    <p class={p}>
      Postrust and both Hasura products can; Supabase pg_graphql and
      PostGraphile V5 cannot, according to their own documentation and
      repositories. The details differ enough to matter.
    </p>
    <ul class={ul}>
      <li>
        <strong>Postrust</strong> is an opt-in Federation v2 subgraph, enabled
        with <code>PGRST_GRAPHQL_FEDERATION</code>, where a table&rsquo;s
        primary key is its <code>@key</code>. Apollo&rsquo;s subgraph
        compatibility suite runs in CI; <code>@requires</code>,{" "}
        <code>@provides</code>, <code>@override</code>, non-primary keys and
        federated tracing are not supported yet, and Postrust is not on
        Apollo&rsquo;s list of compatible subgraphs. See the{" "}
        <Link href="/docs/graphql/#federation" class={a}>
          Federation documentation
        </Link>
        .
      </li>
      <li>
        <strong>Hasura v2</strong> is a subgraph for Federation v1 only, enabled
        globally and then table by table in metadata (
        <a
          href="https://hasura.io/docs/2.0/data-federation/apollo-federation/"
          target="_blank"
          rel="noopener noreferrer"
          class={a}
        >
          source
        </a>
        ).
      </li>
      <li>
        <strong>Hasura DDN (v3)</strong> is documented as compliant with the
        subgraph specification, configured per object type and model; the docs
        name no Federation version, and other subgraphs cannot extend its types
        (
        <a
          href="https://hasura.io/docs/3.0/graphql-api/apollo-federation"
          target="_blank"
          rel="noopener noreferrer"
          class={a}
        >
          source
        </a>
        ).
      </li>
      <li>
        <strong>PostGraphile</strong>: <code>@graphile/federation</code> for v4
        is a proof of concept, last published in 2021 and unmaintained; we found
        nothing equivalent for V5 (
        <a
          href="https://github.com/graphile/federation"
          target="_blank"
          rel="noopener noreferrer"
          class={a}
        >
          source
        </a>
        ).
      </li>
      <li>
        <strong>Supabase pg_graphql</strong> has no Federation support, and its
        maintainers have said it is not planned (
        <a
          href="https://github.com/supabase/pg_graphql/issues/374"
          target="_blank"
          rel="noopener noreferrer"
          class={a}
        >
          source
        </a>
        ).
      </li>
    </ul>
    <p class={p}>
      A worked example with two databases behind Apollo Router is in{" "}
      <Link href="/blog/postgresql-apollo-federation-subgraph/" class={a}>
        PostgreSQL as an Apollo Federation subgraph
      </Link>
      .
    </p>

    <h2 class={h2}>What about GraphJin, Grafbase and IBM&rsquo;s StepZen?</h2>
    <p class={p}>
      They also generate GraphQL over PostgreSQL, and this guide covers only
      where they stood on Federation in October 2026, from their own
      documentation.
    </p>
    <ul class={ul}>
      <li>
        <strong>GraphJin</strong> serves Federation 2 SDL, but{" "}
        <code>_entities</code> is not implemented yet, so it composes and cannot
        resolve an entity another subgraph references (
        <a
          href="https://github.com/dosco/graphjin"
          target="_blank"
          rel="noopener noreferrer"
          class={a}
        >
          source
        </a>
        ).
      </li>
      <li>
        <strong>Grafbase&rsquo;s Postgres extension</strong> runs inside
        Grafbase Gateway as a virtual subgraph; there is no server for Apollo
        Router to call (
        <a
          href="https://grafbase.com/extensions/postgres"
          target="_blank"
          rel="noopener noreferrer"
          class={a}
        >
          source
        </a>
        ).
      </li>
      <li>
        <strong>IBM API Connect for GraphQL</strong> (formerly StepZen) is on
        Apollo&rsquo;s list of compatible subgraphs and reaches PostgreSQL
        through <code>@dbquery</code>; it is commercial, as SaaS or self-hosted
        software (
        <a
          href="https://www.apollographql.com/docs/graphos/schema-design/federated-schemas/reference/compatible-subgraphs"
          target="_blank"
          rel="noopener noreferrer"
          class={a}
        >
          source
        </a>
        ).
      </li>
    </ul>

    <h2 class={h2}>How do subscriptions differ between them?</h2>
    <p class={p}>
      Hasura treats subscriptions as a core feature, including cursor-based{" "}
      <code>_stream</code>. Postrust and PostGraphile build them on
      PostgreSQL&rsquo;s LISTEN/NOTIFY; Supabase provides a separate Realtime
      service; PostgREST has none.
    </p>
    <p class={p}>
      In Postrust each subscription field is a live query, served over WebSocket
      with the graphql-transport-ws protocol; <code>_stream</code> is not
      implemented. In PostGraphile you define the subscription fields yourself.
      On AWS Lambda Postrust serves GraphQL without subscriptions, because a
      function URL cannot hold a WebSocket.
    </p>

    <h2 class={h2}>How is each one deployed and hosted?</h2>
    <p class={p}>
      Postrust and PostgREST are single binaries, Hasura v2 runs as a container,
      PostGraphile runs on Node.js, pg_graphql is an extension inside the
      database, and Supabase is a managed platform or a stack you self-host.
    </p>
    <p class={p}>
      Postrust is configured by environment variables rather than by state a
      server owns, and runs serverless as well as on servers: on AWS Lambda
      through a native adapter with the same REST and GraphQL API, and on
      Cloudflare Workers (see{" "}
      <Link href="/docs/workers/" class={a}>
        what the Worker serves
      </Link>
      ). Hasura v2 is self-hostable as open source. Supabase is the only one
      here that is managed for you out of the box, with auth, storage and a
      dashboard included.
    </p>

    <h2 class={h2}>
      How closely does Postrust match Hasura and PostgREST, measured?
    </h2>
    <p class={p}>
      Compatibility is measured by replaying each project&rsquo;s own tests
      against both servers, not asserted. The figures below are generated from
      those runs.
    </p>
    <ul class={ul}>
      <li>
        Against Hasura {hasuraConformanceMeta.hasura}, on{" "}
        {hasuraConformanceMeta.cases} cases from Hasura&rsquo;s own corpus,{" "}
        {hasuraConformance.all.sameData.pct}% agree at the &ldquo;same
        data&rdquo; level (same rows, or both refusing), measured{" "}
        {hasuraConformanceMeta.measured}.{" "}
        <Link href="/docs/conformance/hasura/" class={a}>
          Hasura conformance report
        </Link>
        .
      </li>
      <li>
        Against PostgREST {conformanceMeta.postgrest}, on{" "}
        {conformanceMeta.cases} of PostgREST&rsquo;s own test cases,{" "}
        {conformance.all.statusAndBody.pct}% agree on status and body, measured{" "}
        {conformanceMeta.measured}.{" "}
        <Link href="/docs/conformance/postgrest/" class={a}>
          PostgREST conformance report
        </Link>
        .
      </li>
    </ul>
    <p class={p}>
      Throughput comparisons are not published here at the moment: the
      comparison benchmark is being re-measured because its results did not
      reproduce reliably. The method is on the{" "}
      <Link href="/docs/benchmarks/" class={a}>
        benchmarks page
      </Link>
      .
    </p>

    <h2 class={h2}>Which should a SaaS team choose, and when?</h2>
    <p class={p}>
      Choose by the constraint that matters most to you, not by a feature count.
      Each tool is the better fit for some teams, including the ones we do not
      make.
    </p>

    <h3 class={h3}>Choose Hasura when</h3>
    <ul class={ul}>
      <li>You need inherited roles or remote-schema permissions.</li>
      <li>
        Your API spans more than PostgreSQL: remote schemas and cross-source
        joins.
      </li>
      <li>
        You want event triggers, a console and the rest of a platform rather
        than a server. For new projects Hasura recommends v3 / DDN.
      </li>
    </ul>

    <h3 class={h3}>Choose PostGraphile when</h3>
    <ul class={ul}>
      <li>
        You want to reshape the generated GraphQL schema; its plugin system is
        built for that, and Postrust has no answer to it.
      </li>
      <li>Your team writes TypeScript and wants extensions in TypeScript.</li>
      <li>You want a Relay-compliant schema, and REST is not needed.</li>
    </ul>

    <h3 class={h3}>Choose Supabase when</h3>
    <ul class={ul}>
      <li>
        You want auth, storage, realtime and a dashboard without assembling
        them, and would rather not operate anything.
      </li>
      <li>You are early and want defaults that are already wired together.</li>
    </ul>

    <h3 class={h3}>Choose PostgREST when</h3>
    <ul class={ul}>
      <li>REST is all you need and a GraphQL surface would be weight.</li>
      <li>
        You want the full PostgREST surface and years of production use,
        documentation and community behind it.
      </li>
    </ul>

    <h3 class={h3}>Choose Postrust when</h3>
    <ul class={ul}>
      <li>
        You want REST and GraphQL over the same tables from one process, with
        permissions in PostgreSQL.
      </li>
      <li>
        You have Hasura clients and want to keep their queries while moving to a
        binary instead of a platform.
      </li>
      <li>
        You deploy to AWS Lambda, or want to add endpoints in Rust next to the
        generated API.
      </li>
    </ul>
    <p class={p}>
      Postrust is young: a smaller surface and fewer users than any of the
      others here. Test the queries and endpoints you actually depend on before
      switching anything that matters.
    </p>
  </GuidePage>
));

export const head: DocumentHead = guideHead(guide);
