/**
 * Structured data as the text of a <script type="application/ld+json">.
 * JSON.stringify leaves "<" as it is, so a "</script>" or "<!--" anywhere in
 * a title or description would end the element early; escaped as < it is
 * the same JSON and cannot.
 */
export const jsonLd = (data: unknown) =>
  JSON.stringify(data).replace(/</g, "\\u003c");
