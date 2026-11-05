import { component$ } from "@builder.io/qwik";
import type { DocumentHead } from "@builder.io/qwik-city";
import { Link } from "@builder.io/qwik-city";
import { jsonLd } from "~/components/json-ld";
import { LATEST } from "~/data/version";

const plans = [
  {
    name: "Open Source",
    price: "Free",
    description: "Everything you need to build production APIs",
    features: [
      "Full REST & GraphQL API",
      "JWT authentication",
      "PostgreSQL Row-Level Security",
      "All deployment targets",
      "Admin UI (Swagger, Scalar, GraphQL Playground)",
      "Custom routes support",
      "Community support via GitHub Issues",
      "MIT License",
    ],
    cta: "Get Started",
    ctaHref: "/docs/getting-started/",
    highlighted: true,
  },
  {
    name: "Enterprise",
    price: "Custom",
    description: "For teams with advanced security and support needs",
    features: [
      "Everything in Open Source",
      "Priority support with SLA",
      "Security audits & compliance",
      "Custom feature development",
      "Architecture consulting",
      "Training & onboarding",
      "Dedicated Slack channel",
      "Invoice billing",
    ],
    cta: "Contact Sales",
    ctaHref: "/enterprise/",
    highlighted: false,
  },
];

const faqs = [
  {
    question: "Is Postrust really free?",
    answer:
      "Yes! Postrust is 100% open source under the MIT license. You can use it for personal projects, startups, or enterprise applications without any fees.",
  },
  {
    question: "What's included in enterprise support?",
    answer:
      "Enterprise support includes priority response times (< 4 hours for critical issues), dedicated support engineers, architecture reviews, and custom feature development based on your needs.",
  },
  {
    question: "Can I use Postrust in production?",
    answer: `Postrust ${LATEST} is released and the API is stable, with connection pooling, health checks, OpenTelemetry tracing, and structured logging. Be aware of what is and is not measured: 95% of PostgREST's own test suite and 97% of Hasura's pass, the remaining cases are listed on the conformance page, and only compatibility mode is covered by the PostgREST suite. Read those before you migrate something that matters.`,
  },
  {
    question: "Will the license change?",
    answer:
      "No. The server stays MIT, there is no contributor licence agreement, and the enterprise tier is support and consulting rather than features held back from the open source build.",
  },
  {
    question: "How does Postrust compare to managed services?",
    answer:
      "You run it yourself, on your own servers, AWS Lambda or any cloud provider, with no per-request pricing and no vendor account in the request path. That is a trade: you also carry the upgrades, the monitoring and the on-call.",
  },
  {
    question: "Do you offer consulting services?",
    answer:
      "Yes, through our enterprise tier we offer architecture consulting, migration assistance, and custom development. Contact us to discuss your needs.",
  },
];

export default component$(() => {
  return (
    <div class="section-padding min-h-screen bg-white">
      <script
        type="application/ld+json"
        dangerouslySetInnerHTML={jsonLd({
          "@context": "https://schema.org",
          "@type": "FAQPage",
          mainEntity: faqs.map((f) => ({
            "@type": "Question",
            name: f.question,
            acceptedAnswer: {
              "@type": "Answer",
              text: f.answer,
            },
          })),
        })}
      />
      <div class="container-wide">
        {/* Header */}
        <div class="mx-auto mb-16 max-w-3xl text-center">
          <h1 class="mb-4 text-4xl font-bold text-neutral-900 md:text-5xl">
            Simple, transparent pricing
          </h1>
          <p class="text-lg text-neutral-600">
            Postrust is open source and free forever. Enterprise support
            available for teams that need it.
          </p>
        </div>

        {/* Pricing Cards */}
        <div class="mx-auto mb-20 grid max-w-4xl gap-8 md:grid-cols-2">
          {plans.map((plan) => (
            <div
              key={plan.name}
              class={`rounded-2xl p-8 ${
                plan.highlighted
                  ? "ring-primary-500/20 bg-neutral-900 text-white ring-4"
                  : "border border-neutral-200 bg-neutral-50"
              }`}
            >
              {/* Plan Header */}
              <div class="mb-8">
                {plan.highlighted && (
                  <div class="bg-primary-500 mb-4 inline-flex items-center gap-1.5 rounded-full px-3 py-1 text-xs font-medium text-white">
                    <svg
                      class="h-3.5 w-3.5"
                      fill="currentColor"
                      viewBox="0 0 24 24"
                    >
                      <path d="M12 2L15.09 8.26L22 9.27L17 14.14L18.18 21.02L12 17.77L5.82 21.02L7 14.14L2 9.27L8.91 8.26L12 2Z" />
                    </svg>
                    Most Popular
                  </div>
                )}
                <h2
                  class={`mb-2 text-2xl font-bold ${plan.highlighted ? "text-white" : "text-neutral-900"}`}
                >
                  {plan.name}
                </h2>
                <div class="mb-2 flex items-baseline gap-1">
                  <span
                    class={`text-4xl font-bold ${plan.highlighted ? "text-white" : "text-neutral-900"}`}
                  >
                    {plan.price}
                  </span>
                  {plan.price !== "Free" && plan.price !== "Custom" && (
                    <span
                      class={
                        plan.highlighted
                          ? "text-neutral-300"
                          : "text-neutral-500"
                      }
                    >
                      /month
                    </span>
                  )}
                </div>
                <p
                  class={
                    plan.highlighted ? "text-neutral-300" : "text-neutral-600"
                  }
                >
                  {plan.description}
                </p>
              </div>

              {/* Features */}
              <ul class="mb-8 space-y-4">
                {plan.features.map((feature) => (
                  <li key={feature} class="flex items-start gap-3">
                    <svg
                      class={`mt-0.5 h-5 w-5 flex-shrink-0 ${
                        plan.highlighted
                          ? "text-primary-400"
                          : "text-primary-600"
                      }`}
                      fill="none"
                      stroke="currentColor"
                      viewBox="0 0 24 24"
                    >
                      <path
                        stroke-linecap="round"
                        stroke-linejoin="round"
                        stroke-width="2"
                        d="M5 13l4 4L19 7"
                      />
                    </svg>
                    <span
                      class={
                        plan.highlighted
                          ? "text-neutral-200"
                          : "text-neutral-700"
                      }
                    >
                      {feature}
                    </span>
                  </li>
                ))}
              </ul>

              {/* CTA */}
              <Link
                href={plan.ctaHref}
                class={`block w-full rounded-lg px-6 py-3 text-center font-semibold transition-colors ${
                  plan.highlighted
                    ? "bg-white text-neutral-900 hover:bg-neutral-100"
                    : "bg-neutral-900 text-white hover:bg-neutral-800"
                }`}
              >
                {plan.cta}
              </Link>
            </div>
          ))}
        </div>

        {/* FAQs */}
        <div class="mx-auto max-w-3xl">
          <h2 class="mb-8 text-center text-2xl font-bold text-neutral-900">
            Frequently Asked Questions
          </h2>
          <div class="space-y-6">
            {faqs.map((faq) => (
              <div key={faq.question} class="rounded-xl bg-neutral-50 p-6">
                <h3 class="mb-2 text-lg font-semibold text-neutral-900">
                  {faq.question}
                </h3>
                <p class="text-neutral-600">{faq.answer}</p>
              </div>
            ))}
          </div>
        </div>

        {/* Bottom CTA */}
        <div class="mt-20 text-center">
          <p class="mb-4 text-neutral-600">
            Have questions about pricing or need a custom plan?
          </p>
          <Link
            href="/enterprise/"
            class="text-primary-600 hover:text-primary-700 inline-flex items-center gap-2 font-medium"
          >
            Contact our sales team
            <svg
              class="h-4 w-4"
              fill="none"
              stroke="currentColor"
              viewBox="0 0 24 24"
            >
              <path
                stroke-linecap="round"
                stroke-linejoin="round"
                stroke-width="2"
                d="M13 7l5 5m0 0l-5 5m5-5H6"
              />
            </svg>
          </Link>
        </div>
      </div>
    </div>
  );
});

export const head: DocumentHead = {
  title: "Pricing - Postrust is free and open source (MIT)",
  meta: [
    {
      name: "description",
      content:
        "Postrust is free and open source. Enterprise support available for teams that need priority support and custom features.",
    },
  ],
};
