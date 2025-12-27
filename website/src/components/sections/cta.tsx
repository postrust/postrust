import { component$ } from "@builder.io/qwik";
import { Link } from "@builder.io/qwik-city";

export const CTASection = component$(() => {
  return (
    <section class="section-padding bg-neutral-900 relative overflow-hidden">
      {/* Background Pattern */}
      <div class="absolute inset-0 bg-[linear-gradient(to_right,#ffffff08_1px,transparent_1px),linear-gradient(to_bottom,#ffffff08_1px,transparent_1px)] bg-[size:32px_32px]"></div>

      {/* Gradient Orbs */}
      <div class="absolute top-0 left-1/4 w-96 h-96 bg-primary-500/20 rounded-full blur-3xl"></div>
      <div class="absolute bottom-0 right-1/4 w-96 h-96 bg-accent-500/20 rounded-full blur-3xl"></div>

      <div class="container-wide relative">
        <div class="max-w-3xl mx-auto text-center">
          {/* Badge */}
          <div class="inline-flex items-center gap-2 px-4 py-2 bg-white/10 backdrop-blur border border-white/20 rounded-full mb-8">
            <span class="text-sm font-medium text-white/90">
              Open Source & MIT Licensed
            </span>
          </div>

          {/* Headline */}
          <h2 class="text-3xl md:text-4xl lg:text-5xl font-bold text-white mb-6">
            Ready to build faster APIs?
          </h2>

          {/* Description */}
          <p class="text-lg text-neutral-300 mb-10 max-w-2xl mx-auto">
            Get started with Postrust in minutes. Join developers building
            high-performance PostgreSQL APIs with Rust.
          </p>

          {/* CTAs */}
          <div class="flex flex-col sm:flex-row items-center justify-center gap-4 mb-12">
            <Link
              href="/docs/getting-started"
              class="inline-flex items-center gap-2 px-8 py-4 text-base font-semibold text-neutral-900 bg-white hover:bg-neutral-100 rounded-lg transition-all shadow-lg hover:shadow-xl"
            >
              Get Started for Free
              <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13 7l5 5m0 0l-5 5m5-5H6"/>
              </svg>
            </Link>
            <a
              href="https://github.com/postrust/postrust"
              target="_blank"
              rel="noopener noreferrer"
              class="inline-flex items-center gap-2 px-8 py-4 text-base font-semibold text-white border border-white/30 hover:bg-white/10 rounded-lg transition-all"
            >
              <svg class="w-5 h-5" fill="currentColor" viewBox="0 0 24 24">
                <path fill-rule="evenodd" d="M12 2C6.477 2 2 6.484 2 12.017c0 4.425 2.865 8.18 6.839 9.504.5.092.682-.217.682-.483 0-.237-.008-.868-.013-1.703-2.782.605-3.369-1.343-3.369-1.343-.454-1.158-1.11-1.466-1.11-1.466-.908-.62.069-.608.069-.608 1.003.07 1.531 1.032 1.531 1.032.892 1.53 2.341 1.088 2.91.832.092-.647.35-1.088.636-1.338-2.22-.253-4.555-1.113-4.555-4.951 0-1.093.39-1.988 1.029-2.688-.103-.253-.446-1.272.098-2.65 0 0 .84-.27 2.75 1.026A9.564 9.564 0 0112 6.844c.85.004 1.705.115 2.504.337 1.909-1.296 2.747-1.027 2.747-1.027.546 1.379.202 2.398.1 2.651.64.7 1.028 1.595 1.028 2.688 0 3.848-2.339 4.695-4.566 4.943.359.309.678.92.678 1.855 0 1.338-.012 2.419-.012 2.747 0 .268.18.58.688.482A10.019 10.019 0 0022 12.017C22 6.484 17.522 2 12 2z" clip-rule="evenodd"/>
              </svg>
              Star on GitHub
            </a>
          </div>

          {/* Community Links */}
          <div class="flex flex-wrap items-center justify-center gap-8">
            <a
              href="https://github.com/postrust/postrust/issues"
              target="_blank"
              rel="noopener noreferrer"
              class="flex items-center gap-2 text-neutral-400 hover:text-white transition-colors"
            >
              <svg class="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 8v4m0 4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"/>
              </svg>
              <span class="text-sm font-medium">Report Issues</span>
            </a>
            <a
              href="https://x.com/postrustorg"
              target="_blank"
              rel="noopener noreferrer"
              class="flex items-center gap-2 text-neutral-400 hover:text-white transition-colors"
            >
              <svg class="w-5 h-5" fill="currentColor" viewBox="0 0 24 24">
                <path d="M18.244 2.25h3.308l-7.227 8.26 8.502 11.24H16.17l-5.214-6.817L4.99 21.75H1.68l7.73-8.835L1.254 2.25H8.08l4.713 6.231zm-1.161 17.52h1.833L7.084 4.126H5.117z"/>
              </svg>
              <span class="text-sm font-medium">Follow @postrustorg</span>
            </a>
            <Link
              href="/community"
              class="flex items-center gap-2 text-neutral-400 hover:text-white transition-colors"
            >
              <svg class="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 4.354a4 4 0 110 5.292M15 21H3v-1a6 6 0 0112 0v1zm0 0h6v-1a6 6 0 00-9-5.197M13 7a4 4 0 11-8 0 4 4 0 018 0z"/>
              </svg>
              <span class="text-sm font-medium">Contributing</span>
            </Link>
          </div>
        </div>
      </div>
    </section>
  );
});
