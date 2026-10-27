import { component$, Slot } from "@builder.io/qwik";
import { LastUpdated } from "~/components/layout/last-updated";

export default component$(() => (
  <>
    <Slot />
    <LastUpdated />
  </>
));
