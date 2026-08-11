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
- a load generator: [`oha`](https://github.com/hatoo/oha) (best percentiles),
  `hey`, or `ab` (ships with macOS)

## Options

All options are environment variables:

| Variable | Default | Purpose |
|----------|---------|---------|
| `REQUESTS` | `3000` | Requests per scenario |
| `CONCURRENCY` | `50` | Concurrent connections |
| `WARMUP` | `200` | Warm-up requests before measuring |
| `BENCH_FEATURES` | `admin-ui` | Cargo features to build. Set to `""` for a minimal build |
| `PG_IMAGE` | `postgres:16-alpine` | PostgreSQL image |
| `PG_PORT` | `55432` | Host port for the database |
| `BENCH_PORT` | `3999` | Port the server listens on |
| `SKIP_BUILD` | `0` | Reuse an existing `target/release/postrust` |
| `KEEP` | `0` | Leave the database and server running for manual poking |
| `RESULTS_DIR` | temp dir | Where results and the server log are written |

Examples:

```bash
# Heavier load
REQUESTS=20000 CONCURRENCY=100 ./scripts/bench.sh

# Measure a minimal build instead of the shipped feature set
BENCH_FEATURES="" ./scripts/bench.sh

# Keep everything up afterwards to run your own queries
KEEP=1 ./scripts/bench.sh
```

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

Measured with the harness at its defaults on an Apple M-series laptop. Treat
these as a shape to compare against, not a target — see the caveats below.

```
 host           : Darwin 25.2.0 arm64
 postgres       : postgres:18-alpine
 load generator : ab (n=3000, c=50)
 dataset        : bench_items, 100000 rows

 binary         : 5220864 bytes (4.98 MiB), stripped: yes
 features       : admin-ui
 memory (idle)  : 10.0 MB
 memory (final) : 13.3 MB

 scenario                         req/s   p50 ms   p95 ms   p99 ms        RSS
 ---------------------------- --------- -------- -------- -------- ----------
 point lookup (id=eq.N)            6065      8.0     10.0     22.0    14.1 MB
 25-row page                       6065      8.0     11.0     13.0    14.4 MB
 filtered + ordered page           5025      9.0     14.0     23.0    14.6 MB
 page with exact count             6373      7.0     11.0     19.0    14.7 MB
 range filter on numeric           6468      7.0     10.0     14.0    13.3 MB
```

### Binary size

Size depends on the feature set, so quoting one number without the other is
misleading:

| Build | Bytes | Size |
|-------|-------|------|
| `cargo build --release -p postrust-server` | 3,070,080 | 2.93 MiB |
| `cargo build --release -p postrust-server --features admin-ui` | 5,220,864 | 4.98 MiB |

The published Docker image and release binaries are built with `admin-ui`
(Swagger UI, Scalar, OpenAPI, GraphQL playground), so **~5 MB is the number
that describes what you actually download**. A minimal build of just the REST
and GraphQL API is ~3 MB. Both are measured on macOS arm64; a Linux x86_64
build differs somewhat.

## Caveats

Read these before quoting any throughput number:

- **Everything shares one machine.** PostgreSQL, Postrust and the load
  generator compete for the same cores over loopback. The numbers are useful
  for comparing Postrust against itself across changes, not for capacity
  planning.
- **Latency here is dominated by contention, not by Postrust.** At concurrency
  50 on a laptop, p50 sits around 8 ms; the same request against an idle server
  is roughly an order of magnitude faster.
- **`ab` is the weakest of the three load generators.** It is HTTP/1.0 only and
  its percentiles are coarse. Install `oha` for numbers worth comparing between
  runs.
- **Cold start is not measured here.** The ~50 ms Lambda figure comes from a
  different environment and is not produced by this harness.
- **PostgREST is not measured here.** Comparison-table figures for other
  projects come from their own documentation.

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
