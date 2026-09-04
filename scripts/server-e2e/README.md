# Server end-to-end checks

Runs the actual `postrust` binary against an actual PostgreSQL and makes
requests over the network.

```bash
scripts/server-e2e/e2e.sh
```

Self-contained: it starts its own PostgreSQL container, loads
`scripts/init-db.sql` and `scripts/test-fixtures.sql` into it, builds the
server, runs 50 checks, and removes the container on exit.

## Why this exists separately from `cargo test`

The integration tests in `crates/postrust-server/tests/` build a `Router`
in-process and call it through `tower::ServiceExt::oneshot`. That covers the
request path well and covers `main.rs` not at all — no listener is bound, no
CORS layer is applied, no pool is built, no background task is spawned.

Everything configured in `main.rs` was therefore untested, and most of it had
never worked:

| Option | What it did before |
| --- | --- |
| `PGRST_ADMIN_SERVER_PORT` | nothing — no second listener existed |
| `PGRST_SERVER_CORS_ORIGINS` | nothing — CORS was hardcoded to `Any` |
| `PGRST_MAX_BODY_SIZE` | nothing — the limit was a constant in the handler |
| `PGRST_LOG_LEVEL` | nothing — only `RUST_LOG` was read |
| `PGRST_SERVER_UNIX_SOCKET` | nothing |
| `PGRST_DB_CHANNEL_ENABLED` | nothing |
| `PGRST_DB_POOL_SIZE` | nothing — the code read `PGRST_DB_POOL` |
| `PGRST_DB_PRE_REQUEST`, `PGRST_APP_SETTINGS_*`, `PGRST_ROLE_SETTINGS` | nothing |
| `PGRST_JWT_SECRET_IS_BASE64`, `PGRST_JWT_ROLE_CLAIM_KEY` | nothing |
| `PGRST_OPENAPI_MODE`, `PGRST_OPENAPI_SERVER_PROXY_URI` | nothing |

A unit test cannot tell the difference between an option that is applied and
one that is ignored, when the thing applying it is the process itself.

## What the checks are careful about

Several of these are easy to write in a way that passes either way, and the
first drafts of two of them did:

- **The body-size check asserts the reason, not the status.** An oversized
  insert earns a 4xx anyway — a 401 from the anonymous role, for one — so
  `is_client_error()` passes with the limit ignored entirely. The response has
  to say `length limit exceeded`.
- **The `openapi_mode=disabled` check first proves the admin surface is
  mounted.** A build without `admin-ui` 404s on every admin path, which reads
  as a disabled specification.
- **The NOTIFY check has a control either side.** The table is confirmed 404
  *after* `CREATE TABLE` and before the `NOTIFY`, so what is measured is the
  reload rather than the table's existence.
- **The base64-secret check runs the same token against the flag off**, and
  requires a 401. Otherwise a server ignoring the flag passes.

## Options

| Variable | Default | Purpose |
| --- | --- | --- |
| `E2E_DATABASE_URL` | (none) | Use your own database instead of a container |
| `E2E_KEEP_DB` | `0` | Leave the container running afterwards |
| `E2E_DB_PORT` | `55440` | Host port for the container |
| `E2E_DB_IMAGE` | `postgres:16-alpine` | |
| `E2E_BASE_PORT` | `3900` | First of ~17 consecutive ports the servers bind |
| `E2E_SOCKET` | `/tmp/postrust-e2e.sock` | Must be short: `sun_path` is 108 bytes |

## Files

- `e2e.sh` — database, fixtures, build, counters
- `cases.sh` — the checks, sourced by `e2e.sh`
- `mint_jwt.py` — HS256 tokens, hand-rolled so there is nothing to install
