# Configuration

Postrust is configured entirely through environment variables, making it easy to deploy in containerized and serverless environments.

## Required Variables

| Variable | Description | Example |
|----------|-------------|---------|
| `DATABASE_URL` | PostgreSQL connection string | `postgres://user:pass@host:5432/db` |

## Database Settings

| Variable | Description | Default |
|----------|-------------|---------|
| `PGRST_DB_SCHEMAS` | Comma-separated list of schemas to expose | `public` |
| `PGRST_DB_ANON_ROLE` | Role for unauthenticated requests | (none) |
| `PGRST_DB_POOL_SIZE` | Connection pool size | `10` |
| `PGRST_DB_POOL_TIMEOUT` | Pool timeout in seconds | `30` |
| `PGRST_DB_TX_ISOLATION` | Transaction isolation level | `read committed` |

### Database URL Format

```
postgres://[user[:password]@][host][:port][/database][?options]
```

Examples:
```bash
# Local development
DATABASE_URL="postgres://postgres:postgres@localhost:5432/mydb"

# With SSL
DATABASE_URL="postgres://user:pass@host:5432/db?sslmode=require"

# AWS RDS
DATABASE_URL="postgres://user:pass@mydb.xxx.us-east-1.rds.amazonaws.com:5432/mydb"
```

### Schema Configuration

Expose multiple schemas:

```bash
# Expose public and api schemas
PGRST_DB_SCHEMAS="public,api"
```

Access different schemas via the `Accept-Profile` header:

```bash
curl http://localhost:3000/users \
  -H "Accept-Profile: api"
```

## Authentication Settings

| Variable | Description | Default |
|----------|-------------|---------|
| `PGRST_JWT_SECRET` | Secret for HS256/HS384/HS512 | (none) |
| `PGRST_JWT_SECRET_IS_BASE64` | Is secret base64 encoded? | `false` |
| `PGRST_JWT_AUD` | Required audience claim | (none) |
| `PGRST_JWT_ROLE_CLAIM_KEY` | Claim key containing role | `role` |

### JWT Secret

```bash
# Plain text secret (min 32 characters for HS256)
PGRST_JWT_SECRET="your-super-secret-key-at-least-32-characters"

# Base64 encoded secret
PGRST_JWT_SECRET_IS_BASE64=true
PGRST_JWT_SECRET="eW91ci1zdXBlci1zZWNyZXQta2V5LWF0LWxlYXN0LTMyLWNoYXJhY3RlcnM="
```

### Custom Role Claim

By default, Postrust looks for the role in the `role` claim. Override this:

```bash
# Use nested claim
PGRST_JWT_ROLE_CLAIM_KEY="user.role"

# JWT payload: {"user": {"role": "admin"}}
```

## Server Settings

| Variable | Description | Default |
|----------|-------------|---------|
| `PGRST_SERVER_HOST` | Server bind address | `127.0.0.1` |
| `PGRST_SERVER_PORT` | Server port | `3000` |
| `PGRST_SERVER_CORS_ORIGINS` | Allowed CORS origins | `*` |

### CORS Configuration

```bash
# Allow specific origins
PGRST_SERVER_CORS_ORIGINS="https://example.com,https://app.example.com"

# Allow all origins (development only)
PGRST_SERVER_CORS_ORIGINS="*"
```

## Compatibility Settings

| Variable | Description | Default |
|----------|-------------|---------|
| `PGRST_COMPAT_MODE` | PostgREST compatibility mode (alias: `POSTRUST_COMPAT_MODE`) | `false` |

Setting `PGRST_COMPAT_MODE=true` makes the API behave more like PostgREST:

- **Canonical paths at the root.** The full REST surface is also served at `/`,
  so `POST /rpc/<name>`, `GET /<table>`, etc. work in addition to the
  `/api`-prefixed paths. Explicit routes (`/`, `/_`, `/admin`, `/api`) still
  take precedence, and GraphQL stays at `/api/graphql`.
- **PostgREST-shaped RPC responses.** Results from `POST /rpc/<name>` are
  un-wrapped: a non-set-returning function returns its bare object/scalar
  (`{...}` / `42`) and a set-returning function returns a top-level array,
  instead of the default `[{"<name>": ...}]` shape.

```bash
# Default mode
curl -X POST http://localhost:3000/api/rpc/get_statistics
# -> [{"get_statistics": {"users": 10}}]

# Compatibility mode (PGRST_COMPAT_MODE=true)
curl -X POST http://localhost:3000/rpc/get_statistics
# -> {"users": 10}
```

### Key ordering: a build-time choice

One difference `PGRST_COMPAT_MODE` cannot switch on is the order of keys within
each object. Postrust returns them alphabetically; PostgREST returns them in the
order of the `select` list.

That is decided when the binary is compiled rather than at run time, because it
depends on the map type used to hold a JSON object, so it is a Cargo feature:

```bash
cargo build --release -p postrust-server --features compat-key-order
```

```bash
# Default build
curl 'http://localhost:3000/api/users?select=status,name,id&limit=1'
# -> [{"id":1,"name":"Alice","status":"active"}]

# Built with --features compat-key-order
curl 'http://localhost:3000/api/users?select=status,name,id&limit=1'
# -> [{"status":"active","name":"Alice","id":1}]
```

It is off by default because it is not free. Measured by running both builds as
containers against the same database and alternating between them, a three-column
response cost 1% and an eight-column response 15%: for small objects a few short
string comparisons beat hashing every key, so the sorted map is genuinely the
faster one. Enable it when byte-level compatibility matters more than that.

Running with `PGRST_COMPAT_MODE=true` on a binary built without the feature logs
a warning at startup, so the difference is not left to be discovered by diffing
responses.

This is opt-in so existing deployments keep their current behavior. See
"Differences from PostgREST" in the README for the remaining intentional
differences.

## Logging Settings

| Variable | Description | Default |
|----------|-------------|---------|
| `PGRST_LOG_LEVEL` | Log level | `info` |
| `RUST_LOG` | Detailed Rust logging | (none) |

### Log Levels

- `error` - Only errors
- `warn` - Warnings and errors
- `info` - General information (default)
- `debug` - Detailed debugging
- `trace` - Very verbose

```bash
# Production
PGRST_LOG_LEVEL="warn"

# Development
PGRST_LOG_LEVEL="debug"
RUST_LOG="postrust=debug,sqlx=info"
```

## Request Limits

| Variable | Description | Default |
|----------|-------------|---------|
| `PGRST_MAX_ROWS` | Maximum rows returned by a single request. Caps requests that specify no `limit`, and bounds larger ones. Alias: `PGRST_DB_MAX_ROWS` | unset (unlimited) |
| `PGRST_MAX_BODY_SIZE` | Maximum request body (bytes) | `10485760` |

## GraphQL Names

| Variable | Description | Default |
|----------|-------------|---------|
| `PGRST_GRAPHQL_NAMES` | Names for tables, relationships and computed fields that the schema cannot supply. A JSON document, or a path to a file holding one. | unset (every name derived) |

Almost everything in the generated GraphQL API is derived: a table's name gives
its root fields, a foreign key gives a relationship, a function gives a
computed field. Hasura derives none of it — every one of those names is written
into metadata by a person, and reflection cannot recover a name nobody wrote
down. This is where those names go when a schema is migrated from one.

```json
{
  "public.author": {
    "name": "Author",
    "relationships": {
      "article_author_id_fkey": "posts",
      "fetch_articles_plain": "get_articles"
    },
    "computed_fields": { "author_upper_name": "upper_name" }
  }
}
```

- **`name`** replaces the base name every root field and type is built from, so
  `Author`, `Author_by_pk`, `insert_Author`, `Author_bool_exp` and the rest all
  follow from this one entry.
- **`relationships`** is keyed by *constraint* name, or by *function* name for a
  computed relationship, because a constraint names exactly one relationship
  even where two of them point at the same table. The name being replaced would
  not: that is what this is for.
- **`computed_fields`** is keyed by the function behind the field.

Keys are `schema.table`, so a table in the default schema is still
`public.author`. A table absent from the document is exposed exactly as before.
Set the variable to the document itself, or to a path:

```bash
PGRST_GRAPHQL_NAMES='{"public.author": {"name": "Author"}}'
PGRST_GRAPHQL_NAMES="/etc/postrust/graphql-names.json"
```

This is a lookup table, not a metadata model. It grants no permissions and
tracks no tables; a name is all it can change. A document that cannot be read
or parsed stops the server rather than being ignored — serving derived names
instead would answer every request under a name the client does not send.

## Example Configurations

### Development

```bash
DATABASE_URL="postgres://postgres:postgres@localhost:5432/dev"
PGRST_DB_ANON_ROLE="anon"
PGRST_LOG_LEVEL="debug"
PGRST_SERVER_HOST="0.0.0.0"
```

### Production

```bash
DATABASE_URL="postgres://user:pass@prod-db:5432/app?sslmode=require"
PGRST_DB_SCHEMAS="api"
PGRST_DB_ANON_ROLE="web_anon"
PGRST_DB_POOL_SIZE="20"
PGRST_JWT_SECRET="${JWT_SECRET}"
PGRST_LOG_LEVEL="warn"
PGRST_SERVER_HOST="0.0.0.0"
PGRST_SERVER_PORT="8080"
PGRST_MAX_ROWS="500"
```

### AWS Lambda

```bash
DATABASE_URL="${DATABASE_URL}"
PGRST_DB_ANON_ROLE="web_anon"
PGRST_JWT_SECRET="${JWT_SECRET}"
PGRST_LOG_LEVEL="info"
# Note: Pool size should be small for Lambda
PGRST_DB_POOL_SIZE="1"
```

## Configuration File (Optional)

You can also use a `.env` file:

```bash
# .env
DATABASE_URL=postgres://user:pass@localhost:5432/mydb
PGRST_DB_ANON_ROLE=web_anon
PGRST_JWT_SECRET=your-secret-key
```

Load it automatically:
```bash
# The server reads .env files by default
./postrust
```

## Validation

Postrust validates configuration on startup:

```bash
./postrust

# Output:
# INFO postrust: Configuration validated
# INFO postrust: Connected to database
# INFO postrust: Server listening on 0.0.0.0:3000
```

Common validation errors:

- `DATABASE_URL is required` - Set the database connection string
- `Invalid DATABASE_URL format` - Check the URL syntax
- `JWT_SECRET must be at least 32 characters` - Use a longer secret
- `Unknown schema: xyz` - Schema doesn't exist in database
