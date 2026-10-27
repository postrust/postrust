import { component$ } from "@builder.io/qwik";
import type { DocumentHead } from "@builder.io/qwik-city";
import { Link } from "@builder.io/qwik-city";
import { Code } from "~/components/blog/code";
import { GuidePage, guideHead } from "~/components/guides/guide-page";
import { guideBySlug } from "~/data/guides";

const guide = guideBySlug("graphql-from-postgresql-schema");

const h2 =
  "mb-5 mt-16 text-2xl font-bold tracking-tight text-neutral-950 md:text-[1.75rem]";
const p = "mb-5 text-[1.0625rem] leading-8 text-neutral-700";
const a =
  "font-medium text-primary-700 underline decoration-primary-300 underline-offset-2 hover:decoration-primary-600";
const ul =
  "mb-5 list-disc space-y-2 pl-6 text-[1.0625rem] leading-8 text-neutral-700";
const th = "px-4 py-3 text-left font-medium text-neutral-900";
const td = "px-4 py-3 align-top text-neutral-700";

/** For tables `author` and `article`, where article.author_id references author.id. */
const mapping = [
  {
    object: "Table or view",
    source: "pg_class, pg_namespace",
    construct: "Object type, plus a <table>_bool_exp filter input",
    example: "type author { id: Int! name: String! }",
  },
  {
    object: "Column and its type",
    source: "pg_attribute, pg_type",
    construct: "Field with a scalar; NOT NULL and key columns are non-null",
    example:
      "int4 → Int, text → String, jsonb → jsonb, timestamptz → timestamptz",
  },
  {
    object: "Primary key",
    source: "pg_index",
    construct: "A _by_pk root field taking the key columns",
    example: "author_by_pk(id: 1)",
  },
  {
    object: "Foreign key",
    source: "pg_constraint",
    construct:
      "An object relationship on the referencing side, an array relationship on the referenced side",
    example: "article.author, author.articles, author.articles_aggregate",
  },
  {
    object: "Function returning SETOF a table",
    source: "pg_proc",
    construct:
      "A root field with its arguments under args; STABLE or IMMUTABLE on the query root, VOLATILE on the mutation root",
    example: 'search_articles(args: { term: "rust" })',
  },
  {
    object: "Comment on a table, column or function",
    source: "obj_description, col_description",
    construct: "The GraphQL description",
    example: "COMMENT ON TABLE author IS '…'",
  },
];

const nestedQuery = `query {
  author(
    where: { name: { _ilike: "%rust%" } }
    order_by: [{ name: asc }]
    limit: 10
  ) {
    id
    name
    articles(where: { published: { _eq: true } }, limit: 5) {
      id
      title
    }
  }
}`;

const nestedSql = `SELECT row_to_json(t) FROM (
  SELECT src.*,
         COALESCE((
           SELECT json_agg(row_to_json(e1_j)) FROM (
             SELECT "id", "title"
             FROM "public"."article" AS "e1"
             WHERE "e1"."author_id" = "src"."id"   -- the foreign key, correlated
               AND "e1"."published" = $2
             LIMIT 5                              -- per author, not overall
           ) e1_j
         ), '[]'::json) AS "articles"
  FROM (
    SELECT * FROM "public"."author" AS "pgrst_src"
    WHERE "name" ILIKE $1
    ORDER BY "name" ASC
    LIMIT 10
  ) AS src
) t`;

const metadata = `{
  "tables": {
    "public.products": {
      "name": "Product",
      "column_types": { "id": "ID" },
      "roots": { "select_by_pk": "product" }
    }
  }
}`;

export default component$(() => (
  <GuidePage
    guide={guide}
    answer="A generator reads the PostgreSQL catalogue (tables, columns and their types, primary and foreign keys, views and functions) and turns each table into a GraphQL object type with root fields to list, fetch by key, aggregate and write, each column type into a scalar, and each foreign key into a nested relationship field. At request time it compiles the whole nested selection into SQL, so PostgreSQL does the joining, and the database's roles and row-level security decide which rows come back. Postrust does this in Hasura's dialect at /v1/graphql."
    faq={[
      {
        q: "Do I have to write resolvers to get a GraphQL API from PostgreSQL?",
        a: "No. A schema-driven server derives the types, root fields and filters from the catalogue, and answers each request by generating SQL rather than calling hand-written resolvers. You write SQL objects instead: tables, views, functions and row-level security policies.",
      },
      {
        q: "How does a generated GraphQL API avoid the N+1 query problem?",
        a: "It plans the query as a whole instead of resolving field by field. Postrust attaches each requested relationship to the parent query as a correlated subselect that builds a JSON array, so a root field and everything nested under it is answered by one SQL statement.",
      },
      {
        q: "Where do relationship field names come from?",
        a: "From foreign keys. In Postrust the referenced side gets a pluralised array relationship (author.articles) and the referencing side a singular object relationship (article.author); PGRST_GRAPHQL_METADATA can give a relationship another name.",
      },
      {
        q: "Does the GraphQL schema update when I run a migration?",
        a: "Yes, with PGRST_DB_CHANNEL_ENABLED set: a migration that ends with NOTIFY pgrst, 'reload schema' makes Postrust re-read the catalogue and rebuild the GraphQL schema alongside the REST one, without a restart. Without it, the schema is the one read at start-up.",
      },
      {
        q: "Is Postrust's generated GraphQL compatible with Hasura clients?",
        a: "That is its purpose: the same root fields, arguments, operators, headers and error envelope, at the same /v1/graphql endpoint. How closely the two agree is measured by replaying Hasura's own test corpus against both servers and published on the Hasura conformance page.",
      },
    ]}
  >
    <h2 class={h2}>
      What does a GraphQL generator read from a PostgreSQL database?
    </h2>
    <p class={p}>
      It reads the system catalogue, the same metadata <code>\d</code> in psql
      prints. Every GraphQL construct the server exposes is derived from
      something PostgreSQL already records about the schema, so there is no
      separate type definition to keep in step with the tables.
    </p>
    <p class={p}>
      Postrust queries <code>pg_catalog</code> directly when it starts, for the
      schemas named in <code>PGRST_DB_SCHEMAS</code>. It reads{" "}
      <code>pg_class</code> and <code>pg_attribute</code> for tables, views and
      columns, <code>pg_index</code> for primary and unique keys,{" "}
      <code>pg_constraint</code> for foreign keys, <code>pg_proc</code> for
      functions, and <code>pg_type</code> and <code>pg_enum</code> for types.
      Views are followed back to the columns they select from, so a foreign key
      on a base table can still produce a relationship on the view. The mapping,
      for a table <code>author</code> and a table <code>article</code> whose{" "}
      <code>author_id</code> references it:
    </p>
    <div class="mb-8 overflow-x-auto rounded-xl border border-neutral-200">
      <table class="w-full text-sm">
        <thead class="bg-neutral-50">
          <tr>
            <th class={th}>PostgreSQL object</th>
            <th class={th}>Read from</th>
            <th class={th}>GraphQL construct</th>
            <th class={th}>Example</th>
          </tr>
        </thead>
        <tbody class="divide-y divide-neutral-200">
          {mapping.map((row) => (
            <tr key={row.object}>
              <td class={`${td} font-medium text-neutral-900`}>{row.object}</td>
              <td class={td}>
                <code>{row.source}</code>
              </td>
              <td class={td}>{row.construct}</td>
              <td class={td}>
                <code>{row.example}</code>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
    <p class={p}>
      Types with a GraphQL equivalent map to it: <code>int4</code> and{" "}
      <code>int2</code> to <code>Int</code>, <code>float8</code> to{" "}
      <code>Float</code>, <code>bool</code> to <code>Boolean</code>,{" "}
      <code>text</code> and <code>varchar</code> to <code>String</code>. The
      rest keep their PostgreSQL names as custom scalars (<code>bigint</code>,{" "}
      <code>numeric</code>, <code>uuid</code>, <code>jsonb</code>,{" "}
      <code>timestamptz</code>, <code>geometry</code>), because that is how
      Hasura names them and a client declares <code>$x: jsonb!</code> against
      exactly that spelling. Arrays become lists.
    </p>

    <h2 class={h2}>Which root fields does each table get?</h2>
    <p class={p}>
      Each table gets a family of root fields named after it: a list query, a
      lookup by primary key, an aggregate, and inserts, updates and deletes. For{" "}
      <code>author</code> in Postrust:
    </p>
    <ul class={ul}>
      <li>
        <code>author(where:, order_by:, distinct_on:, limit:, offset:)</code>{" "}
        returns the rows; <code>author_by_pk(id: 1)</code> returns one row or
        null, and exists only where the table has a primary key.
      </li>
      <li>
        <code>author_aggregate(where:)</code> returns{" "}
        <code>aggregate {"{ count sum { … } }"}</code> and the matching{" "}
        <code>nodes</code>.
      </li>
      <li>
        <code>insert_author</code>, <code>insert_author_one</code> (both with{" "}
        <code>on_conflict</code> for upserts), <code>update_author</code>,{" "}
        <code>update_author_by_pk</code>, <code>update_author_many</code>,{" "}
        <code>delete_author</code> and <code>delete_author_by_pk</code> on the
        mutation root. A mutation naming several of them runs them in one
        transaction.
      </li>
      <li>
        The subscription root mirrors the query root. Each field is a live
        query: the answer now, then the answer again whenever it changes.
      </li>
    </ul>

    <h2 class={h2}>
      How do where, order_by, limit and distinct_on become SQL?
    </h2>
    <p class={p}>
      Each argument maps to one SQL clause. <code>where</code> becomes the{" "}
      <code>WHERE</code> clause, <code>order_by</code> becomes{" "}
      <code>ORDER BY</code>, <code>limit</code> and <code>offset</code> carry
      over unchanged, and <code>distinct_on</code> becomes PostgreSQL&rsquo;s{" "}
      <code>DISTINCT ON</code>.
    </p>
    <p class={p}>
      <code>where</code> takes a generated <code>author_bool_exp</code>: one
      field per column holding typed comparison operators (<code>_eq</code>,{" "}
      <code>_gt</code>, <code>_in</code>, <code>_ilike</code>,{" "}
      <code>_is_null</code>, and the JSON, ltree and PostGIS operators for those
      types), combined with <code>_and</code>, <code>_or</code> and{" "}
      <code>_not</code>, and able to follow a relationship by naming it. Because
      operators are typed, a wrong operand is refused during validation rather
      than by the database. <code>order_by</code> is a list of single-key
      objects such as{" "}
      <code>[{"{ created_at: desc_nulls_last }, { name: asc }"}]</code>, because
      ordering is ordered. A query with no <code>limit</code> is capped at{" "}
      <code>PGRST_MAX_ROWS</code> when that is set. As in Hasura, a{" "}
      <code>distinct_on</code> that does not match the start of{" "}
      <code>order_by</code> is refused, since PostgreSQL would otherwise keep an
      arbitrary row from each group.
    </p>

    <h2 class={h2}>
      How does a nested GraphQL query become one SQL statement?
    </h2>
    <p class={p}>
      The server plans the whole selection before it runs anything, and nests
      each requested relationship inside the parent query as a subselect. A
      naive resolver-per-field server runs one query for the authors and then
      one more per author for their articles (the N+1 problem). A schema-driven
      server knows the foreign key behind <code>articles</code>, so it can let
      PostgreSQL do the join. Take this query:
    </p>
    <Code label="A nested query" lang="graphql" code={nestedQuery} />
    <p class={p}>
      Postrust compiles it into a single statement of this shape. It is
      simplified from what the server generates (aliases shortened, the bound
      values left as <code>$1</code> and <code>$2</code>), but the structure is
      the real one:
    </p>
    <Code label="The SQL it becomes (simplified)" lang="sql" code={nestedSql} />
    <p class={p}>
      Ordering and the limit apply inside the innermost subquery, so the ten
      authors are the first ten by name. Each author&rsquo;s articles come from
      a subselect correlated on the foreign key and folded into a JSON array
      with <code>json_agg</code>. An author with no articles gets{" "}
      <code>[]</code>, not null. The child <code>limit</code> applies per
      parent. A to-one relationship such as <code>article.author</code> is the
      same subselect with <code>LIMIT 1</code> and <code>row_to_json</code>{" "}
      instead of an array, and <code>articles_aggregate</code> is a correlated
      subselect that computes the aggregate. Deeper nesting repeats the pattern
      inside the child&rsquo;s select list. The query and everything nested in
      it run in one transaction, so parent and child rows come from the same
      snapshot.
    </p>

    <h2 class={h2}>
      Who decides what a caller may see when there are no resolvers?
    </h2>
    <p class={p}>
      The database does. Postrust runs every request in a transaction that has
      switched to the caller&rsquo;s database role with{" "}
      <code>SET LOCAL ROLE</code> and set the caller&rsquo;s session variables,
      so table grants and row-level security policies filter the generated SQL
      like any other query.
    </p>
    <p class={p}>
      An <code>x-hasura-user-id</code> session variable, from a verified token
      or from an administrator acting as another role, is readable in a policy
      as <code>current_setting('hasura.user_id')</code>. A function taking a{" "}
      <code>hasura_session json</code> argument receives the whole session.
      Policies apply to nested subselects, aggregates and subscriptions alike,
      because they all run as the same SQL under the same role. For migrations
      from Hasura, <code>PGRST_GRAPHQL_METADATA</code> can also carry per-role
      permissions. Each role then gets its own schema, built only from the
      tables it may read. The guide to{" "}
      <Link href="/guides/postgresql-row-level-security-api/" class={a}>
        row-level security behind an API
      </Link>{" "}
      covers the database side in more depth.
    </p>

    <h2 class={h2}>How are the generated names customised?</h2>
    <p class={p}>
      By default every name is derived from the catalogue. Where that is not
      what clients expect, <code>PGRST_GRAPHQL_METADATA</code> renames things: a
      JSON document, or a path to one, that can rename tables, columns, root
      fields, relationships and computed fields, choose the root a function
      appears on, and expose a column as <code>ID</code> with{" "}
      <code>column_types</code> without changing its database type:
    </p>
    <Code label="graphql-metadata.json" lang="json" code={metadata} />
    <p class={p}>
      <code>PGRST_GRAPHQL_TYPE_PREFIX</code> prefixes generated types and root
      fields, so two Postrust servers over different databases can be composed
      without their names colliding.
    </p>

    <h2 class={h2}>
      What happens to the GraphQL schema when the database changes?
    </h2>
    <p class={p}>
      In Postrust, a migration reaches GraphQL when it says so. With{" "}
      <code>PGRST_DB_CHANNEL_ENABLED</code> set, a migration that ends with{" "}
      <code>NOTIFY pgrst, 'reload schema'</code> makes the server re-read the
      catalogue and rebuild the GraphQL schema from it, beside the REST schema
      cache, without a restart. The new schema is swapped in only once it is
      built, so a request is answered wholly from the old schema or wholly
      from the new one, and a schema that cannot be built leaves the previous
      one serving. Subscriptions already open stay on the schema they started
      with. Without the setting, the schema is the one read at start-up.
    </p>
    <p class={p}>
      Data changes are a different matter. A live subscription re-runs its query
      when a trigger on the table it reads sends a <code>NOTIFY</code>, and also
      every <code>PGRST_SUBSCRIPTION_REFRESH</code> seconds (30 by default) to
      catch what a trigger cannot see, such as a view. It sends a message only
      when the answer differs from the last one.
    </p>

    <h2 class={h2}>Can a generated schema be an Apollo Federation subgraph?</h2>
    <p class={p}>
      Yes, as an opt-in. With <code>PGRST_GRAPHQL_FEDERATION=true</code>,
      Postrust serves its API as an Apollo Federation v2 subgraph. It adds{" "}
      <code>_service</code> and <code>_entities</code>, makes each table&rsquo;s
      primary key its <code>@key</code>, and names the root types{" "}
      <code>Query</code>, <code>Mutation</code> and <code>Subscription</code>{" "}
      instead of <code>query_root</code> and its siblings. With the setting off,
      the generated SDL is unchanged. The{" "}
      <Link href="/docs/graphql/" class={a}>
        GraphQL documentation
      </Link>{" "}
      covers shared entities and what is not supported yet.
    </p>

    <h2 class={h2}>
      How do Hasura, PostGraphile and pg_graphql approach the same idea?
    </h2>
    <p class={p}>
      All three generate GraphQL from a PostgreSQL schema and compile requests
      to SQL. They differ in dialect and in where configuration lives. Hasura
      tracks tables and relationships in its own metadata and defines
      permissions there. PostGraphile introspects the schema, exposes
      Relay-style connections, and relies on PostgreSQL roles and row-level
      security. pg_graphql, from Supabase, is an extension that resolves GraphQL
      inside the database through a SQL function. Postrust reflects the schema
      the way PostGraphile does, but answers in Hasura&rsquo;s dialect, so
      existing Hasura clients work unchanged. See{" "}
      <Link href="/compare/hasura/" class={a}>
        Postrust vs Hasura
      </Link>{" "}
      and{" "}
      <Link href="/compare/postgraphile/" class={a}>
        Postrust vs PostGraphile
      </Link>
      . The same server also serves REST from the same schema, covered in{" "}
      <Link href="/guides/build-api-from-postgresql/" class={a}>
        building an API from a PostgreSQL database
      </Link>
      .
    </p>
  </GuidePage>
));

export const head: DocumentHead = guideHead(guide);
