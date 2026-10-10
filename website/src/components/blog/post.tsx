import { component$, Slot } from "@builder.io/qwik";
import type { DocumentHead } from "@builder.io/qwik-city";
import { Link } from "@builder.io/qwik-city";
import { formatDate, postUrl, type Post } from "~/data/posts";

/** The frame every post is written into: breadcrumb, title, byline. */
export const PostLayout = component$<{ post: Post }>(({ post }) => (
  <div class="min-h-screen bg-white">
    <div class="border-b border-neutral-200 bg-gradient-to-b from-neutral-50 to-white">
      <div class="container-wide py-12">
        <div class="mb-4 flex items-center gap-2 text-sm text-neutral-500">
          <Link href="/blog" class="hover:text-primary-600">
            Blog
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
          <time dateTime={post.date}>{formatDate(post.date)}</time>
        </div>
        <h1 class="mb-4 max-w-3xl text-4xl font-bold text-neutral-900">
          {post.title}
        </h1>
        <p class="max-w-2xl text-lg text-neutral-600">{post.description}</p>
        <p class="mt-4 text-sm text-neutral-500">{post.author}</p>
      </div>
    </div>
    <article class="container-wide py-12">
      <div class="max-w-3xl">
        <Slot />
      </div>
    </article>
  </div>
));

export const Code = component$<{ label: string; code: string }>(
  ({ label, code }) => (
    <div class="mb-4 overflow-hidden rounded-xl bg-neutral-900">
      <div class="border-b border-neutral-700 bg-neutral-800 px-4 py-2">
        <span class="text-sm text-neutral-400">{label}</span>
      </div>
      <pre class="overflow-x-auto p-4 text-sm">
        <code class="text-neutral-100">{code}</code>
      </pre>
    </div>
  ),
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
            "@type": "Organization",
            name: "Postrust",
            url: "https://postrust.org",
          },
          publisher: {
            "@type": "Organization",
            name: "Postrust",
            url: "https://postrust.org",
          },
          mainEntityOfPage: url,
          keywords: post.tags.join(", "),
        }),
      },
    ],
  };
};
