declare const __PAGES__: Record<string, string | null>;

/** Every page's path, with the date its content last changed; see vite.config.ts. */
export const pages: Record<string, string | null> = __PAGES__;

/**
 * The date (YYYY-MM-DD) the page served at `pathname` last changed, from git
 * at build time; undefined when the build had no history to read.
 */
export const lastModified = (pathname: string): string | undefined =>
  pages[pathname.endsWith("/") ? pathname : `${pathname}/`] ?? undefined;
