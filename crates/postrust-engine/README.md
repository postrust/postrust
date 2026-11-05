# postrust-engine

The REST request pipeline of [Postrust](https://postrust.org/), independent of any web framework: an `http::Request` in, the `http::Response<Bytes>` the PostgREST-dialect API answers with out.

Authentication, request parsing, planning, execution against PostgreSQL inside a transaction that sets the token's role and claims, embedding, counting and media handlers all live here. [`postrust-server`](https://crates.io/crates/postrust-server) wraps it in axum. A host that cannot run axum, such as a Cloudflare Worker, calls it directly ([issue #50](https://github.com/postrust/postrust/issues/50)).

Most users want the `postrust` binary from `postrust-server`, not this crate.

## Part of Postrust

| Crate | Role | Docs |
|-------|------|------|
| [`postrust-server`](https://crates.io/crates/postrust-server) | HTTP server | [docs.rs](https://docs.rs/postrust-server) |
| **`postrust-engine`** (this crate) | REST request pipeline | [docs.rs](https://docs.rs/postrust-engine) |
| [`postrust-core`](https://crates.io/crates/postrust-core) | Parsing, planning, schema cache | [docs.rs](https://docs.rs/postrust-core) |
| [`postrust-sql`](https://crates.io/crates/postrust-sql) | SQL builder | [docs.rs](https://docs.rs/postrust-sql) |
| [`postrust-auth`](https://crates.io/crates/postrust-auth) | JWT authentication | [docs.rs](https://docs.rs/postrust-auth) |
| [`postrust-response`](https://crates.io/crates/postrust-response) | Response formatting | [docs.rs](https://docs.rs/postrust-response) |

## Links

- Website: https://postrust.org/
- Documentation: https://postrust.org/docs/
- REST API Reference: https://postrust.org/docs/api-reference/
- Cloudflare Workers Adapter: https://postrust.org/docs/workers/
- PostgREST Conformance: https://postrust.org/docs/conformance/postgrest/
- Repository: https://github.com/postrust/postrust
- API docs for this crate: https://docs.rs/postrust-engine

## License

MIT
