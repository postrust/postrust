import { component$ } from "@builder.io/qwik";
import { Link } from "@builder.io/qwik-city";

export const HeroSection = component$(() => {
  return (
    <section class="relative overflow-hidden bg-gradient-to-b from-neutral-50 to-white">
      {/* Background Pattern */}
      <div class="absolute inset-0 bg-[linear-gradient(to_right,#80808012_1px,transparent_1px),linear-gradient(to_bottom,#80808012_1px,transparent_1px)] bg-[size:24px_24px]"></div>

      {/* Gradient Orbs */}
      <div class="absolute top-0 left-1/4 w-96 h-96 bg-primary-500/10 rounded-full blur-3xl"></div>
      <div class="absolute bottom-0 right-1/4 w-96 h-96 bg-accent-500/10 rounded-full blur-3xl"></div>

      <div class="relative container-wide section-padding">
        <div class="max-w-4xl mx-auto text-center">
          {/* Badge */}
          <div class="inline-flex items-center gap-2 px-4 py-2 bg-primary-50 border border-primary-200 rounded-full mb-8">
            <span class="w-2 h-2 bg-primary-500 rounded-full animate-pulse-subtle"></span>
            <span class="text-sm font-medium text-primary-700">
              Now with GraphQL support
            </span>
          </div>

          {/* Headline */}
          <h1 class="text-4xl md:text-5xl lg:text-6xl font-extrabold text-neutral-900 tracking-tight mb-6">
            PostgreSQL API
            <br />
            <span class="text-gradient">in Milliseconds</span>
          </h1>

          {/* Subheadline */}
          <p class="text-lg md:text-xl text-neutral-600 max-w-2xl mx-auto mb-10">
            High-performance REST & GraphQL API server for PostgreSQL,
            written in Rust. Native AWS Lambda support with ~50ms cold starts.
            Drop-in PostgREST replacement.
          </p>

          {/* CTAs */}
          <div class="flex flex-col sm:flex-row items-center justify-center gap-4 mb-12">
            <Link
              href="/docs/getting-started"
              class="inline-flex items-center gap-2 px-6 py-3 text-base font-semibold text-white bg-neutral-900 hover:bg-neutral-800 rounded-lg transition-all shadow-lg shadow-neutral-900/20 hover:shadow-xl hover:shadow-neutral-900/30"
            >
              Get Started
              <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13 7l5 5m0 0l-5 5m5-5H6"/>
              </svg>
            </Link>
            <a
              href="https://github.com/postrust/postrust"
              target="_blank"
              rel="noopener noreferrer"
              class="inline-flex items-center gap-2 px-6 py-3 text-base font-semibold text-neutral-700 bg-white hover:bg-neutral-50 border border-neutral-300 rounded-lg transition-all"
            >
              <svg class="w-5 h-5" fill="currentColor" viewBox="0 0 24 24">
                <path fill-rule="evenodd" d="M12 2C6.477 2 2 6.484 2 12.017c0 4.425 2.865 8.18 6.839 9.504.5.092.682-.217.682-.483 0-.237-.008-.868-.013-1.703-2.782.605-3.369-1.343-3.369-1.343-.454-1.158-1.11-1.466-1.11-1.466-.908-.62.069-.608.069-.608 1.003.07 1.531 1.032 1.531 1.032.892 1.53 2.341 1.088 2.91.832.092-.647.35-1.088.636-1.338-2.22-.253-4.555-1.113-4.555-4.951 0-1.093.39-1.988 1.029-2.688-.103-.253-.446-1.272.098-2.65 0 0 .84-.27 2.75 1.026A9.564 9.564 0 0112 6.844c.85.004 1.705.115 2.504.337 1.909-1.296 2.747-1.027 2.747-1.027.546 1.379.202 2.398.1 2.651.64.7 1.028 1.595 1.028 2.688 0 3.848-2.339 4.695-4.566 4.943.359.309.678.92.678 1.855 0 1.338-.012 2.419-.012 2.747 0 .268.18.58.688.482A10.019 10.019 0 0022 12.017C22 6.484 17.522 2 12 2z" clip-rule="evenodd"/>
              </svg>
              View on GitHub
            </a>
          </div>

          {/* Code Preview */}
          <div class="relative max-w-2xl mx-auto">
            <div class="bg-neutral-900 rounded-xl shadow-2xl overflow-hidden">
              {/* Terminal Header */}
              <div class="flex items-center gap-2 px-4 py-3 bg-neutral-800 border-b border-neutral-700">
                <div class="flex gap-1.5">
                  <div class="w-3 h-3 bg-red-500 rounded-full"></div>
                  <div class="w-3 h-3 bg-yellow-500 rounded-full"></div>
                  <div class="w-3 h-3 bg-green-500 rounded-full"></div>
                </div>
                <span class="ml-2 text-sm text-neutral-400 font-mono">Terminal</span>
              </div>
              {/* Code Content */}
              <div class="p-6 text-left">
                <pre class="text-sm md:text-base font-mono">
                  <code>
                    <span class="text-neutral-500"># Start in seconds with Docker</span>
                    {"\n"}
                    <span class="text-green-400">$</span>
                    <span class="text-neutral-100"> docker run -p 3000:3000 \</span>
                    {"\n"}
                    <span class="text-neutral-100">    -e DATABASE_URL="postgres://..." \</span>
                    {"\n"}
                    <span class="text-neutral-100">    postrust/postrust</span>
                    {"\n\n"}
                    <span class="text-neutral-500"># Your API is ready!</span>
                    {"\n"}
                    <span class="text-green-400">$</span>
                    <span class="text-neutral-100"> curl localhost:3000/users</span>
                    {"\n"}
                    <span class="text-accent-400">[{`{"id":1,"name":"Alice"},{"id":2,"name":"Bob"}`}]</span>
                  </code>
                </pre>
              </div>
            </div>
          </div>

          {/* Trust Badges */}
          <div class="mt-12 flex flex-wrap items-center justify-center gap-6 text-sm text-neutral-500">
            <div class="flex items-center gap-2">
              <svg class="w-5 h-5 text-green-500" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 12l2 2 4-4m5.618-4.016A11.955 11.955 0 0112 2.944a11.955 11.955 0 01-8.618 3.04A12.02 12.02 0 003 9c0 5.591 3.824 10.29 9 11.622 5.176-1.332 9-6.03 9-11.622 0-1.042-.133-2.052-.382-3.016z"/>
              </svg>
              <span>MIT Licensed</span>
            </div>
            <div class="flex items-center gap-2">
              <svg class="w-5 h-5 text-primary-500" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13 10V3L4 14h7v7l9-11h-7z"/>
              </svg>
              <span>~50ms Lambda Cold Start</span>
            </div>
            <div class="flex items-center gap-2">
              <svg class="w-5 h-5 text-accent-500" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 3v4M3 5h4M6 17v4m-2-2h4m5-16l2.286 6.857L21 12l-5.714 2.143L13 21l-2.286-6.857L5 12l5.714-2.143L13 3z"/>
              </svg>
              <span>REST + GraphQL</span>
            </div>
          </div>
        </div>
      </div>
    </section>
  );
});
