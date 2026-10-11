# Cloudflare Workers

`postrust-worker` runs Postrust's REST API in a Cloudflare Worker. It reaches
PostgreSQL through [Hyperdrive](https://developers.cloudflare.com/hyperdrive/).

- **REST only.** The PostgREST dialect, served at the root (`/todos`,
  `/rpc/whoami`), as PostgREST serves it. GraphQL is not available on Workers.
- **The same request pipeline as the server.** It is `postrust-engine`, the
  code the server runs, unchanged. A different database driver underneath it
  speaks the PostgreSQL wire protocol over a Workers socket.
- **The same security model.** Every request runs in a transaction that
  switches to the token's role and sets its claims before anything else. The
  login role should be a `NOINHERIT` authenticator, as PostgREST recommends,
  so row-level security applies exactly as it does on the server.
- **Unstable.** The crate is on its own `0.x` line with no stability promise
  yet. See [stability.md](stability.md).

## How it is tested

These are the tests that run on every change:

- `scripts/serverless/worker.sh` builds the Worker with `worker-build` and
  serves it with `wrangler dev`, which runs workerd, the Workers runtime. Its
  Hyperdrive binding points at a PostgreSQL with row-level security and a
  `NOINHERIT` authenticator, and the script checks:
  - per-user rows, and refused writes;
  - a function seeing the token's role and claims;
  - counts, singular responses, CORS and `OPTIONS`;
  - schema reloads by secret and by cron;
  - a fresh isolate reading the schema cache from KV, after a restart;
  - the table state afterwards.

  CI runs it beside the Lambda test (`.github/workflows/serverless.yml`).
- `CONFORMANCE_WORKER_BUILD=... scripts/conformance/conformance.sh` replays
  PostgREST's own test suite against the Worker in workerd. Results are in
  [postgrest-conformance.md](postgrest-conformance.md).
- The Worker has been measured on Cloudflare, through a real Hyperdrive; see
  [What it costs](#what-it-costs).
- `crates/postrust-server/tests/drivers_agree.rs` runs the same statements
  through both drivers, the server's sqlx and the Worker's socket driver. It
  requires identical answers: every column type, parameter typing, and
  errors down to the SQLSTATE, detail and constraint. It also holds
  statements sent together to what they would do one at a time, and counts
  the socket driver's flights: one for a read, two for a write.

## Deploying

You need a Cloudflare account with Workers and Hyperdrive, Rust with the
`wasm32-unknown-unknown` target, and Node for `wrangler`.

### 1. The database role

The Worker connects as one role and becomes the role each request's token
names. Create a login role that holds nothing itself:

```sql
CREATE ROLE authenticator LOGIN NOINHERIT PASSWORD '...';
GRANT web_anon, web_user TO authenticator;   -- the roles tokens may name
```

### 2. Hyperdrive and KV

```bash
npx wrangler hyperdrive create postrust \
  --connection-string="postgres://authenticator:...@db.example.com:5432/app"
npx wrangler kv namespace create POSTRUST_SCHEMA_CACHE
```

### 3. Configuration

Start from `crates/postrust-worker/wrangler.example.toml`, filling in the two
IDs the commands above printed:

```toml
name = "postrust"
main = "build/worker/shim.mjs"
compatibility_date = "2025-09-01"

[build]
command = "cargo install -q worker-build --version 0.8.6 --locked && worker-build --release"

[[hyperdrive]]
binding = "HYPERDRIVE"
id = "<hyperdrive id>"

[[kv_namespaces]]
binding = "POSTRUST_SCHEMA_CACHE"
id = "<namespace id>"

[triggers]
crons = ["*/15 * * * *"]   # reload the schema cache

# Run the Worker near the database rather than near each client: a request
# waits on the database several times and on the client once. See "What it
# costs" below. Or mode = "smart", for Cloudflare to choose. Region hints
# need a recent wrangler: 4.40 refuses them.
# [placement]
# region = "aws:us-east-1"

[vars]
PGRST_DB_SCHEMAS = "public"
PGRST_DB_ANON_ROLE = "web_anon"
```

Put secrets in with `wrangler secret put`, not in the file:

```bash
npx wrangler secret put PGRST_JWT_SECRET
npx wrangler secret put POSTRUST_RELOAD_SECRET   # optional; enables POST /_/reload
```

### 4. Deploy

From `crates/postrust-worker` in a checkout of the release you want:

```bash
npx wrangler deploy
```

## Configuration

The Worker reads the server's own variables, set as Worker variables or
secrets: `PGRST_DB_SCHEMAS`, `PGRST_DB_ANON_ROLE`, `PGRST_JWT_SECRET`,
`PGRST_JWT_AUD`, `PGRST_JWT_ROLE_CLAIM_KEY`, `PGRST_DB_EXTRA_SEARCH_PATH`,
`PGRST_DB_PRE_REQUEST`, `PGRST_APP_SETTINGS_*`, `PGRST_ROLE_SETTINGS`,
`PGRST_SERVER_CORS_ORIGINS` and the rest. See
[configuration.md](configuration.md). Settings that only make sense for a
long-running server are ignored, such as the pool size, the listen address and
`PGRST_DB_CHANNEL`.

Four settings are the Worker's own:

| Setting | Default | What it does |
|---|---|---|
| `PGRST_DB_MAX_ROWS` | `1000` | As on the server, but with a ceiling by default: an isolate has 128 MB. `unlimited` lifts it. |
| `PGRST_WORKER_SCHEMA_TTL` | `60` | How many seconds an isolate trusts its copy of the schema cache before checking KV for a newer one. |
| `POSTRUST_RELOAD_SECRET` | unset | A secret. When set, `POST /_/reload` with `Authorization: Bearer <secret>` reloads the schema cache. When unset, the endpoint does not exist. |
| `compat-key-order` (Cargo feature) | off | Object keys in select order, as PostgREST returns them; build with `worker-build --release --features compat-key-order`. |

## The schema cache

The server reads the catalog once at start-up. A Worker has no start-up worth
the name: isolates come and go. So the cache is kept in Workers KV, under a
key that names the exposed schemas and the build that wrote it, and handled
like this:

- **A request** uses its isolate's copy while it is younger than
  `PGRST_WORKER_SCHEMA_TTL`, then looks in KV for a newer one.
- **A cold isolate** reads KV. Only when KV has nothing does it load from the
  catalog, and then it writes what it loaded to KV for every other isolate.
- **After a migration**, reload with `POST /_/reload` or wait for the cron
  trigger. Both load from the catalog and write KV, and isolates pick the new
  cache up within the TTL.

Without a KV binding everything still works, but each new isolate loads the
catalog itself.

## What it costs

### On Cloudflare, through a real Hyperdrive

Measured on 2026-10-11 on a Workers account, with Hyperdrive's query caching
off so that every request reached the database. The client was a machine in
Singapore. The times are each request's wall time inside the Worker, as
`wrangler tail` reports it, so the client's own distance to Cloudflare is not
in them. They are medians of 64-150 requests, a point lookup
(`GET /bench_items?id=eq.42`) and a one-row insert (`POST /bench_writes`) on
the benchmark dataset:

| Where the Worker ran, and the database | Read | Write |
|---|---|---|
| Singapore, database in Singapore | 41 ms | 45 ms |
| Singapore, database in Delhi (80 ms away) | 255 ms | 318 ms |
| Same database, Worker placed in Delhi (`[placement]`) | 170 ms | 190 ms |
| London, database in Delhi | 428 ms | 574 ms |
| Amsterdam, database in Delhi | 586 ms | 813 ms |

- **The distance to the database is the cost.** Connecting to Hyperdrive takes
  3-6 ms, because Hyperdrive answers the connection near the Worker. Each time
  the Worker waits on a statement, the wait is a round trip to the database:
  about 75-80 ms from Singapore to Delhi.
- **Put the Worker near the database when it is far from your users.** A
  request makes several round trips to the database and one to the client, so
  moving the Worker next to the database is worth more than keeping it next to
  the client. `[placement] region = "..."` in `wrangler.toml` does that, or
  `mode = "smart"` for Cloudflare to decide. In the table, placing the Worker
  in Delhi cut a read from Singapore from 255 ms to 170 ms.
- **CPU is 4-11 ms a request**, at the median. Hyperdrive does not ask the
  Worker for SCRAM: the password exchange that costs about 43 ms of CPU in
  local workerd does not happen against Hyperdrive. Workers Free allows 10 ms
  of CPU a request, so a production deployment belongs on Workers Paid.
- **The bundle** is 2.7 MB of wasm, 0.94 MB gzipped, within the free plan's
  3 MB.

### Statement pipelining

A request sends its statements in as few flights as it can, rather than one
at a time. A read sends `BEGIN`, `SET TRANSACTION READ ONLY`, its settings,
the query and `COMMIT` in one flight. A write or a function call sends
`BEGIN`, the settings, the statement and the read-back of its response
settings in one flight, and `COMMIT` in a second, once the response is known
to be one it can send. Each statement still has its own `Sync`: when one
fails, PostgreSQL refuses the rest of the transaction, and the error reported
is the first one, as if they had been sent one at a time.

Before and after, on Cloudflare, by where the Worker ran (the database in
Delhi unless noted):

| | Read, before | Read, after | Write, before | Write, after |
|---|---|---|---|---|
| Singapore, database in Singapore | 69 ms | 48 ms | 60 ms | 53 ms |
| Singapore | 380 ms | 240 ms | 424 ms | 306 ms |
| London | 869 ms | 428 ms | 836 ms | 574 ms |
| Amsterdam | 1283 ms | 586 ms | 1266 ms | 813 ms |

Hyperdrive does not pass a pipeline on to the database whole. From Singapore,
five statements sent together took about 218 ms against 78 ms for one: about
three round trips, not one. The saving is still a third to a half of every
request whose database is in another region.

With the round trip set exactly, by delaying every packet the database sends
(`tc netem`, local workerd, same machine and database for all three):

| Added round trip | Read, before | Read, after | Write, before | Write, after | Server (sqlx) |
|---|---|---|---|---|---|
| 0 ms | 11.5 ms | 10.4 ms | 11.5 ms | 10.7 ms | 1.0 ms |
| 5 ms | 49 ms | 26 ms | 50 ms | 32 ms | 34 ms |
| 20 ms | 155 ms | 72 ms | 156 ms | 93 ms | 124 ms |
| 60 ms | 436 ms | 192 ms | 436 ms | 253 ms | 364 ms |

Over a plain connection the pipeline is whole: a read is three round trips
(two to connect, and the one flight) where it was seven.
Past a few milliseconds of distance the Worker answers sooner than the server
does, because sqlx waits on every statement.

The native server is unaffected by any of this. Its throughput was measured
against 2.2.1 before and after the change and is the same, within the noise
of the host (see the CHANGELOG).

### Cold starts

The first request in a new isolate, on Cloudflare: each new version
deployed, then one request, timed inside the Worker. Medians of the trials
served from Singapore, a handful each, so read them as sizes, not as
precise figures:

| | Without KV | With the schema cache in KV |
|---|---|---|
| Database in Singapore | ~440 ms | ~230 ms |
| Database in Delhi | ~2,000 ms | ~400 ms |

- **Keep the KV binding.** Without it, a new isolate reads the catalog
  itself, one query per table for its columns, and every query is a round
  trip to the database. With it, the isolate reads one KV value.
- **An isolate's first request uses 60-250 ms of CPU**, against 4-11 ms for
  the requests after it, whichever way the cache arrives: starting the
  WebAssembly is most of it. Workers Free allows 10 ms a request; Workers
  Paid allows far more.
- **A reload reaches other locations at KV's pace.** KV is eventually
  consistent, and a location can serve the previous value for a minute or
  more after a write. Isolates check KV every `PGRST_WORKER_SCHEMA_TTL`
  seconds on top of that.

## What it does not do

- **No GraphQL**, so no Apollo Federation subgraph and no subscriptions. Use
  the server or Lambda for those.
- **No `LISTEN`/`NOTIFY`**: Hyperdrive does not carry it, so
  `PGRST_DB_CHANNEL` reloads are not available. Use the cron trigger or
  `/_/reload`.
- **No admin UI, OpenAPI explorer or custom routes.** The OpenAPI document
  itself is served at `/`, as by the server.
- **One database connection per request**, because a Worker may not share
  sockets between requests. Hyperdrive pools behind it, and answers the
  handshake near the Worker: 3-6 ms on Cloudflare.
- **A cold schema load is one query per table** for column details. On a
  large schema, keep the KV binding so isolates rarely do it.
