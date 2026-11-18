# PostgREST compatibility mode

Compatibility mode is for putting Postrust in front of a database that a
PostgREST deployment, and the clients written against it, already use. It
moves the REST API to the paths PostgREST serves, reshapes RPC responses the
way PostgREST shapes them, and answers database errors in PostgreSQL's own
words, so that those clients keep working without being changed.

Do not turn it on for a new application, or because one of its behaviours is
wanted. Several of the things it matches are PostgREST quirks that Postrust
avoids by default -- a `jsonb` response with a stray byte in front of it, an
`/` that is an API call instead of a directory of endpoints -- and the
behaviours that are worth having can be had one at a time; see
[Taking one behaviour without the rest](#taking-one-behaviour-without-the-rest).

## Turning it on

```bash
PGRST_COMPAT_MODE=true        # or POSTRUST_COMPAT_MODE=true
```

Both names are read; the value is a boolean, and anything else stops start-up.

One thing cannot be switched on at run time: the order of keys within an
object. PostgREST returns them in `select` order, Postrust alphabetically, and
which one a binary does is fixed when it is compiled, by the `compat-key-order`
Cargo feature. See [Key ordering](configuration.md#key-ordering-a-build-time-choice).
The feature is accepted by `postrust-server` and `postrust-worker`.

At start-up compatibility mode logs
`PostgREST compatibility mode enabled: REST surface also served at /`, and, if
the binary was built without `compat-key-order`, a warning that keys will come
back in alphabetical order. If a route given to `Builder::nest` would answer
where a table's name, or `rpc`, would now be served at the root, it logs a
warning naming the route and saying the table is still served at `/api/<table>`.

## What it changes

| Behaviour | Default mode | Compatibility mode | Take it alone with |
|---|---|---|---|
| REST paths | `/api/<table>`, `/api/rpc/<fn>` | The same, and also `/<table>`, `/rpc/<fn>`: the REST API is the router's fallback, so any path nothing else claims is read as a table | Nothing; `/api` is the only prefix without compat mode |
| `GET /` | A JSON directory of this server's endpoints (`name`, `version`, `api`, `custom`, `health`, `admin`, `docs`) | The REST API's own answer for `/`: the schema description, or the refusal for an `Accept-Profile` naming a schema that is not exposed | Nothing |
| RPC response shape | `[{"<fn>": ...}]` | A bare object or scalar for a function that returns one row, a top-level array for a set-returning one | `PGRST_RPC_UNWRAP=true` |
| Database errors | Sanitised message and a Postrust code | PostgreSQL's SQLSTATE, message, details and hint | `PGRST_DB_ERRORS_VERBATIM` |
| Error statuses | Mapped by SQLSTATE, with `P0002` a 404 | The same, except that `P0002` is a 500 | `PGRST_DB_ERROR_STATUS` |
| `jsonb` from a media-type handler | The JSON, as it is | A `0x01` byte in front of it | Nothing |
| OpenAPI document | Paths with the `/api` prefix | Paths without it | Nothing |
| `OPTIONS` preflight | Answers for paths under `/api` | Also answers for the REST paths at the root | Nothing |
| Object key order | Alphabetical | Alphabetical, unless built with `compat-key-order` | The build feature; independent of the mode |

The RPC shape, with the same function in both modes:

```bash
# Default mode
curl -X POST http://localhost:3000/api/rpc/get_statistics
# -> [{"get_statistics": {"users": 10}}]

# Compatibility mode (PGRST_COMPAT_MODE=true)
curl -X POST http://localhost:3000/rpc/get_statistics
# -> {"users": 10}
```

Un-nesting leaves alone a function that returns a composite type or a
`RETURNS TABLE`: those already have columns of their own, even if one of them
shares the function's name.

The `0x01` byte is what PostgREST emits when a handler's `jsonb` result is read
as binary, and its own test suite records it as a bug ("SOH (start of heading)
is being added to results"). It makes the body invalid JSON. Compatibility mode
reproduces it for clients that were written against those bytes, and nothing
else does.

Not everything is affected. The GraphQL API is Hasura's dialect, does not read
the compatibility setting, and stays at `/api/graphql` in both modes. `/admin`,
`/_` (custom routes and `/_/health`) and the other endpoints the server answers
itself keep their places.

## Error statuses and bodies

| Case | Default mode | Compatibility mode |
|---|---|---|
| Body of a database error | `code` a Postrust code, `message` sanitised; `details` and `hint` as PostgreSQL gave them | `code`, `message`, `details` and `hint` all PostgreSQL's own |
| `P0002` (`no_data_found`) | 404 | 500, as PostgREST answers |

`PGRST_DB_ERRORS_VERBATIM` gives the body of the second column for all errors
or for the SQLSTATEs it lists, in either mode. `PGRST_DB_ERROR_STATUS=P0002:500`
gives default mode PostgREST's status, and an explicit mapping applies in
compatibility mode too, so `P0002:404` there gives the other answer.

The same in both modes:

- **A missing grant (`42501`)** is a 401 with `WWW-Authenticate: Bearer` to an
  anonymous request and a 403 to an authenticated one, meaning a request that
  runs as a role other than the configured anonymous role. PostgREST decides
  it the same way.
- **A refusal raised with `RAISE sqlstate 'PGRST'`** answers with the status,
  headers and body the function wrote, unsanitised either way. Its `DETAIL`
  may carry a `body` key that supplies the whole response body verbatim;
  PostgREST ignores the key. See
  [Errors raised by a function](api-reference.md#errors-raised-by-a-function).
- **`PT<nnn>` codes** keep the status they name.
- **Everything that is not a database error**, such as a bad request or an
  invalid token, has the same body and status.

## Taking one behaviour without the rest

| Variable | Gives you |
|---|---|
| `PGRST_RPC_UNWRAP` (alias `POSTRUST_RPC_UNWRAP`) | `true` answers `POST /rpc/<fn>` as PostgREST does: a bare object or scalar, or a top-level array for a set. Compatibility mode implies it. The `0x01` byte is not part of it. Default `false` |
| `PGRST_DB_ERRORS_VERBATIM` | `true` for every database error in PostgreSQL's words, or a comma-separated list of SQLSTATEs and prefixes ending in `*`, such as `P0001,PT*`. Compatibility mode implies `true` |
| `PGRST_DB_ERROR_STATUS` (alias `POSTRUST_DB_ERROR_STATUS`) | A comma-separated list of `PATTERN:STATUS`, such as `P0002:404,23*:422`. A pattern is an exact SQLSTATE or a prefix ending in `*`, as above; a status is 400 to 599; the most specific matching pattern wins. Applies in both modes. A `RAISE sqlstate 'PGRST'` takeover and a `PT<nnn>` code still win over it |

Clients that expect PostgREST's RPC shape and real error messages, on the paths
they already use under `/api`:

```bash
# .env
PGRST_RPC_UNWRAP=true
PGRST_DB_ERRORS_VERBATIM=true
# Keep the default status for a function that finds nothing, but have a unique
# violation answer 422 for the form that posts to it
PGRST_DB_ERROR_STATUS=23505:422
```

Nothing here moves the REST API to the root. The paths, the `GET /` directory,
the OpenAPI document and the `jsonb` byte are the default mode's. Only
compatibility mode changes those, and all of them together.

## What the conformance figures cover

[PostgREST conformance](postgrest-conformance.md) measures Postrust against
PostgREST itself, and the figures there are for the compatibility build: the
harness runs with `PGRST_COMPAT_MODE=true`, verbatim database errors, and
`--features admin-ui,compat-key-order`. Default mode is not what is measured,
so where the two differ -- the paths, the RPC shape, the error bodies, the
`P0002` status -- the figures say nothing about default mode's agreement with
PostgREST on them. The GraphQL API is measured separately, against
graphql-engine, in [Hasura conformance](hasura-conformance.md).

## Moving on or off compatibility mode

Going on, from a default deployment:

- **Paths.** Clients can drop the `/api` prefix; `/api/...` keeps working. Anything
  that used `GET /` as a directory of endpoints now gets the REST API's answer;
  use `/_/health` for health checks.
- **RPC shape.** `[{"<fn>": ...}]` becomes a bare value or an array. Change the
  clients before the switch, or set `PGRST_RPC_UNWRAP` first and move them in
  step.
- **Error bodies.** Clients that read `message` get PostgreSQL's, which may name
  a constraint or a table. Check that you are content to expose that.
- **`P0002`.** A function that raises `no_data_found` goes from 404 to 500. Pin
  it with `PGRST_DB_ERROR_STATUS=P0002:404` if clients depend on it.
- **Key order.** Build with `compat-key-order` if bodies are compared as bytes.
- **Media handlers.** A `jsonb` result from a media-type handler gains the
  `0x01` byte; a client that parses it as JSON will refuse it.
- **OpenAPI.** Generated clients see paths without `/api`; regenerate them.

Going off is the same list reversed. The paths at the root stop answering, so
look for clients still using them in your access logs first, and move the
base URL of any OpenAPI-generated client back to `/api`.
