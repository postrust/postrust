import { lastModified } from "~/data/lastmod";
import { LATEST } from "~/data/version";
import { jsonLd } from "~/components/json-ld";

export const SITE = "https://postrust.org";
const ORG_ID = `${SITE}/#organization`;

/** Names for the path segments a breadcrumb passes through. */
const sectionNames: Record<string, string> = {
  docs: "Docs",
  compare: "Compare",
  blog: "Blog",
  conformance: "Conformance",
  guides: "Guides",
};

const organization = {
  "@type": "Organization",
  "@id": ORG_ID,
  name: "Postrust",
  url: `${SITE}/`,
  logo: `${SITE}/favicon.svg`,
  sameAs: [
    "https://github.com/postrust/postrust",
    "https://crates.io/crates/postrust-server",
  ],
};

const maintainer = {
  "@type": "Person",
  "@id": `${SITE}/community/#maintainer`,
  name: "Bhanu Pratap Chaudhary",
  url: "https://bhanu.io/",
  sameAs: ["https://github.com/bhanuc"],
};

const softwareApplication = {
  "@type": "SoftwareApplication",
  "@id": `${SITE}/#software`,
  name: "Postrust",
  description:
    "An open-source server, written in Rust, that serves a PostgreSQL schema as REST and GraphQL APIs. It runs as one binary, in Docker, on Kubernetes, on AWS Lambda and on Cloudflare Workers.",
  applicationCategory: "DeveloperApplication",
  applicationSubCategory: "API server",
  operatingSystem: "Linux",
  softwareVersion: LATEST,
  license: "https://opensource.org/licenses/MIT",
  url: `${SITE}/`,
  downloadUrl: "https://github.com/postrust/postrust/releases",
  codeRepository: "https://github.com/postrust/postrust",
  programmingLanguage: "Rust",
  offers: { "@type": "Offer", price: "0", priceCurrency: "USD" },
  author: maintainer,
  publisher: { "@id": ORG_ID },
};

/** The docs and guides indexes: lists of pages, not pages of their own. */
export const isIndex = (pathname: string) =>
  pathname === "/docs/" || pathname === "/guides/";

/** A docs page or a guide, which is written as an article. */
export const isArticle = (pathname: string) =>
  /^\/(docs|guides)\/./.test(pathname);

/** The page's title without the " - Postrust …" suffix every title carries. */
const headline = (title: string) =>
  title.replace(/\s+[-|–—]\s+Postrust( Documentation)?$/, "");

const breadcrumbs = (pathname: string, title: string) => {
  const parts = pathname.split("/").filter(Boolean);
  return {
    "@type": "BreadcrumbList",
    itemListElement: [
      { name: "Home", path: "/" },
      ...parts.map((part, i) => ({
        name:
          i === parts.length - 1
            ? headline(title)
            : (sectionNames[part] ?? part),
        path: `/${parts.slice(0, i + 1).join("/")}/`,
      })),
    ].map((crumb, i) => ({
      "@type": "ListItem",
      position: i + 1,
      name: crumb.name,
      item: SITE + crumb.path,
    })),
  };
};

/**
 * Schema.org data for a page, from its address and head: the organisation on
 * every page, the software on the pages that describe it, breadcrumbs below
 * the home page, and an article for each docs page and guide. Pages that
 * carry richer data of their own (a blog post's BlogPosting, a comparison's
 * FAQPage) keep it; this adds to it.
 */
export const structuredData = (
  pathname: string,
  title: string,
  description: string | undefined,
  url: string,
) => {
  const graph: object[] = [organization];

  if (pathname === "/") {
    graph.push({
      "@type": "WebSite",
      "@id": `${SITE}/#website`,
      name: "Postrust",
      url: `${SITE}/`,
      publisher: { "@id": ORG_ID },
    });
  }

  if (["/", "/features/", "/pricing/", "/enterprise/"].includes(pathname)) {
    graph.push(softwareApplication);
  } else {
    graph.push(breadcrumbs(pathname, title));
  }

  if (pathname === "/community/") graph.push(maintainer);

  if (isIndex(pathname)) {
    graph.push({
      "@type": "CollectionPage",
      name: headline(title),
      description,
      url,
      publisher: { "@id": ORG_ID },
      inLanguage: "en",
    });
  } else if (isArticle(pathname)) {
    const modified = lastModified(pathname);
    graph.push({
      "@type": "TechArticle",
      headline: headline(title),
      description,
      url,
      mainEntityOfPage: url,
      ...(modified && { dateModified: modified }),
      author: { "@id": ORG_ID },
      publisher: { "@id": ORG_ID },
      about: { "@id": `${SITE}/#software` },
      inLanguage: "en",
    });
  }

  return jsonLd({ "@context": "https://schema.org", "@graph": graph });
};
