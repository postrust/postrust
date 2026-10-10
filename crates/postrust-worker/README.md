# postrust-worker

A placeholder for a Cloudflare Workers adapter for Postrust. **It does not work
yet:** its fetch handler answers `{"status": "stub"}`, it reaches no database,
and it does not currently build for `wasm32-unknown-unknown`.

Making it real means separating Postrust's engine from its database driver and
adding a driver that reaches PostgreSQL through Hyperdrive over a Workers
socket. The plan, the requirements and how to help are in
[issue #50](https://github.com/postrust/postrust/issues/50).

To run Postrust serverless today, use
[`postrust-lambda`](https://crates.io/crates/postrust-lambda), which serves the
same REST and GraphQL APIs as the server on AWS Lambda.

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
| **`postrust-worker`** (this crate) | Cloudflare Workers | [docs.rs](https://docs.rs/postrust-worker) |
| [`postrust-proxy`](https://crates.io/crates/postrust-proxy) | Reverse proxy | [docs.rs](https://docs.rs/postrust-proxy) |

## Links

- Website: https://postrust.org/
- Documentation: https://postrust.org/docs
- Repository: https://github.com/postrust/postrust
- API docs for this crate: https://docs.rs/postrust-worker

## License

MIT -- see [LICENSE](https://github.com/postrust/postrust/blob/main/LICENSE).
