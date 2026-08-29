import { component$ } from "@builder.io/qwik";
import type { DocumentHead } from "@builder.io/qwik-city";
import { Link } from "@builder.io/qwik-city";

const configVars = [
  {
    category: "Database",
    vars: [
      {
        name: "DATABASE_URL",
        required: true,
        default: "-",
        desc: "PostgreSQL connection string",
      },
      {
        name: "PGRST_DB_SCHEMAS",
        required: false,
        default: "public",
        desc: "Schemas to expose (comma-separated)",
      },
      {
        name: "PGRST_DB_ANON_ROLE",
        required: false,
        default: "-",
        desc: "Role for unauthenticated requests",
      },
      {
        name: "PGRST_DB_POOL_SIZE",
        required: false,
        default: "10",
        desc: "Connection pool size",
      },
    ],
  },
  {
    category: "Authentication",
    vars: [
      {
        name: "PGRST_JWT_SECRET",
        required: false,
        default: "-",
        desc: "JWT signing secret",
      },
      {
        name: "PGRST_JWT_SECRET_IS_BASE64",
        required: false,
        default: "false",
        desc: "If secret is base64 encoded",
      },
      {
        name: "PGRST_JWT_AUD",
        required: false,
        default: "-",
        desc: "Required JWT audience claim",
      },
      {
        name: "PGRST_JWT_ROLE_CLAIM_KEY",
        required: false,
        default: "role",
        desc: "Claim key for role",
      },
    ],
  },
  {
    category: "Server",
    vars: [
      {
        name: "PGRST_SERVER_HOST",
        required: false,
        default: "127.0.0.1",
        desc: "Bind address",
      },
      {
        name: "PGRST_SERVER_PORT",
        required: false,
        default: "3000",
        desc: "Port to listen on",
      },
      {
        name: "PGRST_SERVER_CORS_ORIGINS",
        required: false,
        default: "*",
        desc: "CORS allowed origins",
      },
    ],
  },
  {
    category: "Limits",
    vars: [
      {
        name: "PGRST_MAX_ROWS",
        required: false,
        default: "unlimited",
        desc: "Maximum rows returned by one request; caps requests with no limit",
      },
      {
        name: "PGRST_MAX_BODY_SIZE",
        required: false,
        default: "10485760",
        desc: "Max request body in bytes",
      },
    ],
  },
  {
    category: "Compatibility",
    vars: [
      {
        name: "PGRST_COMPAT_MODE",
        required: false,
        default: "false",
        desc: "PostgREST compatibility mode: serves the REST API at the root (/rpc/fn, /table) in addition to /api, and un-wraps RPC responses to PostgREST's shape. Object key order is a build-time choice, not covered by this setting - see below. Alias: POSTRUST_COMPAT_MODE",
      },
    ],
  },
  {
    category: "Hasura Authentication",
    vars: [
      {
        name: "PGRST_HASURA_ADMIN_SECRET",
        required: false,
        default: "-",
        desc: "Shared secret authenticating an administrator. A caller holding it may ask to be treated as any role. Alias: HASURA_GRAPHQL_ADMIN_SECRET",
      },
      {
        name: "PGRST_HASURA_UNAUTHORIZED_ROLE",
        required: false,
        default: "-",
        desc: "Role for a request nothing authenticated. Unset means such a request is refused, which is the default. Alias: HASURA_GRAPHQL_UNAUTHORIZED_ROLE",
      },
    ],
  },
  {
    category: "GraphQL Names and Permissions",
    vars: [
      {
        name: "PGRST_GRAPHQL_METADATA",
        required: false,
        default: "-",
        desc: "Names for tables, columns, root fields, relationships and computed fields that the schema cannot supply; which root a function is exposed on; and what each role may do with each table. A JSON document, or a path to a file holding one. Unset means every name is derived and there is no permission layer. Also read as PGRST_GRAPHQL_NAMES, which is what it was called when names were all it carried.",
      },
    ],
  },
  {
    category: "Logging",
    vars: [
      {
        name: "PGRST_LOG_LEVEL",
        required: false,
        default: "info",
        desc: "Log level (error, warn, info, debug)",
      },
      {
        name: "RUST_LOG",
        required: false,
        default: "-",
        desc: "Detailed tracing configuration",
      },
    ],
  },
];

export default component$(() => {
  return (
    <div class="min-h-screen bg-white">
      <div class="border-b border-neutral-200 bg-gradient-to-b from-neutral-50 to-white">
        <div class="container-wide py-12">
          <div class="mb-4 flex items-center gap-2 text-sm text-neutral-500">
            <Link href="/docs" class="hover:text-primary-600">
              Docs
            </Link>
            <svg
              class="h-4 w-4"
              fill="none"
              stroke="currentColor"
              viewBox="0 0 24 24"
            >
              <path
                stroke-linecap="round"
                stroke-linejoin="round"
                stroke-width="2"
                d="M9 5l7 7-7 7"
              />
            </svg>
            <span class="text-neutral-900">Configuration</span>
          </div>
          <h1 class="mb-4 text-4xl font-bold text-neutral-900">
            Configuration
          </h1>
          <p class="max-w-2xl text-lg text-neutral-600">
            All environment variables and configuration options for Postrust.
          </p>
        </div>
      </div>

      <div class="container-wide py-12">
        <div class="max-w-4xl">
          {configVars.map((category) => (
            <section key={category.category} class="mb-12">
              <h2 class="mb-6 text-2xl font-bold text-neutral-900">
                {category.category}
              </h2>
              <div class="overflow-x-auto">
                <table class="w-full text-sm">
                  <thead>
                    <tr class="border-b border-neutral-200">
                      <th class="px-4 py-3 text-left font-semibold text-neutral-900">
                        Variable
                      </th>
                      <th class="px-4 py-3 text-left font-semibold text-neutral-900">
                        Required
                      </th>
                      <th class="px-4 py-3 text-left font-semibold text-neutral-900">
                        Default
                      </th>
                      <th class="px-4 py-3 text-left font-semibold text-neutral-900">
                        Description
                      </th>
                    </tr>
                  </thead>
                  <tbody class="divide-y divide-neutral-100">
                    {category.vars.map((v) => (
                      <tr key={v.name}>
                        <td class="text-primary-600 px-4 py-3 font-mono text-xs">
                          {v.name}
                        </td>
                        <td class="px-4 py-3">
                          {v.required ? (
                            <span class="text-red-600">Yes</span>
                          ) : (
                            <span class="text-neutral-400">No</span>
                          )}
                        </td>
                        <td class="px-4 py-3 font-mono text-xs text-neutral-500">
                          {v.default}
                        </td>
                        <td class="px-4 py-3 text-neutral-600">{v.desc}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </section>
          ))}

          {/* Example */}
          <section class="mb-12">
            <h2 class="mb-4 text-2xl font-bold text-neutral-900">
              Key ordering is a build-time choice
            </h2>
            <p class="mb-4 text-neutral-600">
              Postrust returns the keys within each object alphabetically.
              PostgREST returns them in the order of the{" "}
              <code class="rounded bg-neutral-100 px-1 py-0.5 text-sm">
                select
              </code>{" "}
              list. That difference is decided when the binary is compiled
              rather than at run time, because it depends on the map type
              holding a JSON object, so{" "}
              <code class="rounded bg-neutral-100 px-1 py-0.5 text-sm">
                PGRST_COMPAT_MODE
              </code>{" "}
              cannot switch it on. It is a Cargo feature:
            </p>
            <div class="mb-4 overflow-hidden rounded-xl bg-neutral-900">
              <div class="border-b border-neutral-700 bg-neutral-800 px-4 py-2">
                <span class="text-sm text-neutral-400">Terminal</span>
              </div>
              <pre class="overflow-x-auto p-4 text-sm">
                <code class="text-neutral-100">{`cargo build --release -p postrust-server --features compat-key-order

# Default build
curl 'localhost:3000/api/users?select=status,name,id&limit=1'
# -> [{"id":1,"name":"Alice","status":"active"}]

# With compat-key-order
curl 'localhost:3000/api/users?select=status,name,id&limit=1'
# -> [{"status":"active","name":"Alice","id":1}]`}</code>
              </pre>
            </div>
            <p class="mb-4 text-neutral-600">
              It is off by default because it is not free. Measured by running
              both builds as containers against the same database and
              alternating between them, a three-column response cost 1% and an
              eight-column response 15%: for objects this small a few short
              string comparisons beat hashing every key, so the sorted map is
              genuinely the faster one. Turn it on when byte-level compatibility
              matters more.
            </p>
            <p class="text-neutral-600">
              Running with{" "}
              <code class="rounded bg-neutral-100 px-1 py-0.5 text-sm">
                PGRST_COMPAT_MODE=true
              </code>{" "}
              on a binary built without the feature logs a warning at startup,
              so the difference is not left to be found by diffing responses.
            </p>
          </section>

          <section class="mb-12">
            <h2 class="mb-4 text-2xl font-bold text-neutral-900">
              Hasura authentication
            </h2>
            <p class="mb-4 text-neutral-600">
              The GraphQL surface reads Hasura&rsquo;s headers, so a deployment migrating from
              Hasura keeps sending what it already sends. Both spellings of each variable are
              read, so an existing <code class="font-mono">HASURA_GRAPHQL_*</code> environment
              needs no renaming.
            </p>
            <p class="mb-4 text-neutral-600">
              Set the secret, and a caller that holds it is an administrator — and an
              administrator may ask to be treated as someone else:
            </p>
            <div class="mb-4 overflow-hidden rounded-xl bg-neutral-900">
              <div class="border-b border-neutral-700 bg-neutral-800 px-4 py-2">
                <span class="text-sm text-neutral-400">bash</span>
              </div>
              <pre class="overflow-x-auto p-4 text-sm">
                <code class="text-neutral-100">{`curl localhost:3000/v1/graphql \\
  -H 'X-Hasura-Admin-Secret: shh' \\
  -H 'X-Hasura-Role: user' \\
  -H 'X-Hasura-User-Id: 1' \\
  -d '{"query":"{ article { id title } }"}'`}</code>
              </pre>
            </div>
            <p class="mb-4 text-neutral-600">
              That request is answered as <code class="font-mono">user</code>, and{" "}
              <code class="font-mono">x-hasura-user-id</code> becomes a session variable a
              row-level policy reads as{" "}
              <code class="font-mono">current_setting(&#39;hasura.user_id&#39;)</code> — or that a
              function taking <code class="font-mono">hasura_session json</code> receives whole.
              A Hasura role is not a database role: <code class="font-mono">Artist</code> and{" "}
              <code class="font-mono">anonymous</code> need not exist in any catalogue.
            </p>

            <h3 class="mb-2 mt-6 text-lg font-semibold text-neutral-900">
              Choosing a role with a token
            </h3>
            <p class="mb-4 text-neutral-600">
              A token that allows more than one identity carries{" "}
              <code class="font-mono">x-hasura-default-role</code> — who the caller is when it asks
              for nothing — beside <code class="font-mono">x-hasura-allowed-roles</code>, the list
              it may ask for instead. The asking is done with an{" "}
              <code class="font-mono">X-Hasura-Role</code> header, and no admin secret is needed
              for it. That list sits inside the signature, so a caller may choose among the
              identities it was issued and cannot add one. Asking for a role the token does not
              list is refused. A token carrying no list allows only the role it already names.
            </p>

            <h3 class="mb-2 mt-6 text-lg font-semibold text-neutral-900">
              One place this deliberately differs from Hasura
            </h3>
            <p class="mb-4 text-neutral-600">
              Hasura with no admin secret configured treats every caller as an administrator —
              which also means an unsecured deployment lets any caller name its own role and its
              own identity. Postrust does not: with no secret configured,{" "}
              <code class="font-mono">x-hasura-*</code> headers carry no weight and session
              variables come only from a verified token. A policy reading a value the caller chose
              is not a policy, and the failure is silent — the query succeeds, against the wrong
              rows.
            </p>
            <p class="text-neutral-600">
              See{" "}
              <Link href="/docs/hasura-conformance" class="text-primary-600 hover:underline">
                Hasura conformance
              </Link>{" "}
              for how much of the dialect this covers, measured.
            </p>
          </section>

          <section class="mb-12">
            <h2 class="mb-4 text-2xl font-bold text-neutral-900">
              Example Configuration
            </h2>
            <div class="overflow-hidden rounded-xl bg-neutral-900">
              <div class="border-b border-neutral-700 bg-neutral-800 px-4 py-2">
                <span class="text-sm text-neutral-400">.env</span>
              </div>
              <pre class="overflow-x-auto p-4 text-sm">
                <code class="text-neutral-100">{`# Required
DATABASE_URL=postgres://user:password@localhost:5432/mydb

# Authentication
PGRST_DB_ANON_ROLE=web_anon
PGRST_JWT_SECRET=your-secret-key-at-least-32-characters

# Hasura-dialect GraphQL at /v1/graphql
PGRST_HASURA_ADMIN_SECRET=shh
PGRST_HASURA_UNAUTHORIZED_ROLE=anonymous

# Server
PGRST_SERVER_HOST=0.0.0.0
PGRST_SERVER_PORT=3000
PGRST_SERVER_CORS_ORIGINS=https://myapp.com

# Limits
PGRST_MAX_ROWS=100
PGRST_LOG_LEVEL=info`}</code>
              </pre>
            </div>
          </section>

          {/* Next */}
          <div class="flex items-center justify-between border-t border-neutral-200 pt-8">
            <Link
              href="/docs/authentication"
              class="hover:text-primary-600 flex items-center gap-2 text-neutral-600"
            >
              <svg
                class="h-4 w-4"
                fill="none"
                stroke="currentColor"
                viewBox="0 0 24 24"
              >
                <path
                  stroke-linecap="round"
                  stroke-linejoin="round"
                  stroke-width="2"
                  d="M15 19l-7-7 7-7"
                />
              </svg>
              Authentication
            </Link>
            <Link
              href="/docs/deployment"
              class="hover:text-primary-600 flex items-center gap-2 text-neutral-600"
            >
              Deployment
              <svg
                class="h-4 w-4"
                fill="none"
                stroke="currentColor"
                viewBox="0 0 24 24"
              >
                <path
                  stroke-linecap="round"
                  stroke-linejoin="round"
                  stroke-width="2"
                  d="M9 5l7 7-7 7"
                />
              </svg>
            </Link>
          </div>
        </div>
      </div>
    </div>
  );
});

export const head: DocumentHead = {
  title: "Configuration - Postrust Documentation",
  meta: [
    {
      name: "description",
      content:
        "All configuration options and environment variables for Postrust.",
    },
  ],
};
