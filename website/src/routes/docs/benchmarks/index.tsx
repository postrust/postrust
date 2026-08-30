import { component$ } from "@builder.io/qwik";
import type { DocumentHead } from "@builder.io/qwik-city";
import { Link } from "@builder.io/qwik-city";

const ruledOut = [
  [
    "Thermal throttling and measurement order",
    "Each server was measured both first and second in an alternating design. No advantage follows the position.",
  ],
  [
    "Host load",
    "The harness scored higher at load average 18 than at load 5, while measuring by hand held steady at both.",
  ],
  [
    "A cold database",
    "The tables are 49 MB against 128 MB of shared_buffers, the fixtures ANALYZE, and the harness pre-warms with count(*).",
  ],
  [
    "The bulk load and checkpointing",
    "Reloading 400,000 rows and forcing a CHECKPOINT moved nothing.",
  ],
  [
    "Server and database start-up",
    "A restarted server reaches full throughput on its first sample; so does a restarted PostgreSQL.",
  ],
  [
    "Pausing the other containers",
    "A run with a single target, where there is nothing to pause, is equally depressed.",
  ],
  [
    "The harness's own measurement code",
    "Calling it directly on a settled stack returns the right answer, and replicating its full sequence by hand does too.",
  ],
  [
    "Docker's port forwarding",
    "The same load generator over the forwarded path and from inside the container network agree.",
  ],
];

export default component$(() => {
  return (
    <div class="min-h-screen bg-white">
      <div class="border-b border-neutral-200 bg-gradient-to-b from-neutral-50 to-white">
        <div class="container-wide py-12">
          <div class="mb-4 flex items-center gap-2 text-sm text-neutral-500">
            <Link href="/docs" class="hover:text-primary-600">
              Docs
            </Link>
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
                d="M9 5l7 7-7 7"
              />
            </svg>
            <span class="text-neutral-900">Benchmarks</span>
          </div>
          <div class="mb-4 flex items-center gap-3">
            <span class="rounded border border-amber-200 bg-amber-100 px-2 py-0.5 text-xs font-semibold text-amber-800">
              WITHDRAWN
            </span>
          </div>
          <h1 class="mb-4 text-4xl font-bold text-neutral-900">Benchmarks</h1>
          <p class="max-w-2xl text-lg text-neutral-600">
            There are no throughput numbers on this page. The ones that used to
            be here were measured with an instrument that turned out to be
            reporting the order of a run as much as the speed of a server, so
            they have been taken down rather than restated.
          </p>
        </div>
      </div>

      <div class="container-wide py-12">
        <div class="max-w-3xl">
          <section class="mb-12">
            <h2 class="mb-4 text-2xl font-bold text-neutral-900">
              What went wrong
            </h2>
            <p class="mb-4 text-neutral-600">
              The same server, in the same container, measured seconds apart,
              differed by about a factor of two: the harness reported around
              6,000 requests per second where measuring the identical endpoint
              by hand immediately afterwards gave around 13,000.
            </p>
            <p class="mb-4 text-neutral-600">
              The harness was not misreporting. Its load generator was
              instrumented to print its own arguments, and the rate it published
              matched wall-clock time. The server really was slower while the
              run was happening.
            </p>
            <p class="text-neutral-600">
              The clearest symptom: within a single run, the REST scenarios
              settled at about 6,200 requests per second while the GraphQL
              scenarios, which run afterwards, settled at about 12,600 &mdash;
              the same server, in the same process, during the same run.
              Whatever was measured first was the most understated, which makes
              the order of the scenarios a determinant of the published ratio.
              That is an ordering artefact wearing the costume of a result.
            </p>
          </section>

          <section class="mb-12">
            <h2 class="mb-4 text-2xl font-bold text-neutral-900">
              What has been ruled out
            </h2>
            <p class="mb-4 text-neutral-600">
              Each of these was eliminated by a direct test rather than by
              argument. The mechanism is still not identified, which is the
              reason nothing is published.
            </p>
            <div class="space-y-3">
              {ruledOut.map(([what, how]) => (
                <div key={what} class="rounded-lg bg-neutral-50 p-4">
                  <h3 class="mb-1 font-semibold text-neutral-900">{what}</h3>
                  <p class="text-sm text-neutral-600">{how}</p>
                </div>
              ))}
            </div>
          </section>

          <section class="mb-12">
            <h2 class="mb-4 text-2xl font-bold text-neutral-900">
              A fix that was written and thrown away
            </h2>
            <p class="text-neutral-600">
              The fixed warm-up was replaced with one that warms in rounds until
              throughput stops climbing. It changed nothing, and the round
              logging showed why: the warm-up ran 80,000 requests and its final
              round measured the same depressed figure that was then recorded.
              The system climbs more slowly than the tolerance, so the loop
              announced a plateau at exactly the value it was meant to escape.
              It was reverted rather than shipped, because a warm-up that
              reports success at the wrong number is worse than one that is
              obviously too short.
            </p>
          </section>

          <section class="mb-12">
            <h2 class="mb-4 text-2xl font-bold text-neutral-900">
              What happens next
            </h2>
            <p class="mb-4 text-neutral-600">
              The comparison will be re-run on a machine with native Docker
              rather than a virtualised one, and a measurement repeated at the
              start and the end of a run will have to agree with itself before
              anything is published. Numbers will return here when they do.
            </p>
            <p class="text-neutral-600">
              The method is unchanged and documented in{" "}
              <code class="font-mono text-sm">docs/benchmarking.md</code>, and
              the full record of the investigation is in{" "}
              <code class="font-mono text-sm">scripts/BENCH-FINDINGS.md</code>.
            </p>
          </section>

          <section class="mb-12">
            <div class="rounded-lg border border-neutral-200 bg-neutral-50 p-5">
              <h2 class="mb-2 text-lg font-bold text-neutral-900">
                Conformance is unaffected
              </h2>
              <p class="text-neutral-700">
                The conformance reports measure whether two servers give the
                same answer, not how fast they give it, and nothing about this
                touches them.{" "}
                <Link
                  href="/docs/conformance"
                  class="text-primary-600 font-medium hover:underline"
                >
                  Read the conformance reports
                </Link>
                .
              </p>
            </div>
          </section>

          <div class="flex items-center justify-between border-t border-neutral-200 pt-8">
            <Link
              href="/docs/conformance"
              class="hover:text-primary-600 flex items-center gap-2 text-neutral-600"
            >
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
                  d="M15 19l-7-7 7-7"
                />
              </svg>
              Conformance
            </Link>
            <Link
              href="/compare"
              class="hover:text-primary-600 flex items-center gap-2 text-neutral-600"
            >
              Compare
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
                  d="M9 5l7 7-7 7"
                />
              </svg>
            </Link>
          </div>
        </div>
      </div>
    </div>
  );
});

export const head: DocumentHead = {
  title: "Benchmarks - Postrust Documentation",
  links: [{ rel: "canonical", href: "https://postrust.org/docs/benchmarks" }],
  meta: [
    {
      name: "description",
      content:
        "Postrust's throughput figures are withdrawn pending re-measurement: the benchmark was found to report the order of a run as much as the speed of a server.",
    },
  ],
};
