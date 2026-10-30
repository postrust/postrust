# Stability and versioning

What a Postrust version number promises, and what it does not.

## Two version lines

The workspace does not ship at a single version. There are two lines, because
the crates in them are at genuinely different stages and one number cannot
honestly describe both.

| Line | Crates | Promise |
| --- | --- | --- |
| **Stable** | `postrust-core`, `postrust-sql`, `postrust-auth`, `postrust-response`, `postrust-engine`, `postrust-graphql`, `postrust-server`, `postrust-lambda` | Semver. A breaking change to the public Rust API needs a major bump. |
| **Stable, versioned on its own** | `postrust-worker` (from 1.0.0) | Semver, over its configuration and behaviour rather than a Rust API. See below. |
| **Stable, versioned on its own** | `postrust-proxy` (from 1.0.0) | Semver, over its Rust API, its configuration, and the HTTP API of its SaaS and admin routers. See below. |

`postrust-proxy` depends on `postrust-core`, not the other way round, and
`postrust-worker` is built as a `cdylib`, which cannot be a Rust library
dependency at all.

Every line releases on the same git tags. A tag publishes whatever version
each crate currently declares and skips what is already on crates.io, so the
proxy is not dragged along to a version it has not earned, and the worker
keeps a number of its own.

One wrinkle from the transition: both `postrust-proxy` and `postrust-worker`
were published at `1.0.0-alpha.1` and `1.0.0-alpha.2` before the split. Those
are prereleases, so Cargo does not select them by default, and they predate
the split. The proxy's stable `1.0.0` follows `0.6.0`; the prereleases came
before the split and share nothing with it but a name. The worker's stable
`1.0.0` is not those prereleases' successor either: it is the Workers adapter
released in 2.3.0 as `0.7.0`, and shares nothing with them but a name.

## What the worker promises

`postrust-worker` is not used as a Rust library -- a Worker is built from it
-- so its surface is configuration and behaviour, and that is what its semver
covers from 1.0.0. A change that would break a deployment that upgrades
without touching its `wrangler.toml` needs a major version:

- **Configuration.** The variables it reads (the server's `PGRST_*` names it
  documents, and `PGRST_WORKER_SCHEMA_TTL`), what each means, and their
  defaults -- the 1000-row ceiling included.
- **Bindings and secrets.** The names it looks for: `HYPERDRIVE`, the KV
  namespace `POSTRUST_SCHEMA_CACHE`, the `POSTRUST_RELOAD_SECRET` secret, and
  what a Cron Trigger does.
- **Endpoints.** The REST API under the same rules as the server, `/_/health`
  and the authenticated `POST /_/reload`.

Not covered: the KV key and record the schema cache is kept under, which carry
the worker's and `postrust-core`'s versions and are rebuilt from the database
when they do not match, and the `worker` crate it builds against, which is
itself `0.x` and does not appear in the Worker's surface.

## What the proxy promises

From 1.0.0, `postrust-proxy` follows semver on a line of its own. A breaking
change to any of these needs a major version:

- **Its Rust API**: the public types, traits and functions of the crate, as
  for the stable line. Configuration types that grow with features
  (`ServerConfig`, `RateLimitDefaults`, `SaasOptions`, `DomainPolicy` and
  their neighbours) are `#[non_exhaustive]` where adding a field should be a
  minor release.
- **Its configuration**: the TOML keys and database tables it reads, what each
  means, and their defaults.
- **The HTTP API of `saas_router` and the admin routers**: routes, request and
  response shapes, status codes, and the `code` an error carries.
- **Its migrations**: a new release adds migrations; it never edits one that
  shipped.

The crates its Rust API exposes count as part of it: its constructors and
`saas::db` take a sqlx `PgPool`. That is why it is 2.0.0 beside Postrust
3.0.0 -- the move to sqlx 0.9 changed nothing in the proxy's own code, and is
a major version of it all the same.

It reached 1.0.0 by being audited rather than by being lived with: the
transport layer is measured by h2spec, Autobahn and the HTTP Garden
differential fuzzer (`scripts/h2spec/`, `scripts/autobahn/`,
`scripts/http-garden/`), and 1.0.0 closes what a security review of the data
plane and the multi-tenant module found. Two limits stay documented rather
than promised away: there is no configuration reload, so a change needs a
restart, and a route's `retry_count` is still unread (see docs/proxy.md).

## What semver covers in the stable line

Covered — a breaking change here requires a major bump:

- The public Rust API of each stable crate: exported types, traits, functions,
  their signatures, and public enum variants and struct fields.
- The HTTP surface of `postrust-server`: routes, request grammar, status codes,
  and the response shapes the conformance suites replay.
- Configuration keys and their meanings.
- The minimum supported Rust version.

Configuration, schema and metadata types that grow with features --
`AppConfig`, `SchemaConfig`, `TableNames` and their neighbours -- are
`#[non_exhaustive]`. A new field or variant on one of those is a minor
release, not a major one; the price is that another crate cannot build one with
a struct literal, and starts from `Default::default()` and assigns the fields it
needs instead. This is what 2.0.0 changed, so that a feature adding a setting
does not need a major release.

3.0.0 extends the same rule to two types 2.0.0 missed:
`postrust_auth::JwtConfig`, which is built with
`postrust_engine::jwt_config(&AppConfig)` or from `Default::default()`, and
`postrust_server::AppState`, which is built with `AppState::new`.

Not covered:

- `postrust-worker`'s Rust items: it is promised as a Worker (above), not as
  a library.
- Anything reachable only through a `#[doc(hidden)]` item or a private module.
- The exact wording of error messages, and log output.
- Behaviour under configurations the documentation calls unsupported.
- Test fixtures, benchmark harnesses, and everything under `scripts/`.
- Generated data files under `website/src/data/`.

## Conformance is measured, not promised

Postrust answers PostgREST's REST dialect and Hasura's GraphQL dialect. How
closely it answers each is a measurement, taken by replaying the other
project's own test corpus against both servers and diffing the live responses.

Those percentages are **not** a semver commitment. They move when the upstream
projects change, when the corpus grows, and when we fix things. What semver
covers is that we will not break a documented behaviour of our own without a
major bump; it does not promise a particular agreement figure with a third
party whose releases we do not control.

See [PostgREST conformance](./postgrest-conformance.md) and
[Hasura conformance](./hasura-conformance.md) for the method and the current
numbers, and for the cases where the two deliberately disagree.

## What is measured

Every figure the project publishes is generated from a run's own artifacts
rather than typed by hand, and the generator refuses to emit a partial file.
If a number cannot account for itself, it does not ship.

Throughput figures are currently **withdrawn**. The benchmark that produced the
previous ones was found to report the order of a run as much as the speed of a
server; see [benchmarking](./benchmarking.md) and `scripts/BENCH-FINDINGS.md`.
They return when the harness can be trusted, and not before.

## Prereleases

A prerelease carries a hyphen (`1.0.0-alpha.2`) and no stability promise of any
kind. Cargo excludes prereleases from default version resolution, so
`cargo add postrust-server` will not pick one up unless asked.

## Deprecation

A stable item that is going away is marked `#[deprecated]` with a note saying
what to use instead, and stays for at least one minor release before removal in
the next major.

## Reporting a problem

Bugs and questions: [GitHub Issues](https://github.com/postrust/postrust/issues).

For anything with a security impact, see
[SECURITY.md](https://github.com/postrust/postrust/blob/main/SECURITY.md) --
report it privately rather than opening a public issue.
