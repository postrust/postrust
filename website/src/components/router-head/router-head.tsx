import { component$ } from "@builder.io/qwik";
import { useDocumentHead, useLocation } from "@builder.io/qwik-city";
import { isArticle, SITE, structuredData } from "./structured-data";

/** The card social sites and chat apps show for a link to any page. */
const OG_IMAGE = `${SITE}/og.png`;

/** The address a path is served at in production, without redirecting. */
const canonicalFor = (pathname: string) =>
  SITE + (pathname.endsWith("/") ? pathname : `${pathname}/`);

/**
 * The RouterHead component is placed inside of the document `<head>` element.
 */
export const RouterHead = component$(() => {
  const head = useDocumentHead();
  const loc = useLocation();

  const canonical =
    head.links.find((l) => l.rel === "canonical")?.href ??
    canonicalFor(loc.url.pathname);
  const description = head.meta.find((m) => m.name === "description")?.content;

  // Open Graph and Twitter card tags, from the title and description every
  // page already has. A page that sets one of its own keeps it.
  const has = (key: string) =>
    head.meta.some((m) => m.property === key || m.name === key);
  const social = [
    { property: "og:title", content: head.title },
    { property: "og:description", content: description },
    {
      property: "og:type",
      content: isArticle(loc.url.pathname) ? "article" : "website",
    },
    { property: "og:url", content: canonical },
    { property: "og:site_name", content: "Postrust" },
    { property: "og:image", content: OG_IMAGE },
    { property: "og:image:width", content: "1200" },
    { property: "og:image:height", content: "630" },
    { name: "twitter:card", content: "summary_large_image" },
    { name: "twitter:title", content: head.title },
    { name: "twitter:description", content: description },
    { name: "twitter:image", content: OG_IMAGE },
  ].filter((m) => m.content && !has(m.property ?? m.name!));

  return (
    <>
      <title>{head.title}</title>

      {/* A page that names its canonical address keeps it; two canonicals
          that disagree are as good as none. Otherwise it is the production
          address of this path, with the trailing slash the server redirects
          to and no query string -- not wherever this copy is being served. */}
      {!head.links.some((l) => l.rel === "canonical") && (
        <link rel="canonical" href={canonicalFor(loc.url.pathname)} />
      )}
      <meta name="viewport" content="width=device-width, initial-scale=1.0" />
      <link rel="icon" type="image/svg+xml" href="/favicon.svg" />

      {head.meta.map((m) => (
        <meta key={m.key} {...m} />
      ))}
      {social.map((m) => (
        <meta key={m.property ?? m.name} {...m} />
      ))}

      <script
        type="application/ld+json"
        dangerouslySetInnerHTML={structuredData(
          loc.url.pathname,
          head.title,
          description,
          canonical,
        )}
      />

      {head.links.map((l) => (
        <link key={l.key} {...l} />
      ))}

      {head.styles.map((s) => (
        <style
          key={s.key}
          {...s.props}
          {...(s.props?.dangerouslySetInnerHTML
            ? {}
            : { dangerouslySetInnerHTML: s.style })}
        />
      ))}

      {head.scripts.map((s) => (
        <script
          key={s.key}
          {...s.props}
          {...(s.props?.dangerouslySetInnerHTML
            ? {}
            : { dangerouslySetInnerHTML: s.script })}
        />
      ))}
    </>
  );
});
