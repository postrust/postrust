import { component$ } from "@builder.io/qwik";
import type { DocumentHead } from "@builder.io/qwik-city";
import { Link } from "@builder.io/qwik-city";
import { emailOff } from "~/components/email-off";

export default component$(() => {
  return (
    <div class="min-h-screen bg-white">
      <div class="border-b border-neutral-200 bg-gradient-to-b from-neutral-50 to-white">
        <div class="container-wide py-12">
          <div class="mb-4 flex items-center gap-2 text-sm text-neutral-500">
            <Link href="/docs/" class="hover:text-primary-600">
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
            <span class="text-neutral-900">Authentication</span>
          </div>
          <h1 class="mb-4 text-4xl font-bold text-neutral-900">
            Authentication
          </h1>
          <p class="max-w-2xl text-lg text-neutral-600">
            Secure your API with JWT authentication and PostgreSQL Row-Level
            Security.
          </p>
        </div>
      </div>

      <div class="container-wide py-12">
        <div class="max-w-3xl">
          {/* JWT Auth */}
          <section class="mb-12">
            <h2 class="mb-4 text-2xl font-bold text-neutral-900">
              JWT Authentication
            </h2>
            <p class="mb-4 text-neutral-600">
              Postrust validates JWT tokens and extracts role claims to
              determine database access.
            </p>
            <div class="mb-6 overflow-hidden rounded-xl bg-neutral-900">
              <div class="border-b border-neutral-700 bg-neutral-800 px-4 py-2">
                <span class="text-sm text-neutral-400">JWT Payload</span>
              </div>
              <pre class="overflow-x-auto p-4 text-sm">
                <code
                  class="text-neutral-100"
                  dangerouslySetInnerHTML={emailOff(`{
  "role": "authenticated_user",
  "sub": "user_123",
  "email": "user@example.com",
  "exp": 1704067200
}`)}
                />
              </pre>
            </div>
            <div class="overflow-hidden rounded-xl bg-neutral-900">
              <div class="border-b border-neutral-700 bg-neutral-800 px-4 py-2">
                <span class="text-sm text-neutral-400">Request with JWT</span>
              </div>
              <pre class="overflow-x-auto p-4 text-sm">
                <code class="text-neutral-100">{`curl http://localhost:3000/users \\
  -H "Authorization: Bearer eyJhbGciOiJIUzI1NiIs..."`}</code>
              </pre>
            </div>
          </section>

          {/* How claims are checked */}
          <section class="mb-12">
            <h2 class="mb-4 text-2xl font-bold text-neutral-900">
              How the claims are checked
            </h2>
            <p class="mb-4 text-neutral-600">
              A claim that is absent is not a claim that is wrong: a token
              carrying no <code class="font-mono">exp</code> does not expire,
              and one carrying no <code class="font-mono">aud</code> is for
              anybody. Only a claim that is present and is not the kind of thing
              that claim is gets rejected.
            </p>
            <ul class="space-y-3 text-neutral-600">
              <li class="flex gap-3">
                <span class="text-primary-600 mt-0.5 font-mono text-sm">
                  exp
                </span>
                <span>
                  Checked to the second. Forgiving an expiry keeps a session
                  alive past the moment its issuer said it ended, so no slack is
                  given here.
                </span>
              </li>
              <li class="flex gap-3">
                <span class="text-primary-600 mt-0.5 font-mono text-sm">
                  nbf, iat
                </span>
                <span>
                  Thirty seconds of clock skew allowed. These describe a token
                  that is not valid yet, and a client whose clock runs slightly
                  fast mints one through nobody's fault; refusing it makes a
                  working deployment fail intermittently.
                </span>
              </li>
              <li class="flex gap-3">
                <span class="text-primary-600 mt-0.5 font-mono text-sm">
                  role
                </span>
                <span>
                  Must be a string. A role claim of any other shape, or{" "}
                  <code class="font-mono">""</code> or{" "}
                  <code class="font-mono">none</code>, names no role and falls
                  back to the anonymous role, rather than asking PostgreSQL for
                  a role of that spelling.
                </span>
              </li>
              <li class="flex gap-3">
                <span class="text-primary-600 mt-0.5 font-mono text-sm">
                  iss
                </span>
                <span>
                  Not looked at unless{" "}
                  <code class="font-mono">PGRST_JWT_ISS</code> is set. Then it
                  must be present and equal it exactly: where an identity
                  provider&rsquo;s keys are shared across tenants, the issuer is
                  what tells one tenant&rsquo;s tokens from another&rsquo;s.
                </span>
              </li>
            </ul>
            <p class="mt-4 text-neutral-600">
              A <code class="font-mono">401</code> names which refusal in its{" "}
              <code class="font-mono">WWW-Authenticate</code> challenge, so a
              client can tell &ldquo;rotate your key&rdquo; from &ldquo;fix your
              clock&rdquo; without reading the body.
            </p>
          </section>

          {/* Keys */}
          <section class="mb-12">
            <h2 class="mb-4 text-2xl font-bold text-neutral-900">
              Signing keys
            </h2>
            <p class="mb-4 text-neutral-600">
              <code class="font-mono">PGRST_JWT_SECRET</code> is read as
              PostgREST reads <code class="font-mono">jwt-secret</code>. A plain
              string is an HMAC secret and admits HS256 tokens. A JSON Web Key,
              or a JSON Web Key Set, is a set of public keys: how an identity
              provider&rsquo;s keys are given.
            </p>
            <div class="mb-6 overflow-hidden rounded-xl bg-neutral-900">
              <div class="border-b border-neutral-700 bg-neutral-800 px-4 py-2">
                <span class="text-sm text-neutral-400">Environment</span>
              </div>
              <pre class="overflow-x-auto p-4 text-sm">
                <code class="text-neutral-100">{`# A shared HMAC secret (HS256)
PGRST_JWT_SECRET="your-secret-key-at-least-32-characters"

# A JWKS, as an identity provider publishes it
PGRST_JWT_SECRET='{"keys":[{"kid":"2024-07","kty":"RSA","alg":"RS256","n":"0vx7...","e":"AQAB"}]}'`}</code>
              </pre>
            </div>
            <div class="mb-4 overflow-x-auto">
              <table class="w-full text-left text-sm text-neutral-600">
                <thead class="border-b border-neutral-200 text-neutral-900">
                  <tr>
                    <th class="py-2 pr-4">kty</th>
                    <th class="py-2 pr-4">Algorithms</th>
                  </tr>
                </thead>
                <tbody>
                  <tr class="border-b border-neutral-100">
                    <td class="py-2 pr-4 font-mono">RSA</td>
                    <td class="py-2 pr-4">
                      RS256/384/512, PS256/384/512 (modulus of at least 2048
                      bits)
                    </td>
                  </tr>
                  <tr class="border-b border-neutral-100">
                    <td class="py-2 pr-4 font-mono">EC</td>
                    <td class="py-2 pr-4">ES256 (P-256), ES384 (P-384)</td>
                  </tr>
                  <tr class="border-b border-neutral-100">
                    <td class="py-2 pr-4 font-mono">OKP</td>
                    <td class="py-2 pr-4">EdDSA (Ed25519)</td>
                  </tr>
                  <tr>
                    <td class="py-2 pr-4 font-mono">oct</td>
                    <td class="py-2 pr-4">HS256/384/512</td>
                  </tr>
                </tbody>
              </table>
            </div>
            <p class="text-neutral-600">
              <strong>
                The algorithm comes from the key, never from the token
              </strong>
              , so neither
              <code class="font-mono">alg: none</code> nor an HS256 token
              &ldquo;signed&rdquo; with an RSA public key gets through. A token
              with a <code class="font-mono">kid</code> is checked against the
              key with that <code class="font-mono">kid</code> only; one without
              is tried against every key that admits its algorithm. Keys that
              cannot verify (encryption keys, unsupported curves) are skipped
              with a warning; a malformed key stops the server from starting.
            </p>
          </section>

          {/* JWKS URL */}
          <section class="mb-12">
            <h2 class="mb-4 text-2xl font-bold text-neutral-900">
              Keys from an identity provider
            </h2>
            <p class="mb-4 text-neutral-600">
              Give <code class="font-mono">PGRST_JWT_JWKS_URL</code> the
              provider&rsquo;s <code class="font-mono">jwks_uri</code> and
              Postrust fetches the key set from there and keeps it current, so a
              key the provider rotates in is picked up without a restart and a
              key it withdraws stops being accepted.
            </p>
            <div class="mb-6 overflow-hidden rounded-xl bg-neutral-900">
              <div class="border-b border-neutral-700 bg-neutral-800 px-4 py-2">
                <span class="text-sm text-neutral-400">Environment</span>
              </div>
              <pre class="overflow-x-auto p-4 text-sm">
                <code class="text-neutral-100">{`# Supabase Auth, on asymmetric signing keys
PGRST_JWT_JWKS_URL="https://<project>.supabase.co/auth/v1/.well-known/jwks.json"
# Auth0
PGRST_JWT_JWKS_URL="https://<tenant>.auth0.com/.well-known/jwks.json"
# Keycloak
PGRST_JWT_JWKS_URL="https://<host>/realms/<realm>/protocol/openid-connect/certs"

# Optional: only accept tokens this issuer minted
PGRST_JWT_ISS="https://<tenant>.auth0.com/"`}</code>
              </pre>
            </div>
            <ul class="list-disc space-y-2 pl-6 text-neutral-600">
              <li>
                Fetched at start-up, then again after the response&rsquo;s{" "}
                <code class="font-mono">Cache-Control: max-age</code>
                (held to between 60 seconds and a day), or every{" "}
                <code class="font-mono">PGRST_JWT_JWKS_REFRESH</code> seconds
                (default 300).
              </li>
              <li>
                A token naming a <code class="font-mono">kid</code> the set does
                not hold triggers one more fetch first &mdash; a key just
                rotated in &mdash; at most once every 30 seconds.
              </li>
              <li>
                <code class="font-mono">https</code> only (
                <code class="font-mono">http</code> just for loopback),
                redirects only to the same host, 5 seconds and 1 MiB per fetch.
              </li>
              <li>
                Every change of the set empties the token cache, so a withdrawn
                key&rsquo;s tokens are refused from the next request on.
              </li>
              <li>
                If the provider is down, the last set that loaded is kept. If
                none ever loaded, tokens are refused (fail closed) and{" "}
                <code class="font-mono">/_/ready</code> answers{" "}
                <code class="font-mono">503</code> with
                <code class="font-mono">"jwks": false</code>.
              </li>
              <li>
                With <code class="font-mono">PGRST_JWT_SECRET</code> set as
                well, a token either one verifies is accepted: a backend&rsquo;s
                service tokens beside the provider&rsquo;s user tokens.
              </li>
            </ul>
          </section>

          {/* Configuration */}
          <section class="mb-12">
            <h2 class="mb-4 text-2xl font-bold text-neutral-900">
              Configuration
            </h2>
            <div class="space-y-4">
              {[
                {
                  var: "PGRST_JWT_SECRET",
                  desc: "An HMAC secret (HS256), or a JWK or JWKS of public keys (RS, PS, ES, EdDSA)",
                },
                {
                  var: "PGRST_JWT_SECRET_IS_BASE64",
                  desc: "Set true if secret is base64 encoded",
                },
                {
                  var: "PGRST_JWT_JWKS_URL",
                  desc: "Fetch the keys from an identity provider's JWKS URL and keep them current",
                },
                {
                  var: "PGRST_JWT_JWKS_REFRESH",
                  desc: "Seconds between fetches when the response has no max-age (default: 300)",
                },
                {
                  var: "PGRST_JWT_AUD",
                  desc: "Required audience claim (optional)",
                },
                {
                  var: "PGRST_JWT_ISS",
                  desc: "Required issuer: a token's iss must equal it, and must be present (optional)",
                },
                {
                  var: "PGRST_JWT_ROLE_CLAIM_KEY",
                  desc: "Claim key containing role (default: role)",
                },
              ].map((item) => (
                <div key={item.var} class="rounded-lg bg-neutral-50 p-4">
                  <code class="text-primary-600 font-mono text-sm">
                    {item.var}
                  </code>
                  <p class="mt-1 text-sm text-neutral-600">{item.desc}</p>
                </div>
              ))}
            </div>
          </section>

          {/* Row-Level Security */}
          <section class="mb-12">
            <h2 class="mb-4 text-2xl font-bold text-neutral-900">
              Row-Level Security
            </h2>
            <p class="mb-4 text-neutral-600">
              PostgreSQL RLS policies are enforced on every request. JWT claims
              are available as session variables.
            </p>
            <div class="overflow-hidden rounded-xl bg-neutral-900">
              <div class="border-b border-neutral-700 bg-neutral-800 px-4 py-2">
                <span class="text-sm text-neutral-400">SQL - RLS Policy</span>
              </div>
              <pre class="overflow-x-auto p-4 text-sm">
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
            <h2 class="mb-4 text-2xl font-bold text-neutral-900">
              Accessing JWT Claims in SQL
            </h2>
            <div class="overflow-hidden rounded-xl bg-neutral-900">
              <pre class="overflow-x-auto p-4 text-sm">
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
            <h2 class="mb-4 text-2xl font-bold text-neutral-900">
              The GraphQL surface
            </h2>
            <p class="mb-4 text-neutral-600">
              Everything above is how the REST surface authenticates. The
              GraphQL surface at <code class="font-mono">/v1/graphql</code>{" "}
              reads Hasura&rsquo;s contract instead — an admin secret,{" "}
              <code class="font-mono">x-hasura-*</code> session variables, and a
              role that decides which schema answers before it decides anything
              about a row.
            </p>
            <p class="text-neutral-600">
              The two meet at the token: a verified JWT names the role either
              way, and on the GraphQL side an{" "}
              <code class="font-mono">X-Hasura-Role</code> header may select any
              role the token&rsquo;s{" "}
              <code class="font-mono">x-hasura-allowed-roles</code> lists. See{" "}
              <Link
                href="/docs/configuration/"
                class="text-primary-600 hover:underline"
              >
                Configuration
              </Link>{" "}
              for the settings and{" "}
              <Link
                href="/docs/graphql/"
                class="text-primary-600 hover:underline"
              >
                GraphQL
              </Link>{" "}
              for the surface itself.
            </p>
            <p class="mt-4 text-neutral-600">
              Either way the verified claims reach the database, so one RLS
              policy serves both:{" "}
              <code class="font-mono">request.jwt.claims</code> is set in every
              GraphQL transaction as it is in every REST one. That includes
              subscriptions. A WebSocket is authenticated once, from the{" "}
              <code class="font-mono">connection_init</code> payload &mdash;{" "}
              <code class="font-mono">{`{"headers": {"Authorization": "Bearer <token>"}}`}</code>
              , since a browser cannot set headers on a WebSocket &mdash; and
              every live query on it re-reads as that identity.
            </p>
          </section>

          {/* Next */}
          <div class="flex items-center justify-between border-t border-neutral-200 pt-8">
            <Link
              href="/docs/api-reference/"
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
              API Reference
            </Link>
            <Link
              href="/docs/configuration/"
              class="hover:text-primary-600 flex items-center gap-2 text-neutral-600"
            >
              Configuration
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
  title: "Authentication - Postrust Documentation",
  meta: [
    {
      name: "description",
      content:
        "Secure your Postrust API with JWT authentication and PostgreSQL Row-Level Security.",
    },
  ],
};
