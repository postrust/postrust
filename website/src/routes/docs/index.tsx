import { component$ } from "@builder.io/qwik";
import type { DocumentHead } from "@builder.io/qwik-city";
import { Link } from "@builder.io/qwik-city";
import { docSections } from "~/data/docs";
import { guides } from "~/data/guides";

const iconPaths: Record<string, string> = {
  rocket: "M13 10V3L4 14h7v7l9-11h-7z",
  code: "M10 20l4-16m4 4l4 4-4 4M6 16l-4-4 4-4",
  graphql: "M12 2L2 7l10 5 10-5-10-5zM2 17l10 5 10-5M2 12l10 5 10-5",
  realtime: "M13 10V3L4 14h7v7l9-11h-7z",
  vector:
    "M9 3v2m6-2v2M9 19v2m6-2v2M5 9H3m2 6H3m18-6h-2m2 6h-2M7 19h10a2 2 0 002-2V7a2 2 0 00-2-2H7a2 2 0 00-2 2v10a2 2 0 002 2zM9 9h6v6H9V9z",
  shield:
    "M9 12l2 2 4-4m5.618-4.016A11.955 11.955 0 0112 2.944a11.955 11.955 0 01-8.618 3.04A12.02 12.02 0 003 9c0 5.591 3.824 10.29 9 11.622 5.176-1.332 9-6.03 9-11.622 0-1.042-.133-2.052-.382-3.016z",
  settings:
    "M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.065 2.572c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.572 1.065c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.065-2.572c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z M15 12a3 3 0 11-6 0 3 3 0 016 0z",
  puzzle:
    "M11 4a2 2 0 114 0v1a1 1 0 001 1h3a1 1 0 011 1v3a1 1 0 01-1 1h-1a2 2 0 100 4h1a1 1 0 011 1v3a1 1 0 01-1 1h-3a1 1 0 01-1-1v-1a2 2 0 10-4 0v1a1 1 0 01-1 1H7a1 1 0 01-1-1v-3a1 1 0 00-1-1H4a2 2 0 110-4h1a1 1 0 001-1V7a1 1 0 011-1h3a1 1 0 001-1V4z",
  globe:
    "M21 12a9 9 0 01-9 9m9-9a9 9 0 00-9-9m9 9H3m9 9a9 9 0 01-9-9m9 9c1.657 0 3-4.03 3-9s-1.343-9-3-9m0 18c-1.657 0-3-4.03-3-9s1.343-9 3-9m-9 9a9 9 0 019-9",
  cloud:
    "M7 16a4 4 0 01-.88-7.903A5 5 0 1115.9 6L16 6a5 5 0 011 9.9M15 13l-3-3m0 0l-3 3m3-3v12",
  chart:
    "M9 19v-6a2 2 0 00-2-2H5a2 2 0 00-2 2v6a2 2 0 002 2h2a2 2 0 002-2zm0 0V9a2 2 0 012-2h2a2 2 0 012 2v10m-6 0a2 2 0 002 2h2a2 2 0 002-2m0 0V5a2 2 0 012-2h2a2 2 0 012 2v14a2 2 0 01-2 2h-2a2 2 0 01-2-2z",
};

export default component$(() => {
  return (
    <div class="section-padding min-h-screen bg-neutral-50">
      <div class="container-wide">
        {/* Header */}
        <div class="mb-12 max-w-3xl">
          <h1 class="mb-4 text-4xl font-bold text-neutral-900">
            Documentation
          </h1>
          <p class="text-lg text-neutral-600">
            Everything you need to build, deploy, and scale your PostgreSQL API
            with Postrust.
          </p>
        </div>

        {/* Quick Start */}
        <div class="from-primary-600 to-primary-700 mb-12 rounded-2xl bg-gradient-to-r p-8 text-white">
          <div class="flex flex-col items-start justify-between gap-6 lg:flex-row lg:items-center">
            <div>
              <h2 class="mb-2 text-2xl font-bold">Quick Start</h2>
              <p class="text-primary-100 max-w-xl">
                Get your PostgreSQL API running in under 5 minutes with Docker.
              </p>
            </div>
            <div class="min-w-[300px] rounded-lg bg-neutral-900/50 p-4 font-mono text-sm">
              <span class="text-green-400">$</span> docker run -p 3000:3000 \
              <br />
              &nbsp;&nbsp;-e DATABASE_URL="..." \
              <br />
              &nbsp;&nbsp;postrust/postrust
            </div>
          </div>
        </div>

        {/* Doc Sections Grid */}
        <div class="grid gap-6 md:grid-cols-2 lg:grid-cols-3">
          {docSections.map((section) => (
            <Link
              key={section.href}
              href={section.href}
              class="group hover:border-primary-200 rounded-xl border border-neutral-200 bg-white p-6 transition-all hover:shadow-lg"
            >
              <div class="bg-primary-100 group-hover:bg-primary-500 mb-4 flex h-12 w-12 items-center justify-center rounded-lg transition-colors">
                <svg
                  class="text-primary-600 h-6 w-6 transition-colors group-hover:text-white"
                  fill="none"
                  stroke="currentColor"
                  viewBox="0 0 24 24"
                  stroke-width="2"
                  stroke-linecap="round"
                  stroke-linejoin="round"
                >
                  <path d={iconPaths[section.icon]} />
                </svg>
              </div>
              <h3 class="group-hover:text-primary-600 mb-2 text-lg font-semibold text-neutral-900 transition-colors">
                {section.title}
              </h3>
              <p class="text-sm text-neutral-600">{section.description}</p>
              <div class="text-primary-600 group-hover:text-primary-700 mt-4 flex items-center text-sm font-medium transition-colors">
                Read {section.title} docs
                <svg
                  class="ml-1 h-4 w-4"
                  fill="none"
                  stroke="currentColor"
                  viewBox="0 0 24 24"
                >
                  <path
                    stroke-linecap="round"
                    stroke-linejoin="round"
                    stroke-width="2"
                    d="M13 7l5 5m0 0l-5 5m5-5H6"
                  />
                </svg>
              </div>
            </Link>
          ))}
        </div>

        {/* Engineering Guides */}
        <div class="mt-16">
          <div class="mb-8 flex flex-col justify-between gap-4 sm:flex-row sm:items-end">
            <div>
              <h2 class="mb-2 text-2xl font-bold text-neutral-900">
                Engineering Guides
              </h2>
              <p class="text-neutral-600">
                Deep dives into generating REST and GraphQL APIs from
                PostgreSQL, enforcing Row-Level Security, and comparing API
                servers.
              </p>
            </div>
            <Link
              href="/guides/"
              class="text-primary-600 hover:text-primary-700 text-sm font-semibold whitespace-nowrap"
            >
              View all PostgreSQL API guides &rarr;
            </Link>
          </div>
          <div class="grid gap-6 md:grid-cols-2 lg:grid-cols-3">
            {guides.map((guide) => (
              <Link
                key={guide.slug}
                href={`/guides/${guide.slug}/`}
                class="group hover:border-primary-200 rounded-xl border border-neutral-200 bg-white p-6 transition-all hover:shadow-lg"
              >
                <h3 class="group-hover:text-primary-600 mb-2 text-base font-semibold text-neutral-900 transition-colors">
                  {guide.headTitle}
                </h3>
                <p class="text-sm text-neutral-600">{guide.description}</p>
              </Link>
            ))}
          </div>
        </div>

        {/* Help Section */}
        <div class="mt-16 rounded-2xl border border-neutral-200 bg-white p-8">
          <div class="flex flex-col items-start justify-between gap-6 md:flex-row md:items-center">
            <div>
              <h2 class="mb-2 text-xl font-bold text-neutral-900">
                Need help?
              </h2>
              <p class="text-neutral-600">
                Join our community for support, discussions, and updates.
              </p>
            </div>
            <div class="flex items-center gap-4">
              <a
                href="https://github.com/postrust/postrust/issues"
                target="_blank"
                rel="noopener noreferrer"
                class="inline-flex items-center gap-2 rounded-lg bg-neutral-100 px-4 py-2 text-sm font-medium text-neutral-700 transition-colors hover:bg-neutral-200"
              >
                <svg
                  class="h-5 w-5"
                  fill="none"
                  stroke="currentColor"
                  viewBox="0 0 24 24"
                >
                  <path
                    stroke-linecap="round"
                    stroke-linejoin="round"
                    stroke-width="2"
                    d="M12 8v4m0 4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"
                  />
                </svg>
                Issues
              </a>
              <a
                href="https://github.com/postrust/postrust"
                target="_blank"
                rel="noopener noreferrer"
                class="inline-flex items-center gap-2 rounded-lg bg-neutral-100 px-4 py-2 text-sm font-medium text-neutral-700 transition-colors hover:bg-neutral-200"
              >
                <svg class="h-5 w-5" fill="currentColor" viewBox="0 0 24 24">
                  <path
                    fill-rule="evenodd"
                    d="M12 2C6.477 2 2 6.484 2 12.017c0 4.425 2.865 8.18 6.839 9.504.5.092.682-.217.682-.483 0-.237-.008-.868-.013-1.703-2.782.605-3.369-1.343-3.369-1.343-.454-1.158-1.11-1.466-1.11-1.466-.908-.62.069-.608.069-.608 1.003.07 1.531 1.032 1.531 1.032.892 1.53 2.341 1.088 2.91.832.092-.647.35-1.088.636-1.338-2.22-.253-4.555-1.113-4.555-4.951 0-1.093.39-1.988 1.029-2.688-.103-.253-.446-1.272.098-2.65 0 0 .84-.27 2.75 1.026A9.564 9.564 0 0112 6.844c.85.004 1.705.115 2.504.337 1.909-1.296 2.747-1.027 2.747-1.027.546 1.379.202 2.398.1 2.651.64.7 1.028 1.595 1.028 2.688 0 3.848-2.339 4.695-4.566 4.943.359.309.678.92.678 1.855 0 1.338-.012 2.419-.012 2.747 0 .268.18.58.688.482A10.019 10.019 0 0022 12.017C22 6.484 17.522 2 12 2z"
                    clip-rule="evenodd"
                  />
                </svg>
                GitHub
              </a>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
});

export const head: DocumentHead = {
  title: "Postrust Documentation: Quick Start, REST, GraphQL & Deployment",
  meta: [
    {
      name: "description",
      content:
        "Official documentation for Postrust, the Rust PostgreSQL API server. Learn how to configure JWT auth, Row-Level Security, GraphQL subscriptions, AWS Lambda, and Cloudflare Workers.",
    },
  ],
};
