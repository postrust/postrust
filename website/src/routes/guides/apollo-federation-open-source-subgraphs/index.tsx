import { component$ } from "@builder.io/qwik";
import type { DocumentHead } from "@builder.io/qwik-city";
import { Link } from "@builder.io/qwik-city";
import { Code } from "~/components/blog/code";
import { GuidePage, guideHead } from "~/components/guides/guide-page";
import { guideBySlug } from "~/data/guides";

const guide = guideBySlug("apollo-federation-open-source-subgraphs");

const POST = "/blog/postgresql-apollo-federation-subgraph/";
const EXAMPLE =
  "https://github.com/postrust/postrust/tree/main/examples/federation";
const COMPATIBLE =
  "https://www.apollographql.com/docs/graphos/schema-design/federated-schemas/reference/compatible-subgraphs";
const SUITE =
  "https://github.com/apollographql/apollo-federation-subgraph-compatibility";

const h2 =
  "mb-5 mt-14 text-2xl font-bold tracking-tight text-neutral-950 md:text-[1.75rem]";
const p = "mb-5 text-[1.0625rem] leading-8 text-neutral-700";
const a =
  "font-medium text-primary-700 underline decoration-primary-300 underline-offset-2 hover:decoration-primary-600";
const ul =
  "marker:text-primary-500 mb-5 list-disc space-y-2 pl-6 text-[1.0625rem] leading-8 text-neutral-700";
const code = "font-mono";

/**
 * As of October 2026, from each project's own documentation. Taken from the
 * blog post's comparison, plus Postrust.
 */
const options = [
  {
    tool: "Postrust",
    answer:
      "A Federation v2 subgraph, generated from the schema. Turned on with PGRST_GRAPHQL_FEDERATION=true; primary keys become @key and _entities resolves them. Apollo's compatibility suite runs in its CI, but it is not on Apollo's list of compatible subgraphs.",
    href: "/docs/graphql/#federation",
    internal: true,
  },
  {
    tool: "Hasura v2",
    answer:
      "A subgraph, Federation v1 only. Turned on with HASURA_GRAPHQL_ENABLE_APOLLO_FEDERATION, then table by table with apollo_federation_config in metadata.",
    href: "https://hasura.io/docs/2.0/data-federation/apollo-federation/",
  },
  {
    tool: "Hasura DDN (v3)",
    answer:
      "Documented as compliant with the subgraph specification, configured per object type and model. The docs name no Federation version, and other subgraphs cannot extend DDN's types.",
    href: "https://hasura.io/docs/3.0/graphql-api/apollo-federation",
  },
  {
    tool: "IBM API Connect for GraphQL (formerly StepZen)",
    answer:
      "On Apollo's list of compatible subgraphs; PostgreSQL through @dbquery. Commercial, as SaaS or self-hosted software.",
    href: COMPATIBLE,
  },
  {
    tool: "PostGraphile",
    answer:
      "@graphile/federation for v4 is a proof of concept, last published in 2021 and unmaintained. Nothing equivalent was found for V5.",
    href: "https://github.com/graphile/federation",
  },
  {
    tool: "GraphJin",
    answer:
      "Serves Federation 2 SDL, but _entities is not implemented yet, so it composes and cannot resolve an entity another subgraph references.",
    href: "https://github.com/dosco/graphjin",
  },
  {
    tool: "Grafbase Postgres extension",
    answer:
      "Runs inside Grafbase Gateway as a virtual subgraph; there is no server for Apollo Router to call.",
    href: "https://grafbase.com/extensions/postgres",
  },
  {
    tool: "Supabase pg_graphql",
    answer:
      "No Federation support; its maintainers have said it is not planned.",
    href: "https://github.com/supabase/pg_graphql/issues/374",
  },
];

export default component$(() => (
  <GuidePage
    guide={guide}
    answer="Apollo Federation composes several GraphQL services, called subgraphs, into one supergraph that a router serves; a subgraph declares which of its types are entities with @key and resolves them by key through the _entities field. For PostgreSQL, the options that work as a subgraph Apollo Router can call include Postrust (open source, Federation v2, generated from the schema), Hasura v2 (Federation v1) and Hasura DDN, while PostGraphile, GraphJin, Grafbase's Postgres extension and pg_graphql fall short in different ways."
    faq={[
      {
        q: "What is a subgraph in Apollo Federation?",
        a: "A subgraph is one GraphQL service in a federated graph. Beyond its own schema it answers _service, which returns its SDL for composition, and _entities, which returns objects by key so the router can fetch fields this service owns for objects another service found.",
      },
      {
        q: "What does the @key directive do?",
        a: '@key marks an object type as an entity and names the fields that identify it, such as @key(fields: "id"). The router uses those fields to ask any subgraph that contributes to the entity for its part of the object.',
      },
      {
        q: "Can PostgreSQL be an Apollo Federation subgraph without writing resolvers?",
        a: "Yes, with a server that generates the subgraph from the schema. Postrust does this: each table with a primary key becomes an entity keyed by that primary key, and _entities runs one query per type for all the keys the router sends.",
      },
      {
        q: "Does row-level security still apply behind Apollo Router?",
        a: "With Postrust, yes, as long as the router passes the Authorization header on to the subgraphs. Each subgraph verifies the token and runs its part of the query, _entities included, as the role the token names, so PostgreSQL's policies decide what each caller sees.",
      },
      {
        q: "Which Federation directives does Postrust not support?",
        a: "Keys other than the primary key, @requires, @provides, @external, @override, @tag, @inaccessible, @composeDirective, @interfaceObject and federated tracing. These are optional in Apollo's subgraph compatibility suite; the required checks pass in Postrust's CI.",
      },
      {
        q: "Is Hasura an Apollo Federation v2 subgraph?",
        a: "Hasura v2 supports Federation v1 only, enabled with HASURA_GRAPHQL_ENABLE_APOLLO_FEDERATION and per-table metadata. Hasura DDN is documented as compliant with the subgraph specification but its docs name no Federation version, as of October 2026.",
      },
    ]}
  >
    <h2 class={h2}>What is Apollo Federation?</h2>
    <p class={p}>
      Apollo Federation is a way to build one GraphQL API out of several
      independent GraphQL services. Each service, a <em>subgraph</em>, owns part
      of the schema; their schemas are <em>composed</em> into a{" "}
      <em>supergraph</em>, and a <em>router</em> such as Apollo Router serves
      it, splitting each client query into requests to the subgraphs that own
      the fields asked for.
    </p>
    <p class={p}>
      What ties the subgraphs together is the <em>entity</em>: an object type
      that more than one subgraph can contribute fields to, identified by the
      fields named in its <code class={code}>@key</code> directive. A catalog
      service and a reviews service can both define{" "}
      <code class={code}>{`Product @key(fields: "id")`}</code>, each with its
      own fields, and the router joins them by <code class={code}>id</code>.
      Composition &mdash; done with Rover&rsquo;s{" "}
      <code class={code}>rover supergraph compose</code>, or by Apollo GraphOS
      &mdash; checks that the subgraph schemas fit together before anything is
      served.
    </p>

    <h2 class={h2}>
      Which open-source projects can serve PostgreSQL as a subgraph?
    </h2>
    <p class={p}>
      Among tools that generate GraphQL from PostgreSQL, few are a working
      subgraph Apollo Router can call. Where each stood in October 2026, from
      its own documentation:
    </p>
    <div class="mb-8 overflow-x-auto rounded-xl border border-neutral-200">
      <table class="w-full text-left text-sm">
        <thead class="bg-neutral-50">
          <tr>
            <th class="px-4 py-3 font-semibold text-neutral-900">Tool</th>
            <th class="px-4 py-3 font-semibold text-neutral-900">
              As a Federation subgraph, October 2026
            </th>
          </tr>
        </thead>
        <tbody class="divide-y divide-neutral-200">
          {options.map((o) => (
            <tr key={o.tool} class="align-top">
              <td class="w-56 px-4 py-3 font-medium text-neutral-900">
                {o.tool}
              </td>
              <td class="px-4 py-3 leading-6 text-neutral-600">
                {o.answer}{" "}
                {o.internal ? (
                  <Link href={o.href} class={a}>
                    Docs
                  </Link>
                ) : (
                  <a
                    href={o.href}
                    target="_blank"
                    rel="noopener noreferrer"
                    class={a}
                  >
                    Source
                  </a>
                )}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
    <p class={p}>
      The differences that matter are in the details: Hasura v2 speaks only
      Federation v1, GraphJin composes but cannot resolve entities,
      PostGraphile&rsquo;s plugin is unmaintained, and Grafbase&rsquo;s
      extension needs Grafbase Gateway rather than Apollo Router. Postrust is
      the project this site is about, so read its row with that in mind.
    </p>

    <h2 class={h2}>What does a service have to implement to be a subgraph?</h2>
    <p class={p}>
      Two fields beyond its own schema, plus the Federation directives in its
      SDL.
    </p>
    <ul class={ul}>
      <li>
        <code class={code}>_service {"{ sdl }"}</code> returns the
        subgraph&rsquo;s schema, with Federation directives, for composition.
      </li>
      <li>
        <code class={code}>_entities(representations: [...])</code> takes a list
        of keys &mdash;{" "}
        <code class={code}>{`{ __typename: "Product", id: 1 }`}</code> &mdash;
        and returns those objects in the same order, so the router can fetch
        this subgraph&rsquo;s fields for objects another subgraph found.
      </li>
    </ul>
    <p class={p}>
      Apollo publishes a{" "}
      <a href={SUITE} target="_blank" rel="noopener noreferrer" class={a}>
        subgraph compatibility suite
      </a>{" "}
      that tests these against a reference schema, separating required behaviour
      from optional features, and keeps a{" "}
      <a href={COMPATIBLE} target="_blank" rel="noopener noreferrer" class={a}>
        list of compatible subgraph libraries
      </a>
      .
    </p>

    <h2 class={h2}>How do Federation v1 and v2 differ?</h2>
    <p class={p}>
      Federation v2 changed how subgraphs share types, and the two versions are
      not interchangeable at the subgraph level. In v1, a subgraph that added
      fields to another&rsquo;s entity wrote it as{" "}
      <code class={code}>extend type</code> and marked the borrowed key fields{" "}
      <code class={code}>@external</code>. In v2, every subgraph defines the
      entity as an ordinary type with <code class={code}>@key</code>, a field
      that more than one subgraph may resolve must be marked{" "}
      <code class={code}>@shareable</code>, and directives such as{" "}
      <code class={code}>@override</code>,{" "}
      <code class={code}>@inaccessible</code> and{" "}
      <code class={code}>@interfaceObject</code> were added. Which version a
      tool speaks matters: it decides which of these a subgraph can express.
    </p>

    <h2 class={h2}>Why generate a subgraph from the database?</h2>
    <p class={p}>
      Because for tables, a hand-written subgraph is mostly a transcription of
      the catalog. A resolver for every field and an entity resolver for every
      type restate what PostgreSQL already knows: a table is a type, a primary
      key is an <code class={code}>@key</code>, a foreign key is a relationship,
      and <code class={code}>_entities</code> is a{" "}
      <code class={code}>WHERE key IN (...)</code>.
    </p>
    <p class={p}>
      Generating that is the easy part. To be a real subgraph, a generated
      schema also has to:
    </p>
    <ul class={ul}>
      <li>
        resolve <code class={code}>_entities</code>, not just print Federation
        SDL &mdash; a subgraph that composes but cannot resolve a reference is
        what GraphJin is today;
      </li>
      <li>
        resolve many keys at once rather than one query per key, and return{" "}
        <code class={code}>null</code> in place for a key that matches nothing;
      </li>
      <li>
        let two databases agree that a table is the same entity, which the
        catalog cannot say on its own;
      </li>
      <li>
        keep generated names from colliding when several subgraphs from similar
        schemas are composed;
      </li>
      <li>
        apply the same access rules to entity lookups as to any other read.
      </li>
    </ul>

    <h2 class={h2}>How do you turn on Federation in Postrust?</h2>
    <p class={p}>
      Set <code class={code}>PGRST_GRAPHQL_FEDERATION=true</code>. The GraphQL
      API at <code class={code}>/v1/graphql</code> then gains{" "}
      <code class={code}>_service</code>, <code class={code}>_entities</code>{" "}
      and the Federation v2 directives, and its root types are named{" "}
      <code class={code}>Query</code>, <code class={code}>Mutation</code> and{" "}
      <code class={code}>Subscription</code>. With it unset, the schema is
      unchanged.
    </p>
    <Code
      label="a subgraph over one database"
      lang="bash"
      code={`PGRST_GRAPHQL_FEDERATION=true
PGRST_GRAPHQL_TYPE_PREFIX=catalog            # optional: avoid name collisions
PGRST_GRAPHQL_METADATA=/config/metadata.json # optional: shared entities, names`}
    />
    <p class={p}>
      Every readable table with a primary key becomes an entity keyed by it.
      When another subgraph contributes to the same entity, mark the table
      shared in the metadata: it keeps its unprefixed type name, and its non-key
      fields are marked <code class={code}>@shareable</code>.
    </p>
    <Code
      label="metadata.json"
      lang="json"
      code={`{
  "tables": {
    "public.products": {
      "name": "Product",
      "federation": { "shared": true }
    }
  }
}`}
    />
    <p class={p}>
      Each type&rsquo;s representations are resolved in one query, keys are
      checked against the key column&rsquo;s type before anything reaches the
      database, and an unknown key is a <code class={code}>null</code> in its
      position. Postrust runs Apollo&rsquo;s compatibility suite in CI on
      changes to its GraphQL code and the required checks must pass; the{" "}
      <Link href="/docs/graphql/#federation" class={a}>
        Federation documentation
      </Link>{" "}
      lists what is not supported yet, such as{" "}
      <code class={code}>@requires</code>, <code class={code}>@provides</code>{" "}
      and keys other than the primary key.
    </p>

    <h2 class={h2}>
      How do authentication and row-level security pass through the router?
    </h2>
    <p class={p}>
      The router forwards the caller&rsquo;s token, and each subgraph enforces
      it. The router has to pass the token on rather than stop at it, so
      propagate the <code class={code}>Authorization</code> header:
    </p>
    <Code
      label="router.yaml"
      lang="yaml"
      code={`headers:
  all:
    request:
      - propagate:
          named: authorization`}
    />
    <p class={p}>
      Each Postrust subgraph verifies the JWT itself and runs its part of the
      query &mdash; <code class={code}>_entities</code> included &mdash; as the
      role the token names, or the anonymous role without one.
      PostgreSQL&rsquo;s row-level security policies then decide which rows come
      back; nothing in the router filters anything. A token that does not verify
      is refused as <code class={code}>invalid-jwt</code> rather than answered
      as anonymous. Each subgraph can have its own roles and policies; the token
      is the only thing they share.
    </p>

    <h2 class={h2}>Where is a complete worked example?</h2>
    <p class={p}>
      <Link href={POST} class={a}>
        PostgreSQL as an Apollo Federation subgraph
      </Link>{" "}
      walks through two databases, a Postrust subgraph over each, Rover and
      Apollo Router, with queries in both directions and a policy that shows
      editors unpublished reviews. It runs as{" "}
      <a href={EXAMPLE} target="_blank" rel="noopener noreferrer" class={a}>
        examples/federation
      </a>
      , whose <code class={code}>check.sh</code> also runs in Postrust&rsquo;s
      CI. For Postrust against these tools more broadly, see{" "}
      <Link href="/compare/hasura/" class={a}>
        Postrust vs Hasura
      </Link>{" "}
      and{" "}
      <Link href="/compare/postgraphile/" class={a}>
        Postrust vs PostGraphile
      </Link>
      .
    </p>
  </GuidePage>
));

export const head: DocumentHead = guideHead(guide);
