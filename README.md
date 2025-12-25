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
- **Compatible**: Drop-in replacement for PostgREST in most use cases
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
| **Pagination** | ✅ | `limit`, `offset`, Range headers |
| **Column Selection** | ✅ | `select=col1,col2,relation(nested)` |
| **Resource Embedding** | ✅ | Nested resources via foreign keys |
| **RPC Functions** | ✅ | Call stored procedures via `/rpc/function_name` |
| **JWT Authentication** | ✅ | Role-based access with PostgreSQL RLS |
| **Content Negotiation** | ✅ | JSON, CSV, GeoJSON responses |
| **GraphQL API** | ✅ | Full GraphQL support via `/graphql` endpoint |

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
```

## Documentation

### API Examples

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

# Range header
curl http://localhost:3000/users -H "Range: 0-9"
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

Postrust provides a full GraphQL API alongside the REST API:

```bash
# Query users
curl -X POST http://localhost:3000/graphql \
  -H "Content-Type: application/json" \
  -d '{
    "query": "{ users { id name email } }"
  }'

# Query with filtering and pagination
curl -X POST http://localhost:3000/graphql \
  -H "Content-Type: application/json" \
  -d '{
    "query": "{ users(filter: {status: {eq: \"active\"}}, limit: 10) { id name } }"
  }'

# Nested queries (relationships)
curl -X POST http://localhost:3000/graphql \
  -H "Content-Type: application/json" \
  -d '{
    "query": "{ orders { id total customer { name email } items { product { name price } } } }"
  }'

# Mutations
curl -X POST http://localhost:3000/graphql \
  -H "Content-Type: application/json" \
  -d '{
    "query": "mutation { insertUsers(objects: [{name: \"John\", email: \"john@example.com\"}]) { id name } }"
  }'

# GraphQL Playground available at GET /graphql
open http://localhost:3000/graphql
```

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

| Crate | Description |
|-------|-------------|
| `postrust-core` | Core library: request parsing, schema cache, query planning |
| `postrust-sql` | Type-safe SQL builder with parameterized queries |
| `postrust-auth` | JWT authentication and role extraction |
| `postrust-response` | Response formatting (JSON, CSV, headers) |
| `postrust-graphql` | GraphQL API with dynamic schema generation |
| `postrust-server` | Standalone HTTP server (Axum) |
| `postrust-lambda` | AWS Lambda adapter |
| `postrust-worker` | Cloudflare Workers adapter |

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

## Comparison with PostgREST

| Feature | Postrust | PostgREST |
|---------|----------|-----------|
| Language | Rust | Haskell |
| Binary Size | ~5 MB | ~20 MB |
| Cold Start (Lambda) | ~50ms | N/A |
| Memory Usage | Lower | Higher |
| Serverless Support | Native | Via containers |
| Configuration | Env vars | Config file + env |
| OpenAPI | ✅ (admin-ui feature) | ✅ |
| GraphQL | ✅ | ❌ |
| Admin UI | ✅ (Swagger, Scalar) | ❌ |

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
