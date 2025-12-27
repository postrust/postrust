import { component$ } from "@builder.io/qwik";
import type { DocumentHead } from "@builder.io/qwik-city";

const communityLinks = [
  {
    name: "GitHub",
    description: "Star the repo, report issues, and contribute code",
    href: "https://github.com/postrust/postrust",
    icon: "github",
    cta: "View Repository",
  },
  {
    name: "Issues & Features",
    description: "Report bugs, request features, and get involved",
    href: "https://github.com/postrust/postrust/issues",
    icon: "issues",
    cta: "Open an Issue",
  },
  {
    name: "Twitter / X",
    description: "Follow for updates, tips, and announcements",
    href: "https://x.com/postrustorg",
    icon: "twitter",
    cta: "Follow @postrustorg",
  },
];

const contributionAreas = [
  {
    title: "Core Development",
    description: "Help improve the Rust codebase, add features, and fix bugs",
    icon: "code",
  },
  {
    title: "Documentation",
    description: "Improve docs, write tutorials, and create examples",
    icon: "book",
  },
  {
    title: "Testing",
    description: "Write tests, report bugs, and help with QA",
    icon: "test",
  },
  {
    title: "Community Support",
    description: "Answer questions, help users, and moderate discussions",
    icon: "users",
  },
];

const iconPaths: Record<string, string> = {
  github: "M12 2C6.477 2 2 6.484 2 12.017c0 4.425 2.865 8.18 6.839 9.504.5.092.682-.217.682-.483 0-.237-.008-.868-.013-1.703-2.782.605-3.369-1.343-3.369-1.343-.454-1.158-1.11-1.466-1.11-1.466-.908-.62.069-.608.069-.608 1.003.07 1.531 1.032 1.531 1.032.892 1.53 2.341 1.088 2.91.832.092-.647.35-1.088.636-1.338-2.22-.253-4.555-1.113-4.555-4.951 0-1.093.39-1.988 1.029-2.688-.103-.253-.446-1.272.098-2.65 0 0 .84-.27 2.75 1.026A9.564 9.564 0 0112 6.844c.85.004 1.705.115 2.504.337 1.909-1.296 2.747-1.027 2.747-1.027.546 1.379.202 2.398.1 2.651.64.7 1.028 1.595 1.028 2.688 0 3.848-2.339 4.695-4.566 4.943.359.309.678.92.678 1.855 0 1.338-.012 2.419-.012 2.747 0 .268.18.58.688.482A10.019 10.019 0 0022 12.017C22 6.484 17.522 2 12 2z",
  issues: "M12 8v4m0 4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z",
  twitter: "M18.244 2.25h3.308l-7.227 8.26 8.502 11.24H16.17l-5.214-6.817L4.99 21.75H1.68l7.73-8.835L1.254 2.25H8.08l4.713 6.231zm-1.161 17.52h1.833L7.084 4.126H5.117z",
  code: "M10 20l4-16m4 4l4 4-4 4M6 16l-4-4 4-4",
  book: "M12 6.253v13m0-13C10.832 5.477 9.246 5 7.5 5S4.168 5.477 3 6.253v13C4.168 18.477 5.754 18 7.5 18s3.332.477 4.5 1.253m0-13C13.168 5.477 14.754 5 16.5 5c1.747 0 3.332.477 4.5 1.253v13C19.832 18.477 18.247 18 16.5 18c-1.746 0-3.332.477-4.5 1.253",
  test: "M9 5H7a2 2 0 00-2 2v12a2 2 0 002 2h10a2 2 0 002-2V7a2 2 0 00-2-2h-2M9 5a2 2 0 002 2h2a2 2 0 002-2M9 5a2 2 0 012-2h2a2 2 0 012 2m-6 9l2 2 4-4",
  users: "M12 4.354a4 4 0 110 5.292M15 21H3v-1a6 6 0 0112 0v1zm0 0h6v-1a6 6 0 00-9-5.197M13 7a4 4 0 11-8 0 4 4 0 018 0z",
};

export default component$(() => {
  return (
    <div class="min-h-screen bg-white">
      {/* Hero */}
      <section class="section-padding bg-gradient-to-b from-neutral-50 to-white">
        <div class="container-wide">
          <div class="max-w-3xl mx-auto text-center">
            <h1 class="text-4xl md:text-5xl font-bold text-neutral-900 mb-6">
              Join the Community
            </h1>
            <p class="text-lg text-neutral-600">
              Postrust is built by developers, for developers. Join us to contribute,
              learn, and help shape the future of PostgreSQL APIs.
            </p>
          </div>
        </div>
      </section>

      {/* Community Links */}
      <section class="section-padding">
        <div class="container-wide">
          <div class="grid md:grid-cols-3 gap-8">
            {communityLinks.map((link) => (
              <a
                key={link.name}
                href={link.href}
                target="_blank"
                rel="noopener noreferrer"
                class="group bg-white rounded-2xl p-8 border border-neutral-200 hover:border-primary-200 hover:shadow-lg transition-all"
              >
                <div class="w-16 h-16 bg-neutral-100 group-hover:bg-primary-100 rounded-2xl flex items-center justify-center mb-6 transition-colors">
                  <svg
                    class="w-8 h-8 text-neutral-600 group-hover:text-primary-600 transition-colors"
                    fill="currentColor"
                    viewBox="0 0 24 24"
                  >
                    <path d={iconPaths[link.icon]} fill-rule="evenodd" clip-rule="evenodd" />
                  </svg>
                </div>
                <h2 class="text-2xl font-bold text-neutral-900 mb-2 group-hover:text-primary-600 transition-colors">
                  {link.name}
                </h2>
                <p class="text-neutral-600 mb-4">
                  {link.description}
                </p>
                <span class="inline-flex items-center text-primary-600 font-medium">
                  {link.cta}
                  <svg class="w-4 h-4 ml-2 group-hover:translate-x-1 transition-transform" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13 7l5 5m0 0l-5 5m5-5H6"/>
                  </svg>
                </span>
              </a>
            ))}
          </div>
        </div>
      </section>

      {/* Contributing */}
      <section class="section-padding bg-neutral-50">
        <div class="container-wide">
          <div class="max-w-3xl mx-auto text-center mb-12">
            <h2 class="text-3xl font-bold text-neutral-900 mb-4">
              How to Contribute
            </h2>
            <p class="text-lg text-neutral-600">
              There are many ways to contribute to Postrust, regardless of your skill level.
            </p>
          </div>

          <div class="grid md:grid-cols-2 lg:grid-cols-4 gap-6">
            {contributionAreas.map((area) => (
              <div
                key={area.title}
                class="bg-white rounded-xl p-6 border border-neutral-200"
              >
                <div class="w-12 h-12 bg-primary-100 rounded-lg flex items-center justify-center mb-4">
                  <svg
                    class="w-6 h-6 text-primary-600"
                    fill="none"
                    stroke="currentColor"
                    viewBox="0 0 24 24"
                    stroke-width="2"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                  >
                    <path d={iconPaths[area.icon]} />
                  </svg>
                </div>
                <h3 class="text-lg font-semibold text-neutral-900 mb-2">
                  {area.title}
                </h3>
                <p class="text-sm text-neutral-600">
                  {area.description}
                </p>
              </div>
            ))}
          </div>

          <div class="mt-12 text-center">
            <a
              href="https://github.com/postrust/postrust/blob/main/CONTRIBUTING.md"
              target="_blank"
              rel="noopener noreferrer"
              class="inline-flex items-center gap-2 px-6 py-3 text-base font-semibold text-white bg-neutral-900 hover:bg-neutral-800 rounded-lg transition-colors"
            >
              Read Contributing Guide
              <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M10 6H6a2 2 0 00-2 2v10a2 2 0 002 2h10a2 2 0 002-2v-4M14 4h6m0 0v6m0-6L10 14"/>
              </svg>
            </a>
          </div>
        </div>
      </section>

      {/* Code of Conduct */}
      <section class="section-padding">
        <div class="container-wide">
          <div class="max-w-3xl mx-auto bg-neutral-900 rounded-2xl p-8 text-center">
            <h2 class="text-2xl font-bold text-white mb-4">
              Our Commitment
            </h2>
            <p class="text-neutral-300 mb-6">
              We are committed to providing a welcoming and inclusive community.
              All participants are expected to follow our Code of Conduct.
            </p>
            <a
              href="https://github.com/postrust/postrust/blob/main/CODE_OF_CONDUCT.md"
              target="_blank"
              rel="noopener noreferrer"
              class="inline-flex items-center text-primary-400 hover:text-primary-300 font-medium"
            >
              Read Code of Conduct
              <svg class="w-4 h-4 ml-2" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M10 6H6a2 2 0 00-2 2v10a2 2 0 002 2h10a2 2 0 002-2v-4M14 4h6m0 0v6m0-6L10 14"/>
              </svg>
            </a>
          </div>
        </div>
      </section>
    </div>
  );
});

export const head: DocumentHead = {
  title: "Community - Postrust",
  meta: [
    {
      name: "description",
      content: "Join the Postrust community. Contribute to the project, get support, and connect with other developers.",
    },
  ],
};
