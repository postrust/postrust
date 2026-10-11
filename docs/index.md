# Postrust Documentation

Welcome to the Postrust documentation. Postrust serves a PostgreSQL schema as a REST API in PostgREST's dialect and a GraphQL API in Hasura's, from one Rust binary that runs as a server, in a container, or on AWS Lambda.

## Quick Links

- [Getting Started](./getting-started.md) - Installation and first steps
- [Configuration](./configuration.md) - Environment variables and options
- [API Reference](./api-reference.md) - Complete API documentation
- [Authentication](./authentication.md) - JWT and role-based access
- [Realtime Subscriptions](./realtime.md) - Table changes streamed over GraphQL subscriptions
- [pgvector Integration](./pgvector.md) - Vector similarity search for AI applications
- [Custom Routes](./custom-routes.md) - Add webhooks, health checks, and custom endpoints
- [Reverse Proxy](./proxy.md) - Routing, TLS, HTTP/2 and WebSocket at the front door
- [SaaS Domain Management](./saas-domains.md) - Multi-tenant custom domains with automatic SSL (Beta)
- [Deployment](./deployment.md) - Deploy to various platforms
- [Stability and Versioning](./stability.md) - What a version number promises, and which crates it covers
- [Benchmarking](./benchmarking.md) - The method, and why the throughput figures are currently withdrawn
- [PostgREST Conformance](./postgrest-conformance.md) - How closely the two agree, measured against PostgREST itself
- [Hasura Conformance](./hasura-conformance.md) - How closely the GraphQL dialects agree, measured against graphql-engine itself

## What is Postrust?

Postrust automatically generates a RESTful API from your PostgreSQL database schema. It's inspired by [PostgREST](https://postgrest.org) and provides:

- **Automatic API generation** from database tables, views, and functions
- **Powerful filtering** with operators like `eq`, `gt`, `like`, `fts`, and more
- **Resource embedding** via foreign key relationships
- **JWT authentication** with PostgreSQL Row-Level Security
- **Serverless deployment** to AWS Lambda, serving the same REST and GraphQL
  APIs as the server, and to Cloudflare Workers through Hyperdrive, serving
  the REST API ([workers.md](workers.md))

## Why Postrust?

| Feature | Postrust | Traditional API |
|---------|----------|-----------------|
| Development Time | Minutes | Days/Weeks |
| Code to Maintain | Zero | Thousands of lines |
| Type Safety | Database-driven | Manual |
| Performance | Native Rust | Varies |
| Serverless | Native | Complex setup |

## Architecture Overview

```
┌──────────────┐     ┌──────────────┐     ┌──────────────┐
│   Client     │────▶│   Postrust   │────▶│  PostgreSQL  │
│  (Browser,   │◀────│   Server     │◀────│   Database   │
│   Mobile)    │     └──────────────┘     └──────────────┘
└──────────────┘            │
                            │
                     ┌──────┴──────┐
                     │             │
              ┌──────▼─────┐ ┌─────▼──────┐
              │ AWS Lambda │ │ Cloudflare │
              │            │ │ (planned)  │
              └────────────┘ └────────────┘
```

## Getting Help

- [GitHub Issues](https://github.com/postrust/postrust/issues) - Bug reports and feature requests
- [GitHub Discussions](https://github.com/postrust/postrust/discussions) - Questions and community
- [Contributing Guide](https://github.com/postrust/postrust/blob/main/CONTRIBUTING.md) - How to contribute

## License

Postrust is open source software licensed under the [MIT License](https://github.com/postrust/postrust/blob/main/LICENSE).
