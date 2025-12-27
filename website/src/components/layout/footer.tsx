import { component$ } from "@builder.io/qwik";
import { Link } from "@builder.io/qwik-city";

export const Footer = component$(() => {
  const currentYear = new Date().getFullYear();

  const footerLinks = {
    product: [
      { href: "/features", label: "Features" },
      { href: "/pricing", label: "Pricing" },
      { href: "/compare", label: "Compare" },
      { href: "/enterprise", label: "Enterprise" },
    ],
    resources: [
      { href: "/docs", label: "Documentation" },
      { href: "/docs/getting-started", label: "Getting Started" },
      { href: "/docs/api-reference", label: "API Reference" },
      { href: "/docs/deployment", label: "Deployment" },
    ],
    community: [
      { href: "https://github.com/postrust/postrust", label: "GitHub", external: true },
      { href: "https://github.com/postrust/postrust/issues", label: "Issues & Features", external: true },
      { href: "https://x.com/postrustorg", label: "Twitter / X", external: true },
      { href: "/community", label: "Contributing" },
    ],
    company: [
      { href: "/blog", label: "Blog" },
      { href: "/about", label: "About" },
      { href: "/privacy", label: "Privacy Policy" },
      { href: "/terms", label: "Terms of Service" },
    ],
  };

  return (
    <footer class="bg-neutral-950 text-neutral-300">
      <div class="container-wide section-padding">
        {/* Main Footer */}
        <div class="grid grid-cols-2 md:grid-cols-5 gap-8 lg:gap-12">
          {/* Brand */}
          <div class="col-span-2 md:col-span-1">
            <Link href="/" class="flex items-center gap-2 mb-4">
              <div class="w-8 h-8 bg-gradient-to-br from-primary-500 to-primary-700 rounded-lg flex items-center justify-center">
                <svg class="w-5 h-5 text-white" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5">
                  <path d="M13 2L3 14h9l-1 8 10-12h-9l1-8z" stroke-linecap="round" stroke-linejoin="round"/>
                </svg>
              </div>
              <span class="text-xl font-bold text-white">Postrust</span>
            </Link>
            <p class="text-sm text-neutral-400 mb-6 max-w-xs">
              High-performance PostgreSQL REST & GraphQL API server, written in Rust for serverless environments.
            </p>
            {/* Social Links */}
            <div class="flex items-center gap-3">
              <a
                href="https://github.com/postrust/postrust"
                target="_blank"
                rel="noopener noreferrer"
                class="p-2 text-neutral-400 hover:text-white hover:bg-neutral-800 rounded-lg transition-colors"
                aria-label="GitHub"
              >
                <svg class="w-5 h-5" fill="currentColor" viewBox="0 0 24 24">
                  <path fill-rule="evenodd" d="M12 2C6.477 2 2 6.484 2 12.017c0 4.425 2.865 8.18 6.839 9.504.5.092.682-.217.682-.483 0-.237-.008-.868-.013-1.703-2.782.605-3.369-1.343-3.369-1.343-.454-1.158-1.11-1.466-1.11-1.466-.908-.62.069-.608.069-.608 1.003.07 1.531 1.032 1.531 1.032.892 1.53 2.341 1.088 2.91.832.092-.647.35-1.088.636-1.338-2.22-.253-4.555-1.113-4.555-4.951 0-1.093.39-1.988 1.029-2.688-.103-.253-.446-1.272.098-2.65 0 0 .84-.27 2.75 1.026A9.564 9.564 0 0112 6.844c.85.004 1.705.115 2.504.337 1.909-1.296 2.747-1.027 2.747-1.027.546 1.379.202 2.398.1 2.651.64.7 1.028 1.595 1.028 2.688 0 3.848-2.339 4.695-4.566 4.943.359.309.678.92.678 1.855 0 1.338-.012 2.419-.012 2.747 0 .268.18.58.688.482A10.019 10.019 0 0022 12.017C22 6.484 17.522 2 12 2z" clip-rule="evenodd"/>
                </svg>
              </a>
              <a
                href="https://github.com/postrust/postrust/issues"
                target="_blank"
                rel="noopener noreferrer"
                class="p-2 text-neutral-400 hover:text-white hover:bg-neutral-800 rounded-lg transition-colors"
                aria-label="Issues"
              >
                <svg class="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 8v4m0 4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"/>
                </svg>
              </a>
              <a
                href="https://x.com/postrustorg"
                target="_blank"
                rel="noopener noreferrer"
                class="p-2 text-neutral-400 hover:text-white hover:bg-neutral-800 rounded-lg transition-colors"
                aria-label="Twitter"
              >
                <svg class="w-5 h-5" fill="currentColor" viewBox="0 0 24 24">
                  <path d="M18.244 2.25h3.308l-7.227 8.26 8.502 11.24H16.17l-5.214-6.817L4.99 21.75H1.68l7.73-8.835L1.254 2.25H8.08l4.713 6.231zm-1.161 17.52h1.833L7.084 4.126H5.117z"/>
                </svg>
              </a>
            </div>
          </div>

          {/* Links */}
          <div>
            <h4 class="text-sm font-semibold text-white mb-4">Product</h4>
            <ul class="space-y-3">
              {footerLinks.product.map((link) => (
                <li key={link.href}>
                  <Link
                    href={link.href}
                    class="text-sm text-neutral-400 hover:text-white transition-colors"
                  >
                    {link.label}
                  </Link>
                </li>
              ))}
            </ul>
          </div>

          <div>
            <h4 class="text-sm font-semibold text-white mb-4">Resources</h4>
            <ul class="space-y-3">
              {footerLinks.resources.map((link) => (
                <li key={link.href}>
                  <Link
                    href={link.href}
                    class="text-sm text-neutral-400 hover:text-white transition-colors"
                  >
                    {link.label}
                  </Link>
                </li>
              ))}
            </ul>
          </div>

          <div>
            <h4 class="text-sm font-semibold text-white mb-4">Community</h4>
            <ul class="space-y-3">
              {footerLinks.community.map((link) => (
                <li key={link.href}>
                  {link.external ? (
                    <a
                      href={link.href}
                      target="_blank"
                      rel="noopener noreferrer"
                      class="text-sm text-neutral-400 hover:text-white transition-colors inline-flex items-center gap-1"
                    >
                      {link.label}
                      <svg class="w-3 h-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M10 6H6a2 2 0 00-2 2v10a2 2 0 002 2h10a2 2 0 002-2v-4M14 4h6m0 0v6m0-6L10 14"/>
                      </svg>
                    </a>
                  ) : (
                    <Link
                      href={link.href}
                      class="text-sm text-neutral-400 hover:text-white transition-colors"
                    >
                      {link.label}
                    </Link>
                  )}
                </li>
              ))}
            </ul>
          </div>

          <div>
            <h4 class="text-sm font-semibold text-white mb-4">Company</h4>
            <ul class="space-y-3">
              {footerLinks.company.map((link) => (
                <li key={link.href}>
                  <Link
                    href={link.href}
                    class="text-sm text-neutral-400 hover:text-white transition-colors"
                  >
                    {link.label}
                  </Link>
                </li>
              ))}
            </ul>
          </div>
        </div>

        {/* Bottom Bar */}
        <div class="mt-12 pt-8 border-t border-neutral-800">
          <div class="flex flex-col md:flex-row justify-between items-center gap-4">
            <p class="text-sm text-neutral-500">
              &copy; {currentYear} Postrust. Open source under MIT License.
            </p>
            <div class="flex items-center gap-2 text-sm text-neutral-500">
              <span>Built with</span>
              <svg class="w-4 h-4 text-red-500" fill="currentColor" viewBox="0 0 24 24">
                <path d="M12 21.35l-1.45-1.32C5.4 15.36 2 12.28 2 8.5 2 5.42 4.42 3 7.5 3c1.74 0 3.41.81 4.5 2.09C13.09 3.81 14.76 3 16.5 3 19.58 3 22 5.42 22 8.5c0 3.78-3.4 6.86-8.55 11.54L12 21.35z"/>
              </svg>
              <span>in Rust</span>
            </div>
          </div>
        </div>
      </div>
    </footer>
  );
});
