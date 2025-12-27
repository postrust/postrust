import { component$ } from "@builder.io/qwik";
import type { DocumentHead } from "@builder.io/qwik-city";
import { Link } from "@builder.io/qwik-city";

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
    ctaHref: "/docs/getting-started",
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
    ctaHref: "/enterprise",
    highlighted: false,
  },
];

const faqs = [
  {
    question: "Is Postrust really free?",
    answer: "Yes! Postrust is 100% open source under the MIT license. You can use it for personal projects, startups, or enterprise applications without any fees.",
  },
  {
    question: "What's included in enterprise support?",
    answer: "Enterprise support includes priority response times (< 4 hours for critical issues), dedicated support engineers, architecture reviews, and custom feature development based on your needs.",
  },
  {
    question: "Can I use Postrust in production?",
    answer: "Absolutely. Postrust is designed for production workloads with features like connection pooling, health checks, and comprehensive logging. Many teams run it in production today.",
  },
  {
    question: "How does Postrust compare to managed services?",
    answer: "Postrust gives you full control over your infrastructure while being significantly more cost-effective. You can run it on your own servers, AWS Lambda, or any cloud provider without per-request pricing.",
  },
  {
    question: "Do you offer consulting services?",
    answer: "Yes, through our enterprise tier we offer architecture consulting, migration assistance, and custom development. Contact us to discuss your needs.",
  },
];

export default component$(() => {
  return (
    <div class="section-padding bg-white min-h-screen">
      <div class="container-wide">
        {/* Header */}
        <div class="text-center max-w-3xl mx-auto mb-16">
          <h1 class="text-4xl md:text-5xl font-bold text-neutral-900 mb-4">
            Simple, transparent pricing
          </h1>
          <p class="text-lg text-neutral-600">
            Postrust is open source and free forever. Enterprise support available for teams that need it.
          </p>
        </div>

        {/* Pricing Cards */}
        <div class="grid md:grid-cols-2 gap-8 max-w-4xl mx-auto mb-20">
          {plans.map((plan) => (
            <div
              key={plan.name}
              class={`rounded-2xl p-8 ${
                plan.highlighted
                  ? "bg-neutral-900 text-white ring-4 ring-primary-500/20"
                  : "bg-neutral-50 border border-neutral-200"
              }`}
            >
              {/* Plan Header */}
              <div class="mb-8">
                {plan.highlighted && (
                  <div class="inline-flex items-center gap-1.5 px-3 py-1 bg-primary-500 text-white rounded-full text-xs font-medium mb-4">
                    <svg class="w-3.5 h-3.5" fill="currentColor" viewBox="0 0 24 24">
                      <path d="M12 2L15.09 8.26L22 9.27L17 14.14L18.18 21.02L12 17.77L5.82 21.02L7 14.14L2 9.27L8.91 8.26L12 2Z"/>
                    </svg>
                    Most Popular
                  </div>
                )}
                <h2 class={`text-2xl font-bold mb-2 ${plan.highlighted ? "text-white" : "text-neutral-900"}`}>
                  {plan.name}
                </h2>
                <div class="flex items-baseline gap-1 mb-2">
                  <span class={`text-4xl font-bold ${plan.highlighted ? "text-white" : "text-neutral-900"}`}>
                    {plan.price}
                  </span>
                  {plan.price !== "Free" && plan.price !== "Custom" && (
                    <span class={plan.highlighted ? "text-neutral-300" : "text-neutral-500"}>/month</span>
                  )}
                </div>
                <p class={plan.highlighted ? "text-neutral-300" : "text-neutral-600"}>
                  {plan.description}
                </p>
              </div>

              {/* Features */}
              <ul class="space-y-4 mb-8">
                {plan.features.map((feature) => (
                  <li key={feature} class="flex items-start gap-3">
                    <svg
                      class={`w-5 h-5 mt-0.5 flex-shrink-0 ${
                        plan.highlighted ? "text-primary-400" : "text-primary-600"
                      }`}
                      fill="none"
                      stroke="currentColor"
                      viewBox="0 0 24 24"
                    >
                      <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 13l4 4L19 7"/>
                    </svg>
                    <span class={plan.highlighted ? "text-neutral-200" : "text-neutral-700"}>
                      {feature}
                    </span>
                  </li>
                ))}
              </ul>

              {/* CTA */}
              <Link
                href={plan.ctaHref}
                class={`block w-full py-3 px-6 text-center font-semibold rounded-lg transition-colors ${
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
        <div class="max-w-3xl mx-auto">
          <h2 class="text-2xl font-bold text-neutral-900 text-center mb-8">
            Frequently Asked Questions
          </h2>
          <div class="space-y-6">
            {faqs.map((faq) => (
              <div key={faq.question} class="bg-neutral-50 rounded-xl p-6">
                <h3 class="text-lg font-semibold text-neutral-900 mb-2">
                  {faq.question}
                </h3>
                <p class="text-neutral-600">
                  {faq.answer}
                </p>
              </div>
            ))}
          </div>
        </div>

        {/* Bottom CTA */}
        <div class="mt-20 text-center">
          <p class="text-neutral-600 mb-4">
            Have questions about pricing or need a custom plan?
          </p>
          <Link
            href="/enterprise"
            class="inline-flex items-center gap-2 text-primary-600 hover:text-primary-700 font-medium"
          >
            Contact our sales team
            <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13 7l5 5m0 0l-5 5m5-5H6"/>
            </svg>
          </Link>
        </div>
      </div>
    </div>
  );
});

export const head: DocumentHead = {
  title: "Pricing - Postrust",
  meta: [
    {
      name: "description",
      content: "Postrust is free and open source. Enterprise support available for teams that need priority support and custom features.",
    },
  ],
};
