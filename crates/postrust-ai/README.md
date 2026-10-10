# postrust-ai

On-demand AI model serving for Postrust: models load when asked for and unload when idle, work goes through
priority queues in PostgreSQL, and a request can be pinned to the instance that holds its conversation.

- Model servers (vLLM, whisper.cpp, Ollama, or any HTTP server) run as separate processes, containers or attached
  servers. Nothing runs in this process.
- Models load lazily, and are parked in host memory or stopped when something else needs the room.
- Jobs queue in PostgreSQL by priority. Background jobs can be paused for urgent ones, resuming from a checkpoint or
  starting over, as the queue says.
- Workflows chain jobs into a graph with fan-out and fan-in. Synchronous requests go through a gateway with session
  locks, and the node can register itself with `postrust-proxy`.

> **On its own `0.x` line, with no stability promise.** A minor release may break it. It has been tested against stub
> servers and a real PostgreSQL, but not against real vLLM, Ollama, whisper.cpp or Docker. See
> [Stability and Versioning](https://github.com/postrust/postrust/blob/main/docs/stability.md).

## Install

```bash
cargo add postrust-ai
```

Or run the node:

```bash
postrust-ai path/to/postrust-ai.toml
```

A complete example configuration is in
[`examples/postrust-ai.toml`](https://github.com/postrust/postrust/blob/main/crates/postrust-ai/examples/postrust-ai.toml).
The guide is [docs/ai-workers.md](https://github.com/postrust/postrust/blob/main/docs/ai-workers.md).

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
| [`postrust-worker`](https://crates.io/crates/postrust-worker) | Cloudflare Workers | [docs.rs](https://docs.rs/postrust-worker) |
| [`postrust-proxy`](https://crates.io/crates/postrust-proxy) | Reverse proxy | [docs.rs](https://docs.rs/postrust-proxy) |
| **`postrust-ai`** (this crate) | AI model serving | [docs.rs](https://docs.rs/postrust-ai) |

## Links

- Website: https://postrust.org/
- Documentation: https://postrust.org/docs
- Repository: https://github.com/postrust/postrust
- API docs for this crate: https://docs.rs/postrust-ai

## License

MIT -- see [LICENSE](https://github.com/postrust/postrust/blob/main/LICENSE).
