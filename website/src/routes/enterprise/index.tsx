import { component$ } from "@builder.io/qwik";
import type { DocumentHead } from "@builder.io/qwik-city";
import { Link } from "@builder.io/qwik-city";
import { emailOff } from "~/components/email-off";
import { jsonLd } from "~/components/json-ld";

const enterpriseFeatures = [
  {
    title: "Priority Support",
    description: "< 4 hour response time for critical issues, dedicated support engineers, and direct access to the core team.",
    icon: "support",
  },
  {
    title: "Security & Compliance",
    description: "Security audits, compliance assistance, and custom security configurations for your requirements.",
    icon: "shield",
  },
  {
    title: "Custom Development",
    description: "Feature development, integrations, and customizations tailored to your business needs.",
    icon: "code",
  },
  {
    title: "Architecture Consulting",
    description: "Expert guidance on database design, API architecture, and performance optimization.",
    icon: "architecture",
  },
  {
    title: "Training & Onboarding",
    description: "Comprehensive training for your team, custom documentation, and hands-on workshops.",
    icon: "training",
  },
  {
    title: "SLA Guarantee",
    description: "Service level agreements with guaranteed uptime and response times for your peace of mind.",
    icon: "sla",
  },
];

/** Asked of every open-source API server a team considers for production. */
const enterpriseFaq = [
  {
    q: "Is there commercial support for Postrust?",
    a: "Yes. Support agreements, security reviews, custom development and consulting are arranged directly with the people who maintain Postrust; write to founder@postrust.org. The server itself is MIT licensed and free, with or without an agreement.",
  },
  {
    q: "Does Postrust have team management and billing?",
    a: "Not in the API server, which serves your database and nothing else. The open-source Postrust SaaS starter kit adds them on top: teams, invitations and roles, Stripe or LemonSqueezy subscriptions, API keys and audit logs, all enforced by PostgreSQL row-level security.",
  },
  {
    q: "How do teams control who can see which rows?",
    a: "With PostgreSQL itself. Each request runs as the database role named in its JWT, with the token's claims available to row-level security policies, so tenant and team boundaries hold the same way for REST and GraphQL.",
  },
  {
    q: "Where can Postrust run?",
    a: "Anywhere you can run a binary or a container: on servers, in Docker, on Kubernetes with the Helm chart, on Fly.io or Railway, and serverless on AWS Lambda and Cloudflare Workers.",
  },
  {
    q: "Can we move from PostgREST or Hasura without rewriting clients?",
    a: "Mostly, and you can check how far for yourself. Postrust answers PostgREST's REST dialect and Hasura's GraphQL dialect, and replaying each project's own test suite against both agrees on about 97% of cases; the conformance reports list every known difference, so test the queries your clients depend on before you switch.",
  },
];

const faqJsonLd = jsonLd({
  "@context": "https://schema.org",
  "@type": "FAQPage",
  mainEntity: enterpriseFaq.map((entry) => ({
    "@type": "Question",
    name: entry.q,
    acceptedAnswer: { "@type": "Answer", text: entry.a },
  })),
});

const iconPaths: Record<string, string> = {
  support: "M18.364 5.636l-3.536 3.536m0 5.656l3.536 3.536M9.172 9.172L5.636 5.636m3.536 9.192l-3.536 3.536M21 12a9 9 0 11-18 0 9 9 0 0118 0zm-5 0a4 4 0 11-8 0 4 4 0 018 0z",
  shield: "M9 12l2 2 4-4m5.618-4.016A11.955 11.955 0 0112 2.944a11.955 11.955 0 01-8.618 3.04A12.02 12.02 0 003 9c0 5.591 3.824 10.29 9 11.622 5.176-1.332 9-6.03 9-11.622 0-1.042-.133-2.052-.382-3.016z",
  code: "M10 20l4-16m4 4l4 4-4 4M6 16l-4-4 4-4",
  architecture: "M19 21V5a2 2 0 00-2-2H7a2 2 0 00-2 2v16m14 0h2m-2 0h-5m-9 0H3m2 0h5M9 7h1m-1 4h1m4-4h1m-1 4h1m-5 10v-5a1 1 0 011-1h2a1 1 0 011 1v5m-4 0h4",
  training: "M12 6.253v13m0-13C10.832 5.477 9.246 5 7.5 5S4.168 5.477 3 6.253v13C4.168 18.477 5.754 18 7.5 18s3.332.477 4.5 1.253m0-13C13.168 5.477 14.754 5 16.5 5c1.747 0 3.332.477 4.5 1.253v13C19.832 18.477 18.247 18 16.5 18c-1.746 0-3.332.477-4.5 1.253",
  sla: "M9 12l2 2 4-4M7.835 4.697a3.42 3.42 0 001.946-.806 3.42 3.42 0 014.438 0 3.42 3.42 0 001.946.806 3.42 3.42 0 013.138 3.138 3.42 3.42 0 00.806 1.946 3.42 3.42 0 010 4.438 3.42 3.42 0 00-.806 1.946 3.42 3.42 0 01-3.138 3.138 3.42 3.42 0 00-1.946.806 3.42 3.42 0 01-4.438 0 3.42 3.42 0 00-1.946-.806 3.42 3.42 0 01-3.138-3.138 3.42 3.42 0 00-.806-1.946 3.42 3.42 0 010-4.438 3.42 3.42 0 00.806-1.946 3.42 3.42 0 013.138-3.138z",
};

export default component$(() => {
  return (
    <div class="min-h-screen bg-white">
      {/* Hero */}
      <section class="section-padding bg-gradient-to-b from-neutral-900 to-neutral-800 text-white">
        <div class="container-wide">
          <div class="max-w-3xl mx-auto text-center">
            <div class="inline-flex items-center gap-2 px-4 py-2 bg-white/10 backdrop-blur border border-white/20 rounded-full mb-8">
              <span class="text-sm font-medium text-white/90">
                Enterprise Solutions
              </span>
            </div>
            <h1 class="text-4xl md:text-5xl font-bold mb-6">
              Postrust for Enterprise
            </h1>
            <p class="text-lg text-neutral-300">
              Get priority support, security audits, custom development, and expert
              consulting for your mission-critical applications.
            </p>
          </div>
        </div>
      </section>

      {/* Features */}
      <section class="section-padding">
        <div class="container-wide">
          <div class="grid md:grid-cols-2 lg:grid-cols-3 gap-8">
            {enterpriseFeatures.map((feature) => (
              <div
                key={feature.title}
                class="bg-white rounded-xl p-6 border border-neutral-200 hover:shadow-lg transition-shadow"
              >
                <div class="w-12 h-12 bg-primary-100 rounded-lg flex items-center justify-center mb-4">
                  <svg
                    class="w-6 h-6 text-primary-600"
                    fill="none"
                    stroke="currentColor"
                    viewBox="0 0 24 24"
                    stroke-width="2"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                  >
                    <path d={iconPaths[feature.icon]} />
                  </svg>
                </div>
                <h3 class="text-xl font-semibold text-neutral-900 mb-2">
                  {feature.title}
                </h3>
                <p class="text-neutral-600">
                  {feature.description}
                </p>
              </div>
            ))}
          </div>
        </div>
      </section>

      {/* FAQ */}
      <section class="section-padding">
        <script type="application/ld+json" dangerouslySetInnerHTML={faqJsonLd} />
        <div class="container-wide">
          <div class="max-w-3xl mx-auto">
            <h2 class="text-3xl font-bold text-neutral-900 mb-8">
              Running Postrust in production: common questions
            </h2>
            <div class="space-y-6">
              {enterpriseFaq.map((entry) => (
                <div key={entry.q}>
                  <h3 class="mb-2 text-lg font-semibold text-neutral-900">
                    {entry.q}
                  </h3>
                  <p
                    class="text-neutral-600"
                    dangerouslySetInnerHTML={emailOff(entry.a)}
                  />
                </div>
              ))}
            </div>
            <p class="mt-8 text-neutral-600">
              See the{" "}
              <Link
                href="/docs/conformance/"
                class="text-primary-600 hover:text-primary-700"
              >
                conformance reports
              </Link>
              , the{" "}
              <a
                href="https://github.com/postrust/postrust-saas-starter"
                target="_blank"
                rel="noopener noreferrer"
                class="text-primary-600 hover:text-primary-700"
              >
                SaaS starter kit
              </a>{" "}
              and the{" "}
              <Link
                href="/guides/postgresql-row-level-security-api/"
                class="text-primary-600 hover:text-primary-700"
              >
                guide to row-level security behind an API
              </Link>
              .
            </p>
          </div>
        </div>
      </section>

      {/* Contact Form */}
      <section class="section-padding bg-neutral-50">
        <div class="container-wide">
          <div class="max-w-2xl mx-auto">
            <div class="text-center mb-12">
              <h2 class="text-3xl font-bold text-neutral-900 mb-4">
                Talk to us
              </h2>
              <p class="text-lg text-neutral-600">
                Support and consulting for teams running Postrust in
                production.
              </p>
            </div>

            <div class="bg-white rounded-2xl border border-neutral-200 shadow-sm p-8">
              <p class="text-neutral-700 mb-6">
                Enterprise support is handled by email, by the people who
                maintain the server. There is no form and no sales team in
                between.
              </p>

              {/* Raw HTML so the email_off markers reach the response:
                  without them Cloudflare's Email Obfuscation rewrites the
                  address into a /cdn-cgi/l/email-protection link that search
                  engines then index. */}
              <div
                dangerouslySetInnerHTML={`<!--email_off--><a href="mailto:founder@postrust.org?subject=Postrust%20enterprise%20support" class="inline-flex items-center gap-2 px-6 py-3 text-base font-semibold text-white bg-neutral-900 hover:bg-neutral-800 rounded-lg transition-colors"><svg class="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M3 8l7.89 5.26a2 2 0 002.22 0L21 8M5 19h14a2 2 0 002-2V7a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2z"/></svg>founder@postrust.org</a><!--/email_off-->`}
              />

              <p class="mt-6 text-sm text-neutral-500">
                It helps to include your team size, what you are running today,
                and what you need from a support agreement. Bug reports and
                feature requests are better off in{" "}
                <a
                  href="https://github.com/postrust/postrust/issues"
                  target="_blank"
                  rel="noopener noreferrer"
                  class="text-primary-600 hover:text-primary-700"
                >
                  GitHub Issues
                </a>
                , where everyone can see them.
              </p>
            </div>
          </div>
        </div>
      </section>

      {/* Trust -- disabled until there are real logos to show.
          Re-enable only when named companies have agreed to be listed:
          five grey placeholders under "Trusted by teams at companies of
          all sizes" claims social proof the project does not have yet,
          and reads as an unfinished template.

          Note: the inner `Placeholder logos` comment was dropped, since a
          nested block comment would close this one early.

      <section class="section-padding">
        <div class="container-wide text-center">
          <p class="text-neutral-500 mb-8">
            Trusted by teams at companies of all sizes
          </p>
          <div class="flex flex-wrap items-center justify-center gap-12 opacity-50">
            {[1, 2, 3, 4, 5].map((i) => (
              <div key={i} class="w-32 h-8 bg-neutral-200 rounded"></div>
            ))}
          </div>
        </div>
      </section>

      */}
    </div>
  );
});

export const head: DocumentHead = {
  title: "Enterprise support for Postrust, the PostgreSQL API",
  meta: [
    {
      name: "description",
      content: "Paid support for Postrust: priority support, security reviews, custom development and consulting, plus team management and billing via the SaaS starter.",
    },
  ],
};
