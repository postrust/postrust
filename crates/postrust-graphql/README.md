# postrust-graphql

GraphQL API and realtime subscriptions generated from a PostgreSQL schema.

- Builds a GraphQL schema from database tables, views and relationships.
- Queries, mutations, filtering and ordering that mirror the REST surface.
- Subscriptions backed by PostgreSQL `LISTEN`/`NOTIFY`.

## Install

```bash
cargo add postrust-graphql
```

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
| **`postrust-graphql`** (this crate) | GraphQL | [docs.rs](https://docs.rs/postrust-graphql) |
| [`postrust-lambda`](https://crates.io/crates/postrust-lambda) | AWS Lambda | [docs.rs](https://docs.rs/postrust-lambda) |
| [`postrust-worker`](https://crates.io/crates/postrust-worker) | Cloudflare Workers | [docs.rs](https://docs.rs/postrust-worker) |
| [`postrust-proxy`](https://crates.io/crates/postrust-proxy) | Reverse proxy | [docs.rs](https://docs.rs/postrust-proxy) |

## Links

- Website: https://postrust.org/
- Documentation: https://postrust.org/docs
- Repository: https://github.com/postrust/postrust
- API docs for this crate: https://docs.rs/postrust-graphql

## License

MIT -- see [LICENSE](https://github.com/postrust/postrust/blob/main/LICENSE).
