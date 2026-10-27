import { component$, Slot } from "@builder.io/qwik";
import type { DocumentHead } from "@builder.io/qwik-city";
import { Link } from "@builder.io/qwik-city";
import type { Guide } from "~/data/guides";
import { guideUrl } from "~/data/guides";
import { jsonLd } from "~/components/json-ld";

export interface Faq {
  q: string;
  a: string;
}

interface Props {
  guide: Guide;
  /**
   * The short answer, shown under the title before anything else: what an
   * answer engine quotes and what a reader skimming needs.
   */
  answer: string;
  faq?: Faq[];
}

/** A guide's frame: breadcrumb, title, the short answer, the body, then FAQ. */
export const GuidePage = component$<Props>(({ guide, answer, faq }) => {
  // The questions on the page and in the FAQPage data are the same list.
  const faqJsonLd =
    faq &&
    jsonLd({
      "@context": "https://schema.org",
      "@type": "FAQPage",
      mainEntity: faq.map((entry) => ({
        "@type": "Question",
        name: entry.q,
        acceptedAnswer: { "@type": "Answer", text: entry.a },
      })),
    });

  return (
    <div class="min-h-screen bg-white">
      {faqJsonLd && (
        <script
          type="application/ld+json"
          dangerouslySetInnerHTML={faqJsonLd}
        />
      )}

      <div class="border-b border-neutral-200 bg-gradient-to-b from-neutral-50 to-white">
        <div class="container-wide py-12">
          <nav class="mb-4 text-sm text-neutral-500">
            <Link href="/guides/" class="hover:text-primary-600">
              Guides
            </Link>
          </nav>
          <h1 class="mb-6 max-w-3xl text-4xl font-bold text-neutral-900">
            {guide.title}
          </h1>
          <p class="border-primary-500 max-w-3xl border-l-4 pl-4 text-lg text-neutral-700">
            {answer}
          </p>
        </div>
      </div>

      <div class="container-wide py-12">
        <article class="guide-body max-w-3xl">
          <Slot />
        </article>

        {faq && (
          <section class="mt-4 max-w-3xl">
            <h2 class="mb-6 text-2xl font-bold text-neutral-900">
              Questions people ask: {guide.headTitle}
            </h2>
            <div class="space-y-6">
              {faq.map((entry) => (
                <div key={entry.q}>
                  <h3 class="mb-2 text-lg font-semibold text-neutral-900">
                    {entry.q}
                  </h3>
                  <p class="text-neutral-600">{entry.a}</p>
                </div>
              ))}
            </div>
          </section>
        )}
      </div>
    </div>
  );
});

/** Title, canonical address and description for a guide. */
export const guideHead = (guide: Guide): DocumentHead => ({
  title: guide.headTitle,
  links: [{ rel: "canonical", href: guideUrl(guide.slug) }],
  meta: [{ name: "description", content: guide.description }],
});
