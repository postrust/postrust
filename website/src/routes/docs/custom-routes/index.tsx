import { component$ } from "@builder.io/qwik";
import type { DocumentHead } from "@builder.io/qwik-city";
import { Link } from "@builder.io/qwik-city";

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
            <span class="text-neutral-900">Custom Routes</span>
          </div>
          <h1 class="mb-4 text-4xl font-bold text-neutral-900">
            Custom Routes
          </h1>
          <p class="max-w-2xl text-lg text-neutral-600">
            Add Rust handlers of your own beside Postrust's API, for webhooks,
            jobs and business logic, running as the caller under row-level
            security.
          </p>
        </div>
      </div>

      <div class="container-wide py-12">
        <div class="max-w-3xl">
          {/* Overview */}
          <section class="mb-12">
            <h2 class="mb-4 text-2xl font-bold text-neutral-900">
              How custom routes work
            </h2>
            <p class="mb-4 text-neutral-600">
              Custom routes allow you to add your own endpoints alongside the
              auto-generated REST and GraphQL APIs. This is useful for:
            </p>
            <ul class="space-y-2 text-neutral-700">
              <li class="flex items-start gap-3">
                <svg
                  class="text-primary-500 mt-0.5 h-5 w-5 flex-shrink-0"
                  fill="none"
                  stroke="currentColor"
                  viewBox="0 0 24 24"
                >
                  <path
                    stroke-linecap="round"
                    stroke-linejoin="round"
                    stroke-width="2"
                    d="M5 13l4 4L19 7"
                  />
                </svg>
                Webhook handlers (Stripe, GitHub, etc.)
              </li>
              <li class="flex items-start gap-3">
                <svg
                  class="text-primary-500 mt-0.5 h-5 w-5 flex-shrink-0"
                  fill="none"
                  stroke="currentColor"
                  viewBox="0 0 24 24"
                >
                  <path
                    stroke-linecap="round"
                    stroke-linejoin="round"
                    stroke-width="2"
                    d="M5 13l4 4L19 7"
                  />
                </svg>
                Custom authentication flows
              </li>
              <li class="flex items-start gap-3">
                <svg
                  class="text-primary-500 mt-0.5 h-5 w-5 flex-shrink-0"
                  fill="none"
                  stroke="currentColor"
                  viewBox="0 0 24 24"
                >
                  <path
                    stroke-linecap="round"
                    stroke-linejoin="round"
                    stroke-width="2"
                    d="M5 13l4 4L19 7"
                  />
                </svg>
                Background job triggers
              </li>
              <li class="flex items-start gap-3">
                <svg
                  class="text-primary-500 mt-0.5 h-5 w-5 flex-shrink-0"
                  fill="none"
                  stroke="currentColor"
                  viewBox="0 0 24 24"
                >
                  <path
                    stroke-linecap="round"
                    stroke-linejoin="round"
                    stroke-width="2"
                    d="M5 13l4 4L19 7"
                  />
                </svg>
                Custom business logic
              </li>
            </ul>
          </section>

          {/* A crate of your own */}
          <section class="mb-12">
            <h2 class="mb-4 text-2xl font-bold text-neutral-900">
              A crate of your own
            </h2>
            <p class="mb-4 text-neutral-600">
              You don't fork Postrust. Write a small binary crate that depends
              on{" "}
              <code class="rounded bg-neutral-100 px-2 py-0.5 text-sm">
                postrust-server
              </code>
              , build the application with{" "}
              <code class="rounded bg-neutral-100 px-2 py-0.5 text-sm">
                postrust_server::Builder
              </code>
              , and nest your own
              <a
                href="https://docs.rs/axum/0.8"
                class="text-primary-600 hover:underline"
              >
                axum
              </a>{" "}
              routers into it. Everything the{" "}
              <code class="rounded bg-neutral-100 px-2 py-0.5 text-sm">
                postrust
              </code>{" "}
              binary serves is still served, configured from the same
              <code class="rounded bg-neutral-100 px-2 py-0.5 text-sm">
                PGRST_*
              </code>{" "}
              environment variables. It needs Rust 1.94 or later.
            </p>
            <div class="overflow-hidden rounded-xl bg-neutral-900">
              <div class="border-b border-neutral-700 bg-neutral-800 px-4 py-2">
                <span class="text-sm text-neutral-400">Cargo.toml</span>
              </div>
              <pre class="overflow-x-auto p-4 text-sm">
                <code class="text-neutral-100">{`[dependencies]
postrust-server = "3"
postrust-core = "3"
axum = "0.8"
tokio = { version = "1", features = ["full"] }
sqlx = { version = "0.9", default-features = false, features = ["postgres"] }
serde_json = "1"
anyhow = "1"`}</code>
              </pre>
            </div>
          </section>

          {/* Builder */}
          <section class="mb-12">
            <h2 class="mb-4 text-2xl font-bold text-neutral-900">
              Adding routes
            </h2>
            <p class="mb-4 text-neutral-600">
              A nested router takes the same state as the API's own routes: the
              pool, the schema cache and the configuration.
            </p>
            <div class="overflow-hidden rounded-xl bg-neutral-900">
              <div class="border-b border-neutral-700 bg-neutral-800 px-4 py-2">
                <span class="text-sm text-neutral-400">Rust</span>
              </div>
              <pre class="overflow-x-auto p-4 text-sm">
                <code class="text-neutral-100">{`use axum::{extract::State, routing::{get, post}, Json, Router};
use postrust_server::{AppState, Builder, Host};
use std::sync::Arc;

fn hooks() -> Router<Arc<AppState>> {
    Router::new()
        .route("/stripe", post(stripe))
        .route("/stats", get(stats))
}

async fn stripe(Json(event): Json<serde_json::Value>) -> Json<serde_json::Value> {
    // Verify and handle the webhook...
    Json(serde_json::json!({ "received": true }))
}

async fn stats(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let tables = state.schema_cache().await.tables.len();
    Json(serde_json::json!({ "tables": tables }))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = postrust_core::AppConfig::try_from_env()?;
    let addr = format!("{}:{}", config.server_host, config.server_port);

    let app = Builder::new(config, Host::SERVER)
        .nest("/hooks", hooks())
        .build()
        .await?;

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app.router).await?;
    Ok(())
}`}</code>
              </pre>
            </div>
            <p class="mt-4 text-neutral-600">
              <code class="rounded bg-neutral-100 px-2 py-0.5 text-sm">
                Builder::layer
              </code>{" "}
              wraps the whole application, the API's own routes included, in a
              tower layer of your own: rate limiting, a header, a log line.
            </p>
          </section>

          {/* As the caller */}
          <section class="mb-12">
            <h2 class="mb-4 text-2xl font-bold text-neutral-900">
              Querying as the caller, under RLS
            </h2>
            <p class="mb-4 text-neutral-600">
              <code class="rounded bg-neutral-100 px-2 py-0.5 text-sm">
                state.pool
              </code>{" "}
              connects as the authenticator role. A query sent on it directly
              has no row-level security and every grant the authenticator has.
              For a query that should see what the caller may see, take a{" "}
              <code class="rounded bg-neutral-100 px-2 py-0.5 text-sm">
                User
              </code>{" "}
              and begin the transaction with
              <code class="rounded bg-neutral-100 px-2 py-0.5 text-sm">
                begin_as
              </code>
              :
            </p>
            <div class="overflow-hidden rounded-xl bg-neutral-900">
              <div class="border-b border-neutral-700 bg-neutral-800 px-4 py-2">
                <span class="text-sm text-neutral-400">Rust</span>
              </div>
              <pre class="overflow-x-auto p-4 text-sm">
                <code class="text-neutral-100">{`use postrust_server::{AppState, User};

async fn my_todos(
    user: User,
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, postrust_server::Error> {
    let mut tx = state.begin_as(&user).await?;
    let (total,): (i64,) = sqlx::query_as("SELECT count(*) FROM todos")
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Json(serde_json::json!({ "role": user.role(), "todos": total })))
}`}</code>
              </pre>
            </div>
            <ul class="mt-4 list-disc space-y-2 pl-6 text-neutral-700">
              <li>
                <code class="rounded bg-neutral-100 px-2 py-0.5 text-sm">
                  User
                </code>{" "}
                authenticates the request as REST does: the same token, the same
                anonymous role, the same 401 for a token that does not verify.
                Use{" "}
                <code class="rounded bg-neutral-100 px-2 py-0.5 text-sm">
                  Option&lt;User&gt;
                </code>{" "}
                for a route that also serves requests without a token.
              </li>
              <li>
                <code class="rounded bg-neutral-100 px-2 py-0.5 text-sm">
                  begin_as
                </code>{" "}
                sets the role,{" "}
                <code class="rounded bg-neutral-100 px-2 py-0.5 text-sm">
                  request.jwt.claims
                </code>
                ,{" "}
                <code class="rounded bg-neutral-100 px-2 py-0.5 text-sm">
                  request.headers
                </code>
                ,
                <code class="rounded bg-neutral-100 px-2 py-0.5 text-sm">
                  app.settings.*
                </code>{" "}
                and the rest exactly as a REST request's transaction is set up,
                and calls{" "}
                <code class="rounded bg-neutral-100 px-2 py-0.5 text-sm">
                  PGRST_DB_PRE_REQUEST
                </code>
                .{" "}
                <code class="rounded bg-neutral-100 px-2 py-0.5 text-sm">
                  begin_as_with
                </code>{" "}
                takes
                <code class="rounded bg-neutral-100 px-2 py-0.5 text-sm">
                  TxOptions
                </code>{" "}
                for a read-only transaction or another isolation level.
              </li>
              <li>
                <code class="rounded bg-neutral-100 px-2 py-0.5 text-sm">
                  postrust_server::Error
                </code>{" "}
                answers a failure with PostgREST's status and body, so a
                permission the database refuses is the same 401 or 403 REST
                would give.
              </li>
            </ul>
          </section>

          {/* Prefixes */}
          <section class="mb-12">
            <h2 class="mb-4 text-2xl font-bold text-neutral-900">
              Where routes may go
            </h2>
            <p class="mb-4 text-neutral-600">
              <code class="rounded bg-neutral-100 px-2 py-0.5 text-sm">
                build()
              </code>{" "}
              returns an error, before connecting to anything, for a prefix that
              is
              <code class="rounded bg-neutral-100 px-2 py-0.5 text-sm">/</code>,
              contains a parameter or wildcard, or overlaps a path the server
              serves itself:
              <code class="rounded bg-neutral-100 px-2 py-0.5 text-sm">
                /api
              </code>
              ,{" "}
              <code class="rounded bg-neutral-100 px-2 py-0.5 text-sm">
                /healthz
              </code>
              ,{" "}
              <code class="rounded bg-neutral-100 px-2 py-0.5 text-sm">
                /v1/version
              </code>
              ,{" "}
              <code class="rounded bg-neutral-100 px-2 py-0.5 text-sm">
                /v1/graphql
              </code>
              ,
              <code class="rounded bg-neutral-100 px-2 py-0.5 text-sm">
                /v1alpha1/graphql
              </code>
              ,{" "}
              <code class="rounded bg-neutral-100 px-2 py-0.5 text-sm">
                /admin
              </code>
              ,{" "}
              <code class="rounded bg-neutral-100 px-2 py-0.5 text-sm">
                /_/health
              </code>
              ,{" "}
              <code class="rounded bg-neutral-100 px-2 py-0.5 text-sm">
                /_/ready
              </code>{" "}
              or
              <code class="rounded bg-neutral-100 px-2 py-0.5 text-sm">
                /_
              </code>{" "}
              itself. Two prefixes you nest may not overlap either. Beneath{" "}
              <code class="rounded bg-neutral-100 px-2 py-0.5 text-sm">/_</code>{" "}
              is yours:{" "}
              <code class="rounded bg-neutral-100 px-2 py-0.5 text-sm">
                .nest("/_/hooks", router)
              </code>{" "}
              serves next to the health checks.
            </p>
            <p class="text-neutral-600">
              In compatibility mode REST is also served at{" "}
              <code class="rounded bg-neutral-100 px-2 py-0.5 text-sm">/</code>,
              and a nested route comes first: a prefix of{" "}
              <code class="rounded bg-neutral-100 px-2 py-0.5 text-sm">
                /orders
              </code>{" "}
              answers in place of the{" "}
              <code class="rounded bg-neutral-100 px-2 py-0.5 text-sm">
                orders
              </code>{" "}
              table, which is still at
              <code class="rounded bg-neutral-100 px-2 py-0.5 text-sm">
                /api/orders
              </code>
              . Postrust logs a warning at start-up when that happens.
            </p>
          </section>

          {/* Run */}
          <section class="mb-12">
            <h2 class="mb-4 text-2xl font-bold text-neutral-900">Running it</h2>
            <div class="overflow-hidden rounded-xl bg-neutral-900">
              <div class="border-b border-neutral-700 bg-neutral-800 px-4 py-2">
                <span class="text-sm text-neutral-400">Shell</span>
              </div>
              <pre class="overflow-x-auto p-4 text-sm">
                <code class="text-neutral-100">{`PGRST_DB_URI=postgres://... PGRST_JWT_SECRET=... cargo run --release
curl -X POST localhost:3000/hooks/stripe -d '{"type":"charge.succeeded"}'`}</code>
              </pre>
            </div>
            <p class="mt-4 text-neutral-600">
              A complete, compiling example with a row-level security policy is
              in
              <a
                href="https://github.com/postrust/postrust/tree/main/examples/custom-routes"
                class="text-primary-600 hover:underline"
              >
                examples/custom-routes
              </a>
              .
            </p>
          </section>

          {/* Next */}
          <div class="flex items-center justify-between border-t border-neutral-200 pt-8">
            <Link
              href="/docs/graphql/"
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
              GraphQL
            </Link>
            <Link
              href="/docs/"
              class="hover:text-primary-600 flex items-center gap-2 text-neutral-600"
            >
              Back to Docs
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
  title: "Custom Routes - Postrust Documentation",
  meta: [
    {
      name: "description",
      content:
        "Add Rust handlers of your own beside Postrust's API, for webhooks, jobs and business logic, running as the caller under row-level security.",
    },
  ],
};
