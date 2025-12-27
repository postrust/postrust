import { component$, Slot } from "@builder.io/qwik";
import type { RequestHandler } from "@builder.io/qwik-city";
import { Header } from "~/components/layout/header";
import { Footer } from "~/components/layout/footer";

export const onGet: RequestHandler = async ({ cacheControl }) => {
  cacheControl({
    staleWhileRevalidate: 60 * 60 * 24 * 7,
    maxAge: 5,
  });
};

export default component$(() => {
  return (
    <>
      <Header />
      <main class="min-h-screen pt-16">
        <Slot />
      </main>
      <Footer />
    </>
  );
});
