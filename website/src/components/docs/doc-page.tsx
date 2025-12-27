import { component$, Slot } from "@builder.io/qwik";
import { Link } from "@builder.io/qwik-city";

interface DocPageProps {
  title: string;
  description: string;
}

export const DocPage = component$<DocPageProps>(({ title, description }) => {
  return (
    <div class="min-h-screen bg-white">
      {/* Header */}
      <div class="bg-gradient-to-b from-neutral-50 to-white border-b border-neutral-200">
        <div class="container-wide py-12">
          <div class="flex items-center gap-2 text-sm text-neutral-500 mb-4">
            <Link href="/docs" class="hover:text-primary-600">Docs</Link>
            <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 5l7 7-7 7"/>
            </svg>
            <span class="text-neutral-900">{title}</span>
          </div>
          <h1 class="text-4xl font-bold text-neutral-900 mb-4">{title}</h1>
          <p class="text-lg text-neutral-600 max-w-2xl">{description}</p>
        </div>
      </div>

      {/* Content */}
      <div class="container-wide py-12">
        <div class="max-w-3xl">
          <Slot />
        </div>
      </div>
    </div>
  );
});

export const DocSection = component$<{ title: string }>(({ title }) => {
  return (
    <section class="mb-12">
      <h2 class="text-2xl font-bold text-neutral-900 mb-4">{title}</h2>
      <Slot />
    </section>
  );
});

export const CodeBlock = component$<{ language?: string; code: string }>(({ language, code }) => {
  return (
    <div class="bg-neutral-900 rounded-xl overflow-hidden mb-4">
      {language && (
        <div class="flex items-center justify-between px-4 py-2 bg-neutral-800 border-b border-neutral-700">
          <span class="text-sm text-neutral-400">{language}</span>
        </div>
      )}
      <pre class="p-4 text-sm overflow-x-auto">
        <code class="text-neutral-100">{code}</code>
      </pre>
    </div>
  );
});
