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
            <span class="text-neutral-900">SaaS Domain Management</span>
          </div>
          <div class="flex items-center gap-3 mb-4">
            <h1 class="text-4xl font-bold text-neutral-900">SaaS Domain Management</h1>
            <span class="px-3 py-1 text-sm font-medium bg-amber-100 text-amber-800 rounded-full">Beta</span>
          </div>
          <p class="text-lg text-neutral-600 max-w-2xl">
            Multi-tenant domain management API for SaaS applications with custom domains, automatic SSL, and dynamic routing.
          </p>
          <p class="text-sm text-amber-700 mt-2">
            This feature is in beta and will move to stable in Q2 2026. APIs may change before the stable release.
          </p>
        </div>
      </div>

      <div class="container-wide py-12">
        <div class="max-w-4xl">
          {/* Features */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-6">Features</h2>
            <div class="grid md:grid-cols-2 gap-4">
              {[
                { title: "Domain Verification", desc: "DNS TXT and HTTP challenge methods" },
                { title: "Automatic SSL", desc: "ACME (Let's Encrypt) + manual upload" },
                { title: "Dual Authentication", desc: "JWT + API Key support" },
                { title: "Multi-tenant", desc: "Complete tenant isolation with quotas" },
                { title: "Dynamic Routing", desc: "Per-domain routes without restart" },
                { title: "Hot Reload", desc: "PostgreSQL NOTIFY for instant updates" },
              ].map((feature) => (
                <div key={feature.title} class="p-4 bg-neutral-50 rounded-lg">
                  <h3 class="font-semibold text-neutral-900">{feature.title}</h3>
                  <p class="text-sm text-neutral-600 mt-1">{feature.desc}</p>
                </div>
              ))}
            </div>
          </section>

          {/* Authentication */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">Authentication</h2>
            <p class="text-neutral-600 mb-4">
              The SaaS API supports both JWT tokens and API keys for authentication.
            </p>

            <h3 class="text-lg font-semibold text-neutral-900 mb-3">JWT Authentication</h3>
            <div class="bg-neutral-900 rounded-xl overflow-hidden mb-6">
              <div class="px-4 py-2 bg-neutral-800 border-b border-neutral-700">
                <span class="text-sm text-neutral-400">Request with JWT</span>
              </div>
              <pre class="p-4 text-sm overflow-x-auto">
                <code class="text-neutral-100">{`curl http://localhost:8080/api/v1/domains \\
  -H "Authorization: Bearer eyJhbGciOiJIUzI1NiIs..."`}</code>
              </pre>
            </div>

            <h3 class="text-lg font-semibold text-neutral-900 mb-3">API Key Authentication</h3>
            <div class="bg-neutral-900 rounded-xl overflow-hidden">
              <div class="px-4 py-2 bg-neutral-800 border-b border-neutral-700">
                <span class="text-sm text-neutral-400">Request with API Key</span>
              </div>
              <pre class="p-4 text-sm overflow-x-auto">
                <code class="text-neutral-100">{`curl http://localhost:8080/api/v1/domains \\
  -H "Authorization: ApiKey pk_live_abc123..."`}</code>
              </pre>
            </div>
          </section>

          {/* API Endpoints */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">API Endpoints</h2>

            <h3 class="text-lg font-semibold text-neutral-900 mb-3">Domains</h3>
            <div class="overflow-x-auto mb-6">
              <table class="w-full text-sm">
                <thead>
                  <tr class="border-b border-neutral-200">
                    <th class="text-left py-2 px-3 font-medium text-neutral-900">Method</th>
                    <th class="text-left py-2 px-3 font-medium text-neutral-900">Endpoint</th>
                    <th class="text-left py-2 px-3 font-medium text-neutral-900">Description</th>
                  </tr>
                </thead>
                <tbody class="text-neutral-600">
                  <tr class="border-b border-neutral-100"><td class="py-2 px-3"><code class="text-primary-600">GET</code></td><td class="py-2 px-3 font-mono text-xs">/api/v1/domains</td><td class="py-2 px-3">List domains</td></tr>
                  <tr class="border-b border-neutral-100"><td class="py-2 px-3"><code class="text-green-600">POST</code></td><td class="py-2 px-3 font-mono text-xs">/api/v1/domains</td><td class="py-2 px-3">Add domain</td></tr>
                  <tr class="border-b border-neutral-100"><td class="py-2 px-3"><code class="text-primary-600">GET</code></td><td class="py-2 px-3 font-mono text-xs">/api/v1/domains/:id</td><td class="py-2 px-3">Get domain details</td></tr>
                  <tr class="border-b border-neutral-100"><td class="py-2 px-3"><code class="text-amber-600">PUT</code></td><td class="py-2 px-3 font-mono text-xs">/api/v1/domains/:id</td><td class="py-2 px-3">Update domain</td></tr>
                  <tr class="border-b border-neutral-100"><td class="py-2 px-3"><code class="text-red-600">DELETE</code></td><td class="py-2 px-3 font-mono text-xs">/api/v1/domains/:id</td><td class="py-2 px-3">Remove domain</td></tr>
                  <tr class="border-b border-neutral-100"><td class="py-2 px-3"><code class="text-green-600">POST</code></td><td class="py-2 px-3 font-mono text-xs">/api/v1/domains/:id/verify</td><td class="py-2 px-3">Trigger verification</td></tr>
                  <tr class="border-b border-neutral-100"><td class="py-2 px-3"><code class="text-green-600">POST</code></td><td class="py-2 px-3 font-mono text-xs">/api/v1/domains/:id/ssl/provision</td><td class="py-2 px-3">Provision SSL via ACME</td></tr>
                  <tr><td class="py-2 px-3"><code class="text-green-600">POST</code></td><td class="py-2 px-3 font-mono text-xs">/api/v1/domains/:id/ssl/upload</td><td class="py-2 px-3">Upload certificate</td></tr>
                </tbody>
              </table>
            </div>

            <h3 class="text-lg font-semibold text-neutral-900 mb-3">Routes & Upstreams</h3>
            <div class="overflow-x-auto">
              <table class="w-full text-sm">
                <thead>
                  <tr class="border-b border-neutral-200">
                    <th class="text-left py-2 px-3 font-medium text-neutral-900">Method</th>
                    <th class="text-left py-2 px-3 font-medium text-neutral-900">Endpoint</th>
                    <th class="text-left py-2 px-3 font-medium text-neutral-900">Description</th>
                  </tr>
                </thead>
                <tbody class="text-neutral-600">
                  <tr class="border-b border-neutral-100"><td class="py-2 px-3"><code class="text-green-600">POST</code></td><td class="py-2 px-3 font-mono text-xs">/api/v1/domains/:id/routes</td><td class="py-2 px-3">Create route</td></tr>
                  <tr class="border-b border-neutral-100"><td class="py-2 px-3"><code class="text-green-600">POST</code></td><td class="py-2 px-3 font-mono text-xs">/api/v1/upstreams</td><td class="py-2 px-3">Create upstream</td></tr>
                  <tr><td class="py-2 px-3"><code class="text-green-600">POST</code></td><td class="py-2 px-3 font-mono text-xs">/api/v1/upstreams/:id/backends</td><td class="py-2 px-3">Add backend server</td></tr>
                </tbody>
              </table>
            </div>
          </section>

          {/* Domain Verification */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">Domain Verification</h2>

            <h3 class="text-lg font-semibold text-neutral-900 mb-3">DNS TXT Verification</h3>
            <p class="text-neutral-600 mb-4">
              Add a TXT record to prove domain ownership.
            </p>
            <div class="bg-neutral-900 rounded-xl overflow-hidden mb-6">
              <div class="px-4 py-2 bg-neutral-800 border-b border-neutral-700">
                <span class="text-sm text-neutral-400">1. Add domain</span>
              </div>
              <pre class="p-4 text-sm overflow-x-auto">
                <code class="text-neutral-100">{`curl -X POST http://localhost:8080/api/v1/domains \\
  -H "Authorization: ApiKey pk_live_..." \\
  -H "Content-Type: application/json" \\
  -d '{"domain": "app.example.com", "verification_method": "dns"}'`}</code>
              </pre>
            </div>
            <div class="bg-neutral-900 rounded-xl overflow-hidden mb-6">
              <div class="px-4 py-2 bg-neutral-800 border-b border-neutral-700">
                <span class="text-sm text-neutral-400">2. Create DNS TXT record</span>
              </div>
              <pre class="p-4 text-sm overflow-x-auto">
                <code class="text-neutral-100">{`_postrust-verification.app.example.com TXT "postrust-verify=<token>"`}</code>
              </pre>
            </div>
            <div class="bg-neutral-900 rounded-xl overflow-hidden">
              <div class="px-4 py-2 bg-neutral-800 border-b border-neutral-700">
                <span class="text-sm text-neutral-400">3. Trigger verification</span>
              </div>
              <pre class="p-4 text-sm overflow-x-auto">
                <code class="text-neutral-100">{`curl -X POST http://localhost:8080/api/v1/domains/<id>/verify \\
  -H "Authorization: ApiKey pk_live_..."`}</code>
              </pre>
            </div>
          </section>

          {/* SSL Provisioning */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">SSL/TLS Certificates</h2>

            <h3 class="text-lg font-semibold text-neutral-900 mb-3">Automatic ACME (Let's Encrypt)</h3>
            <div class="bg-neutral-900 rounded-xl overflow-hidden mb-6">
              <pre class="p-4 text-sm overflow-x-auto">
                <code class="text-neutral-100">{`curl -X POST http://localhost:8080/api/v1/domains/<id>/ssl/provision \\
  -H "Authorization: ApiKey pk_live_..."`}</code>
              </pre>
            </div>

            <h3 class="text-lg font-semibold text-neutral-900 mb-3">Manual Certificate Upload</h3>
            <div class="bg-neutral-900 rounded-xl overflow-hidden">
              <pre class="p-4 text-sm overflow-x-auto">
                <code class="text-neutral-100">{`curl -X POST http://localhost:8080/api/v1/domains/<id>/ssl/upload \\
  -H "Authorization: ApiKey pk_live_..." \\
  -H "Content-Type: application/json" \\
  -d '{
    "certificate": "-----BEGIN CERTIFICATE-----...",
    "private_key": "-----BEGIN PRIVATE KEY-----...",
    "chain": "-----BEGIN CERTIFICATE-----..."
  }'`}</code>
              </pre>
            </div>
          </section>

          {/* Routing */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">Routing Configuration</h2>
            <div class="bg-neutral-900 rounded-xl overflow-hidden mb-6">
              <div class="px-4 py-2 bg-neutral-800 border-b border-neutral-700">
                <span class="text-sm text-neutral-400">Create upstream with backend</span>
              </div>
              <pre class="p-4 text-sm overflow-x-auto">
                <code class="text-neutral-100">{`# Create upstream
curl -X POST http://localhost:8080/api/v1/upstreams \\
  -H "Authorization: ApiKey pk_live_..." \\
  -d '{"name": "api-backend", "lb_strategy": "round_robin"}'

# Add backend server
curl -X POST http://localhost:8080/api/v1/upstreams/<id>/backends \\
  -H "Authorization: ApiKey pk_live_..." \\
  -d '{"address": "10.0.0.1:8080", "scheme": "http"}'`}</code>
              </pre>
            </div>
            <div class="bg-neutral-900 rounded-xl overflow-hidden">
              <div class="px-4 py-2 bg-neutral-800 border-b border-neutral-700">
                <span class="text-sm text-neutral-400">Create route</span>
              </div>
              <pre class="p-4 text-sm overflow-x-auto">
                <code class="text-neutral-100">{`curl -X POST http://localhost:8080/api/v1/domains/<domain_id>/routes \\
  -H "Authorization: ApiKey pk_live_..." \\
  -d '{
    "name": "api-route",
    "path_pattern": "/api",
    "path_type": "prefix",
    "upstream_id": "<upstream_id>",
    "strip_path": true
  }'`}</code>
              </pre>
            </div>

            <h3 class="text-lg font-semibold text-neutral-900 mt-6 mb-3">Load Balancing Strategies</h3>
            <div class="space-y-2">
              {[
                { strategy: "round_robin", desc: "Distribute requests evenly across backends" },
                { strategy: "random", desc: "Random backend selection" },
                { strategy: "ip_hash", desc: "Consistent hashing by client IP" },
                { strategy: "least_connections", desc: "Route to least busy backend" },
                { strategy: "weighted", desc: "Weighted distribution based on backend weights" },
              ].map((item) => (
                <div key={item.strategy} class="p-3 bg-neutral-50 rounded-lg flex items-start gap-3">
                  <code class="font-mono text-primary-600 text-sm">{item.strategy}</code>
                  <span class="text-neutral-600 text-sm">{item.desc}</span>
                </div>
              ))}
            </div>
          </section>

          {/* API Keys */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">API Keys</h2>
            <div class="bg-neutral-900 rounded-xl overflow-hidden mb-6">
              <div class="px-4 py-2 bg-neutral-800 border-b border-neutral-700">
                <span class="text-sm text-neutral-400">Create API Key</span>
              </div>
              <pre class="p-4 text-sm overflow-x-auto">
                <code class="text-neutral-100">{`curl -X POST http://localhost:8080/api/v1/auth/api-keys \\
  -H "Authorization: Bearer <jwt>" \\
  -H "Content-Type: application/json" \\
  -d '{
    "name": "Production Key",
    "scopes": ["domains:read", "domains:write", "routes:write"],
    "expires_in_days": 365
  }'`}</code>
              </pre>
            </div>

            <h3 class="text-lg font-semibold text-neutral-900 mb-3">Available Scopes</h3>
            <div class="grid md:grid-cols-2 gap-2">
              {[
                { scope: "*", desc: "Full access" },
                { scope: "domains:read", desc: "Read domain information" },
                { scope: "domains:write", desc: "Create/update/delete domains" },
                { scope: "routes:read", desc: "Read routes" },
                { scope: "routes:write", desc: "Create/update/delete routes" },
                { scope: "upstreams:read", desc: "Read upstreams" },
                { scope: "upstreams:write", desc: "Create/update/delete upstreams" },
                { scope: "api-keys:write", desc: "Create/revoke API keys" },
              ].map((item) => (
                <div key={item.scope} class="p-2 bg-neutral-50 rounded flex items-center gap-2">
                  <code class="font-mono text-primary-600 text-xs">{item.scope}</code>
                  <span class="text-neutral-600 text-xs">{item.desc}</span>
                </div>
              ))}
            </div>
          </section>

          {/* Navigation */}
          <div class="flex items-center justify-between pt-8 border-t border-neutral-200">
            <Link
              href="/docs/custom-routes"
              class="flex items-center gap-2 text-neutral-600 hover:text-primary-600"
            >
              <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 19l-7-7 7-7"/>
              </svg>
              Custom Routes
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
  title: "SaaS Domain Management - Postrust Documentation",
  meta: [
    {
      name: "description",
      content: "Multi-tenant domain management API for SaaS applications with custom domains, automatic SSL, and dynamic reverse proxy routing.",
    },
  ],
};
