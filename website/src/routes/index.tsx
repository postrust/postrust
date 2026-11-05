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
  title: "Postrust: PostgreSQL REST & GraphQL API Server in Rust",
  meta: [
    {
      name: "description",
      content:
        "Expose your PostgreSQL schema as a fast, PostgREST- and Hasura-compatible REST & GraphQL API from one 3 MB Rust binary. Run on Docker, AWS Lambda, or Cloudflare Workers.",
    },
    {
      name: "keywords",
      content:
        "PostgreSQL, REST API, GraphQL, Rust, PostgREST alternative, Hasura alternative, serverless Postgres API, pgvector, AWS Lambda, Cloudflare Workers",
    },
    {
      property: "og:title",
      content: "Postrust: PostgreSQL REST & GraphQL API Server in Rust",
    },
    {
      property: "og:description",
      content:
        "Expose your PostgreSQL schema as a fast, PostgREST- and Hasura-compatible REST & GraphQL API from one 3 MB Rust binary. Run on Docker, AWS Lambda, or Cloudflare Workers.",
    },
    {
      property: "og:type",
      content: "website",
    },
    {
      property: "og:url",
      content: "https://postrust.org/",
    },
    {
      name: "twitter:card",
      content: "summary_large_image",
    },
    {
      name: "twitter:title",
      content: "Postrust: PostgreSQL REST & GraphQL API Server in Rust",
    },
    {
      name: "twitter:description",
      content:
        "Expose your PostgreSQL schema as a fast, PostgREST- and Hasura-compatible REST & GraphQL API from one 3 MB Rust binary. Run on Docker, AWS Lambda, or Cloudflare Workers.",
    },
  ],
};
