# Configuration

Postrust is configured entirely through environment variables, making it easy to deploy in containerized and serverless environments.

## Required Variables

| Variable | Description | Example |
|----------|-------------|---------|
| `DATABASE_URL` | PostgreSQL connection string | `postgres://user:pass@host:5432/db` |

## Database Settings

| Variable | Description | Default |
|----------|-------------|---------|
| `PGRST_DB_URI` | Connection URI. `DATABASE_URL` is accepted too and wins where both are set | `postgresql://localhost/postgres` |
| `PGRST_DB_SCHEMAS` | Comma-separated list of schemas to expose | `public` |
| `PGRST_DB_ANON_ROLE` | Role for unauthenticated requests | (none) |
| `PGRST_DB_POOL_SIZE` | Connection pool size. `PGRST_DB_POOL` is accepted as PostgREST spells it | `10` |
| `PGRST_DB_POOL_TIMEOUT` | Seconds to wait for a pool connection before giving up | `10` |
| `PGRST_DB_TX_ISOLATION` | Isolation level of the transaction every request runs in: `read committed`, `repeatable read` or `serializable` | `read committed` |
| `PGRST_DB_EXTRA_SEARCH_PATH` | Comma-separated schemas appended to each request's `search_path`. Not exposed as endpoints — this is where a function or type that an exposed schema references is allowed to live | `public` |
| `PGRST_DB_AGGREGATES_ENABLED` | Allow aggregate functions in `select`. Off by default, as in PostgREST: an aggregate over a large table costs far more than the request looks like it asks for | `false` |
| `PGRST_DB_PREPARED_STATEMENTS` | Use prepared statements. Set `false` behind a connection pooler in transaction mode (PgBouncer), where the connection a statement was prepared on is not the one the next query lands on | `true` |
| `PGRST_DB_PRE_REQUEST` | Function called on the request's transaction before its own query, after every setting is applied. Anything it raises aborts the request. Schema-qualify it as `auth.check` | (none) |
| `PGRST_DB_CHANNEL` | LISTEN channel for schema cache reloads | `pgrst` |
| `PGRST_DB_CHANNEL_ENABLED` | Reload the schema cache on `NOTIFY <channel>` instead of on a restart | `false` |

`PGRST_DB_MAX_ROWS` is under [Request Limits](#request-limits).

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
| `PGRST_JWT_CACHE_ENABLED` | Cache validated tokens, so a repeated token is not verified again | `true` |
| `PGRST_JWT_CACHE_MAX_LIFETIME` | How long a cached validation may be reused, in seconds. A token's own `exp` bounds this — the cache never answers with a token past its expiry. `0` turns the cache off | `3600` |

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

## Hasura Authentication

For the GraphQL surface at `/v1/graphql`. These are what let a client written
against Hasura keep sending the headers it already sends.

| Variable | Description | Default |
|----------|-------------|---------|
| `PGRST_HASURA_ADMIN_SECRET` | Shared secret authenticating an administrator (alias: `HASURA_GRAPHQL_ADMIN_SECRET`) | (none) |
| `PGRST_HASURA_UNAUTHORIZED_ROLE` | Role for a request nothing authenticated (alias: `HASURA_GRAPHQL_UNAUTHORIZED_ROLE`) | (none) |

Both spellings are read, so a deployment migrating from Hasura can keep the
names already in its compose file; the `PGRST_` name wins where both are set.

### What the secret switches on

Set it, and a caller that holds it is an administrator — and an administrator
may ask to be treated as someone else:

```bash
curl localhost:3000/v1/graphql \
  -H 'X-Hasura-Admin-Secret: shh' \
  -H 'X-Hasura-Role: user' \
  -H 'X-Hasura-User-Id: 1' \
  -d '{"query":"{ article { id title } }"}'
```

That request is answered as `user`, and `x-hasura-user-id` becomes a session
variable a row-level policy can read as `current_setting('hasura.user_id')` —
or that a function taking `hasura_session json` receives whole, alongside
`x-hasura-role`. The role is also available on its own as
`current_setting('hasura.role')`.

A Hasura role is not a database role. `Artist` and `anonymous` need not exist
in any catalogue: what the role decides is what `x-hasura-role` reads as, and
(once permissions land) which rules apply. Which database user the transaction
runs as is still `PGRST_DB_ANON_ROLE` or the JWT's role claim.

### Choosing a role with a token

A token that allows more than one identity carries both claims Hasura mints for
it: `x-hasura-default-role`, who the caller is when it asks for nothing, and
`x-hasura-allowed-roles`, the list it may ask for instead.

```json
{
  "https://hasura.io/jwt/claims": {
    "x-hasura-default-role": "user",
    "x-hasura-allowed-roles": ["user", "editor"],
    "x-hasura-user-id": "1"
  }
}
```

The asking is done with a header, and no admin secret is needed for it:

```bash
curl localhost:3000/v1/graphql \
  -H 'Authorization: Bearer <token>' \
  -H 'X-Hasura-Role: editor' \
  -d '{"query":"{ article { id title } }"}'
```

This is the one header read without the secret beside it, and the list is why
it is safe to read: it sits inside the signature, so a caller may choose among
the identities it was issued and cannot add one. Asking for a role the token
does not list is refused:

```json
{"errors":[{"message":"Your requested role is not in allowed roles",
            "extensions":{"path":"$","code":"access-denied"}}]}
```

A token carrying no list allows only the role it already names — an absent list
is not permission to be anyone. Every other `x-hasura-*` header is still
ignored on this path, for the reason below.

### One place this deliberately differs from Hasura

Hasura with no admin secret configured treats every caller as an
administrator — which also means an unsecured deployment lets any caller name
its own role and its own identity. Postrust does not:

```bash
# No PGRST_HASURA_ADMIN_SECRET set. These headers are ignored entirely.
curl localhost:3000/v1/graphql \
  -H 'X-Hasura-Role: user' -H 'X-Hasura-User-Id: 1' ...
```

With no secret configured, `x-hasura-*` headers carry no weight and session
variables come only from a verified token. A policy reading a value the caller
chose is not a policy, and the failure is silent — the query succeeds, against
the wrong rows.

### What a request with no credentials gets

With a secret configured and none offered, the request is refused:

```json
{"errors":[{"message":"\"x-hasura-admin-secret\" required, but not found",
            "extensions":{"path":"$","code":"access-denied"}}]}
```

Set `PGRST_HASURA_UNAUTHORIZED_ROLE` to answer such a request as a named role
instead — with no session variables, since nothing established any. A wrong
secret is always refused rather than falling back to that role: a caller that
tried to be an administrator and failed is not then treated as a stranger.

## Server Settings

| Variable | Description | Default |
|----------|-------------|---------|
| `PGRST_SERVER_HOST` | Server bind address | `127.0.0.1` |
| `PGRST_SERVER_PORT` | Server port | `3000` |
| `PGRST_SERVER_CORS_ORIGINS` | Allowed CORS origins | `*` |
| `PGRST_SERVER_UNIX_SOCKET` | Listen on a Unix domain socket instead of host and port. A stale socket file left by a previous run is removed; anything at the path that is not a socket is refused | (none) |
| `PGRST_ADMIN_SERVER_PORT` | Serve `/live`, `/ready` and `/health` on a second port, so a probe does not need to reach the API | (none) |

### CORS Configuration

```bash
# Allow specific origins
PGRST_SERVER_CORS_ORIGINS="https://example.com,https://app.example.com"

# Allow all origins (development only)
PGRST_SERVER_CORS_ORIGINS="*"
```

An origin must be given as a scheme and host — `https://example.com`, not
`example.com` — because that is what a browser puts in the `Origin` header and
the comparison is exact. Naming any origin restricts the policy to that list:
a request from anywhere else receives no `Access-Control-Allow-Origin` and the
browser refuses it. The default, and `*`, allow every origin.

## Compatibility Settings

How closely Postrust matches PostgREST is measured against PostgREST itself;
see [PostgREST conformance](postgrest-conformance.md) for what is covered and
where the two differ deliberately.

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
| `RUST_LOG` | Detailed Rust logging. Wins where both are set | (none) |
| `PGRST_DEBUG` | Include the database's own error detail, hint and constraint name in error responses. Off in production: those strings describe the schema | `false` |

### Log Levels

- `crit` - Errors only, as `error` (accepted for PostgREST parity)
- `error` - Only errors
- `warn` - Warnings and errors
- `info` - General information (default)
- `debug` - Detailed debugging

`RUST_LOG` takes a full [tracing filter](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html)
and can name targets and spans individually, so it overrides `PGRST_LOG_LEVEL`
rather than combining with it.

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

## OpenAPI

| Variable | Description | Default |
|----------|-------------|---------|
| `PGRST_OPENAPI_MODE` | `disabled`, `follow-privileges` or `ignore-privileges` | `follow-privileges` |
| `PGRST_OPENAPI_SERVER_PROXY_URI` | Address to advertise in the specification's `servers`, for when the server sits behind a reverse proxy and its own address is not the one clients use | (none) |

The specification is served at `/admin/openapi.json` and requires the
`admin-ui` feature.

`disabled` answers that path with 404 — there is no specification to serve, not
an empty one. The other two both list a path per exposed table:
`follow-privileges` gives each table only the operations it permits, which is
what `information_schema.table_privileges` said when the schema cache was
loaded; `ignore-privileges` lists all of them regardless, documenting the API
rather than the caller.

Those privileges are the connecting role's, read once at load. They are not
re-evaluated per request against the role in a JWT.

PostgREST also has a `security-definer` mode, where the specification comes from
a user-supplied `SECURITY DEFINER` function. There is no configuration here for
naming that function, so the value is rejected rather than accepted and quietly
treated as one of the others.

## Role Settings

| Variable | Description | Default |
|----------|-------------|---------|
| `PGRST_ROLE_SETTINGS` | JSON object mapping a role to settings applied to its requests | `{}` |

```bash
PGRST_ROLE_SETTINGS='{"web_user":{"isolation_level":"serializable","statement_timeout":5000}}'
```

`isolation_level` takes the same spellings as `PGRST_DB_TX_ISOLATION` and
overrides it for that role. `statement_timeout` is in milliseconds and is
applied as a `SET LOCAL` on the request's transaction.

JSON rather than one variable per role, because a PostgreSQL role name may hold
characters an environment variable name cannot. A value that does not parse is
rejected whole, with a warning, rather than half-applied.

## App Settings

| Variable | Description |
|----------|-------------|
| `PGRST_APP_SETTINGS_<NAME>` | Exposed to every request as `app.settings.<name>` |

```bash
PGRST_APP_SETTINGS_JWT_LIFETIME=3600
```

Read back in a function or a row-level policy:

```sql
current_setting('app.settings.jwt_lifetime', true)
```

The name is lower-cased, which is the form PostgREST uses and the one a policy
expects.

## GraphQL Subscriptions

| Variable | Description | Default |
|----------|-------------|---------|
| `PGRST_SUBSCRIPTION_REFRESH` | How often a live query re-reads itself with nothing having notified it, in seconds. `0` turns the refresh off and leaves only the notifications. | `30` |

A subscription is a live query: it is woken by the trigger on the table it
reads, which is instant and costs nothing while nothing is written. The refresh
is a floor under what a trigger cannot see — a view has none, an embedded row
may live in a table that carries none, and a predicate written against the
clock changes with no write at all. Every tick costs one query per subscriber,
so raise it, or set it to `0`, where every subscribable table has a trigger and
nothing depends on time. See [Realtime](./realtime.md).

## GraphQL Names

| Variable | Description | Default |
|----------|-------------|---------|
| `PGRST_GRAPHQL_METADATA` | Names for tables, columns, root fields, relationships and computed fields that the schema cannot supply; which root a function is exposed on; and what each role may do with each table. A JSON document, or a path to a file holding one. Also read as `PGRST_GRAPHQL_NAMES`, which is what it was called when names were all it carried. | unset (every name derived, no permission layer) |

Almost everything in the generated GraphQL API is derived: a table's name gives
its root fields, a foreign key gives a relationship, a function gives a
computed field. Hasura derives none of it — every one of those names is written
into metadata by a person, and reflection cannot recover a name nobody wrote
down. This is where those names go when a schema is migrated from one.

```json
{
  "public.author": {
    "name": "Authors",
    "roots": { "select_by_pk": "Author", "select_aggregate": "AuthorAgg" },
    "columns": { "id": "AuthorId" },
    "relationships": {
      "article_author_id_fkey": "posts",
      "fetch_articles_plain": "get_articles"
    },
    "computed_fields": { "author_upper_name": "upper_name" }
  }
}
```

- **`name`** replaces the base name every root field and type is built from, so
  `Authors`, `Authors_by_pk`, `insert_Authors`, `Authors_bool_exp` and the rest
  all follow from this one entry.
- **`roots`** names a single root field, for the cases a base name cannot
  reach: Hasura names each root independently, and `select_by_pk: Author`
  beside `select: Authors` is not a pair one word derives. The keys are
  Hasura's own — `select`, `select_by_pk`, `select_aggregate`, `insert`,
  `insert_one`, `update`, `update_by_pk`, `delete`, `delete_by_pk`. Only the
  field is renamed; the types keep the base name, which is what a generated
  client reads them as.
- **`columns`** exposes a column under another name, keyed by the column. This
  is the one entry that reaches everywhere: the field is renamed in the rows
  that come back, in `where`, in `order_by`, in `distinct_on`, in the key
  arguments, in `_set` and `objects`, in `on_conflict.update_columns` and
  inside every embed and aggregate. The database keeps its own name throughout.
- **`relationships`** is keyed by *constraint* name, or by *function* name for a
  computed relationship, because a constraint names exactly one relationship
  even where two of them point at the same table. The name being replaced would
  not: that is what this is for.
- **`computed_fields`** is keyed by the function behind the field.
- **`comments`** carries the descriptions Hasura keeps in metadata rather than
  in the database, under `table`, `columns`, `roots` and `computed_fields`. A
  description given here replaces the database's own comment; an **empty
  string** removes it, which is how `set_table_customization` says a field has
  no description at all. Nothing said leaves the comment — or the generated
  default — alone.

```json
{
  "public.author": {
    "comments": {
      "table": "Everyone who has written something",
      "columns": { "name": "As it should be printed" },
      "roots": { "select": "Every author" },
      "computed_fields": { "author_upper_name": "Shouted" }
    }
  }
}
```

Keys are `schema.table`, so a table in the default schema is still
`public.author`. A table absent from the document is exposed exactly as before.

### What each role may do

The one entry that is not a name. A permission is not derived from anything, so
unlike every other key here it is written whether or not it differs from a
default — a role the document says nothing about has no access at all.

```json
{
  "public.article": {
    "permissions": {
      "user": {
        "select": {
          "columns": ["id", "title", "content"],
          "filter": { "$or": [{ "author_id": "X-Hasura-User-Id" },
                              { "is_published": true }] },
          "limit": 10,
          "allow_aggregations": true,
          "computed_fields": ["get_articles"]
        },
        "insert": {
          "columns": "*",
          "check": { "author_id": "X-Hasura-User-Id" },
          "set": { "author_id": "X-Hasura-User-Id" },
          "backend_only": false
        },
        "update": { "columns": ["title"], "filter": {}, "check": {} },
        "delete": { "filter": { "is_published": false } }
      },
      "anonymous": {
        "select": { "columns": "*", "filter": { "is_published": true } }
      }
    }
  }
}
```

- **`columns`** is a list, or `"*"` for every column the table has. The two
  differ for a column added later: `"*"` covers it and a list does not. A
  column outside the set is not merely unreadable — it is absent from the type,
  which is what makes a permission a statement about the schema rather than
  about a request.
- **`filter`** and **`check`** are boolean expressions in the same shape a
  `where` argument takes, with one addition: a string like `X-Hasura-User-Id`
  stands for the caller's session variable of that name. `filter` chooses rows
  before the operation; `check` is what a written row must satisfy after it.
- **`set`** fills columns in from the server, overriding whatever the request
  said — which is how `author_id` comes from the caller's identity rather than
  from the caller.
- **`limit`** is a ceiling, not a default: a request asking for more gets this,
  and one asking for fewer gets what it asked for.
- **`allow_aggregations`** is separate from reading rows because counting rows
  you cannot see is a way of seeing them.
- **`backend_only`** hides a mutation from anything but a caller that proved it
  holds the admin secret (see [Hasura Authentication](#hasura-authentication)),
  whatever role it then claims.

Absent entries mean different things at different levels, and the difference
matters:

| | meaning |
|---|---|
| no `permissions` key anywhere in the document | no permission layer at all — database roles and RLS, as before |
| a role absent from a table's `permissions` | that role cannot touch that table |
| `select` absent for a role that has `insert` | it may write rows it cannot read |
| `columns` absent from a write permission | every column, which is Hasura's own default |

The third column of that table is why one permission anywhere turns the layer
on for every table: a document granting `user` a filtered view of `article` and
saying nothing about `author` is saying `user` cannot read `author` — not that
`author` is open to everyone.

`scripts/hasura-names.py` converts these from Hasura's own metadata alongside
the names, from a running engine, a metadata directory, an export, or a list of
migration commands.

### Where a function is exposed

A function that answers with rows of a table is a root field, and which root it
goes on follows from what PostgreSQL says it does: `VOLATILE` may write, so it
is a mutation; `STABLE` and `IMMUTABLE` may not, so they are queries. Hasura
lets metadata override that, and a `VOLATILE` function tracked with
`exposed_as: query` is a decision a person made that no catalogue remembers.

Written down, the document grows a second section — and the table names move
into one of their own, which is the only shape change. The flat document above
is still read:

```json
{
  "tables": {
    "public.author": { "name": "Authors" }
  },
  "functions": {
    "public.volatile_func1": { "exposed_as": "query" }
  }
}
```

Keys are `schema.function`. `exposed_as` is `query` or `mutation`; a function
absent from the section is placed by its volatility as before.

Set the variable to the document itself, or to a path:

```bash
PGRST_GRAPHQL_NAMES='{"public.author": {"name": "Author"}}'
PGRST_GRAPHQL_NAMES="/etc/postrust/graphql-names.json"
```

### Converting from Hasura

Nobody should write this by hand for a migration — Hasura already has every one
of these names, and `scripts/hasura-names.py` reads them out:

```bash
scripts/hasura-names.py --url http://localhost:8080 --admin-secret "$SECRET" > graphql-names.json
scripts/hasura-names.py --metadata-dir ./metadata > graphql-names.json
scripts/hasura-names.py --file metadata.json > graphql-names.json
```

It emits only the names that differ from what this server derives on its own,
so the document is the exceptions rather than the whole schema — usually a
short file. Where a table's custom root fields all follow from one word, that
word is carried as `name`, because it names the generated *types* as well and a
client reads those too; whatever the word cannot account for is written down
root by root.

It converts names, descriptions and function placement, and nothing else:
permissions, tracked-table lists, actions, remote schemas and event triggers
are the metadata model rather than names, and this server does not have one.

Relationships can be keyed by constraint name, by the foreign key column, or by
`table.column` on the far side. That is what lets the converter work from the
metadata alone: Hasura names a relationship by its column, and turning a column
into the constraint that carries it would otherwise need a database connection.

This is a lookup table, not a metadata model. It grants no permissions and
tracks no tables; what a client sees something called is all it can change. A document that cannot be read
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

## There is no configuration file, and no `.env`

Every setting above is read from the process environment, by
`AppConfig::from_env`. Nothing reads a `.env` file — the server has no dotenv
dependency — so a `.env` sitting next to the binary is not picked up. Use the
shell, the unit file, or the container runtime:

```bash
# systemd
Environment=PGRST_DB_ANON_ROLE=web_anon

# docker compose
environment:
  PGRST_DB_ANON_ROLE: web_anon

# a shell, for development, if you want a file
set -a && . ./my.env && set +a && ./postrust
```

## What happens to a value it cannot use

There is no separate validation pass, and no "configuration validated" line.
Two things happen instead.

**A value that cannot be parsed is ignored, with a warning naming it**, and the
default stands:

```
WARN postrust_core::config: Ignoring PGRST_DB_POOL_SIZE="lots": expected a positive integer
```

A rejected value and an unset one are different mistakes, and the operator can
only fix the one they are told about. This is why the log subscriber is
installed before the configuration is read.

**Anything that has to be right to start at all fails the start**, and says so
on the way out: a database URI that will not parse, a database that cannot be
reached, a port already bound, a socket path occupied by something that is not a
socket.
