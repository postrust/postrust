import { component$ } from "@builder.io/qwik";
import type { DocumentHead } from "@builder.io/qwik-city";
import { HeroSection } from "~/components/sections/hero";
import { FeaturesSection } from "~/components/sections/features";
import { PerformanceSection } from "~/components/sections/performance";
import { CodeExamplesSection } from "~/components/sections/code-examples";
import { SaasStarterSection } from "~/components/sections/saas-starter";
import { DeploymentSection } from "~/components/sections/deployment";
import { CTASection } from "~/components/sections/cta";

export default component$(() => {
  return (
    <>
      <HeroSection />
      <FeaturesSection />
      <PerformanceSection />
      <CodeExamplesSection />
      <SaasStarterSection />
      <DeploymentSection />
      <CTASection />
    </>
  );
});

export const head: DocumentHead = {
  title: "Postrust - PostgreSQL REST & GraphQL API Server",
  meta: [
    {
      name: "description",
      content: "High-performance PostgreSQL REST & GraphQL API server written in Rust. Native AWS Lambda support with ~50ms cold starts. Drop-in PostgREST replacement.",
    },
    {
      name: "keywords",
      content: "PostgreSQL, REST API, GraphQL, Rust, serverless, PostgREST, Lambda, database API",
    },
    {
      property: "og:title",
      content: "Postrust - PostgreSQL REST & GraphQL API Server",
    },
    {
      property: "og:description",
      content: "High-performance PostgreSQL REST & GraphQL API server written in Rust. Native AWS Lambda support with ~50ms cold starts.",
    },
    {
      property: "og:type",
      content: "website",
    },
    {
      property: "og:url",
      content: "https://postrust.org",
    },
    {
      name: "twitter:card",
      content: "summary_large_image",
    },
    {
      name: "twitter:title",
      content: "Postrust - PostgreSQL REST & GraphQL API Server",
    },
    {
      name: "twitter:description",
      content: "High-performance PostgreSQL REST & GraphQL API server written in Rust. Native AWS Lambda support with ~50ms cold starts.",
    },
  ],
};
