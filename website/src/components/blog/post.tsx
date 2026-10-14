import { component$, Slot, useSignal } from "@builder.io/qwik";
import type { DocumentHead } from "@builder.io/qwik-city";
import { Link } from "@builder.io/qwik-city";
import { formatDate, postUrl, type Post } from "~/data/posts";
import { highlight, TOKEN_CLASS, type Lang } from "./highlight";

/**
 * The frame every post is written into. The text sits in a reading column;
 * anything wrapped in <Wide> -- code side by side, mostly -- may use more of
 * the page than prose should.
 */
export const PostLayout = component$<{ post: Post }>(({ post }) => (
  <div class="min-h-screen bg-white">
    <header class="relative overflow-hidden border-b border-neutral-200">
      {/* A dotted ground, fading out, under a warm glow. */}
      <div
        aria-hidden="true"
        class="pointer-events-none absolute inset-0 bg-[radial-gradient(var(--color-neutral-300)_1px,transparent_1px)] [mask-image:radial-gradient(ellipse_70%_80%_at_50%_0%,black,transparent)] bg-[size:22px_22px]"
      />
      <div
        aria-hidden="true"
        class="bg-primary-200/50 pointer-events-none absolute -top-48 left-1/2 h-96 w-[56rem] -translate-x-1/2 rounded-full blur-3xl"
      />
      <div class="container-wide relative pt-14 pb-12 md:pt-20 md:pb-16">
        <div class="mx-auto max-w-[46rem]">
          <div class="mb-8 flex flex-wrap items-center gap-x-4 gap-y-3">
            <Link
              href="/blog/"
              class="inline-flex items-center gap-1.5 text-sm font-medium text-neutral-500 transition-colors hover:text-neutral-900"
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
              Blog
            </Link>
            <span
              class="hidden h-4 w-px bg-neutral-300 sm:block"
              aria-hidden="true"
            />
            <ul class="flex flex-wrap gap-2">
              {post.tags.map((tag) => (
                <li
                  key={tag}
                  class="border-primary-200 bg-primary-50/80 text-primary-800 rounded-full border px-2.5 py-0.5 text-xs font-medium"
                >
                  {tag}
                </li>
              ))}
            </ul>
          </div>

          <h1 class="text-4xl leading-[1.1] font-bold tracking-tight text-balance text-neutral-950 md:text-[3.25rem]">
            {post.title}
          </h1>
          <p class="mt-6 text-lg leading-relaxed text-pretty text-neutral-600 md:text-xl">
            {post.description}
          </p>

          <div class="mt-10 flex items-center gap-3 border-t border-neutral-200/80 pt-6">
            <div class="from-primary-500 to-primary-700 ring-primary-100 flex h-10 w-10 shrink-0 items-center justify-center rounded-full bg-gradient-to-br shadow-sm ring-4">
              <svg
                class="h-5 w-5 text-white"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="2.5"
              >
                <path
                  d="M13 2L3 14h9l-1 8 10-12h-9l1-8z"
                  stroke-linecap="round"
                  stroke-linejoin="round"
                />
              </svg>
            </div>
            <div class="text-sm leading-tight">
              <p class="font-semibold text-neutral-900">
                <a
                  href={post.authorUrl}
                  rel="author"
                  class="hover:text-primary-700 transition-colors"
                >
                  {post.author}
                </a>
              </p>
              <p class="mt-1 text-neutral-500">
                <time dateTime={post.date}>{formatDate(post.date)}</time>
                <span class="mx-2" aria-hidden="true">
                  ·
                </span>
                {post.readingMinutes} min read
              </p>
            </div>
          </div>
        </div>
      </div>
    </header>

    <article class="container-wide py-14 md:py-16">
      <div class="post-body">
        <Slot />
      </div>
    </article>
  </div>
));

/** Lets what it wraps use more of the page than the reading column. */
export const Wide = component$(() => (
  <div class="post-wide">
    <Slot />
  </div>
));

/** Two snippets side by side on a wide screen, one above the other on a narrow one. */
export const CodePair = component$(() => (
  <div class="post-wide mb-8 grid items-start gap-4 lg:grid-cols-2 [&>*]:mb-0">
    <Slot />
  </div>
));

const LANG_NAME: Record<Lang, string> = {
  sql: "SQL",
  bash: "Shell",
  json: "JSON",
  graphql: "GraphQL",
  yaml: "YAML",
  text: "",
};

export const Code = component$<{ label: string; code: string; lang?: Lang }>(
  ({ label, code, lang = "text" }) => {
    const copied = useSignal(false);
    return (
      <figure class="group mb-8 overflow-hidden rounded-xl bg-neutral-950 shadow-lg ring-1 shadow-neutral-900/10 ring-neutral-900/10">
        <figcaption class="flex items-center gap-3 border-b border-white/10 bg-white/[0.03] px-4 py-2.5">
          <span class="flex gap-1.5" aria-hidden="true">
            <span class="h-2.5 w-2.5 rounded-full bg-white/15" />
            <span class="h-2.5 w-2.5 rounded-full bg-white/15" />
            <span class="h-2.5 w-2.5 rounded-full bg-white/15" />
          </span>
          <span class="min-w-0 flex-1 truncate font-mono text-xs text-neutral-400">
            {label}
          </span>
          {LANG_NAME[lang] && (
            <span class="hidden text-[0.6875rem] font-medium tracking-wide text-neutral-500 uppercase sm:inline">
              {LANG_NAME[lang]}
            </span>
          )}
          <button
            type="button"
            class="rounded-md px-2 py-1 text-xs font-medium text-neutral-400 transition-colors hover:bg-white/10 hover:text-white"
            aria-label={`Copy ${label}`}
            onClick$={async () => {
              await navigator.clipboard.writeText(code);
              copied.value = true;
              setTimeout(() => (copied.value = false), 1500);
            }}
          >
            {copied.value ? "Copied" : "Copy"}
          </button>
        </figcaption>
        <pre class="overflow-x-auto px-5 py-4 text-[0.8125rem] leading-6">
          <code class="text-neutral-200">
            {highlight(code, lang).map((t, i) =>
              t.kind ? (
                <span key={i} class={TOKEN_CLASS[t.kind]}>
                  {t.text}
                </span>
              ) : (
                t.text
              ),
            )}
          </code>
        </pre>
      </figure>
    );
  },
);

/** Title, canonical address, Open Graph and BlogPosting data for a post. */
export const postHead = (post: Post): DocumentHead => {
  const url = postUrl(post.slug);
  return {
    title: `${post.title} | Postrust`,
    links: [{ rel: "canonical", href: url }],
    meta: [
      { name: "description", content: post.description },
      { name: "keywords", content: post.tags.join(", ") },
      { name: "author", content: post.author },
      { property: "og:title", content: post.title },
      { property: "og:description", content: post.description },
      { property: "og:type", content: "article" },
      { property: "og:url", content: url },
      { property: "article:published_time", content: post.date },
      { name: "twitter:card", content: "summary_large_image" },
      { name: "twitter:title", content: post.title },
      { name: "twitter:description", content: post.description },
    ],
    scripts: [
      {
        props: { type: "application/ld+json" },
        script: JSON.stringify({
          "@context": "https://schema.org",
          "@type": "BlogPosting",
          headline: post.title,
          description: post.description,
          datePublished: post.date,
          author: {
            "@type": "Person",
            name: post.author,
            url: post.authorUrl,
          },
          publisher: {
            "@type": "Organization",
            name: "Postrust",
            url: "https://postrust.org/",
          },
          mainEntityOfPage: url,
          keywords: post.tags.join(", "),
        }),
      },
    ],
  };
};
