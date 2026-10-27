import { component$ } from "@builder.io/qwik";
import type { DocumentHead } from "@builder.io/qwik-city";
import { Link } from "@builder.io/qwik-city";

export default component$(() => {
  return (
    <div class="min-h-screen bg-white">
      <div class="bg-gradient-to-b from-neutral-50 to-white border-b border-neutral-200">
        <div class="container-wide py-12">
          <div class="flex items-center gap-2 text-sm text-neutral-500 mb-4">
            <Link href="/docs/" class="hover:text-primary-600">Docs</Link>
            <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 5l7 7-7 7"/>
            </svg>
            <span class="text-neutral-900">Deployment</span>
          </div>
          <h1 class="text-4xl font-bold text-neutral-900 mb-4">Deployment</h1>
          <p class="text-lg text-neutral-600 max-w-2xl">
            Postrust is not tied to one platform. The same server runs as a
            single binary, in Docker, on Kubernetes (there is a Helm chart), on
            Fly.io or Railway, and on AWS Lambda through a native adapter. The
            REST API also runs in a Cloudflare Worker.
          </p>
        </div>
      </div>

      <div class="container-wide py-12">
        <div class="max-w-3xl">
          {/* Docker */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">Docker</h2>
            <div class="bg-neutral-900 rounded-xl overflow-hidden mb-4">
              <pre class="p-4 text-sm overflow-x-auto">
                <code class="text-neutral-100">{`docker run -d \\
  --name postrust \\
  -p 3000:3000 \\
  -e DATABASE_URL="postgres://user:pass@host:5432/db" \\
  -e PGRST_DB_ANON_ROLE="web_anon" \\
  -e PGRST_JWT_SECRET="your-secret-key" \\
  postrust/postrust:latest`}</code>
              </pre>
            </div>
          </section>

          {/* Docker Compose */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">Docker Compose</h2>
            <div class="bg-neutral-900 rounded-xl overflow-hidden">
              <div class="px-4 py-2 bg-neutral-800 border-b border-neutral-700">
                <span class="text-sm text-neutral-400">docker-compose.yml</span>
              </div>
              <pre class="p-4 text-sm overflow-x-auto">
                <code class="text-neutral-100">{`version: '3.8'

services:
  postgres:
    image: postgres:16-alpine
    environment:
      POSTGRES_PASSWORD: postgres
      POSTGRES_DB: app
    volumes:
      - postgres_data:/var/lib/postgresql/data

  postrust:
    image: postrust/postrust:latest
    depends_on:
      - postgres
    environment:
      DATABASE_URL: postgres://postgres:postgres@postgres:5432/app
      PGRST_DB_ANON_ROLE: web_anon
      PGRST_JWT_SECRET: \${JWT_SECRET}
    ports:
      - "3000:3000"

volumes:
  postgres_data:`}</code>
              </pre>
            </div>
          </section>

          {/* AWS Lambda */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">AWS Lambda</h2>
            <p class="text-neutral-600 mb-4">
              The function serves what the server serves &mdash; the same router, so REST, GraphQL,
              JWT authentication and row-level security behave identically. GraphQL subscriptions
              are left out, since a function URL cannot hold a WebSocket. Connect it as a
              least-privilege <code class="font-mono">authenticator</code> role, never as a
              superuser, which would bypass row-level security.
            </p>
            <div class="bg-neutral-900 rounded-xl overflow-hidden mb-4">
              <pre class="p-4 text-sm overflow-x-auto">
                <code class="text-neutral-100">{`# Build for Lambda
cargo lambda build --release -p postrust-lambda

# Deploy with AWS SAM
sam build && sam deploy --guided`}</code>
              </pre>
            </div>
            <p class="text-neutral-600 text-sm">
              See the full{" "}
              <a
                href="https://github.com/postrust/postrust/blob/main/docs/deployment.md"
                target="_blank"
                rel="noopener noreferrer"
                class="text-primary-600 hover:text-primary-700 underline"
              >
                deployment guide
              </a>{" "}
              for SAM templates and Serverless Framework examples.
            </p>
          </section>

          {/* Cloudflare Workers */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">Cloudflare Workers</h2>
            <p class="text-neutral-600 mb-4">
              The REST API runs in a Worker, reaching PostgreSQL through Hyperdrive. It is the
              server&rsquo;s own request pipeline over a socket driver that is tested to answer as
              the server&rsquo;s does, and every request takes on the token&rsquo;s role and claims,
              so row-level security applies unchanged. The schema cache is kept in Workers KV. There
              is no GraphQL on Workers; use the server or Lambda for that.
            </p>
            <div class="bg-neutral-900 rounded-xl overflow-hidden mb-4">
              <pre class="p-4 text-sm overflow-x-auto">
                <code class="text-neutral-100">{`npx wrangler hyperdrive create postrust --connection-string="postgres://authenticator:...@db:5432/app"
npx wrangler kv namespace create POSTRUST_SCHEMA_CACHE

# In a checkout: copy wrangler.example.toml to wrangler.toml, fill in the IDs
cd crates/postrust-worker
npx wrangler secret put PGRST_JWT_SECRET
npx wrangler deploy`}</code>
              </pre>
            </div>
            <p class="text-neutral-600 text-sm">
              How it differs from the server &mdash; what it serves, where its answers differ,
              conformance, measured costs and limits &mdash; is on the{" "}
              <Link href="/docs/workers/" class="text-primary-600 hover:text-primary-700 underline">
                Cloudflare Workers
              </Link>{" "}
              page.
            </p>
          </section>

          {/* Fly.io */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">Fly.io</h2>
            <div class="bg-neutral-900 rounded-xl overflow-hidden">
              <pre class="p-4 text-sm overflow-x-auto">
                <code class="text-neutral-100">{`# Install Fly CLI
curl -L https://fly.io/install.sh | sh

# Launch app
fly launch

# Set secrets
fly secrets set DATABASE_URL="postgres://..."
fly secrets set PGRST_JWT_SECRET="..."

# Deploy
fly deploy`}</code>
              </pre>
            </div>
          </section>

          {/* Railway */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">Railway</h2>
            <div class="bg-neutral-900 rounded-xl overflow-hidden">
              <pre class="p-4 text-sm overflow-x-auto">
                <code class="text-neutral-100">{`# Install Railway CLI
npm install -g @railway/cli

# Login and initialize
railway login
railway init

# Set variables
railway variables set DATABASE_URL="postgres://..."

# Deploy
railway up`}</code>
              </pre>
            </div>
          </section>

          {/* Checklist */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">Production Checklist</h2>
            <div class="space-y-3">
              {[
                "Use HTTPS/TLS in production",
                "Set strong JWT secret (min 32 characters)",
                "Configure proper CORS origins",
                "Enable Row Level Security on all tables",
                "Set appropriate MAX_ROWS limit",
                "Configure health checks",
                "Set up log aggregation",
                "Enable database backups",
              ].map((item) => (
                <div key={item} class="flex items-start gap-3">
                  <div class="w-5 h-5 border-2 border-neutral-300 rounded mt-0.5 flex-shrink-0"></div>
                  <span class="text-neutral-700">{item}</span>
                </div>
              ))}
            </div>
          </section>

          {/* Next */}
          <div class="flex items-center justify-between pt-8 border-t border-neutral-200">
            <Link
              href="/docs/configuration/"
              class="flex items-center gap-2 text-neutral-600 hover:text-primary-600"
            >
              <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 19l-7-7 7-7"/>
              </svg>
              Configuration
            </Link>
            <Link
              href="/docs/graphql/"
              class="flex items-center gap-2 text-neutral-600 hover:text-primary-600"
            >
              GraphQL
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
  title: "Deployment - Postrust Documentation",
  meta: [
    {
      name: "description",
      content: "Deploy Postrust to Docker, AWS Lambda, Cloudflare Workers, Kubernetes, Fly.io, Railway, and more.",
    },
  ],
};
