import { component$ } from "@builder.io/qwik";
import { useLocation } from "@builder.io/qwik-city";
import { lastModified } from "~/data/lastmod";
import { formatDate } from "~/data/posts";

/**
 * The file a reader who wants to correct this page should open. A comparison
 * page is a wrapper around its entry in the comparisons data, which is where
 * every word of it is written.
 */
const sourceFor = (pathname: string) =>
  /^\/compare\/[^/]+\/$/.test(pathname)
    ? "src/data/comparisons.ts"
    : `src/routes${pathname}index.tsx`;

/**
 * When this page last changed and where its source is, below the content of
 * a reference page. Readers and AI answer engines both weigh a technical page
 * by how recent it is, and with no date shown they can only guess.
 */
export const LastUpdated = component$(() => {
  const loc = useLocation();
  const date = lastModified(loc.url.pathname);
  if (!date) return null;
  const source = `https://github.com/postrust/postrust/blob/main/website/${sourceFor(loc.url.pathname)}`;
  return (
    <div class="container-wide pb-12 text-sm text-neutral-500">
      <p class="border-t border-neutral-200 pt-6">
        Last updated <time dateTime={date}>{formatDate(date)}</time>
        {" · "}
        <a
          href={source}
          target="_blank"
          rel="noopener noreferrer"
          class="hover:text-neutral-900"
        >
          Edit this page on GitHub
        </a>
      </p>
    </div>
  );
});
