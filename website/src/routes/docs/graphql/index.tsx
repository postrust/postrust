import { component$ } from "@builder.io/qwik";
import type { DocumentHead } from "@builder.io/qwik-city";
import { Link } from "@builder.io/qwik-city";

const endpoints = [
  {
    method: "POST",
    path: "/v1/graphql",
    note: "Execute a query or mutation. This is where a Hasura client sends them, and for most generated clients the only address they can be told about.",
  },
  {
    method: "GET",
    path: "/v1/graphql",
    note: "GraphQL Playground (interactive IDE)",
  },
  {
    method: "GET",
    path: "/v1/graphql/ws",
    note: "Subscriptions over WebSocket, using the graphql-transport-ws protocol",
  },
  {
    method: "POST",
    path: "/v1alpha1/graphql",
    note: "The address Hasura served before /v1, for a client old enough that it cannot be repointed",
  },
  {
    method: "POST",
    path: "/api/graphql",
    note: "The same surface, for anything already pointed here",
  },
];

const rootFields = [
  ["author(where:, order_by:, distinct_on:, limit:, offset:)", "the rows"],
  ["author_by_pk(id: 1)", "one row, or null"],
  ["author_aggregate(where: …)", "aggregate { count sum { … } } and nodes { … }"],
  ["insert_author(objects:, on_conflict:)", "affected_rows and returning { … }"],
  ["insert_author_one(object:, on_conflict:)", "the row written"],
  ["update_author(where:, _set:, _inc:, …)", "affected_rows and returning { … }"],
  ["update_author_by_pk(pk_columns: {id: 1}, _set:)", "the row written"],
  ["update_author_many(updates: [{where, _set, …}])", "one mutation response per update"],
  ["delete_author(where:)", "affected_rows and returning { … }"],
  ["delete_author_by_pk(id: 1)", "the row removed"],
];

const operatorGroups = [
  {
    group: "Any column",
    ops: "_eq _neq _gt _gte _lt _lte _in _nin _is_null",
  },
  {
    group: "Text",
    ops: "_like _nlike _ilike _nilike _similar _nsimilar _regex _iregex _nregex _niregex",
  },
  {
    group: "json / jsonb",
    ops: "_contains _contained_in _has_key _has_keys_any _has_keys_all _jsonb_path_exists _jsonb_path_match _cast",
  },
  {
    group: "ltree",
    ops: "_ancestor _descendant _matches _matches_fulltext, and their _any forms",
  },
  {
    group: "PostGIS",
    ops: "_st_contains _st_crosses _st_equals _st_intersects _st_overlaps _st_touches _st_within _st_d_within _st_3d_d_within _cast",
  },
];

const Code = component$<{ label: string; code: string }>(({ label, code }) => (
  <div class="bg-neutral-900 rounded-xl overflow-hidden">
    <div class="px-4 py-2 bg-neutral-800 border-b border-neutral-700">
      <span class="text-sm text-neutral-400">{label}</span>
    </div>
    <pre class="p-4 text-sm overflow-x-auto">
      <code class="text-neutral-100">{code}</code>
    </pre>
  </div>
));

export default component$(() => {
  return (
    <div class="min-h-screen bg-white">
      <div class="bg-gradient-to-b from-neutral-50 to-white border-b border-neutral-200">
        <div class="container-wide py-12">
          <div class="flex items-center gap-2 text-sm text-neutral-500 mb-4">
            <Link href="/docs" class="hover:text-primary-600">Docs</Link>
            <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 5l7 7-7 7"/>
            </svg>
            <span class="text-neutral-900">GraphQL</span>
          </div>
          <h1 class="text-4xl font-bold text-neutral-900 mb-4">GraphQL API</h1>
          <p class="text-lg text-neutral-600 max-w-2xl">
            A GraphQL API generated from your database schema, in the dialect Hasura speaks. A
            client generated against Hasura — its queries, its codegen output, its endpoint —
            points at this server unchanged.
          </p>
          <p class="mt-4">
            <Link href="/docs/hasura-conformance" class="text-primary-600 hover:underline font-medium">
              How closely the two agree, measured →
            </Link>
          </p>
        </div>
      </div>

      <div class="container-wide py-12">
        <div class="max-w-3xl">
          {/* Endpoints */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">Endpoints</h2>
            <div class="space-y-3">
              {endpoints.map((e) => (
                <div key={e.method + e.path} class="p-4 bg-neutral-50 rounded-lg">
                  <code class="font-mono text-primary-600">{e.method} {e.path}</code>
                  <p class="text-neutral-600 text-sm mt-1">{e.note}</p>
                </div>
              ))}
            </div>
          </section>

          {/* Schema shape */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">Shape of the schema</h2>
            <p class="text-neutral-600 mb-4">
              For a table <code class="font-mono">author</code>, with a to-many relationship{" "}
              <code class="font-mono">articles</code>:
            </p>
            <div class="overflow-x-auto">
              <table class="w-full text-sm">
                <thead>
                  <tr class="border-b border-neutral-200">
                    <th class="text-left py-2 pr-4 font-semibold text-neutral-900">Root field</th>
                    <th class="text-left py-2 font-semibold text-neutral-900">What it answers</th>
                  </tr>
                </thead>
                <tbody>
                  {rootFields.map(([field, answers]) => (
                    <tr key={field} class="border-b border-neutral-100">
                      <td class="py-2 pr-4"><code class="font-mono text-neutral-700 text-xs">{field}</code></td>
                      <td class="py-2 text-neutral-600">{answers}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
            <p class="text-neutral-600 mt-4">
              The root types are named <code class="font-mono">query_root</code>,{" "}
              <code class="font-mono">mutation_root</code> and{" "}
              <code class="font-mono">subscription_root</code>. The subscription root mirrors the
              query root, and each of its fields is a live query.
            </p>
          </section>

          {/* Queries */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">Queries</h2>
            <Code label="GraphQL" code={`query {
  author(
    where: { name: { _ilike: "%rust%" } }
    order_by: [{ created_at: desc_nulls_last }, { name: asc }]
    limit: 10
    offset: 20
  ) {
    id
    name
    articles(where: { published: { _eq: true } }, limit: 5) {
      id
      title
    }
    articles_aggregate {
      aggregate {
        count
      }
    }
  }
}`} />
            <p class="text-neutral-600 mt-4">
              <code class="font-mono">order_by</code> takes a <strong>list</strong> of single-key
              objects, because ordering is ordered:{" "}
              <code class="font-mono">{`{name: asc, id: desc}`}</code> is one object whose two keys
              have no defined precedence, and the client that wrote it meant name first.
            </p>
          </section>

          {/* Filtering */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">Filtering</h2>
            <p class="text-neutral-600 mb-4">
              <code class="font-mono">where</code> takes a generated{" "}
              <code class="font-mono">&lt;table&gt;_bool_exp</code>. Every comparison is named for
              the type it applies to, so an unknown operator or an ill-typed operand is refused by
              validation rather than by the database.
            </p>
            <div class="space-y-3 mb-4">
              {operatorGroups.map((g) => (
                <div key={g.group} class="p-4 bg-neutral-50 rounded-lg">
                  <h3 class="font-semibold text-neutral-900 text-sm mb-1">{g.group}</h3>
                  <code class="font-mono text-xs text-neutral-600">{g.ops}</code>
                </div>
              ))}
            </div>
            <p class="text-neutral-600 mb-4">
              Combine them with <code class="font-mono">_and</code>,{" "}
              <code class="font-mono">_or</code> and <code class="font-mono">_not</code>, and
              follow a relationship by naming it. A question can also be asked about a whole
              related set rather than any one row of it:
            </p>
            <Code label="GraphQL" code={`{
  authors(where: { books_aggregate: { count: { predicate: { _gt: 2 } } } }) {
    name
  }
}`} />
            <p class="text-neutral-600 mt-4">
              Over no related rows at all <code class="font-mono">count</code> is zero and the
              boolean folds are null — which is how &ldquo;authors with no books&rdquo; is written.
            </p>
          </section>

          {/* Mutations */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">Mutations</h2>
            <Code label="GraphQL" code={`mutation {
  insert_author(
    objects: [{ name: "Ada", articles: { data: [{ title: "On engines" }] } }]
    on_conflict: { constraint: author_name_key, update_columns: [name] }
  ) {
    affected_rows
    returning { id name }
  }

  update_article_by_pk(pk_columns: { id: 7 }, _set: { published: true }) {
    id
  }

  delete_article(where: { views: { _lt: 10 } }) {
    affected_rows
  }
}`} />
            <p class="text-neutral-600 mt-4">
              Nested writes, upserts with <code class="font-mono">on_conflict</code>,{" "}
              <code class="font-mono">update_many</code> and the document operators are all
              supported. A mutation naming several root fields runs them in one transaction: if any
              fails, none of them happened.
            </p>
          </section>

          {/* Subscriptions */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">Subscriptions</h2>
            <p class="text-neutral-600 mb-4">
              Each subscription field is a live query — the answer now, and the answer again
              whenever it changes. Connect to{" "}
              <code class="font-mono">ws://localhost:3000/v1/graphql/ws</code> using the
              graphql-transport-ws protocol.
            </p>
            <Code label="GraphQL" code={`subscription {
  article(where: { published: { _eq: true } }, order_by: [{ created_at: desc }], limit: 10) {
    id
    title
  }
}`} />
            <p class="text-neutral-600 mt-4">
              The cursor-based half of Hasura&rsquo;s subscription surface —{" "}
              <code class="font-mono">_stream</code> — is not implemented. See{" "}
              <Link href="/docs/realtime" class="text-primary-600 hover:underline">Realtime</Link>.
            </p>
          </section>

          {/* Auth */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">Authentication</h2>
            <p class="text-neutral-600 mb-4">
              A caller holding the admin secret is an administrator, and an administrator may ask
              to be treated as someone else. Other <code class="font-mono">x-hasura-*</code>{" "}
              headers become session variables that a row-level policy can read.
            </p>
            <Code label="bash" code={`curl localhost:3000/v1/graphql \\
  -H 'X-Hasura-Admin-Secret: shh' \\
  -H 'X-Hasura-Role: user' \\
  -H 'X-Hasura-User-Id: 1' \\
  -d '{"query":"{ article { id title } }"}'`} />
            <p class="text-neutral-600 mt-4">
              With a token instead, the role comes from{" "}
              <code class="font-mono">x-hasura-default-role</code>, and an{" "}
              <code class="font-mono">X-Hasura-Role</code> header may select any role listed in the
              token&rsquo;s <code class="font-mono">x-hasura-allowed-roles</code>. That list sits
              inside the signature, so a caller may choose among the identities it was issued and
              cannot add one.
            </p>
            <p class="text-neutral-600 mt-4">
              With <strong>no admin secret configured</strong>,{" "}
              <code class="font-mono">x-hasura-*</code> headers are ignored entirely and session
              variables come only from a verified token. This is a deliberate divergence from
              Hasura, which treats every caller on an unsecured deployment as an administrator. See{" "}
              <Link href="/docs/configuration" class="text-primary-600 hover:underline">Configuration</Link>.
            </p>
          </section>

          {/* Errors */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">Errors</h2>
            <p class="text-neutral-600 mb-4">
              Errors come back in Hasura&rsquo;s envelope, which client code branches on. There is
              no <code class="font-mono">data</code> key on failure, and the path names a place in
              the <em>request</em> rather than in the response.
            </p>
            <Code label="JSON" code={`{
  "errors": [
    {
      "message": "field 'titel' not found in type: 'article'",
      "extensions": {
        "path": "$.selectionSet.article.selectionSet.titel",
        "code": "validation-failed"
      }
    }
  ]
}`} />
          </section>

          {/* Example */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">Example request</h2>
            <Code label="bash" code={`curl -X POST http://localhost:3000/v1/graphql \\
  -H "Content-Type: application/json" \\
  -d '{"query": "{ article(limit: 5) { id title author { name } } }"}'`} />
          </section>

          <div class="flex items-center justify-between pt-8 border-t border-neutral-200">
            <Link href="/docs/api-reference" class="flex items-center gap-2 text-neutral-600 hover:text-primary-600">
              <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 19l-7-7 7-7"/>
              </svg>
              API Reference
            </Link>
            <Link href="/docs/hasura-conformance" class="flex items-center gap-2 text-neutral-600 hover:text-primary-600">
              Hasura Conformance
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
  title: "GraphQL - Postrust Documentation",
  meta: [
    {
      name: "description",
      content:
        "A GraphQL API generated from your PostgreSQL schema, in the dialect Hasura speaks: queries, mutations, live subscriptions, filtering, and Hasura's error envelope.",
    },
  ],
};
