# API Reference

Complete reference for the Postrust REST API.

## Endpoints

### Tables and Views

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

### Schema

| Method | Endpoint | Description |
|--------|----------|-------------|
| `GET` | `/` | OpenAPI specification |

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
