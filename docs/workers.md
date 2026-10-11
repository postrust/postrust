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
  - the table state afterwards.

  CI runs it beside the Lambda test (`.github/workflows/serverless.yml`).
- `CONFORMANCE_WORKER_BUILD=... scripts/conformance/conformance.sh` replays
  PostgREST's own test suite against the Worker in workerd. Results are in
  [postgrest-conformance.md](postgrest-conformance.md).
- `crates/postrust-server/tests/drivers_agree.rs` runs the same statements
  through both drivers, the server's sqlx and the Worker's socket driver. It
  requires identical answers: every column type, parameter typing, and
  errors down to the SQLSTATE, detail and constraint.

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
| `PGRST_DB_MAX_ROWS` | `1000` | As on the server (`PGRST_MAX_ROWS` too), but with a ceiling by default: an isolate has 128 MB. `unlimited` under either name lifts it. |
| `PGRST_WORKER_SCHEMA_TTL` | `60` | How many seconds an isolate trusts its copy of the schema cache before checking KV for a newer one, and how long the KV copy lives (at least 60, KV's minimum). Anything but a non-negative number of seconds is a configuration error. |
| `POSTRUST_RELOAD_SECRET` | unset | A secret. When set, `POST /_/reload` with `Authorization: Bearer <secret>` reloads the schema cache. When unset, the endpoint does not exist. |
| `compat-key-order` (Cargo feature) | off | Object keys in select order, as PostgREST returns them; build with `worker-build --release --features compat-key-order`. |

A variable may be given unquoted in `wrangler.toml`: wrangler passes
`PGRST_DB_MAX_ROWS = 5000` as a number and `PGRST_JWT_SECRET_IS_BASE64 = true`
as a boolean, and the Worker reads them as `5000` and `true`. A `PGRST_`
variable set to a JSON object or array is ignored, with a warning in the log.

When the configuration cannot be read, every request answers with the reason:
`/_/health` with a 503, anything else with a 500.

## The schema cache

The server reads the catalog once at start-up. A Worker has no start-up worth
the name: isolates come and go. So the cache is kept in Workers KV, under a
key that names the exposed schemas and the versions of `postrust-worker` and
`postrust-core` that wrote it, and handled like this:

- **A request** uses its isolate's copy while it is younger than
  `PGRST_WORKER_SCHEMA_TTL`, then looks in KV for a newer one.
- **A cold isolate** reads KV. Only when KV has nothing, or has a record that
  does not read back, does it load from the catalog, and then it writes what it
  loaded to KV for every other isolate.
- **The KV copy expires** after the TTL, or 60 seconds if the TTL is shorter
  (KV's minimum). The next isolate to miss loads the catalog again, so a
  migration is seen without anyone asking: within the expiration plus the TTL,
  plus up to about 60 seconds that KV may serve a cached read at the edge.
- **After a migration**, reload with `POST /_/reload` or wait for the cron
  trigger. Both load from the catalog and write KV, and isolates pick the new
  cache up within the TTL, plus up to about 60 seconds of KV edge caching.

Without a KV binding everything still works, but each new isolate loads the
catalog itself, and each isolate loads it again every
`PGRST_WORKER_SCHEMA_TTL` seconds.

## What it costs

Measured on one machine. The test ran the server and the Worker under
`wrangler dev` (workerd) side by side against the same PostgreSQL with the
benchmark dataset (100,000 rows), with the same requests and the same
keep-alive load client. Local workerd is not Cloudflare's edge, and there is
no real Hyperdrive in it, so read this as what the Worker's design costs, not
as production latency:

| | One request at a time, p50 | 8 concurrent |
|---|---|---|
| Server, point lookup | 1.1 ms | 2,981 req/s |
| Worker, point lookup, PostgreSQL asking for SCRAM | 54 ms | 22 req/s |
| Worker, point lookup, no password authentication | 11 ms | 254 req/s |
| Worker, 25-row page, no password authentication | 11 ms | 239 req/s |

- **The authentication handshake is most of it.** Each request opens a
  connection. With `scram-sha-256` the client derives its key with 4,096
  rounds of PBKDF2-SHA-256, in WebAssembly, on every request: about 43 ms of
  CPU locally, whatever `wasm-opt` level the build uses. Whether Hyperdrive
  asks a Worker for SCRAM, or for something cheaper over its own network, can
  only be measured on a real deployment. Measure that before relying on these
  numbers, and against your plan's CPU-time limit.
- **Without it, about 11 ms a request.** That goes on wasm, the connection
  handshake, and the statements a read sends one after another: `BEGIN`,
  `SET TRANSACTION READ ONLY`, the settings, the query, `COMMIT`. Each waits
  for the last, so on a real network each is a round trip to Hyperdrive.
  Sending them together is an obvious next step, and is not done yet.
  It would cut a read to two flights (or one) and a write to two. The saving
  depends only on the distance to the database. These are projections, not
  measurements:

  | Round trip to the database | Saved per read | Saved per write |
  |---|---|---|
  | ~1 ms | ~3-4 ms | ~4 ms |
  | ~20 ms | ~60-80 ms | ~80 ms |
  | ~60 ms | ~180-240 ms | ~240 ms |
- **The bundle** is 2.7 MB of wasm, 0.93 MB gzipped, within the free plan's
  3 MB.

The native server is unaffected by any of this. Its throughput was measured
against 2.2.1 before and after the change and is the same, within the noise
of the host (see the CHANGELOG).

## What it does not do

- **No GraphQL**, so no Apollo Federation subgraph and no subscriptions. Use
  the server or Lambda for those.
- **No `LISTEN`/`NOTIFY`**: Hyperdrive does not carry it, so
  `PGRST_DB_CHANNEL` reloads are not available. Use the cron trigger or
  `/_/reload`.
- **No admin UI, OpenAPI explorer or custom routes.** The OpenAPI document
  itself is served at `/`, as by the server.
- **One database connection per request**, because a Worker may not share
  sockets between requests. Hyperdrive pools behind it, but each request
  still pays for a connection handshake to Hyperdrive.
- **A cold schema load is one query per table** for column details. On a
  large schema, keep the KV binding so isolates rarely do it.
