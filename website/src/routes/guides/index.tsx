import { component$ } from "@builder.io/qwik";
import type { DocumentHead } from "@builder.io/qwik-city";
import { Link } from "@builder.io/qwik-city";
import { guides } from "~/data/guides";

export default component$(() => (
  <div class="min-h-screen bg-white">
    <section class="section-padding bg-gradient-to-b from-neutral-50 to-white">
      <div class="container-wide">
        <div class="max-w-3xl">
          <h1 class="mb-6 text-4xl font-bold text-neutral-900 md:text-5xl">
            Guides to APIs on PostgreSQL
          </h1>
          <p class="text-lg text-neutral-600">
            The questions that come up when you put an API in front of a
            PostgreSQL database, answered: how REST and GraphQL are generated
            from a schema, how access control works when the database enforces
            it, and how the tools that do this differ.
          </p>
        </div>
      </div>
    </section>

    <section class="pb-16">
      <div class="container-wide">
        <div class="grid max-w-5xl gap-6 md:grid-cols-2">
          {guides.map((g) => (
            <Link
              key={g.slug}
              href={`/guides/${g.slug}/`}
              class="hover:border-primary-200 block rounded-2xl border border-neutral-200 p-6 transition-all hover:shadow-lg"
            >
              <h2 class="mb-2 text-xl font-bold text-neutral-900">{g.title}</h2>
              <p class="text-neutral-600">{g.description}</p>
            </Link>
          ))}
        </div>
      </div>
    </section>
  </div>
));

export const head: DocumentHead = {
  title: "Guides to REST and GraphQL APIs on PostgreSQL | Postrust",
  meta: [
    {
      name: "description",
      content:
        "How REST and GraphQL APIs are generated from a PostgreSQL schema, how row-level security secures them, and how the tools that do it compare.",
    },
  ],
};
