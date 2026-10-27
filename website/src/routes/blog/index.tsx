import { component$ } from "@builder.io/qwik";
import type { DocumentHead } from "@builder.io/qwik-city";
import { Link } from "@builder.io/qwik-city";
import { formatDate, posts } from "~/data/posts";

export default component$(() => (
  <div class="min-h-screen bg-white">
    <div class="border-b border-neutral-200 bg-gradient-to-b from-neutral-50 to-white">
      <div class="container-wide py-12">
        <h1 class="mb-4 text-4xl font-bold text-neutral-900">Blog</h1>
        <p class="max-w-2xl text-lg text-neutral-600">
          How Postrust works and how to use it, with every example run before it
          was written down.
        </p>
      </div>
    </div>
    <div class="container-wide py-12">
      <div class="max-w-3xl space-y-10">
        {posts.map((post) => (
          <article key={post.slug}>
            <time dateTime={post.date} class="text-sm text-neutral-500">
              {formatDate(post.date)}
            </time>
            <h2 class="mt-1 mb-2 text-2xl font-bold text-neutral-900">
              <Link href={`/blog/${post.slug}/`} class="hover:text-primary-600">
                {post.title}
              </Link>
            </h2>
            <p class="text-neutral-600">{post.description}</p>
            <Link
              href={`/blog/${post.slug}/`}
              class="text-primary-600 mt-3 inline-block font-medium hover:underline"
            >
              Read the post →
            </Link>
          </article>
        ))}
      </div>
    </div>
  </div>
));

const title = "Postrust Blog: PostgreSQL, REST, GraphQL and Federation";
const description =
  "How Postrust works and how to use it: PostgreSQL as REST, GraphQL and Apollo Federation, with every example run before it was written down.";
const url = "https://postrust.org/blog/";

export const head: DocumentHead = {
  title,
  links: [{ rel: "canonical", href: url }],
  meta: [
    { name: "description", content: description },
    { property: "og:title", content: title },
    { property: "og:description", content: description },
    { property: "og:type", content: "website" },
    { property: "og:url", content: url },
  ],
};
