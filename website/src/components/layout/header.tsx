import { component$, useSignal } from "@builder.io/qwik";
import { Link } from "@builder.io/qwik-city";

export const Header = component$(() => {
  const isMenuOpen = useSignal(false);

  const navLinks = [
    { href: "/features", label: "Features" },
    { href: "/docs", label: "Documentation" },
    { href: "/pricing", label: "Pricing" },
    { href: "/compare", label: "Compare" },
    { href: "/community", label: "Community" },
  ];

  return (
    <header class="fixed top-0 left-0 right-0 z-50 bg-white/80 backdrop-blur-lg border-b border-neutral-200">
      <nav class="container-wide">
        <div class="flex items-center justify-between h-16">
          {/* Logo */}
          <Link href="/" class="flex items-center gap-2 group">
            <div class="w-8 h-8 bg-gradient-to-br from-primary-500 to-primary-700 rounded-lg flex items-center justify-center">
              <svg class="w-5 h-5 text-white" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5">
                <path d="M13 2L3 14h9l-1 8 10-12h-9l1-8z" stroke-linecap="round" stroke-linejoin="round"/>
              </svg>
            </div>
            <span class="text-xl font-bold text-neutral-900 group-hover:text-primary-600 transition-colors">
              Postrust
            </span>
          </Link>

          {/* Desktop Navigation */}
          <div class="hidden md:flex items-center gap-1">
            {navLinks.map((link) => (
              <Link
                key={link.href}
                href={link.href}
                class="px-4 py-2 text-sm font-medium text-neutral-600 hover:text-neutral-900 hover:bg-neutral-100 rounded-lg transition-colors"
              >
                {link.label}
              </Link>
            ))}
          </div>

          {/* Desktop CTA */}
          <div class="hidden md:flex items-center gap-3">
            <a
              href="https://github.com/postrust/postrust"
              target="_blank"
              rel="noopener noreferrer"
              class="flex items-center gap-2 px-4 py-2 text-sm font-medium text-neutral-600 hover:text-neutral-900 hover:bg-neutral-100 rounded-lg transition-colors"
            >
              <svg class="w-5 h-5" fill="currentColor" viewBox="0 0 24 24">
                <path fill-rule="evenodd" d="M12 2C6.477 2 2 6.484 2 12.017c0 4.425 2.865 8.18 6.839 9.504.5.092.682-.217.682-.483 0-.237-.008-.868-.013-1.703-2.782.605-3.369-1.343-3.369-1.343-.454-1.158-1.11-1.466-1.11-1.466-.908-.62.069-.608.069-.608 1.003.07 1.531 1.032 1.531 1.032.892 1.53 2.341 1.088 2.91.832.092-.647.35-1.088.636-1.338-2.22-.253-4.555-1.113-4.555-4.951 0-1.093.39-1.988 1.029-2.688-.103-.253-.446-1.272.098-2.65 0 0 .84-.27 2.75 1.026A9.564 9.564 0 0112 6.844c.85.004 1.705.115 2.504.337 1.909-1.296 2.747-1.027 2.747-1.027.546 1.379.202 2.398.1 2.651.64.7 1.028 1.595 1.028 2.688 0 3.848-2.339 4.695-4.566 4.943.359.309.678.92.678 1.855 0 1.338-.012 2.419-.012 2.747 0 .268.18.58.688.482A10.019 10.019 0 0022 12.017C22 6.484 17.522 2 12 2z" clip-rule="evenodd"/>
              </svg>
              GitHub
            </a>
            <Link
              href="/docs"
              class="px-4 py-2 text-sm font-semibold text-white bg-neutral-900 hover:bg-neutral-800 rounded-lg transition-colors"
            >
              Get Started
            </Link>
          </div>

          {/* Mobile Menu Button */}
          <button
            type="button"
            class="md:hidden p-2 text-neutral-600 hover:text-neutral-900 hover:bg-neutral-100 rounded-lg"
            onClick$={() => isMenuOpen.value = !isMenuOpen.value}
            aria-label="Toggle menu"
          >
            {isMenuOpen.value ? (
              <svg class="w-6 h-6" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12"/>
              </svg>
            ) : (
              <svg class="w-6 h-6" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 6h16M4 12h16M4 18h16"/>
              </svg>
            )}
          </button>
        </div>

        {/* Mobile Menu */}
        {isMenuOpen.value && (
          <div class="md:hidden py-4 border-t border-neutral-200">
            <div class="flex flex-col gap-1">
              {navLinks.map((link) => (
                <Link
                  key={link.href}
                  href={link.href}
                  class="px-4 py-3 text-base font-medium text-neutral-600 hover:text-neutral-900 hover:bg-neutral-100 rounded-lg transition-colors"
                >
                  {link.label}
                </Link>
              ))}
              <hr class="my-2 border-neutral-200" />
              <a
                href="https://github.com/postrust/postrust"
                target="_blank"
                rel="noopener noreferrer"
                class="flex items-center gap-2 px-4 py-3 text-base font-medium text-neutral-600 hover:text-neutral-900 hover:bg-neutral-100 rounded-lg"
              >
                <svg class="w-5 h-5" fill="currentColor" viewBox="0 0 24 24">
                  <path fill-rule="evenodd" d="M12 2C6.477 2 2 6.484 2 12.017c0 4.425 2.865 8.18 6.839 9.504.5.092.682-.217.682-.483 0-.237-.008-.868-.013-1.703-2.782.605-3.369-1.343-3.369-1.343-.454-1.158-1.11-1.466-1.11-1.466-.908-.62.069-.608.069-.608 1.003.07 1.531 1.032 1.531 1.032.892 1.53 2.341 1.088 2.91.832.092-.647.35-1.088.636-1.338-2.22-.253-4.555-1.113-4.555-4.951 0-1.093.39-1.988 1.029-2.688-.103-.253-.446-1.272.098-2.65 0 0 .84-.27 2.75 1.026A9.564 9.564 0 0112 6.844c.85.004 1.705.115 2.504.337 1.909-1.296 2.747-1.027 2.747-1.027.546 1.379.202 2.398.1 2.651.64.7 1.028 1.595 1.028 2.688 0 3.848-2.339 4.695-4.566 4.943.359.309.678.92.678 1.855 0 1.338-.012 2.419-.012 2.747 0 .268.18.58.688.482A10.019 10.019 0 0022 12.017C22 6.484 17.522 2 12 2z" clip-rule="evenodd"/>
                </svg>
                GitHub
              </a>
              <Link
                href="/docs"
                class="mx-4 mt-2 px-4 py-3 text-center text-base font-semibold text-white bg-neutral-900 hover:bg-neutral-800 rounded-lg transition-colors"
              >
                Get Started
              </Link>
            </div>
          </div>
        )}
      </nav>
    </header>
  );
});
