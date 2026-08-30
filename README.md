# Postrust

<div align="center">

**A PostgREST-inspired REST API for PostgreSQL, written in Rust**

[![Rust](https://img.shields.io/badge/rust-1.78%2B-orange.svg)](https://www.rust-lang.org/)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Build Status](https://img.shields.io/github/actions/workflow/status/postrust/postrust/ci.yml?branch=main)](https://github.com/postrust/postrust/actions)

[Features](#features) • [Quick Start](#quick-start) • [Documentation](#documentation) • [Deployment](#deployment) • [Contributing](#contributing)

</div>

---

## What is Postrust?

Postrust is a high-performance, serverless-first REST API server for PostgreSQL databases. Inspired by [PostgREST](https://postgrest.org), it automatically generates a RESTful API from your database schema, supporting filtering, pagination, embedding, and full CRUD operations.

**Why Postrust?**

- **Serverless-first**: Native support for AWS Lambda and Cloudflare Workers
- **Fast**: Written in Rust for maximum performance and minimal cold starts
- **Compatible**: Familiar PostgREST-style API; near drop-in with an opt-in [compatibility mode](#postgrest-compatibility) (see [Differences from PostgREST](#differences-from-postgrest))
- **Type-safe**: Parameterized queries prevent SQL injection by design
- **Lightweight**: Single binary with no runtime dependencies

## Features

### Core API Features

| Feature | Status | Description |
|---------|--------|-------------|
| **CRUD Operations** | ✅ | GET, POST, PATCH, PUT, DELETE on tables/views |
| **Filtering** | ✅ | `eq`, `neq`, `gt`, `lt`, `gte`, `lte`, `like`, `ilike`, `in`, `is` |
| **Full-Text Search** | ✅ | `fts`, `plfts`, `phfts`, `wfts` operators |
| **Ordering** | ✅ | `order=column.asc`, `order=column.desc.nullsfirst` |
| **Pagination** | ✅ | `limit`, `offset`, and `Range` headers (`0-9`, `100-149`, `100-`) |
| **Column Selection** | ✅ | `select=col1,col2,relation(nested)` |
| **Resource Embedding** | ✅ | Nested resources via foreign keys |
| **RPC Functions** | ✅ | Call stored procedures via `/api/rpc/function_name` (`/rpc/...` in [compatibility mode](#postgrest-compatibility)) |
| **JWT Authentication** | ✅ | Role-based access with PostgreSQL RLS |
| **Content Negotiation** | ✅ | JSON, CSV, GeoJSON responses |
| **GraphQL API** | ✅ | Hasura-dialect GraphQL at `/v1/graphql`, measured against Hasura's own corpus |

### Deployment Targets

| Platform | Status | Description |
|----------|--------|-------------|
| **HTTP Server** | ✅ | Standalone Axum-based server |
| **AWS Lambda** | ✅ | Native Lambda adapter with connection pooling |
| **Cloudflare Workers** | 🚧 | Stub (requires Hyperdrive for database) |

### Admin & Developer Tools

| Feature | Status | Description |
|---------|--------|-------------|
| **Admin UI** | ✅ | Dashboard at `/admin` (requires `admin-ui` feature) |
| **OpenAPI Spec** | ✅ | OpenAPI 3.0 specification at `/admin/openapi.json` |
| **Swagger UI** | ✅ | Interactive API docs at `/admin/swagger` |
| **Scalar** | ✅ | Modern API docs at `/admin/scalar` |
| **GraphQL Playground** | ✅ | Interactive GraphQL IDE at `/admin/graphql` |

## Quick Start

### Prerequisites

- Rust 1.78+ (`curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`)
- PostgreSQL 12+ (or use Docker)

### Installation

```bash
# Clone the repository
git clone https://github.com/postrust/postrust.git
cd postrust

# Build the project
cargo build --release

# Binary is at target/release/postrust
```

### Running with Docker

```bash
# Start PostgreSQL and Postrust
docker-compose up -d

# API is available at http://localhost:3000
curl http://localhost:3000/users
```

### Configuration

Postrust is configured via environment variables:

```bash
# Required
DATABASE_URL=postgres://user:pass@localhost:5432/mydb

# Optional
PGRST_DB_SCHEMAS=public,api          # Exposed schemas (default: public)
PGRST_DB_ANON_ROLE=web_anon          # Anonymous role
PGRST_JWT_SECRET=your-secret-key     # JWT secret for authentication
PGRST_JWT_SECRET_IS_BASE64=false     # Set true if secret is base64 encoded
PGRST_SERVER_PORT=3000               # Server port (default: 3000)
PGRST_SERVER_HOST=0.0.0.0            # Server host (default: 127.0.0.1)
PGRST_LOG_LEVEL=info                 # Log level: error, warn, info, debug
PGRST_COMPAT_MODE=false              # PostgREST compatibility mode (default: false)
```

## Documentation

### API Examples

> **Note on paths.** By default the REST API is served under `/api` (e.g.
> `GET /api/users`, `POST /api/rpc/my_func`), GraphQL under `/v1/graphql` and
> `/api/graphql`, alongside the admin UI at `/admin`. The examples below use root-level paths
> (`/users`, `/rpc/my_func`) — these work as-is when
> [compatibility mode](#postgrest-compatibility) is enabled; otherwise prefix
> them with `/api`. See [Differences from PostgREST](#differences-from-postgrest).

#### Basic CRUD

```bash
# Get all users
curl http://localhost:3000/users

# Get user by ID
curl "http://localhost:3000/users?id=eq.1"

# Create a user
curl -X POST http://localhost:3000/users \
  -H "Content-Type: application/json" \
  -d '{"name": "John", "email": "john@example.com"}'

# Update a user
curl -X PATCH "http://localhost:3000/users?id=eq.1" \
  -H "Content-Type: application/json" \
  -d '{"name": "Jane"}'

# Delete a user
curl -X DELETE "http://localhost:3000/users?id=eq.1"
```

#### Filtering

```bash
# Equality
curl "http://localhost:3000/users?status=eq.active"

# Greater than
curl "http://localhost:3000/orders?amount=gt.100"

# Pattern matching
curl "http://localhost:3000/users?name=like.*john*"

# In list
curl "http://localhost:3000/users?id=in.(1,2,3)"

# Full-text search
curl "http://localhost:3000/articles?title=fts.postgres"

# Combining filters (AND)
curl "http://localhost:3000/users?status=eq.active&role=eq.admin"

# Negation
curl "http://localhost:3000/users?status=not.eq.deleted"
```

#### Ordering and Pagination

```bash
# Order by column
curl "http://localhost:3000/users?order=created_at.desc"

# Multiple ordering
curl "http://localhost:3000/users?order=role.asc,name.desc"

# Pagination
curl "http://localhost:3000/users?limit=10&offset=20"

# Range header -- any window, not only one starting at the first row
curl http://localhost:3000/users -H "Range: 0-9"
curl http://localhost:3000/users -H "Range: 100-149"   # -> 206, Content-Range: 100-149/1000
curl http://localhost:3000/users -H "Range: 100-"      # row 100 to the end

# What a resource allows, derived from the schema
curl -i -X OPTIONS http://localhost:3000/users
# Allow: OPTIONS,GET,HEAD,POST,PUT,PATCH,DELETE
```

#### Resource Embedding

```bash
# Embed related resources
curl "http://localhost:3000/orders?select=*,customer(name,email)"

# Nested embedding
curl "http://localhost:3000/orders?select=*,items(product(name,price))"

# Filter on embedded resource
curl "http://localhost:3000/orders?select=*,customer!inner(*)&customer.country=eq.US"
```

#### RPC Functions

```bash
# Call a function
curl -X POST http://localhost:3000/rpc/get_statistics

# With parameters
curl -X POST http://localhost:3000/rpc/search_users \
  -H "Content-Type: application/json" \
  -d '{"query": "john", "limit": 10}'

# GET for read-only functions
curl "http://localhost:3000/rpc/get_user_count"
```

#### GraphQL API

Postrust serves a GraphQL API alongside the REST one, in the dialect Hasura
speaks — so a client generated against Hasura points at it unchanged:

```bash
# Query users
curl -X POST http://localhost:3000/v1/graphql \
  -H "Content-Type: application/json" \
  -d '{
    "query": "{ users { id name email } }"
  }'

# Filtering, ordering and pagination
curl -X POST http://localhost:3000/v1/graphql \
  -H "Content-Type: application/json" \
  -d '{
    "query": "{ users(where: {status: {_eq: \"active\"}}, order_by: [{name: asc}], limit: 10) { id name } }"
  }'

# Nested queries (relationships), and an aggregate of them
curl -X POST http://localhost:3000/v1/graphql \
  -H "Content-Type: application/json" \
  -d '{
    "query": "{ orders { id total customer { name } items_aggregate { aggregate { count } } } }"
  }'

# Mutations. Several root fields in one mutation are one transaction.
curl -X POST http://localhost:3000/v1/graphql \
  -H "Content-Type: application/json" \
  -d '{
    "query": "mutation { insert_users(objects: [{name: \"John\", email: \"john@example.com\"}]) { affected_rows returning { id name } } }"
  }'

# GraphQL Playground
open http://localhost:3000/v1/graphql
```

Everything checkable without Hasura runs from one script:

```bash
scripts/check.sh                    # formatting, lints, unit tests
DATABASE_URL=... scripts/check.sh   # and the 82 that need a database
```

Measured against Hasura's own test corpus: see
[the harness](scripts/hasura-conformance/) and
[what it found](scripts/hasura-conformance/FINDINGS.md).

#### Authentication

```bash
# Request with JWT
curl http://localhost:3000/users \
  -H "Authorization: Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9..."

# JWT payload should include role claim:
# {
#   "role": "authenticated_user",
#   "sub": "user123",
#   "exp": 1234567890
# }
```

### Prefer Headers

```bash
# Return created/updated records
curl -X POST http://localhost:3000/users \
  -H "Prefer: return=representation" \
  -d '{"name": "John"}'

# Return only headers (for count)
curl http://localhost:3000/users \
  -H "Prefer: count=exact"

# Upsert (insert or update)
curl -X POST http://localhost:3000/users \
  -H "Prefer: resolution=merge-duplicates" \
  -d '{"id": 1, "name": "Updated Name"}'
```

## Deployment

### Standalone Server

```bash
# Build release binary
cargo build --release -p postrust-server

# Run
DATABASE_URL="postgres://..." ./target/release/postrust
```

### With Admin UI

The Admin UI provides OpenAPI documentation, Swagger UI, Scalar, and GraphQL Playground:

```bash
# Build with admin-ui feature
cargo build --release -p postrust-server --features admin-ui

# Return object keys in select order, as PostgREST does, rather than
# alphabetically. Off by default: it costs 1% on a three-column response and
# 15% on an eight-column one, because for objects this small a few string
# comparisons beat hashing every key. See docs/configuration.md.
cargo build --release -p postrust-server --features compat-key-order

# Run - Admin UI available at /admin
DATABASE_URL="postgres://..." ./target/release/postrust

# Access admin endpoints:
# - /admin          - Dashboard with links to all tools
# - /admin/swagger  - Swagger UI for interactive API testing
# - /admin/scalar   - Scalar modern API documentation
# - /admin/graphql  - GraphQL Playground
# - /admin/openapi.json - Raw OpenAPI 3.0 specification
```

### AWS Lambda

```bash
# Build for Lambda (requires cargo-lambda)
cargo lambda build --release -p postrust-lambda

# Deploy with AWS SAM, Serverless Framework, or CDK
```

Example SAM template:

```yaml
AWSTemplateFormatVersion: '2010-09-09'
Transform: AWS::Serverless-2016-10-31

Resources:
  PostrustFunction:
    Type: AWS::Serverless::Function
    Properties:
      Handler: bootstrap
      Runtime: provided.al2
      CodeUri: target/lambda/postrust-lambda/
      MemorySize: 256
      Timeout: 30
      Environment:
        Variables:
          DATABASE_URL: !Ref DatabaseUrl
          PGRST_JWT_SECRET: !Ref JwtSecret
      Events:
        Api:
          Type: HttpApi
```

### Docker

```dockerfile
FROM rust:1.75 as builder
WORKDIR /app
COPY . .
RUN cargo build --release -p postrust-server

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y libssl3 ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/postrust /usr/local/bin/
EXPOSE 3000
CMD ["postrust"]
```

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                      HTTP Request                           │
│              REST: /users    GraphQL: /graphql              │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                    postrust-server                          │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────┐  │
│  │   Axum      │  │   Lambda    │  │  Cloudflare Workers │  │
│  │   Server    │  │   Adapter   │  │      Adapter        │  │
│  └─────────────┘  └─────────────┘  └─────────────────────┘  │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                     postrust-auth                           │
│              JWT Validation • Role Extraction               │
└─────────────────────────────────────────────────────────────┘
                              │
              ┌───────────────┴───────────────┐
              ▼                               ▼
┌──────────────────────────────┐ ┌──────────────────────────────┐
│       postrust-core          │ │      postrust-graphql        │
│  ┌────────────────────────┐  │ │  ┌────────────────────────┐  │
│  │  REST Request Parser   │  │ │  │  GraphQL Schema        │  │
│  │  • Query params        │  │ │  │  • Dynamic types       │  │
│  │  • Prefer headers      │  │ │  │  • Queries/Mutations   │  │
│  └────────────────────────┘  │ │  └────────────────────────┘  │
│              │               │ │              │               │
│              ▼               │ │              ▼               │
│  ┌────────────────────────┐  │ │  ┌────────────────────────┐  │
│  │  Schema Cache          │◄─┼─┼──│  Resolvers             │  │
│  │  • Tables, columns     │  │ │  │  • Query → ReadPlan    │  │
│  │  • Relationships       │  │ │  │  • Mutation → Plan     │  │
│  │  • Routines            │  │ │  └────────────────────────┘  │
│  └────────────────────────┘  │ └──────────────────────────────┘
│              │               │               │
│              ▼               │               │
│  ┌────────────────────────┐  │               │
│  │  Query Planner         │  │               │
│  │  • ReadPlan            │◄─┼───────────────┘
│  │  • MutatePlan          │  │
│  └────────────────────────┘  │
└──────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                      postrust-sql                           │
│              Type-safe SQL Builder                          │
│              Parameterized Queries                          │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                      PostgreSQL                             │
│              Row-Level Security • Roles                     │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                   postrust-response                         │
│              JSON • CSV • GeoJSON                           │
└─────────────────────────────────────────────────────────────┘
```

## Crate Overview

All crates share the workspace version and are published together, so any two
Postrust crates at the same version work together. Browse them at
[postrust.org/packages](https://postrust.org/packages).

| Crate | Description | Links |
|-------|-------------|-------|
| `postrust-server` | Standalone HTTP server (Axum), ships the `postrust` binary | [crates.io](https://crates.io/crates/postrust-server) · [docs](https://docs.rs/postrust-server) |
| `postrust-core` | Request parsing, schema cache, query planning | [crates.io](https://crates.io/crates/postrust-core) · [docs](https://docs.rs/postrust-core) |
| `postrust-sql` | Type-safe SQL builder with parameterized queries | [crates.io](https://crates.io/crates/postrust-sql) · [docs](https://docs.rs/postrust-sql) |
| `postrust-auth` | JWT authentication and role extraction | [crates.io](https://crates.io/crates/postrust-auth) · [docs](https://docs.rs/postrust-auth) |
| `postrust-response` | Response formatting (JSON, CSV, OpenAPI) | [crates.io](https://crates.io/crates/postrust-response) · [docs](https://docs.rs/postrust-response) |
| `postrust-graphql` | GraphQL API with dynamic schema generation | [crates.io](https://crates.io/crates/postrust-graphql) · [docs](https://docs.rs/postrust-graphql) |
| `postrust-lambda` | AWS Lambda adapter | [crates.io](https://crates.io/crates/postrust-lambda) · [docs](https://docs.rs/postrust-lambda) |
| `postrust-worker` | Cloudflare Workers adapter | [crates.io](https://crates.io/crates/postrust-worker) · [docs](https://docs.rs/postrust-worker) |
| `postrust-proxy` | Reverse proxy with load balancing and automatic TLS | [crates.io](https://crates.io/crates/postrust-proxy) · [docs](https://docs.rs/postrust-proxy) |

## Development

### Building

```bash
# Debug build
cargo build

# Release build
cargo build --release

# Run tests
cargo test

# Run with Docker (includes PostgreSQL)
docker-compose up
```

### Running Tests

```bash
# Unit tests
cargo test

# Integration tests (requires PostgreSQL)
docker-compose up -d postgres
DATABASE_URL="postgres://postgres:postgres@localhost:5432/postrust_test" cargo test

# With coverage
cargo tarpaulin --out Html
```

### Project Structure

```
postrust/
├── Cargo.toml              # Workspace manifest
├── docker-compose.yml      # Development environment
├── crates/
│   ├── postrust-core/      # Core library
│   │   └── src/
│   │       ├── api_request/    # Request parsing
│   │       ├── schema_cache/   # DB introspection
│   │       ├── plan/           # Query planning
│   │       └── query/          # SQL generation
│   ├── postrust-sql/       # SQL builder
│   ├── postrust-auth/      # Authentication
│   ├── postrust-response/  # Response formatting
│   ├── postrust-graphql/   # GraphQL API
│   │   └── src/
│   │       ├── schema/         # Dynamic schema generation
│   │       ├── resolver/       # Query/mutation resolvers
│   │       ├── input/          # Filter/order/mutation inputs
│   │       └── handler.rs      # Axum handler
│   ├── postrust-server/    # HTTP server
│   ├── postrust-lambda/    # Lambda adapter
│   └── postrust-worker/    # Workers adapter
└── docs/                   # Documentation
```

## Benchmarks

**Throughput figures are withdrawn pending re-measurement.**

The comparison benchmark was found to report the order of a run as much as the
speed of a server: the same server, in the same container, measured seconds
apart differed by about a factor of two, and scenarios measured earlier in a
run read lower than ones measured later. The mechanism is not identified.
Thermal throttling, measurement order, host load, a cold database, the bulk
load, checkpointing, server and PostgreSQL start-up, container pausing and
Docker's port forwarding are each ruled out by direct test.

[`scripts/BENCH-FINDINGS.md`](scripts/BENCH-FINDINGS.md) records the
investigation. Numbers will return once they reproduce on a host without a
virtualised Docker network, and once a measurement repeated at the start and
the end of a run agrees with itself.

Size is not affected by any of this, and is deterministic:

| Resource | Measured |
|----------|----------|
| Binary size (`--features admin-ui`, as shipped) | 5,220,864 bytes (4.98 MiB) |
| Binary size (default features) | 3,070,080 bytes (2.93 MiB) |

The method is unchanged and documented in
[Benchmarking](docs/benchmarking.md), and the harness still runs:

```bash
./scripts/bench.sh
```

What it measures against other servers is trustworthy in shape but not yet in
number. Conformance is a separate matter and is unaffected: it measures whether
two servers give the same answer, not how fast.

## Comparison with PostgREST

| Feature | Postrust | PostgREST |
|---------|----------|-----------|
| Language | Rust | Haskell |
| Binary Size | ~5 MB (~3 MB without `admin-ui`) | ~20 MB |
| Serverless Support | Native | Via containers |
| Configuration | Env vars | Config file + env |
| OpenAPI | ✅ (admin-ui feature) | ✅ |
| GraphQL | ✅ | ❌ |
| Admin UI | ✅ (Swagger, Scalar) | ❌ |
| Object key order | Alphabetical, or select order with `compat-key-order` | Select order |

Binary sizes are measured; the rest of this table is a feature comparison
rather than a measurement. Throughput is deliberately absent — see
[Benchmarks](#benchmarks) above. How closely Postrust matches PostgREST's
behaviour *is* measured, by replaying PostgREST's own test suite against both:
see [PostgREST Conformance](docs/postgrest-conformance.md).

## Differences from PostgREST

Postrust aims to be familiar to PostgREST users, but a few things differ by
default. Most are addressed by [compatibility mode](#postgrest-compatibility);
the rest are documented here so you don't assume bit-for-bit compatibility.

| Area | PostgREST | Postrust (default) | Compatibility mode |
|------|-----------|--------------------|--------------------|
| REST base path | `/` (e.g. `POST /rpc/foo`) | Nested under `/api` (e.g. `POST /api/rpc/foo`) — leaves room for `/v1/graphql`, `/admin` on the same server | Also served at `/` |
| RPC response shape | Bare result: `{...}` for a single/scalar return, a top-level array for set-returning functions | Array-wrapped, function-name-keyed: `[{"foo": {...}}]` | Un-wrapped to match PostgREST |
| Config source | Config file **and** env vars | Environment variables only | — |
| Root endpoint `/` | OpenAPI spec | Small JSON server-info document | Serves neither yet — see below |
| Object key order | Select order | Alphabetical, or select order with the `compat-key-order` build feature | Unchanged — it is a build-time choice |
| `Prefer: tx=rollback` | Honoured | Not implemented, and not reported as applied | — |
| JWT `exp` / `nbf` | All checked to the second | `exp` to the second; 30s clock skew on `nbf` and `iat` | — |

The differences that are deliberate — including cases where PostgREST is the
one that is wrong — are listed with their evidence in
[PostgREST conformance](docs/postgrest-conformance.md).

The largest gap is the OpenAPI document. PostgREST serves Swagger 2.0 at `/`;
Postrust serves OpenAPI 3.0 for its own surface under `/admin`, behind the
`admin-ui` feature, and does not yet generate PostgREST's. Not every PostgREST
config knob is implemented either. Contributions welcome.

### PostgREST compatibility

Set `PGRST_COMPAT_MODE=true` (alias: `POSTRUST_COMPAT_MODE=true`) to make the
API behave more like PostgREST:

- **Canonical paths at the root.** The full REST surface is also served at `/`,
  so `POST /rpc/<name>`, `GET /<table>`, etc. work in addition to the
  `/api`-prefixed paths. Explicit routes (`/`, `/_`, `/admin`, `/api`) still take
  precedence.
- **PostgREST-shaped RPC responses.** Results from `POST /rpc/<name>` are
  un-wrapped: a non-set-returning function returns its bare object/scalar
  (`{...}` / `42`) and a set-returning function returns a top-level array
  (`[{...}, {...}]`), instead of the default `[{"<name>": ...}]` shape.

```bash
# Default mode
curl -X POST http://localhost:3000/api/rpc/get_statistics
# -> [{"get_statistics": {"users": 10}}]

# Compatibility mode (PGRST_COMPAT_MODE=true)
curl -X POST http://localhost:3000/rpc/get_statistics
# -> {"users": 10}
```

This is opt-in so existing Postrust deployments keep their current behavior.

How closely the two agree is measured rather than asserted: the harness in
`scripts/conformance/` replays PostgREST's own test cases over HTTP against
both servers on identically loaded fixture databases. Over 1499 cases against
PostgREST v16.1, **96.7% agree on status and body, and 94.9% on the full
contract** — status, body, and every header that is part of the answer. See
[PostgREST conformance](docs/postgrest-conformance.md) for what that covers,
where the two disagree on purpose, and what is still open.

## Roadmap

- [x] OpenAPI 3.0 specification generation (via `admin-ui` feature)
- [x] GraphQL adapter (queries, mutations, filtering, relationships)
- [x] Admin UI with Swagger, Scalar, and GraphQL Playground
- [ ] GraphQL subscriptions (LISTEN/NOTIFY)
- [ ] Connection pooling improvements
- [ ] Cloudflare Workers full support (Hyperdrive)
- [ ] Prometheus metrics endpoint
- [ ] Admin API for schema reload

## Contributing

Contributions are welcome! Please read our [Contributing Guide](CONTRIBUTING.md) for details.

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/amazing-feature`)
3. Commit your changes (`git commit -m 'Add amazing feature'`)
4. Push to the branch (`git push origin feature/amazing-feature`)
5. Open a Pull Request

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

## Acknowledgments

- [PostgREST](https://postgrest.org) - The inspiration for this project
- [Axum](https://github.com/tokio-rs/axum) - Web framework
- [SQLx](https://github.com/launchbadge/sqlx) - Async PostgreSQL driver

---

<div align="center">

Made with ❤️ by the Postrust contributors

[Report Bug](https://github.com/postrust/postrust/issues) • [Request Feature](https://github.com/postrust/postrust/issues)

</div>
