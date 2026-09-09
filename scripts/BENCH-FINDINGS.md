# Benchmark findings

What the comparison benchmark has been measuring, and what had to change before
its numbers could be published. Newest first.

## The drift does not reproduce on bare metal, and the gate now proves it

**Status: resolved by changing the host. Numbers published.**

The open item below -- throughput depending on when in a run it was sampled --
does not occur on a dedicated bare-metal host with each component pinned to its
own CCD. The same measurement, taken as the first and then as the last action
of a full four-server run:

| run | first | last | drift |
|---|---|---|---|
| debian 1 | 44163 rps | 44169 rps | 0.0% |
| debian 2 | 44490 rps | 44559 rps | 0.2% |
| debian 3 | 44186 rps | 44132 rps | 0.1% |
| alpine | 43727 rps | 43179 rps | 1.3% |

Those are within-run figures. The same measurement *across* the three debian
runs read 44163, 44490 and 44186 -- a 0.7% spread between independent runs of
the whole harness, against the 1.25x-1.39x run-to-run spread the previous host
produced.

That comparison is now a permanent part of the harness rather than something
done by hand. `bench-compare.sh` repeats its first measurement at the end of
the run and exits non-zero if the two disagree by more than
`GATE_THRESHOLD_PCT` (default 3). The results file is still written, because it
is needed to diagnose a failure, but a drifted run cannot be mistaken for a
publishable one.

This gate exists because `REPEATS` could never have caught the original
problem. `measure_median` runs its repeats back to back, so all of them sit in
the same state and agree with each other while being equally wrong -- the
failure mode the note further down describes as "a systematic bias reproduces
perfectly and reads as precision".

Throughput on the new host is roughly four times what the virtualised host
reported (44163 rps against 6212-13000 for the same scenario). That is not a
change in the software. Nothing in this repository was made faster; the
instrument stopped losing most of the signal.

The clock was sampled for the length of the run rather than checked before it.
Across 843 `turbostat` samples taken while the machine was loaded, `Bzy_MHz`
spanned 4294-4329 -- a 0.82% spread. A ramping clock, which is the leading
hypothesis for the old drift, would have shown as a trend across those samples.

### What actually changed

- Dedicated bare metal (AMD EPYC 9255, 24 cores, single socket), confirmed with
  `systemd-detect-virt` returning `none`, rather than Docker Desktop on WSL2.
- SMT off, so two components pinned to "different CPUs" cannot share a core.
- One CCD per component -- database on 0-5, server under test on 6-11, load
  generator on 12-17, everything else on 18-23 -- so no two of them share a
  32 MB L3 slice. Every target gets the identical `CPUSET_SERVER`.
- `docker-proxy` disabled, removing an unpinned userspace process from the
  request path.

## Where the 4x against PostgREST actually comes from

**Status: investigated after the ratio was questioned. The number stands; the
reading of it needed qualifying, and the harness needed a check it lacked.**

A point lookup measures 44186 rps here against 10931 for PostgREST. That gap is
large enough to deserve an explanation before it is published, so three ways it
could have been an artefact were tested directly.

**It is not a cache.** `pg_stat_statements` records exactly 3000 calls for 3000
requests, for both servers. Every request reaches PostgreSQL.

**It is not a connection pool difference.** postrust holds 10 client backends,
PostgREST 11.

**It is not unequal output.** Both return the same rows, both send
`Content-Range: 0-0/*`, and the bodies are 176 and 177 bytes. postrust's is the
*larger* of the two, because `arbitrary_precision` preserves what PostgreSQL
sent (`0.1000`) where PostgREST trims it (`0.10`).

What differs is the SQL. postrust sends:

```sql
SELECT "id", "name", "category", "price", "stock", "is_active", "metadata",
       "created_at"
FROM "public"."bench_items" WHERE "id" = $1::int4
```

PostgREST sends:

```sql
WITH pgrst_source AS (
  SELECT "public"."bench_items".* FROM "public"."bench_items"
  WHERE "public"."bench_items"."id" = $1
)
SELECT $2::bigint AS total_result_set,
       pg_catalog.count(_postgrest_t) AS page_total,
       coalesce(json_agg(_postgrest_t), $3) AS body,
       nullif(current_setting($4, $5), $6) AS response_headers,
       nullif(current_setting($7, $8), $9) AS response_status,
       $10 AS response_inserted
FROM (SELECT * FROM pgrst_source) _postgrest_t
```

PostgREST builds the JSON *inside PostgreSQL* with `json_agg`, computes a
`count()`, and reads three GUCs -- on every request, whether or not the client
asked for a count or the function set a response header. postrust sends a plain
parameterised select and builds the JSON in Rust.

So a large part of the difference is **where each design does the work**, and
PostgREST paying per-request for generality this benchmark does not use. That
is a real architectural difference with real consequences, and it is a fair
thing to measure. What it is not is evidence that this server's HTTP layer is
four times faster than PostgREST's, and the figure should not be read that way.

Both components are pinned to six cores each, so the two designs load the two
pinned halves differently. A machine where the database had more headroom than
the server would narrow this gap; one where it had less would widen it.

### The harness was not checking that the question was the same

Until this, `rest_ok` checked for an HTTP 200 and `gql_ok` for a `data` key.
Neither compared what came back. One server answering with one row while
another returned twenty-five would have passed both and been measured as though
the work were equal.

`check_rest_equivalence` and `check_gql_equivalence` now run before any
scenario is measured. Row counts must match -- a difference there is never a
dialect difference, so it fails the run. REST column sets must match too. Bytes
are deliberately not compared, because `0.1000` against `0.10` is a real
difference that does not make the work unequal, and GraphQL field names are not
compared because PostGraphile genuinely calls `id` `rowId`.

The first thing the check did was fail on a difference that was not one.
PostGraphile answers a lookup by key with a single *object*, where Postrust and
Hasura answer with an array of one, and the extractor looked for the first
array in the document -- so it read PostGraphile's response as no rows at all
and called the run invalid. Both servers had returned exactly one row. The
extractor now treats a lone object under `data`, or a lone top-level object, as
one row, and is exercised against all seven shapes the three dialects produce.

Worth stating plainly: a check that fails a good run is not obviously better
than no check. This one earned its place only after it stopped doing that.

### What a short window would have published instead

A deliberately bad run -- 2,000 requests, one repeat -- was used to exercise the
new check. It passed equivalence, and the self-consistency gate rejected the run
at 11.4% drift, which is what it is for. The numbers it would have produced:

| | 2,000-request window | 30,000-request window |
|---|---|---|
| postrust, point lookup | 35017 rps | 44186 rps |
| postgrest, point lookup | 2667 rps | 10931 rps |
| **ratio** | **13.1x** | **4.0x** |

The short window understates both servers and understates the GHC-based one far
more, exactly as the note further down describes. A 13x figure was available
from this harness, from a run that looked healthy, and the gate is the only
thing between it and a website.

## The alpine variant moves the other tools, and it should not

**Status: open. One run each. No alpine throughput figure is published.**

`VARIANT=alpine` is meant to answer "how big is the alpine image, and does the
musl build cost anything". It changes more than that: the PostgreSQL image
(`postgres:16` to `postgres:16.11-alpine`) and the PostGraphile base
(`node:22` to `node:22-alpine`) as well as the Postrust image. PostgREST and
Hasura run **byte-identical images in both variants** -- their image tags are
not variant-dependent.

Yet, alpine against debian:

| target | image changed? | change |
|---|---|---|
| postrust | yes (musl) | +2% to +33% |
| hasura | **no** | +1.4% to +2.2% |
| postgraphile | yes (musl node) | -15% to -17% |
| postgrest | **no** | **-27% to -38%** |

postrust improving and PostGraphile losing ~15% are both readable -- a musl
build is a real change. PostgREST is not. Its container is identical, so the
only thing that moved underneath it is the database image, and Hasura saw that
same database move and stayed flat.

Candidate explanations, none established:

- PostgREST noise. Its measured run-to-run spread is already 19.6% (below), the
  widest of the four. -37% is more than that, but not a different universe.
- The alpine PostgreSQL penalising PostgREST's query shapes specifically.
  Hasura's flatness argues against a general database slowdown, but the two
  tools are measured on different surfaces with different queries.

Separating those needs a second alpine run, which has not been done. Until it
is, alpine is used **only** for image size and memory, which is what it was
added for. `gen-measured.mjs` takes request figures from the first results file
only, so no alpine throughput number reaches the website; the guard is
structural rather than a matter of remembering.

## PostgREST's own run-to-run spread is an order of magnitude wider than the others'

**Status: measured across two full runs. A caveat on the PostgREST ratios, not
a fault in the harness.**

Three complete debian runs, same host, same pinning, same configuration. Spread
between the highest and lowest reading of each scenario:

| target | worst scenario | mean |
|---|---|---|
| postrust | 0.8% | 0.5% |
| postgraphile | 3.0% | 1.8% |
| hasura | 3.6% | 2.6% |
| **postgrest** | **19.6%** | **6.8%** |

Two PostgREST scenarios account for it. Point lookup read 9136, 10747, 10931 --
the first run is the outlier and the second and third agree to 1.7%, so that one
looks like a single bad sample rather than continuing instability. Range filter
read 6526, 7140, 6890: 9.4% with no outlier to blame. The remaining three
scenarios are inside 3%.

This is the tool, not the machine. postrust was measured seconds either side of
those same PostgREST samples and held to 0.7%, and the self-consistency gate
passed on both runs. A machine that drifted would have moved everything.

Two consequences, both stated wherever PostgREST figures appear:

- A single run establishes the PostgREST ratios to roughly +/-10%, where
  postrust reaches 1% and the two GraphQL servers 3%. The *direction* of the
  differences is far outside that margin -- postrust leads PostgREST by 3x to
  9x depending on scenario -- but the precise multiples are not, and are not
  quoted to more precision than they carry.
- **The gate does not cover this.** It repeats the first measurement of the run,
  which is one scenario on one target, and so it answers "did the machine hold
  still" -- not "is each target individually reproducible". Extending it to
  re-measure every target at the end would turn this into something the harness
  reports on every run rather than something noticed by comparing two of them.
  That change is not made here, because it has not been exercised.

## The platform firmware ignores every OS frequency request

**Status: understood, worked around by measuring rather than assuming.**

The plan was to pin the CPU to its base clock, on the reasoning that base is
the only clock a vendor guarantees and therefore the only one that reproduces
elsewhere. On this host that is not possible: the BIOS owns P-states and
discards what the OS asks for.

The kernel accepts the request and reports success. `scaling_max_freq` drops to
`3200000` and `cpuinfo_max_freq` follows. The hardware carries on at 4317 MHz.

Caught by timing a fixed quantity of work rather than by reading a sysfs file
back:

| cap requested | `scaling_max_freq` | wall time |
|---|---|---|
| none (boost on) | 4317461 | 1965 / 1904 / 1903 ms |
| base, boost off | 3200000 | 1912 / 1912 / 1940 ms |
| 2.0 GHz | 2000000 | 1938 / 1956 ms |

Identical to within noise across a 2.2x range of requested caps, and
`turbostat` -- which reads APERF/MPERF, the hardware's own account -- reports
4317 MHz throughout.

The consequence is benign but has to be stated: the clock **is** fixed, which is
what the benchmark actually needs, but it is fixed at the firmware's chosen
value and not at base. Absolute figures therefore carry this machine's BIOS
profile with them and will not reproduce on different firmware. The ratios
between the four servers do not depend on it.

The general lesson is that writing a sysfs file is not evidence that anything
changed. Every control in `docs/benchmarking.md` is now paired with a
measurement that would fail if the control did not take.

## Docker costs 8.6%, and almost all of it is the network path

**Status: measured. Kept, because it is charged to every target equally.**

Running every server in a container is deliberate -- it stops any one of them
getting a native-vs-container advantage -- but the size of that charge had
never been established. Same binary, same database, same fixtures, same
cpusets, same generator, only the packaging changing:

| | point lookup |
|---|---|
| A both containers on a bridge, via the published port | 44496 rps |
| B the same containers with `--network host` | 48013 rps |
| C the native binary, host-networked database | 48684 rps |

- A -> B **7.3%**: veth, the bridge, NAT.
- B -> C **1.4%**: seccomp, cgroups, overlayfs. Essentially nothing.
- A -> C **8.6%** total.

So the container is nearly free and the *network plumbing* is not. Ratios are
unaffected; absolute figures are understated by about 8.6% against the same
binary run natively, and that is now stated wherever they are published.

Disabling `docker-proxy` (`"userland-proxy": false`) was part of this work and
is a measurement-integrity fix rather than a tuning choice: it is a userspace
relay, one process per published port, in the request path, and covered by no
`--cpuset-cpus` -- so it lands wherever the scheduler puts it, including on the
cores pinned to the database or the server under test. Removing it recovered
3.2 points (43130 -> 44496 rps) and left B and C unchanged, which is the check
that it touched only the path it was meant to.

## The measured throughput depends on when in a run it is taken

**Status: superseded. Was: reproduced on the virtualised host, mechanism never
identified. Does not reproduce on bare metal -- see the entry at the top. The
mechanism is still unidentified; it was escaped rather than explained, and the
gate is what stops it returning unnoticed. The investigation is kept because
the things it ruled out are still ruled out.**

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
