# Benchmark findings

What the comparison benchmark has been measuring, and why its numbers are not
currently publishable. Newest first.

## The measured throughput depends on when in a run it is taken

**Status: reproduced, mechanism not identified. Numbers withheld.**

The same server, in the same container, measured seconds apart, differs by
about a factor of two:

| | point lookup, postrust, alpine |
|---|---|
| reported by the harness | 5911 - 9731 rps |
| measured by hand, same containers, seconds later | ~13000 rps |

It is not the harness misreporting. `run_oha` was instrumented to print its own
arguments: `-n 30000 -c 50`, correct URL, and the reported rate matches
wall-clock (30000 / 5911 = 5.08s, and the next invocation begins 5.34s later).
The server really is slower while the harness runs.

Within a single run, later scenarios measure higher than earlier ones. In one
run the REST scenarios warmed to a plateau of 6212 rps while the GraphQL
scenarios, which run afterwards, warmed to 12631 -- same server, same process,
same run. Whatever is measured first is the most understated, which makes
scenario and target order a determinant of the published ratios.

### Ruled out, each by direct test

- **Thermal throttling and measurement order.** An ABBA design measuring each
  server both first and second: postrust 12533/13025 first, 12443/13485
  second. No advantage follows the slot.
- **Host load.** The harness scored *higher* at load average 18 (9731) than at
  load 5 (6291), while hand measurement held ~13000 at both.
- **Cold database.** The tables are 16 MB + 33 MB against 128 MB of
  `shared_buffers`, the fixtures `ANALYZE`, and the harness pre-warms with
  `count(*)`.
- **The bulk load and checkpointing.** Reloading 400k rows and forcing a
  `CHECKPOINT` moved nothing: ~13000 before, after, and after the checkpoint.
- **Server start-up.** A restarted server is at 12654 on its first sample.
- **PostgreSQL start-up.** After restarting Postgres, 11657 then ~13000.
- **`docker pause` and the other containers.** A single-target run, with
  nothing to pause, is equally depressed (6506).
- **`docker stats`.** Called once before and once after all measurements, never
  between them.
- **The harness's own measurement code.** Calling its `run_oha` directly on a
  settled stack returns 12981, and replicating its full sequence by hand
  (health check, warm both targets, isolate, measure) returns 12925.
- **Docker Desktop port forwarding.** Same generator over both paths: from the
  host 7226/8130/6711, from inside the network 6508/6070/7179. (`ab` is
  client-bound near 7000 here, so this rules the path out without speaking to
  the plateau `oha` reaches.)

### Warm-to-plateau was tried and does not fix it

Replacing the fixed 500-request warm-up with rounds that stop when a round
fails to beat the previous by 3% changed nothing: 6899, against 6890 before.
The round logging showed why -- warm-up ran 4 rounds (80,000 requests) and its
final round measured 6212, matching the 6213 that was then recorded. The system
climbs more slowly than 3% per round, so the loop declares a plateau at exactly
the depressed value it was meant to escape. The change was reverted rather than
shipped, because a warm-up that reports success at the wrong number is worse
than an obviously too-short one.

### Consequence

Every performance figure this repository has published was produced with the
500-request warm-up, inside this effect. They are not trustworthy at the
absolute level, and the ratios inherit it, because there is no reason to expect
four different servers to ramp at the same rate.

Before publishing again: run the comparison on a machine with native Docker
rather than a virtualised one, and confirm that a measurement repeated at the
start and the end of a run agrees with itself.

## The window was too short, and it was not symmetric noise

The default was 3,000 requests per scenario, which completes in about a third
of a second and samples connection setup and cache warmth rather than steady
state. It did not merely add noise: the GHC-based servers reach steady state
later, so a short window flattered this server and penalised them. Raising the
window to 30,000 cut run-to-run spread from 1.33x to 1.25x here and 1.39x to
1.17x for PostGraphile, and moved published ratios down by more than a factor
of two -- point lookup against PostgREST went from 3.35x to 1.11x.

Two passes agreeing is not evidence that the instrument is sound. Both passes
run the same procedure, so a systematic bias reproduces perfectly and reads as
precision.

## The benchmark measured a different Hasura than conformance did

The benchmark pinned `hasura/graphql-engine:v2.44.0` while the conformance
harness measured v2.50.1, so two numbers about "Hasura" on the same website
were not about the same Hasura. Now aligned on v2.50.1. Measured across four
runs, v2.50.1 is about 7% slower than v2.44.0 on these scenarios (single row
by pk 4441 -> 4090), so aligning the version moves this server's ratios up --
a change that comes from the newer reference engine, not from any change here.
