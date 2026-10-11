import { component$ } from "@builder.io/qwik";
import { Link } from "@builder.io/qwik-city";
import { Code, CodePair, PostLayout, postHead } from "~/components/blog/post";
import { posts } from "~/data/posts";

const SLUG = "postgresql-apollo-federation-subgraph";
const post = posts.find((p) => p.slug === SLUG)!;

const EXAMPLE =
  "https://github.com/postrust/postrust/tree/main/examples/federation";

const h2 =
  "mb-5 mt-16 text-2xl font-bold tracking-tight text-neutral-950 md:text-[1.75rem]";
const p = "mb-5 text-[1.0625rem] leading-8 text-neutral-700";
const a =
  "font-medium text-primary-700 underline decoration-primary-300 underline-offset-2 hover:decoration-primary-600";
const code = "font-mono";

/** As of October 2026; each row links to what it rests on. */
const others = [
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
    href: "https://www.apollographql.com/docs/graphos/schema-design/federated-schemas/reference/compatible-subgraphs",
  },
  {
    tool: "PostGraphile",
    answer:
      "@graphile/federation for v4 is a proof of concept, last published in 2021 and unmaintained. We found nothing equivalent for V5.",
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
  <PostLayout post={post}>
    <p class={p}>
      Putting PostgreSQL behind Apollo Router usually means a GraphQL service
      with a resolver for every type and field, and an entity resolver for every
      type another subgraph can reference. For tables that already say what they
      are &mdash; columns, keys, foreign keys &mdash; that service is mostly a
      transcription of the catalog. This post shows the alternative: subgraphs
      generated from the database, composed and queried through Apollo Router,
      with row-level security still deciding what each caller sees.
    </p>
    <p class={p}>
      We make Postrust, so it is the tool used here. Other tools that do
      something similar are compared{" "}
      <a href="#others" class={a}>
        at the end
      </a>
      , with sources.
    </p>

    <h2 class={h2}>What a subgraph has to do</h2>
    <p class={p}>
      A Federation v2 subgraph answers two fields beyond its own schema:
    </p>
    <ul class="marker:text-primary-500 mb-5 list-disc space-y-2 pl-6 text-[1.0625rem] leading-8 text-neutral-700">
      <li>
        <code class={code}>_service {"{ sdl }"}</code> returns its schema, with
        Federation directives, for the router to compose with the others.
      </li>
      <li>
        <code class={code}>_entities(representations: [...])</code> takes a list
        of keys &mdash;{" "}
        <code class={code}>{`{ __typename: "Product", id: 1 }`}</code> &mdash;
        and returns those objects, so the router can fetch the fields this
        subgraph owns for objects another subgraph found.
      </li>
    </ul>
    <p class={p}>
      Everything a generator needs for this is in PostgreSQL&rsquo;s catalog. A
      table is a type, a primary key is an <code class={code}>@key</code>, a
      foreign key is a relationship, and <code class={code}>_entities</code> is
      a <code class={code}>WHERE key IN (...)</code>. What the catalog cannot
      say is which tables are the same entity across databases; that part is
      configuration.
    </p>

    <h2 class={h2}>Two databases, one graph</h2>
    <p class={p}>
      The example has a <em>catalog</em> database that knows what products are,
      and a <em>reviews</em> database that knows what people said about them.
      The reviews database holds products only by key. Reviews are moderated:
      anonymous callers see published ones, and an{" "}
      <code class={code}>editor</code> sees all of them, enforced by a row-level
      security policy rather than by anything in the API layer.
    </p>
    <Code
      label="reviews.sql (excerpt)"
      lang="sql"
      code={`CREATE TABLE products (id int PRIMARY KEY);
CREATE TABLE reviews (
  id int PRIMARY KEY,
  product_id int NOT NULL REFERENCES products,
  rating int NOT NULL,
  body text NOT NULL,
  published boolean NOT NULL DEFAULT false
);

ALTER TABLE reviews ENABLE ROW LEVEL SECURITY;
CREATE POLICY published ON reviews FOR SELECT TO web_anon USING (published);
CREATE POLICY everything ON reviews FOR SELECT TO editor USING (true);`}
    />
    <p class={p}>
      Each database gets its own Postrust. Both log in as an{" "}
      <code class={code}>authenticator</code> role with no privileges of its
      own, which becomes the role the caller&rsquo;s token names for each
      request (<code class={code}>web_anon</code> without one) &mdash; the{" "}
      <Link href="/docs/authentication/" class={a}>
        setup PostgREST recommends
      </Link>
      .
    </p>
    <Code
      label="the catalog subgraph"
      lang="bash"
      code={`docker run -p 3001:3000 \\
  -e DATABASE_URL=postgres://authenticator:authenticator@catalog-db:5432/postgres \\
  -e PGRST_DB_ANON_ROLE=web_anon \\
  -e PGRST_JWT_SECRET=an-example-secret-of-at-least-32-characters \\
  -e PGRST_GRAPHQL_FEDERATION=true \\
  -e PGRST_GRAPHQL_TYPE_PREFIX=catalog \\
  -e PGRST_GRAPHQL_METADATA=/config/metadata.json \\
  -v ./catalog.json:/config/metadata.json:ro \\
  postrust/postrust:2.3.0`}
    />
    <p class={p}>
      <code class={code}>PGRST_GRAPHQL_FEDERATION</code> turns on{" "}
      <code class={code}>_service</code>, <code class={code}>_entities</code>{" "}
      and the Federation directives.{" "}
      <code class={code}>PGRST_GRAPHQL_TYPE_PREFIX</code> prefixes every
      generated type and root field, so two subgraphs generated from tables of
      the same name do not collide when composed. The metadata says the one
      thing the catalog cannot: that <code class={code}>products</code> in both
      databases is the same entity.
    </p>
    <CodePair>
      <Code
        label="catalog.json"
        lang="json"
        code={`{
  "tables": {
    "public.products": {
      "name": "Product",
      "federation": { "shared": true },
      "roots": {
        "select": "products",
        "select_by_pk": "product"
      }
    }
  }
}`}
      />
      <Code
        label="reviews.json"
        lang="json"
        code={`{
  "tables": {
    "public.products": {
      "name": "Product",
      "federation": { "shared": true },
      "relationships": {
        "reviews_product_id_fkey": "reviews"
      }
    },
    "public.reviews": {
      "name": "Review",
      "roots": {
        "select": "reviews",
        "select_by_pk": "review"
      },
      "relationships": {
        "reviews_product_id_fkey": "product"
      }
    }
  }
}`}
      />
    </CodePair>
    <p class={p}>
      A shared table keeps its unprefixed type name,{" "}
      <code class={code}>Product</code>, in both subgraphs, so the router sees
      one entity contributed to by two services. Each subgraph serves it with
      the fields its own database has:
    </p>
    <CodePair>
      <Code
        label="catalog: _service { sdl } (excerpt)"
        lang="graphql"
        code={`type Product @key(fields: "id") {
  id: Int!
  name: String! @shareable
  price_cents: Int! @shareable
}`}
      />
      <Code
        label="reviews: _service { sdl } (excerpt)"
        lang="graphql"
        code={`type Product @key(fields: "id") {
  id: Int!
  reviews(...): [reviews_Review!]! @shareable
  reviews_aggregate(...): ... @shareable
}`}
      />
    </CodePair>
    <p class={p}>
      Every non-key field of a shared type is marked{" "}
      <code class={code}>@shareable</code>, which lets either subgraph resolve
      it if both happen to have it. Here they don&rsquo;t, so each field has
      exactly one owner.
    </p>

    <h2 class={h2}>Composing and routing</h2>
    <p class={p}>
      Rover reads each subgraph&rsquo;s SDL from{" "}
      <code class={code}>_service</code> and composes the supergraph; Apollo
      Router serves it.
    </p>
    <CodePair>
      <Code
        label="supergraph.yaml"
        lang="yaml"
        code={`federation_version: =2.11.2
subgraphs:
  catalog:
    routing_url: http://catalog:3000/v1/graphql
    schema:
      subgraph_url: http://catalog:3000/v1/graphql
  reviews:
    routing_url: http://reviews:3000/v1/graphql
    schema:
      subgraph_url: http://reviews:3000/v1/graphql`}
      />
      <Code
        label="router.yaml"
        lang="yaml"
        code={`supergraph:
  listen: 0.0.0.0:4000
headers:
  all:
    request:
      - propagate:
          named: authorization`}
      />
    </CodePair>
    <Code
      label="bash"
      lang="bash"
      code={`rover supergraph compose --config supergraph.yaml --output supergraph.graphql
router --config router.yaml --supergraph supergraph.graphql`}
    />
    <p class={p}>
      The <code class={code}>headers</code> rule is the part that is easy to
      miss. Each subgraph verifies the caller&rsquo;s token itself and runs the
      query as the role it names, so the router has to pass the token on rather
      than stop at it.
    </p>
    <p class={p}>One query now reads from both databases:</p>
    <CodePair>
      <Code
        label="query"
        lang="graphql"
        code={`{
  catalog_products {
    name
    price_cents
    reviews {
      rating
      body
    }
  }
}`}
      />
      <Code
        label="response, anonymous"
        lang="json"
        code={`{
  "data": {
    "catalog_products": [
      {
        "name": "Mechanical keyboard",
        "price_cents": 12900,
        "reviews": [
          { "rating": 5,
            "body": "Best keyboard I have owned." }
        ]
      },
      {
        "name": "USB-C dock",
        "price_cents": 8900,
        "reviews": [
          { "rating": 4,
            "body": "Does what it says." }
        ]
      }
    ]
  }
}`}
      />
    </CodePair>
    <p class={p}>
      The router asks the catalog for the products, then sends the reviews
      subgraph one <code class={code}>_entities</code> call with every product
      key it got back. Postrust answers it with one query per type, not one per
      key, and an unknown key is a <code class={code}>null</code> in its
      position rather than an error for the whole list:
    </p>
    <CodePair>
      <Code
        label="what the router sends, roughly"
        lang="graphql"
        code={`{
  _entities(representations: [
    { __typename: "Product", id: 1 },
    { __typename: "Product", id: 99 }
  ]) {
    ... on Product { reviews { rating } }
  }
}`}
      />
      <Code
        label="reviews subgraph"
        lang="json"
        code={`{
  "data": {
    "_entities": [
      { "reviews": [{ "rating": 5 }] },
      null
    ]
  }
}`}
      />
    </CodePair>
    <p class={p}>
      Keys are checked against the key column&rsquo;s type before anything
      reaches the database: sending <code class={code}>id: "1"</code> for an{" "}
      <code class={code}>int</code> key is a GraphQL error that names the field
      and the type it expected.
    </p>
    <p class={p}>
      It works in the other direction too. Starting from a review, the router
      follows <code class={code}>product</code> &mdash; a foreign key in the
      reviews database &mdash; to the catalog for the fields only the catalog
      has:
    </p>
    <CodePair>
      <Code
        label="query"
        lang="graphql"
        code={`{
  reviews_reviews {
    rating
    product {
      name
      price_cents
    }
  }
}`}
      />
      <Code
        label="response, anonymous"
        lang="json"
        code={`{
  "data": {
    "reviews_reviews": [
      { "rating": 5,
        "product": { "name": "Mechanical keyboard",
                     "price_cents": 12900 } },
      { "rating": 4,
        "product": { "name": "USB-C dock",
                     "price_cents": 8900 } }
    ]
  }
}`}
      />
    </CodePair>

    <h2 class={h2}>Who is asking, across subgraphs</h2>
    <p class={p}>
      The first query, sent with a token whose <code class={code}>role</code>{" "}
      claim is <code class={code}>editor</code>, returns the review still
      waiting for moderation as well:
    </p>
    <Code
      label="the keyboard's reviews, as an editor"
      lang="json"
      code={`"reviews": [
  { "rating": 5, "body": "Best keyboard I have owned." },
  { "rating": 2, "body": "Pending moderation." }
]`}
    />
    <p class={p}>
      Nothing in the router or the API layer filtered anything. The reviews
      subgraph ran the <code class={code}>_entities</code> query as{" "}
      <code class={code}>editor</code>, and PostgreSQL&rsquo;s policy decided. A
      token that does not verify &mdash; wrong key, expired &mdash; is refused
      as <code class={code}>invalid-jwt</code> rather than quietly answered as
      the anonymous role. Each subgraph can have its own roles and policies; the
      token is the only thing they share.
    </p>

    <h2 class={h2}>Run it</h2>
    <p class={p}>
      Everything above is in{" "}
      <a href={EXAMPLE} target="_blank" rel="noopener noreferrer" class={a}>
        examples/federation
      </a>
      : the two databases, both subgraphs, Rover and Apollo Router in one
      Compose file. Its <code class={code}>check.sh</code> sends these queries
      through the router and checks the answers: both directions, anonymous and
      as an editor, and a token that does not verify.
    </p>
    <Code
      label="bash"
      lang="bash"
      code={`git clone https://github.com/postrust/postrust
cd postrust/examples/federation
docker compose up --wait catalog reviews && docker compose up -d router
# then send queries to http://localhost:4000`}
    />
    <p class={p}>
      The same GraphQL API, Federation included, is also served by the{" "}
      <Link href="/docs/deployment/" class={a}>
        AWS Lambda adapter
      </Link>{" "}
      since 2.2.0. The Cloudflare Workers adapter serves the REST API only,
      so it cannot be a subgraph.
    </p>

    <h2 class={h2}>What it does not do</h2>
    <p class={p}>
      Postrust runs{" "}
      <a
        href="https://github.com/apollographql/apollo-federation-subgraph-compatibility"
        target="_blank"
        rel="noopener noreferrer"
        class={a}
      >
        Apollo&rsquo;s subgraph compatibility suite
      </a>{" "}
      in CI on every change to its GraphQL code, and the required checks have to
      pass. The optional ones it does not pass are the Federation features a
      schema generated from tables has no way to express yet:
    </p>
    <ul class="marker:text-primary-500 mb-5 list-disc space-y-2 pl-6 text-[1.0625rem] leading-8 text-neutral-700">
      <li>
        <code class={code}>@requires</code>, <code class={code}>@provides</code>
        , <code class={code}>@external</code> and{" "}
        <code class={code}>@override</code>;
      </li>
      <li>repeatable and nested keys;</li>
      <li>
        <code class={code}>@tag</code>, <code class={code}>@inaccessible</code>,{" "}
        <code class={code}>@composeDirective</code> and{" "}
        <code class={code}>@interfaceObject</code>;
      </li>
      <li>federated tracing.</li>
    </ul>
    <p class={p}>
      A shared type&rsquo;s fields are marked{" "}
      <code class={code}>@shareable</code> one by one rather than on the type.
      The two compose the same way, but the suite&rsquo;s check looks for the
      type-level form and fails. Postrust is not on Apollo&rsquo;s list of
      compatible subgraphs.
    </p>
    <p class={p}>
      The details are in the{" "}
      <Link href="/docs/graphql/#federation" class={a}>
        Federation documentation
      </Link>
      .
    </p>

    <h2 id="others" class={`${h2} scroll-mt-24`}>
      Other ways to do this
    </h2>
    <p class={p}>
      Where other tools that generate GraphQL from PostgreSQL stood on
      Federation in October 2026, from their own documentation:
    </p>
    <div class="post-wide mb-8 overflow-x-auto rounded-xl border border-neutral-200">
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
          {others.map((o) => (
            <tr key={o.tool} class="align-top">
              <td class="w-56 px-4 py-3 font-medium text-neutral-900">
                {o.tool}
              </td>
              <td class="px-4 py-3 leading-6 text-neutral-600">
                {o.answer}{" "}
                <a
                  href={o.href}
                  target="_blank"
                  rel="noopener noreferrer"
                  class={a}
                >
                  Source
                </a>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
    <p class={p}>
      For how Postrust compares to these tools beyond Federation, see{" "}
      <Link href="/compare/hasura/" class={a}>
        Postrust vs Hasura
      </Link>{" "}
      and{" "}
      <Link href="/compare/postgraphile/" class={a}>
        Postrust vs PostGraphile
      </Link>
      .
    </p>
  </PostLayout>
));

export const head = postHead(post);
