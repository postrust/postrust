# Benchmarking

Postrust ships a benchmark harness so the numbers in the README and on the
website can be reproduced rather than taken on trust. It measures the three
things the project claims: **binary size**, **request latency/throughput**, and
**resident memory**.

## Quick start

```bash
./scripts/bench.sh
```

That command is self-contained. It builds a release binary, starts a throwaway
PostgreSQL container, loads a 100,000-row dataset, runs each scenario, and
prints a table. Everything it created is removed on exit.

Requirements:

- `docker`
- `cargo`
- `curl`
- a load generator: [`oha`](https://github.com/hatoo/oha), or `ab`. `hey` is
  **not** supported -- its text output was once parsed from memory and produced
  silently wrong numbers, so the harness only accepts generators whose output it
  has actually been checked against. `oha` is strongly preferred: it reports
  percentiles as JSON, and `ab` is HTTP/1.0-only and goes client-bound near
  7,000 rps, well below what these servers reach.

## Options

All options are environment variables:

| Variable | Default | Purpose |
|----------|---------|---------|
| `REQUESTS` | `30000` | Requests per scenario |
| `CONCURRENCY` | `50` | Concurrent connections |
| `WARMUP` | `500` | Warm-up requests before measuring |
| `REPEATS` | `3` | Measurements per scenario; the median is reported |
| `BENCH_FEATURES` | `admin-ui` | Cargo features to build. Set to `""` for a minimal build |
| `PG_IMAGE` | `postgres:16-alpine` | PostgreSQL image |
| `PG_PORT` | `55432` | Host port for the database |
| `BENCH_PORT` | `3999` | Port the server listens on |
| `SKIP_BUILD` | `0` | Reuse an existing `target/release/postrust` |
| `KEEP` | `0` | Leave the database and server running for manual poking |
| `RESULTS_DIR` | temp dir | Where results and the server log are written |
| `CPUSET_DB` | unset | Cores the database is confined to. Unset means no pinning |
| `CPUSET_SERVER` | unset | Cores the server under test is confined to. The same value for every target |
| `CPUSET_LOAD` | unset | Cores the load generator is confined to |
| `GATE_THRESHOLD_PCT` | `3` | Drift permitted between the first measurement and its repeat at the end of the run |

Examples:

```bash
# Heavier load
REQUESTS=20000 CONCURRENCY=100 ./scripts/bench.sh

# Measure a minimal build instead of the shipped feature set
BENCH_FEATURES="" ./scripts/bench.sh

# Keep everything up afterwards to run your own queries
KEEP=1 ./scripts/bench.sh
```

## The measurement host

Throughput figures are a property of the machine as much as of the software, so
the machine is part of the method rather than a footnote.

Every published figure comes from a **dedicated bare-metal host**, not a VM and
not a laptop. The requirement is not raw speed; it is that the machine can be
made to hold still. A hypervisor hides the CPU's frequency behaviour from the
guest, and a laptop's is governed by thermal policy that no benchmark can see
or control. Both produce numbers that drift under the harness -- which is
exactly the failure recorded in
[`scripts/BENCH-FINDINGS.md`](../scripts/BENCH-FINDINGS.md).

The current host:

| | |
|---|---|
| CPU | AMD EPYC 9255, 24 cores, single socket, 4 CCDs of 6 cores with 32 MB L3 each |
| Memory | 376 GB (the workload uses about 4) |
| OS | Ubuntu 24.04 LTS, Docker CE from Docker's own repository |

### Confirming it is really bare metal

Before anything is installed:

```bash
systemd-detect-virt
```

It must print `none`. A product page saying "bare metal" is not evidence; this
is. If `/sys/devices/system/cpu/cpufreq/` is also absent, the machine is a
guest whatever it was sold as, and no amount of pinning will make its numbers
reproducible.

### Holding the machine still

```bash
# Sibling threads share a core, so two pinned components can land on one core
# while appearing to have separate CPUs. Runtime change, no reboot.
echo off > /sys/devices/system/cpu/smt/control

# Ask for a fixed clock rather than an opportunistic one.
echo 0 > /sys/devices/system/cpu/cpufreq/boost
for g in /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor; do echo performance > "$g"; done
for e in /sys/devices/system/cpu/cpu*/cpufreq/energy_performance_preference; do echo performance > "$e"; done

echo never > /sys/kernel/mm/transparent_hugepage/enabled
sysctl -w vm.swappiness=1
```

**Verify that the request was honoured, because it may not be.** On this host
it is not: the platform firmware owns P-states and discards the OS's requests.
`scaling_max_freq` dutifully drops to 3200000 and the CPU keeps running at
4317 MHz. Writing the sysfs file is not evidence that anything changed --
measuring is:

```bash
# Same fixed quantity of work, timed. If a cap is in effect, this gets slower.
time taskset -c 0 awk 'BEGIN{for(i=0;i<80000000;i++) x+=i}'
```

Here, a 2.0 GHz cap, a 3.2 GHz cap and no cap at all produce the same wall
time to within noise, and `turbostat` reads 4317 MHz throughout. The clock is
fixed -- just at the firmware's chosen value rather than at base.

That it stays fixed *for the length of a run* is the part that actually matters,
so it is sampled rather than assumed. `turbostat` runs alongside the harness for
the whole run, and the samples taken while the machine was under load (843 of
them, `Busy%` above 20) span **4294-4329 MHz -- a 0.82% spread**. A clock that
ramped would show up here as a trend, and it is the same measurement that would
have exposed the drift on the previous host. That is
acceptable, because what the benchmark needs is a clock that does not *vary*
during a run; it is not the same as pinning to base clock, and the difference
is recorded here rather than papered over. A run on different firmware will
produce different absolute numbers.

### One component per CCD

On a multi-CCD AMD part, cores in different CCDs do not share an L3 slice.
Giving each component its own CCD means the database, the server under test and
the load generator cannot evict each other's cache. Ask the machine for its own
grouping rather than assuming the numbering is linear:

```bash
cat /sys/devices/system/cpu/cpu*/cache/index3/shared_cpu_list | sort -u
```

On this host, with SMT off, that yields four groups which map onto the four
roles:

| CCD | cores | |
|---|---|---|
| 0 | 0-5 | PostgreSQL |
| 1 | 6-11 | the server under test |
| 2 | 12-17 | the load generator |
| 3 | 18-23 | the harness itself, dockerd, the OS |

Passed to the harness as:

```bash
CPUSET_DB=0-5 CPUSET_SERVER=6-11 CPUSET_LOAD=12-17 \
  taskset -c 18-23 ./scripts/bench-compare.sh
```

Check it landed, while a run is in flight -- the same principle as the clock,
that asking is not the same as getting:

```bash
docker inspect -f '{{.Name}} {{.HostConfig.CpusetCpus}}' $(docker ps -q)
taskset -pc "$(pgrep -x oha)"
```

Expected: every `postrust-cmp-*` server on the same cpuset, the database on its
own, and `oha` on a third.

**Every target gets the identical `CPUSET_SERVER`.** Giving one server more
cores than another encodes the bias into the instrument, which is the single
failure a differential benchmark exists to rule out. The harness has no way to
express a per-target allocation, deliberately.

## The self-consistency gate

Repeating a measurement does not establish that it is sound. `measure_median`
runs its repeats back to back, so all of them sit in whatever state the machine
is in at that moment: they agree with each other perfectly and can be equally
wrong. That is precisely how the drift documented in `BENCH-FINDINGS.md` went
unnoticed across several rounds of published figures.

So the first measurement of a run is repeated as its **last** action, with
everything else in between:

```
Self-consistency
 the first measurement (point lookup / postrust) repeated as the last action of the run
 first 44163 rps, last 44169 rps -- drift 0.0% against a 3% threshold: pass
```

If the two disagree by more than `GATE_THRESHOLD_PCT` (default 3), the machine
did not hold still, nothing measured in between is comparable across targets,
and `bench-compare.sh` **exits non-zero**. The results file is still written --
it is needed to diagnose the drift -- but nothing downstream may treat it as
publishable.

A passing gate is a necessary condition, not a sufficient one. It proves the
machine ended the run where it started, which is what the earlier virtualised
runs could not do.

## What Docker costs

Every server runs in a container so that none of them gets a native-vs-container
advantage. That fairness is not free, and the size of the charge is measured
rather than assumed. Same binary, same database, same fixtures, same cpusets,
same generator; only the packaging changes:

| | point lookup |
|---|---|
| **A** both containers on a bridge, measured through the published port | 44,496 rps |
| **B** the same containers with `--network host` | 48,013 rps |
| **C** the native binary against a host-networked database | 48,684 rps |

- **A -> B: 7.3%.** Docker's network plumbing -- veth, the bridge, NAT.
- **B -> C: 1.4%.** Everything else a container does to the process: seccomp,
  cgroups, overlayfs. Close to nothing.
- **A -> C: 8.6%** in total, charged to every target equally, so ratios are
  unaffected and absolutes are understated by about that much.

`docker-proxy` is disabled on the measurement host:

```json
/* /etc/docker/daemon.json */
{ "userland-proxy": false }
```

This is a measurement-integrity fix rather than a performance tweak.
`docker-proxy` is a userspace relay, one process per published port, sitting in
the request path -- and it is covered by no `--cpuset-cpus`, so it lands
wherever the scheduler puts it, including on the cores the database or the
server under test are pinned to. Turning it off moves port publishing to
iptables DNAT, which is kernel-side and has no process to misplace. It recovered
3.2 points on its own (43,130 -> 44,496 rps) while leaving B and C unchanged,
which is the check that it touched only the path it was supposed to.

## Why the window is 30,000 requests

The default used to be 3,000, and that was too few to mean anything. At these
throughputs 3,000 requests complete in about a third of a second, which samples
connection setup and cache warmth rather than steady state. Repeating the
measurement does not rescue it: five medians of a biased sample share the bias.

It biased the comparison rather than merely widening it, because the servers do
not warm up at the same rate. A short window flattered this one and penalised
the GHC-based servers, which reach steady state later. Raising the window to
30,000 cut run-to-run spread from 1.33x to 1.25x here and 1.39x to 1.17x for
PostGraphile -- and moved several published ratios down by more than a factor
of two.

The harness also pauses every server it is not currently measuring
(`docker pause`, so there is no cold start and no lost connection pool). Four
servers competing for the same cores made the measurement a report on
scheduling.

## What it measures

The dataset is defined in [`scripts/bench-fixtures.sql`](../scripts/bench-fixtures.sql):
a single `bench_items` table of 100,000 rows with integer, numeric, text,
boolean, `jsonb` and timestamp columns, indexed on `category` and `price`.

| Scenario | Request |
|----------|---------|
| point lookup | `/api/bench_items?id=eq.42` |
| 25-row page | `/api/bench_items?select=id,name,price&limit=25` |
| filtered + ordered page | `/api/bench_items?category=eq.cat-5&order=id.desc&select=id,name&limit=25` |
| page with exact count | same, with `Prefer: count=exact` |
| range filter on numeric | `/api/bench_items?price=gt.50&select=id,price&limit=25` |

Each scenario is checked with a single request first, and skipped with a warning
if it does not return `200`/`206`. This matters more than it sounds: load
generators report an error path as a perfectly healthy, very fast result. `ab`
in particular prints `Failed requests: 0` for non-2xx responses when the error
bodies are all the same length, so the harness inspects `Non-2xx responses`
separately and reports `ERROR` rather than a throughput figure.

Memory is the server process's RSS, sampled before any request is served
(`memory (idle)`) and after each scenario.

## Reference results

Measured on the pinned bare-metal host described above, with `oha`. The server
runs natively here; the database is a container pinned to its own CCD.

```
 host           : Linux 7.0.0-30-generic x86_64
 cpu            : AMD EPYC 9255 24-Core Processor
 pinning        : db=0-5 server=6-11 load=12-17
 postgres       : postgres:16
 load generator : oha (n=30000, c=50)
 dataset        : bench_items, 100000 rows

 binary         : 8632680 bytes (8.23 MiB), stripped: yes
 features       : admin-ui
 memory (idle)  : 10.5 MB
 memory (final) : 20.5 MB

 scenario                         req/s   p50 ms   p95 ms   p99 ms        RSS
 ---------------------------- --------- -------- -------- -------- ----------
 point lookup (id=eq.N)           46596      1.0      1.1      1.9    19.4 MB
 25-row page                      36711      1.3      1.4      1.5    19.6 MB
 filtered + ordered page          34153      1.4      1.5      1.5    20.4 MB
 page with exact count             2275     21.5     24.7     26.5    20.5 MB
 range filter on numeric          31890      1.5      1.6      1.7    20.5 MB
```

Two things in that table are worth reading rather than skimming.

**`page with exact count` is 15-20x slower than everything else, and that is
correct.** `Prefer: count=exact` asks PostgreSQL to count every matching row
before the page can be returned, so the request does work proportional to the
table rather than to the page. It is in the harness precisely so that cost stays
visible; the cheaper `count=planned` and `count=estimated` do not pay it.

**These are higher than the containerised figures on the comparison pages**, and
by about the margin measured under "What Docker costs" -- 46596 here against
44490 for the same scenario through a published port. Same binary, same
database, same cores.

### Binary size

Size depends on the feature set, so quoting one number without the other is
misleading:

| Build | Bytes | Size |
|-------|-------|------|
| `cargo build --release -p postrust-server` | 4,887,528 | 4.66 MiB |
| `cargo build --release -p postrust-server --features admin-ui` | 8,632,680 | 8.23 MiB |

Both measured on Linux x86_64 with the release profile, which is what the
published Docker image and release binaries are. **~8 MB is the number that
describes what you actually download**, because those artefacts are built with
`admin-ui` (Swagger UI, Scalar, OpenAPI, GraphQL playground). A minimal build of
just the REST and GraphQL API is ~4.7 MB.

An arm64 macOS build is materially smaller -- 5,220,864 bytes with `admin-ui` --
so a size quoted without its target is not a fact about the project. The
container images differ again, and are reported per variant on the comparison
pages: 34.9 MB for the alpine image against 149 MB for the debian one.

## Caveats

Read these before quoting any throughput number:

- **Everything shares one machine.** PostgreSQL, Postrust and the load
  generator run on one host. They are given separate cores and separate L3
  slices, so they do not compete directly, but they do share memory bandwidth
  and one kernel. The numbers are useful for comparing the tools against each
  other, not for capacity planning.
- **Unpinned, this is largely a measurement of the scheduler.** The same point
  lookup that reports p50 ~1.1 ms on the pinned bare-metal host reported ~8 ms
  on an unpinned laptop, and 6,065 rps against 46,596. Numbers from the two
  setups are not comparable in either direction, and only pinned bare-metal
  numbers are published.
- **`ab` is the weakest of the three load generators.** It is HTTP/1.0 only and
  its percentiles are coarse. Install `oha` for numbers worth comparing between
  runs.
- **Cold start is not measured here.** The ~50 ms Lambda figure comes from a
  different environment and is not produced by this harness.
- **`bench.sh` measures this server alone.** PostgREST, Hasura and PostGraphile
  are measured by [`scripts/bench-compare.sh`](../scripts/bench-compare.sh),
  which runs all four as containers on one network against one database. No
  figure on the website is taken from another project's own documentation.
- **The absolute numbers carry the host's firmware with them.** This machine's
  clock is held at 4317 MHz by its BIOS, not at the 3.2 GHz base. A different
  machine, or the same machine with a different system profile, will produce
  different absolutes. The ratios are the durable part.

## Correctness first

A benchmark that measures a broken endpoint is worse than no benchmark. The
query-parameter behaviour the scenarios depend on — `select`, `limit`, `offset`,
`order`, the `Range` header, and filters against non-text columns — is covered
by integration tests that run against a real database:

```bash
DATABASE_URL="postgres://postgres:postgres@localhost:5432/postrust_test" \
  cargo test -p postrust-server --test query_params -- --ignored
```

See [`crates/postrust-server/tests/query_params.rs`](../crates/postrust-server/tests/query_params.rs).
Those tests exist because all three of these once shipped broken: `limit` and
`offset` were parsed and then ignored (so every request returned the whole
table), filters on integer columns failed with `operator does not exist:
integer = text`, and negated filters such as `id=not.eq.4` generated invalid
SQL.
