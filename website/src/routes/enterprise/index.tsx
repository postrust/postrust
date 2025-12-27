import { component$, useSignal } from "@builder.io/qwik";
import type { DocumentHead } from "@builder.io/qwik-city";

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

const iconPaths: Record<string, string> = {
  support: "M18.364 5.636l-3.536 3.536m0 5.656l3.536 3.536M9.172 9.172L5.636 5.636m3.536 9.192l-3.536 3.536M21 12a9 9 0 11-18 0 9 9 0 0118 0zm-5 0a4 4 0 11-8 0 4 4 0 018 0z",
  shield: "M9 12l2 2 4-4m5.618-4.016A11.955 11.955 0 0112 2.944a11.955 11.955 0 01-8.618 3.04A12.02 12.02 0 003 9c0 5.591 3.824 10.29 9 11.622 5.176-1.332 9-6.03 9-11.622 0-1.042-.133-2.052-.382-3.016z",
  code: "M10 20l4-16m4 4l4 4-4 4M6 16l-4-4 4-4",
  architecture: "M19 21V5a2 2 0 00-2-2H7a2 2 0 00-2 2v16m14 0h2m-2 0h-5m-9 0H3m2 0h5M9 7h1m-1 4h1m4-4h1m-1 4h1m-5 10v-5a1 1 0 011-1h2a1 1 0 011 1v5m-4 0h4",
  training: "M12 6.253v13m0-13C10.832 5.477 9.246 5 7.5 5S4.168 5.477 3 6.253v13C4.168 18.477 5.754 18 7.5 18s3.332.477 4.5 1.253m0-13C13.168 5.477 14.754 5 16.5 5c1.747 0 3.332.477 4.5 1.253v13C19.832 18.477 18.247 18 16.5 18c-1.746 0-3.332.477-4.5 1.253",
  sla: "M9 12l2 2 4-4M7.835 4.697a3.42 3.42 0 001.946-.806 3.42 3.42 0 014.438 0 3.42 3.42 0 001.946.806 3.42 3.42 0 013.138 3.138 3.42 3.42 0 00.806 1.946 3.42 3.42 0 010 4.438 3.42 3.42 0 00-.806 1.946 3.42 3.42 0 01-3.138 3.138 3.42 3.42 0 00-1.946.806 3.42 3.42 0 01-4.438 0 3.42 3.42 0 00-1.946-.806 3.42 3.42 0 01-3.138-3.138 3.42 3.42 0 00-.806-1.946 3.42 3.42 0 010-4.438 3.42 3.42 0 00.806-1.946 3.42 3.42 0 013.138-3.138z",
};

export default component$(() => {
  const formSubmitted = useSignal(false);

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

      {/* Contact Form */}
      <section class="section-padding bg-neutral-50">
        <div class="container-wide">
          <div class="max-w-2xl mx-auto">
            <div class="text-center mb-12">
              <h2 class="text-3xl font-bold text-neutral-900 mb-4">
                Contact Sales
              </h2>
              <p class="text-lg text-neutral-600">
                Tell us about your needs and we'll get back to you within one business day.
              </p>
            </div>

            {formSubmitted.value ? (
              <div class="bg-green-50 border border-green-200 rounded-xl p-8 text-center">
                <div class="w-16 h-16 bg-green-100 rounded-full flex items-center justify-center mx-auto mb-4">
                  <svg class="w-8 h-8 text-green-600" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 13l4 4L19 7"/>
                  </svg>
                </div>
                <h3 class="text-xl font-semibold text-neutral-900 mb-2">
                  Thank you for reaching out!
                </h3>
                <p class="text-neutral-600">
                  We've received your message and will get back to you within one business day.
                </p>
              </div>
            ) : (
              <form
                class="bg-white rounded-2xl p-8 border border-neutral-200 shadow-sm"
                preventdefault:submit
                onSubmit$={() => {
                  formSubmitted.value = true;
                }}
              >
                <div class="grid md:grid-cols-2 gap-6 mb-6">
                  <div>
                    <label class="block text-sm font-medium text-neutral-700 mb-2">
                      First Name *
                    </label>
                    <input
                      type="text"
                      required
                      class="w-full px-4 py-3 border border-neutral-300 rounded-lg focus:ring-2 focus:ring-primary-500 focus:border-primary-500 outline-none transition-colors"
                      placeholder="John"
                    />
                  </div>
                  <div>
                    <label class="block text-sm font-medium text-neutral-700 mb-2">
                      Last Name *
                    </label>
                    <input
                      type="text"
                      required
                      class="w-full px-4 py-3 border border-neutral-300 rounded-lg focus:ring-2 focus:ring-primary-500 focus:border-primary-500 outline-none transition-colors"
                      placeholder="Doe"
                    />
                  </div>
                </div>

                <div class="mb-6">
                  <label class="block text-sm font-medium text-neutral-700 mb-2">
                    Work Email *
                  </label>
                  <input
                    type="email"
                    required
                    class="w-full px-4 py-3 border border-neutral-300 rounded-lg focus:ring-2 focus:ring-primary-500 focus:border-primary-500 outline-none transition-colors"
                    placeholder="john@company.com"
                  />
                </div>

                <div class="mb-6">
                  <label class="block text-sm font-medium text-neutral-700 mb-2">
                    Company *
                  </label>
                  <input
                    type="text"
                    required
                    class="w-full px-4 py-3 border border-neutral-300 rounded-lg focus:ring-2 focus:ring-primary-500 focus:border-primary-500 outline-none transition-colors"
                    placeholder="Company Inc."
                  />
                </div>

                <div class="mb-6">
                  <label class="block text-sm font-medium text-neutral-700 mb-2">
                    Team Size
                  </label>
                  <select class="w-full px-4 py-3 border border-neutral-300 rounded-lg focus:ring-2 focus:ring-primary-500 focus:border-primary-500 outline-none transition-colors">
                    <option value="">Select team size</option>
                    <option value="1-10">1-10 employees</option>
                    <option value="11-50">11-50 employees</option>
                    <option value="51-200">51-200 employees</option>
                    <option value="201-1000">201-1000 employees</option>
                    <option value="1000+">1000+ employees</option>
                  </select>
                </div>

                <div class="mb-8">
                  <label class="block text-sm font-medium text-neutral-700 mb-2">
                    How can we help? *
                  </label>
                  <textarea
                    required
                    rows={4}
                    class="w-full px-4 py-3 border border-neutral-300 rounded-lg focus:ring-2 focus:ring-primary-500 focus:border-primary-500 outline-none transition-colors resize-none"
                    placeholder="Tell us about your project and requirements..."
                  ></textarea>
                </div>

                <button
                  type="submit"
                  class="w-full py-4 px-6 text-base font-semibold text-white bg-neutral-900 hover:bg-neutral-800 rounded-lg transition-colors"
                >
                  Submit Request
                </button>

                <p class="mt-4 text-center text-sm text-neutral-500">
                  By submitting this form, you agree to our{" "}
                  <a href="/privacy" class="text-primary-600 hover:text-primary-700">
                    Privacy Policy
                  </a>
                  .
                </p>
              </form>
            )}
          </div>
        </div>
      </section>

      {/* Trust */}
      <section class="section-padding">
        <div class="container-wide text-center">
          <p class="text-neutral-500 mb-8">
            Trusted by teams at companies of all sizes
          </p>
          <div class="flex flex-wrap items-center justify-center gap-12 opacity-50">
            {/* Placeholder logos */}
            {[1, 2, 3, 4, 5].map((i) => (
              <div key={i} class="w-32 h-8 bg-neutral-200 rounded"></div>
            ))}
          </div>
        </div>
      </section>
    </div>
  );
});

export const head: DocumentHead = {
  title: "Enterprise - Postrust",
  meta: [
    {
      name: "description",
      content: "Postrust Enterprise - Priority support, security audits, custom development, and expert consulting for your organization.",
    },
  ],
};
