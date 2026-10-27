const escapeHtml = (text: string) =>
  text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");

/**
 * Text as raw HTML between Cloudflare's email_off markers, for a
 * dangerouslySetInnerHTML. Without them Email Obfuscation rewrites every
 * address it finds -- example ones in code samples included -- into a
 * "[email protected]" link to /cdn-cgi/l/email-protection, which breaks the
 * sample and is a 404 for crawlers.
 */
export const emailOff = (text: string) =>
  `<!--email_off-->${escapeHtml(text)}<!--/email_off-->`;
