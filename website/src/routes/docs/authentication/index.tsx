import { component$ } from "@builder.io/qwik";
import type { DocumentHead } from "@builder.io/qwik-city";
import { Link } from "@builder.io/qwik-city";

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
            <span class="text-neutral-900">Authentication</span>
          </div>
          <h1 class="text-4xl font-bold text-neutral-900 mb-4">Authentication</h1>
          <p class="text-lg text-neutral-600 max-w-2xl">
            Secure your API with JWT authentication and PostgreSQL Row-Level Security.
          </p>
        </div>
      </div>

      <div class="container-wide py-12">
        <div class="max-w-3xl">
          {/* JWT Auth */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">JWT Authentication</h2>
            <p class="text-neutral-600 mb-4">
              Postrust validates JWT tokens and extracts role claims to determine database access.
            </p>
            <div class="bg-neutral-900 rounded-xl overflow-hidden mb-6">
              <div class="px-4 py-2 bg-neutral-800 border-b border-neutral-700">
                <span class="text-sm text-neutral-400">JWT Payload</span>
              </div>
              <pre class="p-4 text-sm overflow-x-auto">
                <code class="text-neutral-100">{`{
  "role": "authenticated_user",
  "sub": "user_123",
  "email": "user@example.com",
  "exp": 1704067200
}`}</code>
              </pre>
            </div>
            <div class="bg-neutral-900 rounded-xl overflow-hidden">
              <div class="px-4 py-2 bg-neutral-800 border-b border-neutral-700">
                <span class="text-sm text-neutral-400">Request with JWT</span>
              </div>
              <pre class="p-4 text-sm overflow-x-auto">
                <code class="text-neutral-100">{`curl http://localhost:3000/users \\
  -H "Authorization: Bearer eyJhbGciOiJIUzI1NiIs..."`}</code>
              </pre>
            </div>
          </section>

          {/* How claims are checked */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">How the claims are checked</h2>
            <p class="text-neutral-600 mb-4">
              A claim that is absent is not a claim that is wrong: a token carrying no{" "}
              <code class="font-mono">exp</code> does not expire, and one carrying no{" "}
              <code class="font-mono">aud</code> is for anybody. Only a claim that is present and
              is not the kind of thing that claim is gets rejected.
            </p>
            <ul class="space-y-3 text-neutral-600">
              <li class="flex gap-3">
                <span class="text-primary-600 font-mono text-sm mt-0.5">exp</span>
                <span>
                  Checked to the second. Forgiving an expiry keeps a session alive past the moment
                  its issuer said it ended, so no slack is given here.
                </span>
              </li>
              <li class="flex gap-3">
                <span class="text-primary-600 font-mono text-sm mt-0.5">nbf, iat</span>
                <span>
                  Thirty seconds of clock skew allowed. These describe a token that is not valid
                  yet, and a client whose clock runs slightly fast mints one through nobody's
                  fault; refusing it makes a working deployment fail intermittently.
                </span>
              </li>
              <li class="flex gap-3">
                <span class="text-primary-600 font-mono text-sm mt-0.5">role</span>
                <span>
                  Must be a string. A role claim of any other shape names no role and falls back to
                  the anonymous role, rather than asking PostgreSQL for a role of that spelling.
                </span>
              </li>
            </ul>
            <p class="text-neutral-600 mt-4">
              A <code class="font-mono">401</code> names which refusal in its{" "}
              <code class="font-mono">WWW-Authenticate</code> challenge, so a client can tell
              &ldquo;rotate your key&rdquo; from &ldquo;fix your clock&rdquo; without reading the
              body.
            </p>
          </section>

          {/* Configuration */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">Configuration</h2>
            <div class="space-y-4">
              {[
                { var: "PGRST_JWT_SECRET", desc: "Secret key for HS256/384/512 validation" },
                { var: "PGRST_JWT_SECRET_IS_BASE64", desc: "Set true if secret is base64 encoded" },
                { var: "PGRST_JWT_AUD", desc: "Required audience claim (optional)" },
                { var: "PGRST_JWT_ROLE_CLAIM_KEY", desc: "Claim key containing role (default: role)" },
              ].map((item) => (
                <div key={item.var} class="p-4 bg-neutral-50 rounded-lg">
                  <code class="font-mono text-primary-600 text-sm">{item.var}</code>
                  <p class="text-neutral-600 text-sm mt-1">{item.desc}</p>
                </div>
              ))}
            </div>
          </section>

          {/* Row-Level Security */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">Row-Level Security</h2>
            <p class="text-neutral-600 mb-4">
              PostgreSQL RLS policies are enforced on every request. JWT claims are available as session variables.
            </p>
            <div class="bg-neutral-900 rounded-xl overflow-hidden">
              <div class="px-4 py-2 bg-neutral-800 border-b border-neutral-700">
                <span class="text-sm text-neutral-400">SQL - RLS Policy</span>
              </div>
              <pre class="p-4 text-sm overflow-x-auto">
                <code class="text-neutral-100">{`-- Enable RLS on table
ALTER TABLE orders ENABLE ROW LEVEL SECURITY;

-- Policy: users can only see their own orders
CREATE POLICY user_orders ON orders
  FOR ALL
  USING (
    user_id = current_setting('request.jwt.claims')::json->>'sub'
  );

-- Policy: admins can see all orders
CREATE POLICY admin_orders ON orders
  FOR ALL
  USING (
    current_setting('request.jwt.claims')::json->>'role' = 'admin'
  );`}</code>
              </pre>
            </div>
          </section>

          {/* Accessing Claims */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">Accessing JWT Claims in SQL</h2>
            <div class="bg-neutral-900 rounded-xl overflow-hidden">
              <pre class="p-4 text-sm overflow-x-auto">
                <code class="text-neutral-100">{`-- Get full claims object
current_setting('request.jwt.claims')::json

-- Get specific claim
current_setting('request.jwt.claims')::json->>'sub'
current_setting('request.jwt.claims')::json->>'email'
current_setting('request.jwt.claims')::json->>'role'

-- Use in function
CREATE FUNCTION get_current_user_id() RETURNS TEXT AS $$
  SELECT current_setting('request.jwt.claims')::json->>'sub';
$$ LANGUAGE SQL STABLE;`}</code>
              </pre>
            </div>
          </section>

          {/* Hasura */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">The GraphQL surface</h2>
            <p class="text-neutral-600 mb-4">
              Everything above is how the REST surface authenticates. The GraphQL surface at{" "}
              <code class="font-mono">/v1/graphql</code> reads Hasura&rsquo;s contract instead — an
              admin secret, <code class="font-mono">x-hasura-*</code> session variables, and a role
              that decides which schema answers before it decides anything about a row.
            </p>
            <p class="text-neutral-600">
              The two meet at the token: a verified JWT names the role either way, and on the
              GraphQL side an <code class="font-mono">X-Hasura-Role</code> header may select any
              role the token&rsquo;s <code class="font-mono">x-hasura-allowed-roles</code> lists.
              See{" "}
              <Link href="/docs/configuration" class="text-primary-600 hover:underline">
                Configuration
              </Link>{" "}
              for the settings and{" "}
              <Link href="/docs/graphql" class="text-primary-600 hover:underline">
                GraphQL
              </Link>{" "}
              for the surface itself.
            </p>
          </section>

          {/* Next */}
          <div class="flex items-center justify-between pt-8 border-t border-neutral-200">
            <Link
              href="/docs/api-reference"
              class="flex items-center gap-2 text-neutral-600 hover:text-primary-600"
            >
              <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 19l-7-7 7-7"/>
              </svg>
              API Reference
            </Link>
            <Link
              href="/docs/configuration"
              class="flex items-center gap-2 text-neutral-600 hover:text-primary-600"
            >
              Configuration
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
  title: "Authentication - Postrust Documentation",
  meta: [
    {
      name: "description",
      content: "Secure your Postrust API with JWT authentication and PostgreSQL Row-Level Security.",
    },
  ],
};
