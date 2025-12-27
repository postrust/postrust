import { component$ } from "@builder.io/qwik";

const starterFeatures = [
  {
    icon: "user",
    title: "Authentication",
    description: "Magic link and OAuth with Google & GitHub",
  },
  {
    icon: "users",
    title: "Team Management",
    description: "Create teams, invite members, manage roles",
  },
  {
    icon: "credit-card",
    title: "Billing",
    description: "Stripe & LemonSqueezy subscription management",
  },
  {
    icon: "key",
    title: "API Keys",
    description: "Generate and manage programmatic access",
  },
  {
    icon: "shield",
    title: "Row Level Security",
    description: "PostgreSQL RLS policies built-in",
  },
  {
    icon: "clipboard",
    title: "Audit Logs",
    description: "Track all team actions for compliance",
  },
];

const iconPaths: Record<string, string> = {
  user: "M20 21v-2a4 4 0 00-4-4H8a4 4 0 00-4 4v2M12 11a4 4 0 100-8 4 4 0 000 8z",
  users: "M17 21v-2a4 4 0 00-4-4H5a4 4 0 00-4 4v2M23 21v-2a4 4 0 00-3-3.87M16 3.13a4 4 0 010 7.75M9 11a4 4 0 100-8 4 4 0 000 8z",
  "credit-card": "M3 10h18M7 15h.01M11 15h2M21 7v10a2 2 0 01-2 2H5a2 2 0 01-2-2V7a2 2 0 012-2h14a2 2 0 012 2z",
  key: "M21 2l-2 2m-7.61 7.61a5.5 5.5 0 11-7.778 7.778 5.5 5.5 0 017.777-7.777zm0 0L15.5 7.5m0 0l3 3L22 7l-3-3m-3.5 3.5L19 4",
  shield: "M9 12l2 2 4-4m5.618-4.016A11.955 11.955 0 0112 2.944a11.955 11.955 0 01-8.618 3.04A12.02 12.02 0 003 9c0 5.591 3.824 10.29 9 11.622 5.176-1.332 9-6.03 9-11.622 0-1.042-.133-2.052-.382-3.016z",
  clipboard: "M9 5H7a2 2 0 00-2 2v12a2 2 0 002 2h10a2 2 0 002-2V7a2 2 0 00-2-2h-2M9 5a2 2 0 002 2h2a2 2 0 002-2M9 5a2 2 0 012-2h2a2 2 0 012 2m-3 7h3m-3 4h3m-6-4h.01M9 16h.01",
};

export const SaasStarterSection = component$(() => {
  return (
    <section class="section-padding bg-gradient-to-b from-neutral-900 to-neutral-800">
      <div class="container-wide">
        <div class="grid lg:grid-cols-2 gap-12 items-center">
          {/* Left: Content */}
          <div>
            <div class="inline-flex items-center gap-2 px-3 py-1 bg-primary-500/20 text-primary-400 rounded-full text-sm font-medium mb-6">
              <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13 10V3L4 14h7v7l9-11h-7z"/>
              </svg>
              SaaS Starter Kit
            </div>

            <h2 class="text-3xl md:text-4xl font-bold text-white mb-4">
              Launch your SaaS in days, not months
            </h2>

            <p class="text-lg text-neutral-300 mb-8">
              A production-ready starter kit built on Postrust with authentication,
              team management, billing, and everything you need to ship fast.
            </p>

            <div class="flex flex-wrap gap-4 mb-8">
              <a
                href="https://github.com/postrust/postrust-saas-starter"
                target="_blank"
                rel="noopener noreferrer"
                class="inline-flex items-center gap-2 px-6 py-3 bg-white text-neutral-900 font-semibold rounded-lg hover:bg-neutral-100 transition-colors"
              >
                <svg class="w-5 h-5" fill="currentColor" viewBox="0 0 24 24">
                  <path fill-rule="evenodd" d="M12 2C6.477 2 2 6.484 2 12.017c0 4.425 2.865 8.18 6.839 9.504.5.092.682-.217.682-.483 0-.237-.008-.868-.013-1.703-2.782.605-3.369-1.343-3.369-1.343-.454-1.158-1.11-1.466-1.11-1.466-.908-.62.069-.608.069-.608 1.003.07 1.531 1.032 1.531 1.032.892 1.53 2.341 1.088 2.91.832.092-.647.35-1.088.636-1.338-2.22-.253-4.555-1.113-4.555-4.951 0-1.093.39-1.988 1.029-2.688-.103-.253-.446-1.272.098-2.65 0 0 .84-.27 2.75 1.026A9.564 9.564 0 0112 6.844c.85.004 1.705.115 2.504.337 1.909-1.296 2.747-1.027 2.747-1.027.546 1.379.202 2.398.1 2.651.64.7 1.028 1.595 1.028 2.688 0 3.848-2.339 4.695-4.566 4.943.359.309.678.92.678 1.855 0 1.338-.012 2.419-.012 2.747 0 .268.18.58.688.482A10.019 10.019 0 0022 12.017C22 6.484 17.522 2 12 2z" clip-rule="evenodd"/>
                </svg>
                View on GitHub
              </a>
              <a
                href="/docs/getting-started"
                class="inline-flex items-center gap-2 px-6 py-3 border border-neutral-600 text-white font-semibold rounded-lg hover:bg-neutral-700 transition-colors"
              >
                Documentation
                <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13 7l5 5m0 0l-5 5m5-5H6"/>
                </svg>
              </a>
            </div>

            {/* Tech Stack */}
            <div class="flex flex-wrap items-center gap-4 text-sm text-neutral-400">
              <span>Built with:</span>
              <span class="px-2 py-1 bg-neutral-700 rounded">Next.js</span>
              <span class="px-2 py-1 bg-neutral-700 rounded">Postrust</span>
              <span class="px-2 py-1 bg-neutral-700 rounded">PostgreSQL</span>
              <span class="px-2 py-1 bg-neutral-700 rounded">Tailwind CSS</span>
            </div>
          </div>

          {/* Right: Feature Grid */}
          <div class="grid sm:grid-cols-2 gap-4">
            {starterFeatures.map((feature) => (
              <div
                key={feature.title}
                class="p-5 bg-neutral-800/50 border border-neutral-700 rounded-xl hover:border-primary-500/50 transition-colors"
              >
                <div class="w-10 h-10 bg-primary-500/20 rounded-lg flex items-center justify-center mb-3">
                  <svg
                    class="w-5 h-5 text-primary-400"
                    fill="none"
                    stroke="currentColor"
                    viewBox="0 0 24 24"
                    stroke-width="2"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                  >
                    <path d={iconPaths[feature.icon]} />
                  </svg>
                </div>
                <h3 class="font-semibold text-white mb-1">{feature.title}</h3>
                <p class="text-sm text-neutral-400">{feature.description}</p>
              </div>
            ))}
          </div>
        </div>
      </div>
    </section>
  );
});
