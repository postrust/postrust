# API Reference

Complete reference for the Postrust REST and GraphQL APIs.

> **Note on paths.** By default the REST API is served under `/api` (e.g.
> `GET /api/users`, `POST /api/rpc/my_func`) and GraphQL under `/v1/graphql`.
> The REST endpoints below are shown as root-level paths — these work as-is
> when [compatibility mode](configuration.md#compatibility-settings)
> (`PGRST_COMPAT_MODE=true`) is enabled; otherwise prefix them with `/api`.
> GraphQL is served at `/v1/graphql` either way.

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

By default, RPC results are array-wrapped and keyed by function name
(`[{"my_func": ...}]`). In [compatibility mode](configuration.md#compatibility-settings)
they match PostgREST: a bare object/scalar for non-set-returning functions and a
top-level array for set-returning ones.

### GraphQL

| Method | Endpoint | Description |
|--------|----------|-------------|
| `POST` | `/v1/graphql` | Execute a query or mutation |
| `GET` | `/v1/graphql` | GraphQL Playground |
| `GET` | `/v1/graphql/ws` | Subscriptions over WebSocket |
| `POST` | `/api/graphql` | The same surface, under the REST prefix |

GraphQL is served at both addresses whether or not compatibility mode is on.
`/v1/graphql` is the one a Hasura client can be told about. See
[GraphQL API](#graphql-api) for the schema it serves.

### Schema

| Method | Endpoint | Description |
|--------|----------|-------------|
| `GET` | `/` | Server info document (the OpenAPI spec is served under `/admin` with the `admin-ui` feature) |

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

Postrust serves a GraphQL API generated from the database schema, in the
dialect Hasura speaks. A client generated against Hasura -- its queries, its
codegen output, its endpoint -- points at this server unchanged.

### Endpoints

| Method | Endpoint | Description |
|--------|----------|-------------|
| `POST` | `/v1/graphql` | Execute a query or mutation |
| `GET` | `/v1/graphql` | GraphQL Playground |
| `GET` | `/v1/graphql/ws` | Subscriptions over WebSocket |
| `POST` | `/api/graphql` | The same surface, for anything already pointed here |

`/v1/graphql` is where a Hasura client sends its queries and, for most
generated clients, the only address they can be told about.

### Shape of the schema

For a table `author`, with a to-many relationship `articles`:

| Root field | What it answers |
|---|---|
| `author(where:, order_by:, distinct_on:, limit:, offset:)` | the rows |
| `author_by_pk(id: 1)` | one row, or null |
| `author_aggregate(where: …)` | `aggregate { count sum { … } }` and `nodes { … }` |
| `insert_author(objects:, on_conflict:)` | `affected_rows` and `returning { … }` |
| `insert_author_one(object:, on_conflict:)` | the row written |
| `update_author(where:, _set:, _inc:, …)` | `affected_rows` and `returning { … }` |
| `update_author_by_pk(pk_columns: {id: 1}, _set:)` | the row written |
| `update_author_many(updates: [{where, _set, …}])` | one mutation response per update |
| `delete_author(where:)` | `affected_rows` and `returning { … }` |
| `delete_author_by_pk(id: 1)` | the row removed |

The root types are named `query_root` and `mutation_root`.

### Queries

```graphql
query {
  author(
    where: { name: { _ilike: "%rust%" } }
    order_by: [{ created_at: desc_nulls_last }, { name: asc }]
    limit: 10
    offset: 20
  ) {
    id
    name
    articles(where: { published: { _eq: true } }, limit: 5) {
      id
      title
    }
    articles_aggregate {
      aggregate {
        count
      }
    }
  }
}
```

An embedded list takes the same five arguments the root field does --
`where`, `order_by`, `distinct_on`, `limit` and `offset` -- applied inside the
child's own subselect, so the limit bounds rows per parent and the ordering
happens before it.

A `json` or `jsonb` column takes a `path`, which reads one part of the
document the way `#>` does:

```graphql
{
  events {
    payload
    actor: payload(path: "actor.login")
    first_file: payload(path: "commits[0].files[0]")
    odd_key: payload(path: "['a key with spaces']")
  }
}
```

A leading `$` is the document itself and may be left out; keys are written
bare or after a dot; an index or a quoted key goes in brackets. A step the
document does not have reads null.

#### Filtering

`where` takes a generated `<table>_bool_exp`. Every comparison is named for the
type it applies to, so an unknown operator or an ill-typed operand is refused
by validation rather than by the database.

| Group | Operators |
|---|---|
| Any column | `_eq` `_neq` `_gt` `_gte` `_lt` `_lte` `_in` `_nin` `_is_null` |
| Text | `_like` `_nlike` `_ilike` `_nilike` `_similar` `_nsimilar` `_regex` `_iregex` `_nregex` `_niregex` |
| `json`/`jsonb` | `_contains` `_contained_in` `_has_key` `_has_keys_any` `_has_keys_all` `_jsonb_path_exists` `_jsonb_path_match` `_cast: { String: … }` |
| `ltree` | `_ancestor` `_descendant` `_matches` `_matches_fulltext` and their `_any` forms |
| PostGIS | `_st_contains` `_st_crosses` `_st_equals` `_st_intersects` `_st_overlaps` `_st_touches` `_st_within` `_st_d_within` `_st_3d_d_within` `_cast: { geography: … }` |

A question can also be asked about a whole related set rather than about any
one row of it:

```graphql
{
  authors(where: { books_aggregate: { count: { predicate: { _gt: 2 } } } }) {
    name
  }
}
```

`count` takes the same `columns` and `distinct` the aggregate field does, and a
`filter` narrowing what is counted; `bool_and` and `bool_or` fold one boolean
column, named under `arguments`. Each takes a `predicate`, which is an ordinary
comparison against the number or the truth value the aggregate produced. Over no
related rows at all `count` is zero and the folds are null, which is how
"authors with no books" is written.

Combine them with `_and`, `_or` and `_not`, and follow a relationship by naming
it:

```graphql
query {
  article(
    where: {
      _or: [
        { author: { name: { _eq: "Ada" } } }
        { _and: [{ views: { _gt: 100 } }, { published: { _eq: true } }] }
      ]
    }
  ) {
    id
    title
  }
}
```

#### Ordering

`order_by` takes a **list** of single-key objects, because ordering is ordered:
`{name: asc, id: desc}` is one object whose two keys have no defined
precedence, and the client that wrote it meant name first.

The direction is an enum: `asc`, `desc`, `asc_nulls_first`, `asc_nulls_last`,
`desc_nulls_first`, `desc_nulls_last`. A direction given as null is no
direction, which is how a client makes an ordering optional.

You can order by a related row's column, or by an aggregate of a row's
children:

```graphql
query {
  author(
    order_by: [
      { articles_aggregate: { count: desc } }
      { contact: { phone: asc } }
    ]
  ) {
    name
  }
}
```

#### Aggregates

```graphql
query {
  article_aggregate(where: { author_id: { _eq: 1 } }) {
    aggregate {
      count
      count_distinct: count(columns: [author_id], distinct: true)
      sum { views }
      avg { views }
      max { published_on }
      min { published_on }
      stddev { views }
      variance { views }
    }
    nodes {
      id
      title
      author { name }
    }
  }
}
```

`nodes` is a rows selection like any other: it takes relationships and computed
fields, not only columns.

`count` counts rows. `count(columns: [category])` counts the rows where those
columns are not null, and `distinct: true` counts the distinct values among
them — so a selection may ask for several counts at once, each under its own
name:

```graphql
{
  widgets_aggregate {
    aggregate {
      count
      named: count(columns: [category])
      categories: count(columns: [category], distinct: true)
    }
  }
}
```

### Computed fields

A function of a table's row is a field of that table. Where it takes more than
the row, the rest goes under `args`, so a parameter called `limit` cannot
shadow the one that pages the result:

```graphql
{
  locations {
    location
    distance(args: { from: { type: "Point", coordinates: [0, 0] } })
  }
}
```

The same applies to a function returning rows of another table, which is a
relationship rather than a column: `get_articles(args: {search: "rust"}) { id }`,
and `get_articles_aggregate(args: {search: "rust"})` beside it. A `hasura_session
json` parameter is filled by the server from the caller's verified claims and is
not an argument the client can write.

### Mutations

#### Insert

```graphql
mutation {
  insert_author(
    objects: [
      { name: "Ada", articles: { data: [{ title: "On the Analytical Engine" }] } }
      { name: "Grace" }
    ]
    on_conflict: { constraint: author_name_key, update_columns: [bio] }
  ) {
    affected_rows
    returning {
      id
      name
      articles { id title }
    }
  }
}
```

A nested object writes the related row in the same transaction, in either
direction, and `affected_rows` counts every row written rather than every row
returned. An empty `update_columns` is `DO NOTHING`.

`insert_author_one(object: {...})` is the same thing spelled for one row, and
answers with the row rather than with a mutation response.

#### Update

```graphql
mutation {
  update_article(
    where: { author: { name: { _eq: "Ada" } } }
    _set: { published: true }
    _inc: { views: 1 }
  ) {
    affected_rows
    returning { id title views }
  }
}
```

| Operator | Effect |
|---|---|
| `_set` | replace the column |
| `_inc` | add to a numeric column |
| `_append` / `_prepend` | concatenate onto a `jsonb` column |
| `_delete_key` | remove a key |
| `_delete_elem` | remove an array element by index |
| `_delete_at_path` | remove at a path, given as a list of keys |

Each takes an input object over the table's `jsonb` columns --
`_append: { config: {...} }`, `_delete_key: { config: "stale" }` -- and a
table with no `jsonb` column is not offered them at all.

`update_article_by_pk(pk_columns: {id: 1}, _set: {...})` addresses one row. An
update that changes nothing changes no rows and says so.

`update_article_many(updates: [...])` applies several updates, each with its
own filter, in the order given, and answers with one mutation response per
entry. They share a transaction, so this is not the same as sending them one at
a time:

```graphql
mutation {
  update_article_many(
    updates: [
      { where: { id: { _eq: 1 } }, _set: { title: "First" } }
      { where: { author_id: { _eq: 2 } }, _inc: { views: 1 } }
    ]
  ) {
    affected_rows
  }
}
```

#### Delete

```graphql
mutation {
  delete_article(where: { published: { _eq: false } }) {
    affected_rows
    returning {
      id
      title
      author { name }
    }
  }
}
```

`returning` can carry relationships: the delete runs in a CTE and the
projection reads from it, while the rows it points at are still there.

`where` is required. A bulk delete with no predicate is a delete of the whole
table, so the schema says it is not a query that can be written -- a client's
own tooling catches it before the request is sent.

#### One mutation is one transaction

A mutation may name several root fields, and they are resolved one after
another inside a single transaction. If any of them fails, none of them
happened.

```graphql
mutation {
  insert_order(objects: { customer_id: 1 }) { affected_rows }
  insert_order_line(objects: { order_id: 1, sku: "X" }) { affected_rows }
}
```

### Validation

A request is validated against the schema before it runs, and a document that
cannot mean what it says is refused rather than answered:

```graphql
query ($limit: String) { author(limit: $limit) { id } }
```

`limit` is an `Int`, so this is refused — `variable 'limit' is declared as
'String', but used where 'Int' is expected` — as is a non-null variable given
an explicit null. A nullable variable may stand where a non-null is expected
when either it or the place has a default, since the default is then what fills
it.

A null written into a comparison is refused too: `where: {id: {_eq: null}}`
reads as `id = NULL`, which is never true, so a client that wrote it meant
something the query cannot mean. A variable standing for a null counts; a
variable that was simply not given does not, since an absent variable makes
the comparison itself absent.

Two deliberate exceptions, both of them things Hasura accepts:

- A variable that is *declared and never used*. The specification says a
  document like that is invalid; Hasura executes it, and a client whose filter
  was edited years ago has been getting answers ever since.
- A value written as a string where a number or a boolean is expected --
  `insert_test_types(objects: [{c1_smallint: "32767", c20_boolean: "true"}])`,
  or `article(offset: "1")`. A column's value is read the way PostgreSQL reads
  a literal, which takes either spelling. `limit` is the exception to the
  exception and stays strict, because that is where Hasura draws the line
  too.

### Errors

Errors come back in Hasura's envelope, which client code branches on: there is
no `data` key at all on failure, and `extensions.code` is the machine-readable
half.

```json
{
  "errors": [
    {
      "message": "duplicate key value violates unique constraint \"author_name_key\"",
      "extensions": { "path": "$", "code": "constraint-violation" }
    }
  ]
}
```

The status is 200 for all of it. A GraphQL error is a value in the response
body, not a transport failure.

### Names the schema cannot carry

Hasura keeps some names in metadata rather than in the database: what a
relationship is called, what a computed field is called, what a table's type is
called — and which root a function is exposed on, which its volatility would
otherwise decide. Reflection cannot recover a decision nobody wrote down, so
they are given to the server through `PGRST_GRAPHQL_NAMES`, and
`scripts/hasura-names.py` converts them out of an existing Hasura deployment.
See [Configuration](./configuration.md#graphql-names).

### Type Mapping

Scalars keep their PostgreSQL names, because `query ($x: jsonb!)` names a type
and a client that declares one is naming this.

| PostgreSQL | GraphQL |
|------------|---------|
| `integer`, `int4`, `int2`, `smallint` | `Int` |
| `bigint`, `int8` | `bigint` |
| `real`, `float4`, `float8`, `double precision` | `Float` |
| `numeric`, `decimal` | `numeric` |
| `boolean` | `Boolean` |
| `text`, `varchar`, `char` | `String` |
| `citext` | `citext` |
| `json`, `jsonb` | `json`, `jsonb` |
| `uuid` | `uuid` |
| `timestamp`, `timestamptz` | `timestamp`, `timestamptz` |
| `date`, `time`, `timetz` | `date`, `time`, `timetz` |
| `geometry`, `geography` | `geometry`, `geography` -- GeoJSON in both directions |
| `ltree` | `ltree` |
| a type this server knows nothing about | a scalar of that name |
| `_type` (arrays) | `[InnerType]` |

A table marked as an enumeration in `PGRST_GRAPHQL_NAMES` becomes a GraphQL
enum built from its rows, and every column with a foreign key to it is typed as
that enum.

### Authentication

GraphQL requests use the same JWT authentication as REST:

```bash
curl -X POST http://localhost:3000/v1/graphql \
  -H "Authorization: Bearer eyJhbGciOiJIUzI1NiIs..." \
  -H "Content-Type: application/json" \
  -d '{"query": "{ author { id name } }"}'
```

The JWT role is used for PostgreSQL row-level security, just as for REST. The
`x-hasura-*` claims of a verified token become `SET LOCAL` settings, so a
policy reading `current_setting('hasura.user_id')` sees what a Hasura
permission rule would have seen. Session variables come from the token, never
from request headers: honouring the header would let any caller name its own
identity.

### Introspection

The schema supports full introspection, which is what a codegen tool reads:

```graphql
query {
  __schema {
    queryType { name }
    types {
      name
      fields { name type { name kind } }
    }
  }
}
```

### GraphQL vs REST Comparison

| Feature | REST | GraphQL |
|---------|------|---------|
| Endpoint | Multiple (`/users`, `/orders`) | Single (`/v1/graphql`) |
| Field selection | `?select=id,name` | Query fields |
| Filtering | `?status=eq.active` | `where: { status: { _eq: "active" } }` |
| Relationships | `?select=*,customer(*)` | Nested fields |
| Pagination | `?limit=10&offset=20` | `limit: 10, offset: 20` |
| Multiple resources | Multiple requests | Single query |
| Multiple writes | Multiple requests | One transaction |
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
