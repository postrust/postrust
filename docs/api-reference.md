# API Reference

Complete reference for the Postrust REST and GraphQL APIs.

## Endpoints

### Tables and Views (REST)

| Method | Endpoint | Description |
|--------|----------|-------------|
| `GET` | `/{table}` | Read rows |
| `POST` | `/{table}` | Create row(s) |
| `PATCH` | `/{table}` | Update rows |
| `PUT` | `/{table}` | Upsert row |
| `DELETE` | `/{table}` | Delete rows |
| `HEAD` | `/{table}` | Get headers only |
| `OPTIONS` | `/{table}` | Get table info |

### RPC Functions

| Method | Endpoint | Description |
|--------|----------|-------------|
| `GET` | `/rpc/{function}` | Call read-only function |
| `POST` | `/rpc/{function}` | Call any function |
| `HEAD` | `/rpc/{function}` | Get function headers |
| `OPTIONS` | `/rpc/{function}` | Get function info |

### GraphQL

| Method | Endpoint | Description |
|--------|----------|-------------|
| `GET` | `/graphql` | GraphQL Playground |
| `POST` | `/graphql` | Execute GraphQL query/mutation |

### Schema

| Method | Endpoint | Description |
|--------|----------|-------------|
| `GET` | `/` | OpenAPI specification |

### Admin UI (requires `admin-ui` feature)

| Method | Endpoint | Description |
|--------|----------|-------------|
| `GET` | `/admin` | Admin dashboard |
| `GET` | `/admin/openapi.json` | OpenAPI 3.0 specification |
| `GET` | `/admin/swagger` | Swagger UI |
| `GET` | `/admin/scalar` | Scalar API documentation |
| `GET` | `/admin/graphql` | GraphQL Playground |

## Query Parameters

### select

Choose which columns to return:

```bash
# Specific columns
GET /users?select=id,name,email

# All columns (default)
GET /users?select=*

# Rename columns
GET /users?select=user_id:id,user_name:name

# Cast types
GET /users?select=id::text,created_at::date

# JSON columns
GET /users?select=id,metadata->preferences
```

### Filtering

Filter rows using column operators:

```bash
# Basic equality
GET /users?status=eq.active

# Comparison
GET /products?price=gt.100
GET /products?price=gte.100
GET /products?price=lt.50
GET /products?price=lte.50

# Not equal
GET /users?role=neq.admin

# Pattern matching
GET /users?name=like.John*
GET /users?email=ilike.*@gmail.com

# In list
GET /users?id=in.(1,2,3)

# Is null / not null
GET /users?deleted_at=is.null
GET /users?email=not.is.null

# Range
GET /products?price=gte.10&price=lte.50
```

### Filter Operators

| Operator | Description | Example |
|----------|-------------|---------|
| `eq` | Equal | `?id=eq.5` |
| `neq` | Not equal | `?status=neq.deleted` |
| `gt` | Greater than | `?age=gt.18` |
| `gte` | Greater than or equal | `?price=gte.100` |
| `lt` | Less than | `?stock=lt.10` |
| `lte` | Less than or equal | `?rating=lte.3` |
| `like` | LIKE pattern (case-sensitive) | `?name=like.J*` |
| `ilike` | ILIKE pattern (case-insensitive) | `?email=ilike.*@GMAIL.COM` |
| `in` | In list | `?id=in.(1,2,3)` |
| `is` | Is null/true/false | `?active=is.true` |
| `not` | Negate operator | `?status=not.eq.deleted` |
| `cs` | Contains (arrays/JSON) | `?tags=cs.{rust,api}` |
| `cd` | Contained by | `?tags=cd.{rust,api,web}` |
| `ov` | Overlaps | `?tags=ov.{rust,go}` |
| `sl` | Strictly left of (ranges) | `?range=sl.[5,10]` |
| `sr` | Strictly right of | `?range=sr.[0,5]` |
| `nxr` | Not extends right | `?range=nxr.[5,10]` |
| `nxl` | Not extends left | `?range=nxl.[0,5]` |
| `adj` | Adjacent to | `?range=adj.[5,10]` |

### Full-Text Search

| Operator | Description | Example |
|----------|-------------|---------|
| `fts` | Full-text search | `?title=fts.postgres` |
| `plfts` | Plain text search | `?content=plfts.database` |
| `phfts` | Phrase search | `?content=phfts.rest%20api` |
| `wfts` | Websearch syntax | `?content=wfts.rust%20OR%20go` |

With language:

```bash
# Specify language
GET /articles?content=fts(english).database
GET /articles?content=plfts(german).datenbank
```

### order

Sort results:

```bash
# Ascending (default)
GET /users?order=name

# Descending
GET /users?order=created_at.desc

# Nulls first/last
GET /users?order=email.asc.nullsfirst
GET /users?order=deleted_at.desc.nullslast

# Multiple columns
GET /users?order=role.asc,name.desc
```

### limit and offset

Paginate results:

```bash
# First 10 rows
GET /users?limit=10

# Skip first 20, get next 10
GET /users?limit=10&offset=20
```

### Range Header

Alternative pagination using HTTP Range header:

```bash
# Rows 0-9 (first 10)
GET /users
Range: 0-9

# Rows 100-149
GET /users
Range: 100-149
```

Response includes `Content-Range` header:

```
Content-Range: 0-9/100
```

## Resource Embedding

Include related resources via foreign keys:

```bash
# Embed customer in orders
GET /orders?select=*,customer(*)

# Embed specific columns
GET /orders?select=id,total,customer(name,email)

# Nested embedding
GET /orders?select=*,items(product(name,price))

# Multiple embeds
GET /posts?select=*,author(*),comments(*)
```

### Embedding Hints

When there are multiple relationships between tables:

```bash
# Use specific foreign key
GET /orders?select=*,users!orders_customer_id_fkey(*)

# Inner join (filter out nulls)
GET /orders?select=*,customer!inner(*)
```

### Filtering on Embedded Resources

```bash
# Filter by embedded resource
GET /orders?select=*,customer!inner(*)&customer.country=eq.US

# Combined with other filters
GET /orders?select=*,customer!inner(*)&status=eq.completed&customer.tier=eq.premium
```

## Request Body

### Creating Records

```bash
# Single record
POST /users
Content-Type: application/json

{"name": "John", "email": "john@example.com"}

# Multiple records
POST /users
Content-Type: application/json

[
  {"name": "John", "email": "john@example.com"},
  {"name": "Jane", "email": "jane@example.com"}
]
```

### Updating Records

```bash
# Update matching rows
PATCH /users?status=eq.pending
Content-Type: application/json

{"status": "active"}
```

### Upsert

```bash
# Insert or update (requires unique constraint)
POST /users
Content-Type: application/json
Prefer: resolution=merge-duplicates

{"id": 1, "name": "Updated Name"}
```

## Request Headers

### Prefer

Control response behavior:

| Prefer Value | Description |
|--------------|-------------|
| `return=minimal` | Don't return body (default for mutations) |
| `return=headers-only` | Return headers only |
| `return=representation` | Return created/updated records |
| `count=exact` | Include exact count in headers |
| `count=planned` | Use EXPLAIN for estimated count |
| `count=estimated` | Use table statistics for count |
| `resolution=merge-duplicates` | Upsert mode |
| `resolution=ignore-duplicates` | Skip duplicates |
| `missing=default` | Use column defaults for missing values |
| `tx=commit` | Commit transaction (default) |
| `tx=rollback` | Rollback transaction (for testing) |

Example:

```bash
POST /users
Content-Type: application/json
Prefer: return=representation, count=exact

{"name": "John", "email": "john@example.com"}
```

### Content-Profile

Select schema:

```bash
GET /users
Content-Profile: api

POST /users
Content-Profile: api
Content-Type: application/json
```

### Accept

Content negotiation:

```bash
# JSON (default)
Accept: application/json

# CSV
Accept: text/csv

# GeoJSON
Accept: application/geo+json

# OpenAPI
Accept: application/openapi+json
```

## Response Headers

| Header | Description |
|--------|-------------|
| `Content-Range` | Pagination info: `0-24/100` |
| `Range-Unit` | Always `items` |
| `Content-Location` | URL of created resource |
| `Preference-Applied` | Applied Prefer values |

## HTTP Status Codes

| Code | Description |
|------|-------------|
| `200` | Success (GET, PATCH, DELETE) |
| `201` | Created (POST) |
| `204` | No Content (DELETE with no return) |
| `206` | Partial Content (paginated) |
| `400` | Bad Request |
| `401` | Unauthorized |
| `403` | Forbidden |
| `404` | Not Found |
| `405` | Method Not Allowed |
| `406` | Not Acceptable |
| `409` | Conflict (constraint violation) |
| `416` | Range Not Satisfiable |
| `500` | Internal Server Error |

## Error Response Format

```json
{
  "code": "PGRST301",
  "message": "Could not find a relationship between 'orders' and 'customers'",
  "details": null,
  "hint": "Check that the foreign key exists and is accessible"
}
```

## GraphQL API

Postrust provides a full GraphQL API that mirrors the REST API functionality. The GraphQL schema is dynamically generated from the database schema.

### GraphQL Playground

Access the interactive GraphQL Playground by visiting `/graphql` in your browser:

```
http://localhost:3000/graphql
```

### Query Structure

The GraphQL schema provides:

- **Query type**: Read operations for all exposed tables
- **Mutation type**: Create/Update/Delete operations for all mutable tables
- **Object types**: One per table with fields for each column
- **Relationship fields**: Nested objects for foreign key relationships
- **Filter inputs**: Type-safe filtering matching REST operators
- **Order/Pagination**: Sorting and pagination arguments

### Queries

#### List Query

Query all rows from a table:

```graphql
query {
  users {
    id
    name
    email
    createdAt
  }
}
```

#### Query by Primary Key

Query a single row by primary key:

```graphql
query {
  userByPk(id: 1) {
    id
    name
    email
  }
}
```

#### Filtering

Apply filters using typed filter inputs:

```graphql
query {
  users(filter: {
    status: { eq: "active" },
    age: { gte: 18 }
  }) {
    id
    name
  }
}
```

Filter operators:

| Operator | Description |
|----------|-------------|
| `eq` | Equal |
| `neq` | Not equal |
| `gt` | Greater than |
| `gte` | Greater than or equal |
| `lt` | Less than |
| `lte` | Less than or equal |
| `like` | SQL LIKE pattern |
| `ilike` | Case-insensitive LIKE |
| `in` | Value in list |
| `isNull` | Is null check |

#### Combining Filters

Use `and`, `or`, and `not` for complex conditions:

```graphql
query {
  users(filter: {
    or: [
      { status: { eq: "active" } },
      { role: { eq: "admin" } }
    ]
  }) {
    id
    name
  }
}
```

#### Ordering

Sort results with `orderBy`:

```graphql
query {
  users(orderBy: ["createdAt_DESC", "name_ASC"]) {
    id
    name
    createdAt
  }
}
```

#### Pagination

Limit and offset results:

```graphql
query {
  users(limit: 10, offset: 20) {
    id
    name
  }
}
```

#### Nested Relationships

Query related data through foreign keys:

```graphql
query {
  orders {
    id
    total
    status
    customer {
      name
      email
    }
    items {
      quantity
      product {
        name
        price
      }
    }
  }
}
```

### Mutations

#### Insert

Insert one or more rows:

```graphql
mutation {
  insertUsers(objects: [
    { name: "John", email: "john@example.com" },
    { name: "Jane", email: "jane@example.com" }
  ]) {
    id
    name
    email
  }
}
```

Insert a single row:

```graphql
mutation {
  insertUsersOne(object: { name: "John", email: "john@example.com" }) {
    id
    name
  }
}
```

#### Update

Update rows matching a filter:

```graphql
mutation {
  updateUsers(
    where: { status: { eq: "pending" } },
    set: { status: "active" }
  ) {
    id
    name
    status
  }
}
```

Update by primary key:

```graphql
mutation {
  updateUsersByPk(
    id: 1,
    set: { name: "Updated Name" }
  ) {
    id
    name
  }
}
```

#### Delete

Delete rows matching a filter:

```graphql
mutation {
  deleteUsers(where: { status: { eq: "deleted" } }) {
    id
    name
  }
}
```

Delete by primary key:

```graphql
mutation {
  deleteUsersByPk(id: 1) {
    id
    name
  }
}
```

### Type Mapping

PostgreSQL types are mapped to GraphQL types:

| PostgreSQL | GraphQL |
|------------|---------|
| `integer`, `int4`, `int2`, `smallint` | `Int` |
| `bigint`, `int8` | `BigInt` |
| `real`, `float4`, `float8`, `double precision` | `Float` |
| `numeric`, `decimal` | `BigDecimal` |
| `boolean` | `Boolean` |
| `text`, `varchar`, `char` | `String` |
| `json`, `jsonb` | `JSON` |
| `uuid` | `UUID` |
| `timestamp`, `timestamptz` | `DateTime` |
| `date` | `Date` |
| `time`, `timetz` | `Time` |
| `_type` (arrays) | `[InnerType]` |

### Authentication

GraphQL requests use the same JWT authentication as REST:

```bash
curl -X POST http://localhost:3000/graphql \
  -H "Authorization: Bearer eyJhbGciOiJIUzI1NiIs..." \
  -H "Content-Type: application/json" \
  -d '{"query": "{ users { id name } }"}'
```

The JWT role is used for PostgreSQL Row-Level Security, just like REST requests.

### Introspection

The GraphQL schema supports full introspection:

```graphql
query {
  __schema {
    types {
      name
      fields {
        name
        type {
          name
        }
      }
    }
  }
}
```

### GraphQL vs REST Comparison

| Feature | REST | GraphQL |
|---------|------|---------|
| Endpoint | Multiple (`/users`, `/orders`) | Single (`/graphql`) |
| Field selection | `?select=id,name` | Query fields |
| Filtering | `?status=eq.active` | `filter: { status: { eq: "active" } }` |
| Relationships | `?select=*,customer(*)` | Nested fields |
| Pagination | `?limit=10&offset=20` | `limit: 10, offset: 20` |
| Multiple resources | Multiple requests | Single query |
| Response shape | Fixed | Matches query |

## Admin UI

The Admin UI is an optional feature that provides development tools and API documentation. To enable it, build with the `admin-ui` feature:

```bash
cargo build --release -p postrust-server --features admin-ui
```

### Admin Dashboard

The admin dashboard at `/admin` provides:

- Quick stats: Tables, functions, and relationships in your schema
- Links to all admin tools
- Modern dark-themed interface

### OpenAPI Specification

Access the OpenAPI 3.0 specification at `/admin/openapi.json`:

```bash
curl http://localhost:3000/admin/openapi.json
```

The spec documents:

- All REST endpoints (tables, RPC, GraphQL)
- Request/response schemas
- Filter operators
- Authentication requirements

### Swagger UI

Interactive API documentation at `/admin/swagger`:

- Test API endpoints directly from the browser
- View request/response schemas
- See example payloads
- Powered by Swagger UI 5.x (CDN)

### Scalar

Modern API documentation at `/admin/scalar`:

- Clean, modern interface
- Alternative to Swagger UI
- Same OpenAPI specification
- Powered by Scalar (CDN)

### GraphQL Playground

Interactive GraphQL IDE at `/admin/graphql`:

- Write and test GraphQL queries
- Schema explorer with documentation
- Query history
- Variable editor
- Powered by GraphQL Playground
