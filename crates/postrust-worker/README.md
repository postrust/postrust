# postrust-worker

Postrust's REST API in a Cloudflare Worker, reaching PostgreSQL through
[Hyperdrive](https://developers.cloudflare.com/hyperdrive/).

- The request pipeline is [`postrust-engine`](https://crates.io/crates/postrust-engine),
  the same one the server runs. The database driver speaks the PostgreSQL wire
  protocol over a Workers socket and is tested to answer exactly as the
  server's does.
- Every request runs in a transaction that takes on the token's role and
  claims, so row-level security applies as it does on the server.
- The schema cache is kept in Workers KV. It is refreshed by a Cron Trigger,
  by an authenticated `POST /_/reload`, or when the KV copy expires after
  `PGRST_WORKER_SCHEMA_TTL`.
- REST only, served at the root as PostgREST serves it. There is no GraphQL
  and no `LISTEN`/`NOTIFY`.

Setup, configuration, measured costs and limits:
[docs/workers.md](https://github.com/postrust/postrust/blob/main/docs/workers.md).
The crate is built with `worker-build` for `wasm32-unknown-unknown`; for any
other target it is only its configuration logic, there to be unit-tested.

From 1.0.0 this crate keeps semver over what a deployment sees -- the
variables it reads, the bindings it looks for, its endpoints and defaults --
on a version of its own. See
[docs/stability.md](https://github.com/postrust/postrust/blob/main/docs/stability.md).

## Part of Postrust

[Postrust](https://github.com/postrust/postrust) turns a PostgreSQL database into a REST and GraphQL API. It is
published as a set of crates so you can depend on only the part you need:

| Crate | Role | Docs |
|-------|------|------|
| [`postrust-server`](https://crates.io/crates/postrust-server) | HTTP server | [docs.rs](https://docs.rs/postrust-server) |
| [`postrust-core`](https://crates.io/crates/postrust-core) | Engine | [docs.rs](https://docs.rs/postrust-core) |
| [`postrust-sql`](https://crates.io/crates/postrust-sql) | SQL builder | [docs.rs](https://docs.rs/postrust-sql) |
| [`postrust-auth`](https://crates.io/crates/postrust-auth) | Authentication | [docs.rs](https://docs.rs/postrust-auth) |
| [`postrust-response`](https://crates.io/crates/postrust-response) | Response formatting | [docs.rs](https://docs.rs/postrust-response) |
| [`postrust-graphql`](https://crates.io/crates/postrust-graphql) | GraphQL | [docs.rs](https://docs.rs/postrust-graphql) |
| [`postrust-lambda`](https://crates.io/crates/postrust-lambda) | AWS Lambda | [docs.rs](https://docs.rs/postrust-lambda) |
| [`postrust-engine`](https://crates.io/crates/postrust-engine) | REST request pipeline | [docs.rs](https://docs.rs/postrust-engine) |
| **`postrust-worker`** (this crate) | Cloudflare Workers | [docs.rs](https://docs.rs/postrust-worker) |
| [`postrust-proxy`](https://crates.io/crates/postrust-proxy) | Reverse proxy | [docs.rs](https://docs.rs/postrust-proxy) |

## Links

- Website: https://postrust.org/
- Documentation: https://postrust.org/docs
- Repository: https://github.com/postrust/postrust
- API docs for this crate: https://docs.rs/postrust-worker

## License

MIT -- see [LICENSE](https://github.com/postrust/postrust/blob/main/LICENSE).
