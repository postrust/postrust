import { component$ } from "@builder.io/qwik";
import type { DocumentHead } from "@builder.io/qwik-city";
import { Link } from "@builder.io/qwik-city";

const configVars = [
  {
    category: "Database",
    vars: [
      { name: "DATABASE_URL", required: true, default: "-", desc: "PostgreSQL connection string" },
      { name: "PGRST_DB_SCHEMAS", required: false, default: "public", desc: "Schemas to expose (comma-separated)" },
      { name: "PGRST_DB_ANON_ROLE", required: false, default: "-", desc: "Role for unauthenticated requests" },
      { name: "PGRST_DB_POOL_SIZE", required: false, default: "10", desc: "Connection pool size" },
    ],
  },
  {
    category: "Authentication",
    vars: [
      { name: "PGRST_JWT_SECRET", required: false, default: "-", desc: "JWT signing secret" },
      { name: "PGRST_JWT_SECRET_IS_BASE64", required: false, default: "false", desc: "If secret is base64 encoded" },
      { name: "PGRST_JWT_AUD", required: false, default: "-", desc: "Required JWT audience claim" },
      { name: "PGRST_JWT_ROLE_CLAIM_KEY", required: false, default: "role", desc: "Claim key for role" },
    ],
  },
  {
    category: "Server",
    vars: [
      { name: "PGRST_SERVER_HOST", required: false, default: "127.0.0.1", desc: "Bind address" },
      { name: "PGRST_SERVER_PORT", required: false, default: "3000", desc: "Port to listen on" },
      { name: "PGRST_SERVER_CORS_ORIGINS", required: false, default: "*", desc: "CORS allowed origins" },
    ],
  },
  {
    category: "Limits",
    vars: [
      { name: "PGRST_MAX_ROWS", required: false, default: "1000", desc: "Maximum rows returned" },
      { name: "PGRST_MAX_BODY_SIZE", required: false, default: "10485760", desc: "Max request body in bytes" },
    ],
  },
  {
    category: "Logging",
    vars: [
      { name: "PGRST_LOG_LEVEL", required: false, default: "info", desc: "Log level (error, warn, info, debug)" },
      { name: "RUST_LOG", required: false, default: "-", desc: "Detailed tracing configuration" },
    ],
  },
];

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
            <span class="text-neutral-900">Configuration</span>
          </div>
          <h1 class="text-4xl font-bold text-neutral-900 mb-4">Configuration</h1>
          <p class="text-lg text-neutral-600 max-w-2xl">
            All environment variables and configuration options for Postrust.
          </p>
        </div>
      </div>

      <div class="container-wide py-12">
        <div class="max-w-4xl">
          {configVars.map((category) => (
            <section key={category.category} class="mb-12">
              <h2 class="text-2xl font-bold text-neutral-900 mb-6">{category.category}</h2>
              <div class="overflow-x-auto">
                <table class="w-full text-sm">
                  <thead>
                    <tr class="border-b border-neutral-200">
                      <th class="text-left py-3 px-4 font-semibold text-neutral-900">Variable</th>
                      <th class="text-left py-3 px-4 font-semibold text-neutral-900">Required</th>
                      <th class="text-left py-3 px-4 font-semibold text-neutral-900">Default</th>
                      <th class="text-left py-3 px-4 font-semibold text-neutral-900">Description</th>
                    </tr>
                  </thead>
                  <tbody class="divide-y divide-neutral-100">
                    {category.vars.map((v) => (
                      <tr key={v.name}>
                        <td class="py-3 px-4 font-mono text-primary-600 text-xs">{v.name}</td>
                        <td class="py-3 px-4">
                          {v.required ? (
                            <span class="text-red-600">Yes</span>
                          ) : (
                            <span class="text-neutral-400">No</span>
                          )}
                        </td>
                        <td class="py-3 px-4 font-mono text-neutral-500 text-xs">{v.default}</td>
                        <td class="py-3 px-4 text-neutral-600">{v.desc}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </section>
          ))}

          {/* Example */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">Example Configuration</h2>
            <div class="bg-neutral-900 rounded-xl overflow-hidden">
              <div class="px-4 py-2 bg-neutral-800 border-b border-neutral-700">
                <span class="text-sm text-neutral-400">.env</span>
              </div>
              <pre class="p-4 text-sm overflow-x-auto">
                <code class="text-neutral-100">{`# Required
DATABASE_URL=postgres://user:password@localhost:5432/mydb

# Authentication
PGRST_DB_ANON_ROLE=web_anon
PGRST_JWT_SECRET=your-secret-key-at-least-32-characters

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
          <div class="flex items-center justify-between pt-8 border-t border-neutral-200">
            <Link
              href="/docs/authentication"
              class="flex items-center gap-2 text-neutral-600 hover:text-primary-600"
            >
              <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 19l-7-7 7-7"/>
              </svg>
              Authentication
            </Link>
            <Link
              href="/docs/deployment"
              class="flex items-center gap-2 text-neutral-600 hover:text-primary-600"
            >
              Deployment
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
  title: "Configuration - Postrust Documentation",
  meta: [
    {
      name: "description",
      content: "All configuration options and environment variables for Postrust.",
    },
  ],
};
