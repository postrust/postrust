# postrust-lambda

AWS Lambda adapter for Postrust, serving its REST and GraphQL APIs over
PostgreSQL.

- Serves the router the `postrust` binary serves, built by
  `postrust_server::build`: REST under `/api`, GraphQL at `/v1/graphql`
  (Apollo Federation included), JWT authentication, the role switch and the
  claims row-level security reads. GraphQL subscriptions are left out, since a
  function cannot hold a WebSocket.
- Connects and loads the schema cache once per sandbox, in the init phase.
- Tested in AWS's `provided:al2023` runtime image against PostgreSQL with
  row-level security, through a `NOINHERIT` authenticator.

Build the function with [cargo-lambda](https://www.cargo-lambda.info/):

```bash
cargo lambda build --release -p postrust-lambda
```

See the [deployment guide](https://github.com/postrust/postrust/blob/main/docs/deployment.md#aws-lambda)
for SAM and Serverless Framework templates, stage paths and pool sizing.

## Install

```bash
cargo add postrust-lambda
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
| [`postrust-graphql`](https://crates.io/crates/postrust-graphql) | GraphQL | [docs.rs](https://docs.rs/postrust-graphql) |
| **`postrust-lambda`** (this crate) | AWS Lambda | [docs.rs](https://docs.rs/postrust-lambda) |
| [`postrust-worker`](https://crates.io/crates/postrust-worker) | Cloudflare Workers | [docs.rs](https://docs.rs/postrust-worker) |
| [`postrust-proxy`](https://crates.io/crates/postrust-proxy) | Reverse proxy | [docs.rs](https://docs.rs/postrust-proxy) |

## Links

- Website: https://postrust.org/
- Documentation: https://postrust.org/docs
- Repository: https://github.com/postrust/postrust
- API docs for this crate: https://docs.rs/postrust-lambda

## License

MIT -- see [LICENSE](https://github.com/postrust/postrust/blob/main/LICENSE).
