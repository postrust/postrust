import type { RequestHandler } from "@builder.io/qwik-city";
import { pages } from "~/data/lastmod";

const SITE = "https://postrust.org";

/**
 * Every page, with the date its content last changed. Built from the same
 * dates the pages show and put in their structured data, so the two cannot
 * disagree, and a new page is listed without anyone remembering to.
 */
export const onGet: RequestHandler = ({ send, cacheControl }) => {
  cacheControl({ maxAge: 3600, staleWhileRevalidate: 60 * 60 * 24 });
  const urls = Object.entries(pages)
    .sort(([a], [b]) => a.localeCompare(b))
    .map(
      ([path, date]) =>
        `  <url>\n    <loc>${SITE}${path}</loc>\n` +
        (date ? `    <lastmod>${date}</lastmod>\n` : "") +
        `  </url>`,
    );
  send(
    new Response(
      `<?xml version="1.0" encoding="UTF-8"?>\n` +
        `<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n` +
        `${urls.join("\n")}\n</urlset>\n`,
      { headers: { "Content-Type": "application/xml; charset=utf-8" } },
    ),
  );
};
