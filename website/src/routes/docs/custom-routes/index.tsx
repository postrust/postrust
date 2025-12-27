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
            <span class="text-neutral-900">Custom Routes</span>
          </div>
          <h1 class="text-4xl font-bold text-neutral-900 mb-4">Custom Routes</h1>
          <p class="text-lg text-neutral-600 max-w-2xl">
            Extend Postrust with custom Rust handlers for webhooks, background jobs, and custom business logic.
          </p>
        </div>
      </div>

      <div class="container-wide py-12">
        <div class="max-w-3xl">
          {/* Overview */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">Overview</h2>
            <p class="text-neutral-600 mb-4">
              Custom routes allow you to add your own endpoints alongside the auto-generated REST and GraphQL APIs.
              This is useful for:
            </p>
            <ul class="space-y-2 text-neutral-700">
              <li class="flex items-start gap-3">
                <svg class="w-5 h-5 text-primary-500 mt-0.5 flex-shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 13l4 4L19 7"/>
                </svg>
                Webhook handlers (Stripe, GitHub, etc.)
              </li>
              <li class="flex items-start gap-3">
                <svg class="w-5 h-5 text-primary-500 mt-0.5 flex-shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 13l4 4L19 7"/>
                </svg>
                Custom authentication flows
              </li>
              <li class="flex items-start gap-3">
                <svg class="w-5 h-5 text-primary-500 mt-0.5 flex-shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 13l4 4L19 7"/>
                </svg>
                Background job triggers
              </li>
              <li class="flex items-start gap-3">
                <svg class="w-5 h-5 text-primary-500 mt-0.5 flex-shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 13l4 4L19 7"/>
                </svg>
                Custom business logic
              </li>
            </ul>
          </section>

          {/* Basic Example */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">Basic Example</h2>
            <p class="text-neutral-600 mb-4">
              Edit <code class="bg-neutral-100 px-2 py-0.5 rounded text-sm">crates/postrust-server/src/custom.rs</code>:
            </p>
            <div class="bg-neutral-900 rounded-xl overflow-hidden">
              <div class="px-4 py-2 bg-neutral-800 border-b border-neutral-700">
                <span class="text-sm text-neutral-400">Rust</span>
              </div>
              <pre class="p-4 text-sm overflow-x-auto">
                <code class="text-neutral-100">{`use axum::{Router, Json, routing::post};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
struct WebhookPayload {
    event: String,
    data: serde_json::Value,
}

#[derive(Serialize)]
struct WebhookResponse {
    received: bool,
}

async fn handle_webhook(
    Json(payload): Json<WebhookPayload>,
) -> Json<WebhookResponse> {
    println!("Received webhook: {}", payload.event);

    // Process the webhook...

    Json(WebhookResponse { received: true })
}

pub fn custom_routes() -> Router {
    Router::new()
        .route("/webhooks/stripe", post(handle_webhook))
}`}</code>
              </pre>
            </div>
          </section>

          {/* With Database */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">Accessing the Database</h2>
            <div class="bg-neutral-900 rounded-xl overflow-hidden">
              <div class="px-4 py-2 bg-neutral-800 border-b border-neutral-700">
                <span class="text-sm text-neutral-400">Rust</span>
              </div>
              <pre class="p-4 text-sm overflow-x-auto">
                <code class="text-neutral-100">{`use axum::{Router, Json, Extension, routing::get};
use sqlx::PgPool;

async fn get_stats(
    Extension(pool): Extension<PgPool>,
) -> Json<serde_json::Value> {
    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users")
        .fetch_one(&pool)
        .await
        .unwrap();

    Json(serde_json::json!({
        "user_count": count.0
    }))
}

pub fn custom_routes() -> Router {
    Router::new()
        .route("/stats", get(get_stats))
}`}</code>
              </pre>
            </div>
          </section>

          {/* Building */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">Building with Custom Routes</h2>
            <div class="bg-neutral-900 rounded-xl overflow-hidden">
              <pre class="p-4 text-sm overflow-x-auto">
                <code class="text-neutral-100">{`# Build with your custom routes
cargo build --release -p postrust-server

# Build with admin UI
cargo build --release -p postrust-server --features admin-ui

# Run
./target/release/postrust`}</code>
              </pre>
            </div>
          </section>

          {/* Next */}
          <div class="flex items-center justify-between pt-8 border-t border-neutral-200">
            <Link
              href="/docs/graphql"
              class="flex items-center gap-2 text-neutral-600 hover:text-primary-600"
            >
              <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 19l-7-7 7-7"/>
              </svg>
              GraphQL
            </Link>
            <Link
              href="/docs"
              class="flex items-center gap-2 text-neutral-600 hover:text-primary-600"
            >
              Back to Docs
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
  title: "Custom Routes - Postrust Documentation",
  meta: [
    {
      name: "description",
      content: "Extend Postrust with custom Rust handlers for webhooks, background jobs, and custom business logic.",
    },
  ],
};
