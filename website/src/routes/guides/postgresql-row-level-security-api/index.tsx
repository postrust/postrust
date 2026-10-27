import { component$ } from "@builder.io/qwik";
import type { DocumentHead } from "@builder.io/qwik-city";
import { Link } from "@builder.io/qwik-city";
import { Code } from "~/components/blog/code";
import { GuidePage, guideHead } from "~/components/guides/guide-page";
import { guideBySlug } from "~/data/guides";

const guide = guideBySlug("postgresql-row-level-security-api");

const h2 =
  "mb-5 mt-16 text-2xl font-bold tracking-tight text-neutral-950 md:text-[1.75rem]";
const h3 = "mb-3 mt-10 text-lg font-semibold text-neutral-900";
const p = "mb-5 text-[1.0625rem] leading-8 text-neutral-700";
const lead = "mb-5 text-[1.0625rem] leading-8 font-medium text-neutral-900";
const ul =
  "mb-6 list-disc space-y-3 pl-6 text-[1.0625rem] leading-8 text-neutral-700";
const a =
  "font-medium text-primary-700 underline decoration-primary-300 underline-offset-2 hover:decoration-primary-600";
const code = "font-mono";
const th = "px-6 py-4 text-left font-medium text-neutral-900";
const td = "px-6 py-4 align-top text-neutral-700";

const PG = "https://www.postgresql.org/docs/current";

const ext = (href: string, label: string) => (
  <a href={href} target="_blank" rel="noopener noreferrer" class={a}>
    {label}
  </a>
);

const models = [
  {
    q: "Where the rules live",
    db: "In the database: GRANTs on tables and columns, RLS policies on rows",
    meta: "In the API server's configuration, per role and per table or field",
  },
  {
    q: "Who enforces them",
    db: "PostgreSQL, on every statement the role runs",
    meta: "The API server, by adding filters to the SQL it generates",
  },
  {
    q: "Other clients (psql, jobs, BI tools)",
    db: "Subject to the same rules if they connect as, or switch to, the same roles",
    meta: "Not covered; they need their own controls",
  },
  {
    q: "Per-field rules",
    db: "Column GRANTs; finer shaping needs views",
    meta: "Usually first-class, alongside presets and input checks",
  },
  {
    q: "How changes ship",
    db: "As SQL migrations, reviewed and versioned with the schema",
    meta: "As configuration or metadata, often edited through a console",
  },
  {
    q: "Debugging a denied or empty result",
    db: "Harder: a filtered row is silently absent",
    meta: "Rules are visible in one place in the API tool",
  },
];

export default component$(() => (
  <GuidePage
    guide={guide}
    answer="Most teams put the access rules in PostgreSQL itself: the API verifies a JWT, switches the transaction to the PostgreSQL role the token names, and exposes the token's claims as settings that row-level security (RLS) policies read. GRANTs then decide which tables and columns a role can touch and policies decide which rows, so every request, whichever API or client sends it, is held to the same rules. The alternative is to keep permissions as metadata in the API layer, which is easier to edit per field but covers only traffic through that API."
    faq={[
      {
        q: "Does row-level security slow down API queries?",
        a: "A policy is a predicate added to the query, so its cost is that of the predicate. Index the columns policies filter on, and wrap current_setting calls in a scalar subquery so PostgreSQL evaluates them once per statement rather than once per row.",
      },
      {
        q: "Why does my RLS policy not apply when I query as the table owner?",
        a: "Table owners bypass RLS unless the table has FORCE ROW LEVEL SECURITY, and superusers and roles with BYPASSRLS always bypass it. Test as the role the API switches to, not as the role that ran the migration.",
      },
      {
        q: "How do I read JWT claims inside a PostgreSQL policy with Postrust?",
        a: "Postrust sets request.jwt.claims to the whole verified claims object as JSON, and each claim with a valid setting name as request.jwt.claims.<name>. Read them with current_setting, for example current_setting('request.jwt.claims.sub', true).",
      },
      {
        q: "What happens to a request with no token?",
        a: "Postrust runs it as the role named by PGRST_DB_ANON_ROLE. If no anonymous role is configured, the request is refused.",
      },
      {
        q: "Should I use RLS or Hasura-style metadata permissions?",
        a: "Use RLS when more than one client reaches the database or you want rules reviewed as SQL migrations. Metadata permissions suit teams that want per-field rules and presets managed in the API tool and whose only path to the data is that API.",
      },
    ]}
  >
    <h2 class={h2}>
      Should access control live in the database or in the API layer?
    </h2>
    <p class={lead}>
      If the API is not the only thing that reads the database, put the rules in
      the database. Rules held by the API protect only requests that pass
      through it.
    </p>
    <p class={p}>
      There are two established models. In the first, used by PostgREST and
      Postrust, the API connects as a role with no privileges of its own and
      becomes a different PostgreSQL role for each request; PostgreSQL&rsquo;s
      GRANTs and {ext(`${PG}/ddl-rowsecurity.html`, "row security policies")}{" "}
      then decide what that request may see. In the second, used by Hasura,
      permissions are metadata kept by the API server and compiled into the SQL
      it generates. Both are reasonable; they put the rules in different places.
    </p>
    <div class="mb-8 overflow-x-auto rounded-xl border border-neutral-200">
      <table class="w-full text-sm">
        <thead class="bg-neutral-50">
          <tr>
            <th class={th}></th>
            <th class="text-primary-600 px-6 py-4 text-left font-medium">
              Database-enforced (roles + RLS)
            </th>
            <th class={th}>API-layer metadata</th>
          </tr>
        </thead>
        <tbody class="divide-y divide-neutral-200">
          {models.map((row) => (
            <tr key={row.q}>
              <td class="px-6 py-4 align-top font-medium text-neutral-900">
                {row.q}
              </td>
              <td class={td}>{row.db}</td>
              <td class={td}>{row.meta}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
    <p class={p}>
      Postrust supports both on its GraphQL surface: Hasura&rsquo;s permission
      model read from a JSON document, or PostgreSQL roles and RLS (see the{" "}
      <Link href="/compare/hasura/" class={a}>
        Hasura comparison
      </Link>
      ). The rest of this guide is about the database-enforced model, which is
      the one that covers REST and GraphQL alike.
    </p>

    <h2 class={h2}>
      How does a JWT become a PostgreSQL role for each request?
    </h2>
    <p class={lead}>
      The server verifies the token, opens a transaction, sets the role the
      token&rsquo;s <code class={code}>role</code> claim names, and stores the
      claims as transaction-local settings. The query then runs as that role.
    </p>
    <p class={p}>
      In Postrust each request runs in one transaction. Before the query, the
      role and the request&rsquo;s settings are applied with{" "}
      <code class={code}>SET LOCAL ROLE</code> and{" "}
      <code class={code}>set_config(name, value, true)</code>, both scoped to
      that transaction, so nothing leaks to the next request on the same pooled
      connection. The claims are available two ways:
    </p>
    <ul class={ul}>
      <li>
        <code class={code}>request.jwt.claims</code> &mdash; the whole verified
        claims object as JSON.
      </li>
      <li>
        <code class={code}>request.jwt.claims.&lt;name&gt;</code> &mdash; each
        claim whose name is a valid setting name, as text, so a policy need not
        parse JSON.
      </li>
    </ul>
    <p class={p}>
      REST requests also get <code class={code}>request.method</code>,{" "}
      <code class={code}>request.path</code>,{" "}
      <code class={code}>request.headers</code> and{" "}
      <code class={code}>request.cookies</code>. The claim that names the role
      is <code class={code}>role</code> unless{" "}
      <code class={code}>PGRST_JWT_ROLE_CLAIM_KEY</code> says otherwise, and a
      request with no token runs as <code class={code}>PGRST_DB_ANON_ROLE</code>
      . The{" "}
      <Link href="/docs/authentication/" class={a}>
        authentication docs
      </Link>{" "}
      cover how <code class={code}>exp</code>, <code class={code}>nbf</code> and{" "}
      <code class={code}>aud</code> are checked, and{" "}
      <Link href="/docs/configuration/" class={a}>
        configuration
      </Link>{" "}
      lists every setting.
    </p>
    <Code
      label=".env"
      lang="bash"
      code={`DATABASE_URL=postgres://authenticator:change-me@db:5432/app
PGRST_DB_ANON_ROLE=web_anon
PGRST_JWT_SECRET=a-secret-of-at-least-32-characters-long`}
    />

    <h2 class={h2}>Which roles and GRANTs does a generated API need?</h2>
    <p class={lead}>
      Three kinds: a login role the server connects as, an anonymous role, and
      one or more roles for signed-in callers. GRANTs decide which tables each
      may touch; policies come after.
    </p>
    <Code
      label="roles.sql"
      lang="sql"
      code={`-- The server logs in as this. It can do nothing but become the others.
CREATE ROLE authenticator LOGIN NOINHERIT PASSWORD 'change-me';

CREATE ROLE web_anon NOLOGIN;
CREATE ROLE app_user NOLOGIN;
GRANT web_anon, app_user TO authenticator;

GRANT USAGE ON SCHEMA public TO web_anon, app_user;
GRANT SELECT ON public.articles TO web_anon;
GRANT SELECT, INSERT, UPDATE, DELETE ON public.projects TO app_user;`}
    />
    <p class={p}>
      A role the token names but the database does not know fails at the{" "}
      <code class={code}>SET ROLE</code>, so a forged or mistyped role cannot
      fall through to broader access. Keep <code class={code}>web_anon</code>
      &rsquo;s grants small: anything granted to it is public.
    </p>

    <h2 class={h2}>
      How do you write RLS policies for users, tenants and teams?
    </h2>
    <p class={lead}>
      Enable RLS on the table, then write policies that compare row columns with
      claims from the token. With RLS enabled and no policy, the role sees no
      rows at all.
    </p>
    <p class={p}>
      Small helper functions keep policies readable. The{" "}
      <code class={code}>nullif</code> matters: a custom setting a pooled
      connection has seen before reads back as an empty string, not NULL, when
      it is not set.
    </p>
    <Code
      label="helpers.sql"
      lang="sql"
      code={`CREATE SCHEMA auth;
GRANT USAGE ON SCHEMA auth TO web_anon, app_user;

CREATE FUNCTION auth.user_id() RETURNS text
  LANGUAGE sql STABLE
  AS $$ SELECT nullif(current_setting('request.jwt.claims.sub', true), '') $$;

CREATE FUNCTION auth.tenant_id() RETURNS uuid
  LANGUAGE sql STABLE
  AS $$ SELECT nullif(current_setting('request.jwt.claims.tenant_id', true), '')::uuid $$;`}
    />

    <h3 class={h3}>Per-user rows</h3>
    <Code
      label="notes.sql"
      lang="sql"
      code={`ALTER TABLE notes ENABLE ROW LEVEL SECURITY;

CREATE POLICY own_notes ON notes
  FOR ALL TO app_user
  USING (owner_id = (SELECT auth.user_id()))
  WITH CHECK (owner_id = (SELECT auth.user_id()));`}
    />
    <p class={p}>
      <code class={code}>USING</code> filters the rows a statement can see or
      change; <code class={code}>WITH CHECK</code> rejects rows an insert or
      update would write. PostgreSQL reuses <code class={code}>USING</code> as
      the check when it is omitted from a <code class={code}>FOR ALL</code>{" "}
      policy, but spelling it out keeps the write rule visible in review. The
      scalar subquery around the function lets PostgreSQL evaluate it once per
      statement rather than once per row.
    </p>

    <h3 class={h3}>Tenant isolation</h3>
    <Code
      label="tenants.sql"
      lang="sql"
      code={`ALTER TABLE invoices ENABLE ROW LEVEL SECURITY;

CREATE POLICY tenant_isolation ON invoices
  FOR ALL TO app_user
  USING (tenant_id = (SELECT auth.tenant_id()))
  WITH CHECK (tenant_id = (SELECT auth.tenant_id()));

CREATE INDEX ON invoices (tenant_id);`}
    />
    <p class={p}>
      The tenant comes from a signed claim, never from a header or query
      parameter the caller chooses. A token without the claim yields NULL, which
      matches no row.
    </p>

    <h3 class={h3}>Team or organisation membership</h3>
    <Code
      label="teams.sql"
      lang="sql"
      code={`CREATE POLICY team_projects ON projects
  FOR SELECT TO app_user
  USING (team_id IN (
    SELECT team_id FROM memberships WHERE user_id = (SELECT auth.user_id())
  ));

CREATE INDEX ON memberships (user_id, team_id);
CREATE INDEX ON projects (team_id);`}
    />
    <p class={p}>
      The subquery runs as the caller, so <code class={code}>app_user</code>{" "}
      needs SELECT on <code class={code}>memberships</code>, and any policy on{" "}
      <code class={code}>memberships</code> applies inside it. If{" "}
      <code class={code}>memberships</code>&rsquo;s own policy refers back to{" "}
      <code class={code}>projects</code>, PostgreSQL reports infinite recursion;
      the usual fix is a <code class={code}>SECURITY DEFINER</code> function
      that returns the caller&rsquo;s team ids, written with the care described
      below.
    </p>

    <h2 class={h2}>
      Does the same policy protect REST and GraphQL in Postrust?
    </h2>
    <p class={lead}>
      Yes. Both surfaces run each query in a transaction that has done{" "}
      <code class={code}>SET LOCAL ROLE</code> and set{" "}
      <code class={code}>request.jwt.claims</code>, so one policy answers both.
    </p>
    <Code
      label="the same rows, two ways"
      lang="bash"
      code={`curl 'http://localhost:3000/projects?select=id,name' \\
  -H "Authorization: Bearer $TOKEN"

curl http://localhost:3000/v1/graphql \\
  -H "Authorization: Bearer $TOKEN" \\
  -H 'content-type: application/json' \\
  -d '{"query":"{ projects { id name } }"}'`}
    />
    <p class={p}>
      That includes GraphQL subscriptions, which authenticate once from the
      WebSocket&rsquo;s <code class={code}>connection_init</code> payload. The
      GraphQL surface additionally exposes Hasura session variables, so a policy
      written for Hasura can read{" "}
      <code class={code}>current_setting(&#39;hasura.user_id&#39;)</code>.
      Without an admin secret configured, Postrust ignores{" "}
      <code class={code}>x-hasura-*</code> headers and takes session variables
      only from a verified token, so a caller cannot choose the value a policy
      compares against. The{" "}
      <Link href="/docs/graphql/" class={a}>
        GraphQL docs
      </Link>{" "}
      cover the rest, and the{" "}
      <Link href="/blog/postgresql-apollo-federation-subgraph/" class={a}>
        Apollo Federation walkthrough
      </Link>{" "}
      shows a policy deciding what each caller sees through a router.
    </p>
    <p class={p}>
      For a check that is not about rows &mdash; a suspended account, say
      &mdash; <code class={code}>PGRST_DB_PRE_REQUEST</code> names a function
      Postrust calls before each REST request&rsquo;s query, in the same
      transaction; if it raises, the request is refused.
    </p>

    <h2 class={h2}>What mistakes break row-level security behind an API?</h2>
    <p class={lead}>
      Most failures are silent: the query succeeds against the wrong rows. These
      are the ones to look for in review.
    </p>
    <ul class={ul}>
      <li>
        <strong>
          Policies without <code class={code}>ENABLE ROW LEVEL SECURITY</code>.
        </strong>{" "}
        <code class={code}>CREATE POLICY</code> on a table without RLS enabled
        does nothing. Query <code class={code}>pg_class.relrowsecurity</code> to
        list exposed tables that lack it.
      </li>
      <li>
        <strong>Testing as the owner.</strong> The table owner bypasses RLS
        unless the table is set to{" "}
        <code class={code}>FORCE ROW LEVEL SECURITY</code>; superusers and{" "}
        <code class={code}>BYPASSRLS</code> roles always do. Never let the
        API&rsquo;s login role own tables.
      </li>
      <li>
        <strong>
          <code class={code}>SECURITY DEFINER</code> functions.
        </strong>{" "}
        They run as their owner, usually past RLS. Give them a fixed{" "}
        <code class={code}>search_path</code>, filter by the caller explicitly,
        and revoke the default EXECUTE from PUBLIC (see{" "}
        {ext(`${PG}/sql-createfunction.html`, "CREATE FUNCTION")}).
      </li>
      <li>
        <strong>Views.</strong> A view checks permissions as its owner unless
        created with <code class={code}>security_invoker</code> (PostgreSQL 15
        and later), so an exposed view can step around the policies on its
        tables.
      </li>
      <li>
        <strong>Unindexed policy columns.</strong> The policy predicate is part
        of every query; without an index on <code class={code}>tenant_id</code>{" "}
        or <code class={code}>owner_id</code> each request scans the table.
      </li>
      <li>
        <strong>Read rules that also govern writes.</strong> A{" "}
        <code class={code}>FOR ALL</code> policy written with reads in mind,
        such as <code class={code}>USING (published)</code>, also decides which
        rows a role may insert and update. Give writes their own{" "}
        <code class={code}>FOR INSERT</code> and{" "}
        <code class={code}>FOR UPDATE</code> policies when the rules differ.
      </li>
    </ul>

    <h2 class={h2}>How do you test RLS policies before they ship?</h2>
    <p class={lead}>
      Reproduce what the API does &mdash; set the claims, switch role &mdash;
      inside a transaction you roll back, and assert on what comes back.
    </p>
    <Code
      label="policy-test.sql"
      lang="sql"
      code={`BEGIN;
SELECT set_config('request.jwt.claims.sub', 'user-1', true),
       set_config('request.jwt.claims.tenant_id',
                  '6f1c2e9a-0000-4000-8000-000000000001', true);
SET LOCAL ROLE app_user;

SELECT count(*) FROM invoices;              -- only tenant ...0001
SELECT count(*) FROM notes WHERE owner_id <> 'user-1';  -- expect 0
ROLLBACK;`}
    />
    <p class={p}>
      Run tests like these in CI against a real PostgreSQL &mdash; tools such as
      pgTAP make the assertions tidy &mdash; and include the negative cases:
      another tenant&rsquo;s id, a token with no tenant claim, and the anonymous
      role. PostgreSQL&rsquo;s{" "}
      {ext(`${PG}/sql-createpolicy.html`, "CREATE POLICY reference")} explains
      how multiple permissive and restrictive policies combine, which is worth
      reading before a table gets its second policy.
    </p>
  </GuidePage>
));

export const head: DocumentHead = guideHead(guide);
