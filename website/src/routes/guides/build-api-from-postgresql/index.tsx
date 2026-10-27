import { component$ } from "@builder.io/qwik";
import type { DocumentHead } from "@builder.io/qwik-city";
import { Link } from "@builder.io/qwik-city";
import { Code } from "~/components/blog/code";
import { GuidePage, guideHead } from "~/components/guides/guide-page";
import { guideBySlug } from "~/data/guides";

const guide = guideBySlug("build-api-from-postgresql");

const REPO = "https://github.com/postrust/postrust";

const h2 =
  "mb-5 mt-16 text-2xl font-bold tracking-tight text-neutral-950 md:text-[1.75rem]";
const h3 = "mb-3 mt-10 text-xl font-semibold text-neutral-900";
const p = "mb-5 text-[1.0625rem] leading-8 text-neutral-700";
const a =
  "font-medium text-primary-700 underline decoration-primary-300 underline-offset-2 hover:decoration-primary-600";
const code = "font-mono";
const ul =
  "marker:text-primary-500 mb-5 list-disc space-y-2 pl-6 text-[1.0625rem] leading-8 text-neutral-700";
const th = "px-4 py-3 text-left font-medium text-neutral-900 align-bottom";
const td = "px-4 py-3 text-neutral-700 align-top";

/** The three ways teams put an API in front of PostgreSQL. */
const approaches = [
  {
    approach: "Hand-written app server",
    examples:
      "Express, Django, Rails, Spring, Axum, with an ORM or query builder",
    write: "Every route, query, validation and serializer",
    auth: "Application code, sometimes with RLS underneath",
    surfaces: "Whatever you build",
    fits: "The API is mostly business logic, or does not mirror the tables",
  },
  {
    approach: "Schema-generated API server",
    examples: "PostgREST, Hasura, PostGraphile, Postrust",
    write: "SQL: tables, views, functions, grants, policies",
    auth: "The database (roles and RLS), or Hasura-style permission metadata",
    surfaces: "REST, GraphQL, or both, depending on the tool",
    fits: "The API is mostly reads and writes over the schema you already have",
  },
  {
    approach: "Backend platform",
    examples: "Supabase",
    write: "SQL, plus platform configuration",
    auth: "The platform's auth service issuing JWTs, enforced by RLS",
    surfaces:
      "REST (PostgREST), GraphQL via an extension, plus auth, storage, realtime",
    fits: "You want auth, storage and a dashboard already wired together",
  },
];

const SCHEMA = `CREATE TABLE authors (
    id SERIAL PRIMARY KEY,
    name TEXT NOT NULL
);

CREATE TABLE books (
    id SERIAL PRIMARY KEY,
    title TEXT NOT NULL,
    year INT NOT NULL,
    author_id INT NOT NULL REFERENCES authors (id)
);

INSERT INTO authors (name) VALUES
    ('Ursula K. Le Guin'),
    ('Octavia E. Butler');

INSERT INTO books (title, year, author_id) VALUES
    ('A Wizard of Earthsea', 1968, 1),
    ('The Dispossessed', 1974, 1),
    ('Kindred', 1979, 2),
    ('Parable of the Sower', 1993, 2);

-- A read-only function, served as RPC
CREATE FUNCTION books_since(since INT) RETURNS SETOF books
    LANGUAGE sql STABLE
    AS $$ SELECT * FROM books WHERE year >= since ORDER BY year $$;

-- The role anonymous requests run as
CREATE ROLE web_anon NOLOGIN;
GRANT USAGE ON SCHEMA public TO web_anon;
GRANT SELECT ON public.authors, public.books TO web_anon;`;

const RUN_BINARY = `# Clone the repository
git clone https://github.com/postrust/postrust.git
cd postrust

# Build in release mode
cargo build --release

# Set required environment variables
export DATABASE_URL="postgres://user:password@localhost:5432/mydb"
export PGRST_DB_ANON_ROLE="web_anon"

# Start the server
./target/release/postrust`;

const RUN_DOCKER = `docker run -d \\
  --name postrust \\
  -p 3000:3000 \\
  -e DATABASE_URL="postgres://user:pass@host:5432/db" \\
  -e PGRST_DB_ANON_ROLE="web_anon" \\
  -e PGRST_JWT_SECRET="your-secret-key" \\
  postrust/postrust:latest`;

const REST = `# Every book, newest first
curl "http://localhost:3000/api/books?order=year.desc"

# Filter and pick columns
curl "http://localhost:3000/api/books?year=lt.1975&select=title,year"

# Embed the related author through the foreign key
curl "http://localhost:3000/api/books?select=title,authors(name)"

# And the other direction: each author with their books
curl "http://localhost:3000/api/authors?select=name,books(title,year)"

# Paginate
curl "http://localhost:3000/api/books?limit=2&offset=2"

# Call the function
curl -X POST http://localhost:3000/api/rpc/books_since \\
  -H "Content-Type: application/json" \\
  -d '{"since": 1975}'`;

const GRAPHQL = `# The same books, each with its author
curl -X POST http://localhost:3000/v1/graphql \\
  -H "Content-Type: application/json" \\
  -d '{"query": "{ books(order_by: [{year: desc}]) { title year author { name } } }"}'

# Each author with a filtered list of books, and a count
curl -X POST http://localhost:3000/v1/graphql \\
  -H "Content-Type: application/json" \\
  -d '{"query": "{ authors { name books(where: {year: {_lt: 1975}}) { title } books_aggregate { aggregate { count } } } }"}'`;

export default component$(() => (
  <GuidePage
    guide={guide}
    answer="Most teams take one of three routes: write an application server by hand, run a server that generates the API from the PostgreSQL schema (PostgREST, Hasura, PostGraphile, Postrust), or adopt a backend platform such as Supabase. In the generated approach the schema is the API definition, and roles and row-level security in the database decide who can read and write what. To serve REST and GraphQL from the same database, Postrust does both from one process; Hasura adds REST endpoints over its GraphQL, and Supabase pairs PostgREST with a GraphQL extension."
    faq={[
      {
        q: "Can I serve REST and GraphQL from the same PostgreSQL database?",
        a: "Yes. Postrust serves a PostgREST-dialect REST API and a Hasura-dialect GraphQL API from one process over the same schema. Alternatives are Hasura's RESTified endpoints over its GraphQL, Supabase's PostgREST plus its GraphQL extension, or running PostgREST and PostGraphile side by side as two deployments.",
      },
      {
        q: "Do I still need a backend if I generate the API from PostgreSQL?",
        a: "For reads and writes over tables, views and functions, no. You still need somewhere for logic that does not belong in SQL, such as webhooks or calls to third-party services; in Postrust that can be a custom Rust route in the same binary.",
      },
      {
        q: "Where does authorization happen in a generated PostgreSQL API?",
        a: "In the database. The server verifies the JWT, switches to the role it names and makes the claims available to SQL, and grants plus row-level security policies decide what that request can see and change.",
      },
      {
        q: "Is an auto-generated API secure enough for production?",
        a: "It is as secure as the grants and policies behind it. Connect as a least-privilege authenticator role rather than a superuser, which bypasses row-level security, and enable RLS on every exposed table that holds per-user data.",
      },
      {
        q: "Can a generated PostgreSQL API run serverless?",
        a: "Postrust has adapters for AWS Lambda, which serves the same REST and GraphQL router as the server, and for Cloudflare Workers, which runs the same REST request pipeline over Hyperdrive.",
      },
    ]}
  >
    <h2 class={h2}>What are the ways to build an API on PostgreSQL?</h2>
    <p class={p}>
      There are three: write the API yourself, generate it from the schema, or
      adopt a platform that generates it and adds services around it. They
      differ in what you write and in where the rule &ldquo;who may see this
      row&rdquo; lives.
    </p>
    <div class="mb-8 overflow-x-auto rounded-xl border border-neutral-200">
      <table class="w-full min-w-[56rem] text-sm">
        <thead class="bg-neutral-50">
          <tr>
            <th class={th}>Approach</th>
            <th class={th}>Examples</th>
            <th class={th}>What you write</th>
            <th class={th}>Where authorization lives</th>
            <th class={th}>API surfaces</th>
            <th class={th}>Fits when</th>
          </tr>
        </thead>
        <tbody class="divide-y divide-neutral-200">
          {approaches.map((row) => (
            <tr key={row.approach}>
              <td class={`${td} font-medium text-neutral-900`}>
                {row.approach}
              </td>
              <td class={td}>{row.examples}</td>
              <td class={td}>{row.write}</td>
              <td class={td}>{row.auth}</td>
              <td class={td}>{row.surfaces}</td>
              <td class={td}>{row.fits}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
    <p class={p}>
      A hand-written server gives you full control and costs a route, a query
      and a test for every operation. A generated server reads the catalog
      &mdash; tables, columns, keys, foreign keys, functions &mdash; and serves
      an API for all of it, so the API follows the schema rather than a list of
      routes. A platform does the same for its API layer and also runs auth,
      storage and a dashboard for you; Supabase&rsquo;s REST API is PostgREST.
    </p>
    <p class={p}>
      The generated tools also differ from each other. PostgREST serves REST
      only. PostGraphile serves GraphQL and is built to have its schema reshaped
      through plugins. Hasura serves GraphQL with permissions kept as metadata,
      plus RESTified endpoints. Postrust serves REST in PostgREST&rsquo;s
      dialect and GraphQL in Hasura&rsquo;s from one process. The trade-offs are
      laid out tool by tool in{" "}
      <Link href="/guides/postgresql-graphql-tools-compared/" class={a}>
        PostgreSQL GraphQL tools compared
      </Link>{" "}
      and on the{" "}
      <Link href="/compare/" class={a}>
        comparison pages
      </Link>
      .
    </p>

    <h2 class={h2}>
      Which open-source projects serve REST and GraphQL from one database?
    </h2>
    <p class={p}>
      Postrust serves both from one process. The other routes pair two
      components or put one dialect on top of the other.
    </p>
    <ul class={ul}>
      <li>
        <strong>Postrust</strong> (MIT): REST in PostgREST&rsquo;s dialect and
        GraphQL in Hasura&rsquo;s, generated from the same schema cache, with
        the same JWT claims reaching the same RLS policies on both.
      </li>
      <li>
        <strong>Hasura</strong>: GraphQL first, with RESTified endpoints defined
        over GraphQL queries.
      </li>
      <li>
        <strong>Supabase</strong>: PostgREST for REST and a PostgreSQL extension
        for GraphQL, as part of the platform.
      </li>
      <li>
        <strong>PostgREST and PostGraphile side by side</strong>: two
        deployments over one database, each with its own configuration and
        dialect.
      </li>
    </ul>

    <h2 class={h2}>What does a schema-generated API give you?</h2>
    <p class={p}>
      Create, read, update and delete on every exposed table and view,
      filtering, ordering, pagination, related rows through foreign keys, and
      SQL functions as endpoints. In Postrust the REST surface covers:
    </p>
    <ul class={ul}>
      <li>
        <strong>CRUD</strong>: <code class={code}>GET</code>,{" "}
        <code class={code}>POST</code>, <code class={code}>PATCH</code>,{" "}
        <code class={code}>PUT</code> (upsert) and{" "}
        <code class={code}>DELETE</code> on{" "}
        <code class={code}>/{"{table}"}</code>.
      </li>
      <li>
        <strong>Filtering</strong> with operators such as{" "}
        <code class={code}>eq</code>, <code class={code}>gt</code>,{" "}
        <code class={code}>ilike</code>, <code class={code}>in</code> and{" "}
        <code class={code}>is</code>, and full-text search with{" "}
        <code class={code}>fts</code>.
      </li>
      <li>
        <strong>Embedding</strong>:{" "}
        <code class={code}>select=*,customer(name)</code> returns related rows
        nested in the response, following foreign keys.
      </li>
      <li>
        <strong>Pagination</strong> by <code class={code}>limit</code> and{" "}
        <code class={code}>offset</code>, or a <code class={code}>Range</code>{" "}
        header, with <code class={code}>Prefer: count=exact</code> for a total.
      </li>
      <li>
        <strong>RPC</strong>: a function is called at{" "}
        <code class={code}>/rpc/{"{function}"}</code>; a stable one also answers{" "}
        <code class={code}>GET</code>.
      </li>
    </ul>
    <p class={p}>
      The GraphQL surface at <code class={code}>/v1/graphql</code> generates the
      matching fields: a root field per table with{" "}
      <code class={code}>where</code>, <code class={code}>order_by</code>,{" "}
      <code class={code}>limit</code> and <code class={code}>offset</code>,{" "}
      <code class={code}>_by_pk</code> and <code class={code}>_aggregate</code>{" "}
      fields, relationships, and insert, update and delete mutations. A mutation
      naming several root fields runs them in one transaction. How the GraphQL
      schema is derived from tables and keys is covered in{" "}
      <Link href="/guides/graphql-from-postgresql-schema/" class={a}>
        GraphQL from a PostgreSQL schema
      </Link>
      ; every operator is in the{" "}
      <Link href="/docs/api-reference/" class={a}>
        API reference
      </Link>{" "}
      and the{" "}
      <Link href="/docs/graphql/" class={a}>
        GraphQL docs
      </Link>
      .
    </p>

    <h2 class={h2}>Where do authentication and permissions live?</h2>
    <p class={p}>
      In PostgreSQL. The API server verifies a JWT, runs the request as the
      database role the token names, and exposes its claims to SQL; grants and
      row-level security (RLS) policies then decide what that request can do.
    </p>
    <p class={p}>
      In Postrust, <code class={code}>PGRST_JWT_SECRET</code> is the key tokens
      are verified with and <code class={code}>PGRST_DB_ANON_ROLE</code> is the
      role for requests without one. A policy reads the caller from{" "}
      <code class={code}>current_setting('request.jwt.claims')</code>, which is
      set in every REST and every GraphQL transaction, so one policy covers both
      surfaces. The GraphQL side also accepts Hasura&rsquo;s contract: a role
      from <code class={code}>x-hasura-default-role</code>, chosen with{" "}
      <code class={code}>X-Hasura-Role</code> from the token&rsquo;s{" "}
      <code class={code}>x-hasura-allowed-roles</code>. Policies are worked
      through in{" "}
      <Link href="/guides/postgresql-row-level-security-api/" class={a}>
        row-level security for a PostgreSQL API
      </Link>{" "}
      and in{" "}
      <Link href="/docs/authentication/" class={a}>
        Authentication
      </Link>
      .
    </p>
    <p class={p}>
      The server should connect as a least-privilege authenticator role, never
      as a superuser: a superuser bypasses row-level security.
    </p>

    <h2 class={h2}>How do clients get realtime updates?</h2>
    <p class={p}>
      Through GraphQL subscriptions. In Postrust each subscription field is a
      live query: the answer now, and the answer again whenever it changes, over
      a WebSocket at <code class={code}>/v1/graphql/ws</code> using the{" "}
      <code class={code}>graphql-transport-ws</code> protocol.
    </p>
    <p class={p}>
      The subscription root mirrors the query root, so the same filters apply,
      and a subscription runs under the same identity and policies as a query.
      Hasura&rsquo;s cursor-based <code class={code}>_stream</code>{" "}
      subscriptions are not implemented. See{" "}
      <Link href="/docs/realtime/" class={a}>
        Realtime
      </Link>
      .
    </p>

    <h2 class={h2}>
      How do you serve a PostgreSQL schema as REST and GraphQL with Postrust?
    </h2>
    <p class={p}>
      Create the schema, start Postrust pointed at the database, and query it.
      The example below uses two related tables and one function; nothing is
      configured per table.
    </p>

    <h3 class={h3}>1. Create the schema and an anonymous role</h3>
    <Code label="schema.sql" code={SCHEMA} lang="sql" />
    <p class={p}>
      Postrust uses PostgreSQL roles for access control. Requests without a
      token run as <code class={code}>web_anon</code>, which may only read these
      two tables.
    </p>

    <h3 class={h3}>2. Start Postrust</h3>
    <p class={p}>
      From source, with a Rust toolchain and PostgreSQL 12 or later, as in{" "}
      <Link href="/docs/getting-started/" class={a}>
        Getting Started
      </Link>
      :
    </p>
    <Code label="Terminal" code={RUN_BINARY} lang="bash" />
    <p class={p}>
      Or with Docker. The image is <code class={code}>postrust/postrust</code>;
      point <code class={code}>DATABASE_URL</code> at your database:
    </p>
    <Code label="Terminal" code={RUN_DOCKER} lang="bash" />
    <p class={p}>
      Pre-built binaries are on the{" "}
      <a
        href={`${REPO}/releases`}
        target="_blank"
        rel="noopener noreferrer"
        class={a}
      >
        releases page
      </a>
      . The server listens on port 3000. By default the REST API is mounted
      under <code class={code}>/api</code>; with{" "}
      <code class={code}>PGRST_COMPAT_MODE=true</code> it is also served at the
      root, as PostgREST serves it.
    </p>

    <h3 class={h3}>3. Query it over REST</h3>
    <Code label="REST" code={REST} lang="bash" />
    <p class={p}>
      The query grammar is PostgREST&rsquo;s. One difference by default: an RPC
      result comes back wrapped and keyed by the function name, as{" "}
      <code class={code}>{`[{"books_since": ...}]`}</code>; compatibility mode
      returns it bare, as PostgREST does.
    </p>

    <h3 class={h3}>4. Query the same data over GraphQL</h3>
    <Code label="GraphQL" code={GRAPHQL} lang="bash" />
    <p class={p}>
      The foreign key from <code class={code}>books</code> to{" "}
      <code class={code}>authors</code> becomes <code class={code}>author</code>{" "}
      on a book and <code class={code}>books</code> on an author. Both requests
      run as <code class={code}>web_anon</code>, under the same grants as the
      REST calls. Opening{" "}
      <code class={code}>http://localhost:3000/v1/graphql</code> in a browser
      serves GraphQL Playground.
    </p>
    <p class={p}>
      Logic that does not belong in SQL, such as a webhook handler, can be added
      as an Axum route in the same binary; see{" "}
      <Link href="/docs/custom-routes/" class={a}>
        Custom Routes
      </Link>
      .
    </p>

    <h2 class={h2}>Where can you deploy a generated PostgreSQL API?</h2>
    <p class={p}>
      Postrust is not tied to one platform: the same API runs as a binary, in a
      container, on Kubernetes, on a platform such as Fly.io or Railway, on AWS
      Lambda, or on Cloudflare Workers. Pick the one your team already operates.
    </p>
    <ul class={ul}>
      <li>
        <strong>Binary</strong>: build with{" "}
        <code class={code}>cargo build --release -p postrust-server</code> and
        run <code class={code}>./target/release/postrust</code>, configured by
        environment variables.
      </li>
      <li>
        <strong>Docker</strong>: the <code class={code}>postrust/postrust</code>{" "}
        image, alone or in Docker Compose next to PostgreSQL.
      </li>
      <li>
        <strong>Kubernetes</strong>: there is a{" "}
        <a
          href={`${REPO}/tree/main/charts/postrust`}
          target="_blank"
          rel="noopener noreferrer"
          class={a}
        >
          Helm chart
        </a>
        .
      </li>
      <li>
        <strong>Fly.io and Railway</strong>: deploy with each CLI and set{" "}
        <code class={code}>DATABASE_URL</code> as a secret or variable.
      </li>
      <li>
        <strong>AWS Lambda</strong>: a native adapter,{" "}
        <code class={code}>postrust-lambda</code>, serves the same router as the
        server, so REST, GraphQL, JWT authentication and row-level security
        behave identically. Build with{" "}
        <code class={code}>
          cargo lambda build --release -p postrust-lambda
        </code>{" "}
        and deploy with AWS SAM or the Serverless Framework.
      </li>
      <li>
        <strong>Cloudflare Workers</strong>: the{" "}
        <code class={code}>postrust-worker</code> adapter runs the
        server&rsquo;s own REST request pipeline in a Worker, reaching
        PostgreSQL through Hyperdrive, with the schema cache in Workers KV and
        every request taking on the token&rsquo;s role and claims. What it
        serves, host by host, is in the{" "}
        <Link href="/docs/workers/" class={a}>
          Cloudflare Workers docs
        </Link>
        .
      </li>
    </ul>
    <p class={p}>
      Commands, templates and a production checklist are in{" "}
      <Link href="/docs/deployment/" class={a}>
        Deployment
      </Link>
      ; every environment variable is in{" "}
      <Link href="/docs/configuration/" class={a}>
        Configuration
      </Link>
      .
    </p>

    <h2 class={h2}>When should you write the API by hand instead?</h2>
    <p class={p}>
      When most endpoints are logic rather than data access, or when the public
      API must not mirror the tables. A generated API exposes what the schema
      and grants say; it does not orchestrate calls to other services.
    </p>
    <p class={p}>
      Many teams use both: a generated API for data access, and a small
      hand-written service or custom route for the rest. Views and functions in
      a dedicated schema, exposed with{" "}
      <code class={code}>PGRST_DB_SCHEMAS</code>, keep the public shape separate
      from the tables underneath. If you want auth, storage and a dashboard run
      for you, a platform is the shorter path; if you already have a PostgreSQL
      database and want an API in front of it rather than a platform around it,
      a generated server is the smaller step.
    </p>
  </GuidePage>
));

export const head: DocumentHead = guideHead(guide);
