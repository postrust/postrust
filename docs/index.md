# Postrust Documentation

Welcome to the Postrust documentation. Postrust is a high-performance, serverless-first REST API server for PostgreSQL databases, written in Rust.

## Quick Links

- [Getting Started](./getting-started.md) - Installation and first steps
- [Configuration](./configuration.md) - Environment variables and options
- [API Reference](./api-reference.md) - Complete API documentation
- [Authentication](./authentication.md) - JWT and role-based access
- [Deployment](./deployment.md) - Deploy to various platforms
- [API Examples](./examples.md) - Common use cases

## What is Postrust?

Postrust automatically generates a RESTful API from your PostgreSQL database schema. It's inspired by [PostgREST](https://postgrest.org) and provides:

- **Automatic API generation** from database tables, views, and functions
- **Powerful filtering** with operators like `eq`, `gt`, `like`, `fts`, and more
- **Resource embedding** via foreign key relationships
- **JWT authentication** with PostgreSQL Row-Level Security
- **Serverless deployment** to AWS Lambda and Cloudflare Workers

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
              │            │ │  Workers   │
              └────────────┘ └────────────┘
```

## Getting Help

- [GitHub Issues](https://github.com/postrust/postrust/issues) - Bug reports and feature requests
- [GitHub Discussions](https://github.com/postrust/postrust/discussions) - Questions and community
- [Contributing Guide](https://github.com/postrust/postrust/blob/main/CONTRIBUTING.md) - How to contribute

## License

Postrust is open source software licensed under the [MIT License](https://github.com/postrust/postrust/blob/main/LICENSE).
