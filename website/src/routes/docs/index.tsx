import { component$ } from "@builder.io/qwik";
import type { DocumentHead } from "@builder.io/qwik-city";
import { Link } from "@builder.io/qwik-city";

const docSections = [
  {
    title: "Getting Started",
    description: "Quick start guide to get your first API running in minutes",
    href: "/docs/getting-started",
    icon: "rocket",
  },
  {
    title: "API Reference",
    description: "Complete reference for REST endpoints, operators, and headers",
    href: "/docs/api-reference",
    icon: "code",
  },
  {
    title: "GraphQL",
    description: "Full GraphQL API with queries, mutations, and filtering",
    href: "/docs/graphql",
    icon: "graphql",
  },
  {
    title: "Authentication",
    description: "JWT authentication and PostgreSQL Row-Level Security",
    href: "/docs/authentication",
    icon: "shield",
  },
  {
    title: "Configuration",
    description: "Environment variables and runtime configuration options",
    href: "/docs/configuration",
    icon: "settings",
  },
  {
    title: "Custom Routes",
    description: "Extend Postrust with custom Rust handlers for webhooks and more",
    href: "/docs/custom-routes",
    icon: "puzzle",
  },
  {
    title: "Deployment",
    description: "Deploy to Docker, AWS Lambda, Kubernetes, and more",
    href: "/docs/deployment",
    icon: "cloud",
  },
];

const iconPaths: Record<string, string> = {
  rocket: "M13 10V3L4 14h7v7l9-11h-7z",
  code: "M10 20l4-16m4 4l4 4-4 4M6 16l-4-4 4-4",
  graphql: "M12 2L2 7l10 5 10-5-10-5zM2 17l10 5 10-5M2 12l10 5 10-5",
  shield: "M9 12l2 2 4-4m5.618-4.016A11.955 11.955 0 0112 2.944a11.955 11.955 0 01-8.618 3.04A12.02 12.02 0 003 9c0 5.591 3.824 10.29 9 11.622 5.176-1.332 9-6.03 9-11.622 0-1.042-.133-2.052-.382-3.016z",
  settings: "M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.065 2.572c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.572 1.065c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.065-2.572c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z M15 12a3 3 0 11-6 0 3 3 0 016 0z",
  puzzle: "M11 4a2 2 0 114 0v1a1 1 0 001 1h3a1 1 0 011 1v3a1 1 0 01-1 1h-1a2 2 0 100 4h1a1 1 0 011 1v3a1 1 0 01-1 1h-3a1 1 0 01-1-1v-1a2 2 0 10-4 0v1a1 1 0 01-1 1H7a1 1 0 01-1-1v-3a1 1 0 00-1-1H4a2 2 0 110-4h1a1 1 0 001-1V7a1 1 0 011-1h3a1 1 0 001-1V4z",
  cloud: "M7 16a4 4 0 01-.88-7.903A5 5 0 1115.9 6L16 6a5 5 0 011 9.9M15 13l-3-3m0 0l-3 3m3-3v12",
};

export default component$(() => {
  return (
    <div class="section-padding bg-neutral-50 min-h-screen">
      <div class="container-wide">
        {/* Header */}
        <div class="max-w-3xl mb-12">
          <h1 class="text-4xl font-bold text-neutral-900 mb-4">
            Documentation
          </h1>
          <p class="text-lg text-neutral-600">
            Everything you need to build, deploy, and scale your PostgreSQL API with Postrust.
          </p>
        </div>

        {/* Quick Start */}
        <div class="bg-gradient-to-r from-primary-600 to-primary-700 rounded-2xl p-8 mb-12 text-white">
          <div class="flex flex-col lg:flex-row items-start lg:items-center justify-between gap-6">
            <div>
              <h2 class="text-2xl font-bold mb-2">Quick Start</h2>
              <p class="text-primary-100 max-w-xl">
                Get your PostgreSQL API running in under 5 minutes with Docker.
              </p>
            </div>
            <div class="bg-neutral-900/50 rounded-lg p-4 font-mono text-sm min-w-[300px]">
              <span class="text-green-400">$</span> docker run -p 3000:3000 \
              <br />
              &nbsp;&nbsp;-e DATABASE_URL="..." \
              <br />
              &nbsp;&nbsp;postrust/postrust
            </div>
          </div>
        </div>

        {/* Doc Sections Grid */}
        <div class="grid md:grid-cols-2 lg:grid-cols-3 gap-6">
          {docSections.map((section) => (
            <Link
              key={section.href}
              href={section.href}
              class="group bg-white rounded-xl p-6 border border-neutral-200 hover:border-primary-200 hover:shadow-lg transition-all"
            >
              <div class="w-12 h-12 bg-primary-100 group-hover:bg-primary-500 rounded-lg flex items-center justify-center mb-4 transition-colors">
                <svg
                  class="w-6 h-6 text-primary-600 group-hover:text-white transition-colors"
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
              <h3 class="text-lg font-semibold text-neutral-900 mb-2 group-hover:text-primary-600 transition-colors">
                {section.title}
              </h3>
              <p class="text-sm text-neutral-600">
                {section.description}
              </p>
              <div class="mt-4 flex items-center text-sm font-medium text-primary-600 opacity-0 group-hover:opacity-100 transition-opacity">
                Read more
                <svg class="w-4 h-4 ml-1" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13 7l5 5m0 0l-5 5m5-5H6"/>
                </svg>
              </div>
            </Link>
          ))}
        </div>

        {/* Help Section */}
        <div class="mt-16 bg-white rounded-2xl p-8 border border-neutral-200">
          <div class="flex flex-col md:flex-row items-start md:items-center justify-between gap-6">
            <div>
              <h2 class="text-xl font-bold text-neutral-900 mb-2">
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
                class="inline-flex items-center gap-2 px-4 py-2 text-sm font-medium text-neutral-700 bg-neutral-100 hover:bg-neutral-200 rounded-lg transition-colors"
              >
                <svg class="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 8v4m0 4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"/>
                </svg>
                Issues
              </a>
              <a
                href="https://github.com/postrust/postrust"
                target="_blank"
                rel="noopener noreferrer"
                class="inline-flex items-center gap-2 px-4 py-2 text-sm font-medium text-neutral-700 bg-neutral-100 hover:bg-neutral-200 rounded-lg transition-colors"
              >
                <svg class="w-5 h-5" fill="currentColor" viewBox="0 0 24 24">
                  <path fill-rule="evenodd" d="M12 2C6.477 2 2 6.484 2 12.017c0 4.425 2.865 8.18 6.839 9.504.5.092.682-.217.682-.483 0-.237-.008-.868-.013-1.703-2.782.605-3.369-1.343-3.369-1.343-.454-1.158-1.11-1.466-1.11-1.466-.908-.62.069-.608.069-.608 1.003.07 1.531 1.032 1.531 1.032.892 1.53 2.341 1.088 2.91.832.092-.647.35-1.088.636-1.338-2.22-.253-4.555-1.113-4.555-4.951 0-1.093.39-1.988 1.029-2.688-.103-.253-.446-1.272.098-2.65 0 0 .84-.27 2.75 1.026A9.564 9.564 0 0112 6.844c.85.004 1.705.115 2.504.337 1.909-1.296 2.747-1.027 2.747-1.027.546 1.379.202 2.398.1 2.651.64.7 1.028 1.595 1.028 2.688 0 3.848-2.339 4.695-4.566 4.943.359.309.678.92.678 1.855 0 1.338-.012 2.419-.012 2.747 0 .268.18.58.688.482A10.019 10.019 0 0022 12.017C22 6.484 17.522 2 12 2z" clip-rule="evenodd"/>
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
  title: "Documentation - Postrust",
  meta: [
    {
      name: "description",
      content: "Complete documentation for Postrust - the high-performance PostgreSQL REST & GraphQL API server written in Rust.",
    },
  ],
};
