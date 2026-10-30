# Changelog

Notable changes, newest first. This file starts at 1.0.0-alpha.1; earlier
releases are described by their tags and the pull requests behind them.

## 3.0.0 (unreleased)

A major release, and a narrow one: two structs that the features in this line
have to grow are made `#[non_exhaustive]`, so that adding a field to either is
a minor release from here on, and the database driver moves to sqlx 0.9, which
raises the minimum supported Rust version to 1.94. The HTTP surface and
configuration keys are unchanged.

### Changed

- **`postrust_auth::JwtConfig` and `postrust_server::AppState` are
  `#[non_exhaustive]`.** 2.0.0 did this for the configuration types and missed
  these two, which asymmetric keys and telemetry both need to extend. Another
  crate can no longer build either with a struct literal.
- **`JwtConfig.secret` and `secret_is_base64` are replaced by `keys`**, the
  parsed key set, and `issuer` is added.
- **A JWT secret that cannot be used is a start-up error.** A `PGRST_JWT_SECRET`
  that begins with `{` and is not a valid JWK or JWKS, or that is not the
  base64 `PGRST_JWT_SECRET_IS_BASE64` says it is, stops the server before it
  connects to the database. Bad base64 was a 401 on every request; a
  malformed JWK would have been an HMAC secret, which PostgREST still treats
  it as -- letting anyone who holds the public key sign tokens. On a Worker,
  which has no start-up, each request is answered PGRST901 until it is fixed.
- A token whose header does not parse is PGRST301 ("No suitable key or wrong
  key type"), not PGRST303.
- **GraphQL transactions are set up as REST's are.** Each one -- a query, an
  aggregate, a federated entity lookup, a subscription's re-read, and the
  transaction a mutation's writes share -- now starts with the same
  `postrust_core::Session` statements a REST request does, rather than with
  GraphQL's own `set_config` and `SET LOCAL ROLE`. The role is still the
  token's database role, never the Hasura role, and the `hasura.*` session
  variables and `request.jwt.claims*` are still set. What GraphQL now gets
  that it did not:
  - a role's own `statement_timeout` and isolation level from
    `PGRST_ROLE_SETTINGS`, and the server-wide `PGRST_DB_TX_ISOLATION`. A
    GraphQL operation as a role limited to 5 seconds is cancelled after 5
    seconds, as a REST request is;
  - `app.settings.*` from `PGRST_APP_SETTINGS_<NAME>`.

  Unchanged: GraphQL leaves the `search_path` as the role and the database
  set it, so PostGIS and other extensions are found wherever they were
  before (`Session::set_search_path(false)`); it does not call
  `PGRST_DB_PRE_REQUEST`, sets no
  `request.method`, `request.path`, `request.headers` or `request.cookies`,
  and does not run a query in a read-only transaction.
- **`postrust_core::query::build_query` takes the plan alone**, and
  `MainQuery.pre_statements` is gone. It held a `SET LOCAL ROLE` that nothing
  had sent since the role moved into the session's `set_config`; the role is
  a `Session`'s now and nowhere else.
- **`GraphQLContext::session_settings` is replaced by
  `GraphQLContext::session`**, which returns the `Session` its transactions
  are set up by, and `GraphQLContext` has a `config` field, set with
  `with_config`. A context built without one is set up from
  `AppConfig::default()`.
- **sqlx 0.9, from 0.8.** sqlx's types are part of the public API --
  `AppState::pool` is a `sqlx::PgPool`, and a route of your own is handed it --
  so a crate that embeds `postrust-server` has to move to sqlx 0.9 with it: a
  pool from sqlx 0.9 is a different type to code written against 0.8. Nothing
  a request can reach behaves differently. The TLS feature is still
  `tls-rustls-ring-native-roots`: in 0.9, as in 0.8, it is the only one that
  reads the host trust store, so a database behind a private CA still verifies
  under `sslmode=verify-full`.
- **The minimum supported Rust version is 1.94, from 1.88.** sqlx 0.9 declares
  1.94, and every crate that talks to a database depends on it. CI's `msrv` job
  checks the workspace on 1.94 `--locked`, and 1.93 is refused.

### Added

- **Routes of your own run as their caller.** A route nested with
  `Builder::nest` takes `postrust_server::User`, an axum extractor that
  authenticates the request as REST does -- the same token cache, anonymous
  role and refusals -- and `AppState::begin_as(&user)` begins a sqlx
  transaction that runs as them, so a row-level security policy written for
  the API applies to the route's own SQL. Pooled connections were the
  authenticator's, with no claims set, and the documentation said so.
  - The transaction is set up by the `Session` REST uses: role, claims,
    `request.method`, `request.path`, `request.headers`, `request.cookies`,
    `request.trace_id`, `search_path`, `app.settings.*` and `db_pre_request`,
    all transaction-local.
  - `begin_as_with(&user, TxOptions)` makes it read-only, sets its isolation
    level, or skips `db_pre_request`. `Session::isolation` is the knob
    beneath it.
  - `Option<User>` is `None` for a request with no `Authorization` header; a
    token that does not verify is refused either way.
  - `postrust_server::Error` answers in PostgREST's error shape, with REST's
    status for each SQLSTATE, and `?` converts a `sqlx::Error` into it.
  - `postrust_engine::authenticate` and `authorization`: how REST
    authenticates a request, for a host's own routes to share.
- **Asymmetric JWTs.** `PGRST_JWT_SECRET` may be a JWK or a JWKS
  (`{"keys": [...]}`), as in PostgREST, and tokens signed with RS256/384/512,
  PS256/384/512, ES256, ES384 or EdDSA are verified with it.
  - The algorithm comes from the key, never from the token: each key admits
    the one its JWK `alg` names, or the defaults for its type. `alg: none`,
    and an HS256 token whose HMAC secret is an RSA public key, are refused
    before any signature is checked.
  - A token with a `kid` is checked against that key only; one without is
    tried against each key that admits its algorithm.
  - Keys marked for encryption, and of types not supported (P-521, X25519),
    are set aside with a warning.
- **`PGRST_JWT_ISS`.** When set, a token's `iss` must name it, and a token
  with no `iss` is refused (PGRST303, "JWT issuer is not the expected one").
- **`PGRST_JWT_JWKS_URL`: keys fetched from an identity provider** (Supabase
  Auth/GoTrue, Auth0, Clerk, Cognito, Keycloak) and kept current as it rotates
  them, on the server, Lambda and the Worker.
  - Fetched at start-up, then again after the response's `Cache-Control:
    max-age` (held to 60 s..1 day) or every `PGRST_JWT_JWKS_REFRESH` seconds
    (default 300). A token naming an unknown `kid` causes one more fetch, at
    most every 30 s, before it is refused with PGRST301. A token whose key is
    held waits for nothing.
  - `https` only, or `http` to a loopback host; same-host redirects only; 5 s
    and 1 MiB per fetch. Any other URL stops the server from starting.
  - A failed fetch keeps the last good set. If none has ever loaded, tokens are
    refused and `/_/ready` (API and admin port) answers 503 with
    `"jwks": false`; the server starts regardless, and recovers by itself.
  - Every change of the set empties the token cache, so a withdrawn key's
    tokens are refused at once.
  - With `PGRST_JWT_SECRET` also set, a token either one verifies is accepted.
  - Server: a background task. Lambda: fetched in the init phase, refreshed by
    the request that finds the set due. Worker: fetched by the first request
    that needs it and copied into `POSTRUST_SCHEMA_CACHE` KV for cold isolates;
    the cron refreshes it. The Worker's release wasm grows by 158 KB (49 KB
    gzipped), from 4,572,934 to 4,731,391 bytes before wasm-opt; no new crate
    is linked.
  - `postrust_auth::jwks` (`Jwks`, the `Fetch` and `JwksStore` traits,
    `JwksStatus` -- refresh and failure counts, last success, key-set age --
    for metrics), `postrust_auth::ensure_keys`, which a host awaits before the
    synchronous check -- `postrust_engine::authenticate` awaits it, so REST,
    GraphQL and a route's `User` all fetch a rotated-in key --
    `JwtConfig::{jwks, with_jwks}`, `JwtKeys::generation`,
    `JwtKeyError::JwksUrl`,
    `JwtCache::clear`, and `postrust_engine::jwks_builder`.
  - CI signs users up with the `supabase/auth` image and checks that RLS sees
    their `sub`, through the JWKS URL and through the legacy HS256 secret
    (`scripts/gotrue/e2e.sh`).
- `postrust_engine::jwt_config(&AppConfig)`: the JWT configuration an
  `AppConfig` describes, and `jwt_config_with_keys` for keys already parsed.
  The server, the Lambda adapter and the Worker each spelled this out by
  hand, and now all take it from here.
- `AppState::new(pool, schema_cache, config)`, which derives the JWT
  configuration and token cache from `config` the way `build` does.
- `postrust_auth::{KeySet, JwtKeys, JwtKeyError}`.
- **OpenTelemetry and Prometheus metrics**, in a new `otel` feature that the
  published images are built with. Inactive until configured: with nothing
  set, nothing is exported or measured. See docs/observability.md.
  - **Traces over OTLP** (HTTP/protobuf) when `OTEL_EXPORTER_OTLP_ENDPOINT`
    is set, following the standard `OTEL_*` variables; `service.name` is
    `postrust` unless `OTEL_SERVICE_NAME` says otherwise. The request span
    joins the caller's `traceparent`, and its exported trace id is the
    `trace_id` the log and `request.trace_id` already carry. It is named
    `{method} {route}`. The debug-level stage spans are exported without
    the log becoming any noisier.
  - **Finer database spans:** `db.count` for `Prefer: count`, `db.embed` for
    an embed fetched by its own query, and `db.commit` for a `COMMIT` sent on
    its own. The `auth` span says whether the token cache answered
    (`jwt.cache`: `hit`, `miss` or `bypassed`), and GraphQL operations get a
    `graphql` span.
  - **Metrics**, over OTLP and as Prometheus text at `GET /metrics` on the
    admin port (`PGRST_ADMIN_SERVER_PORT`), never on the API port:
    `http.server.request.duration`, `http.server.active_requests`, the pool's
    connections, maximum and wait, token cache hits and misses, JWKS fetches
    and the key set's age, schema reloads, and open GraphQL subscriptions.
  - **Lambda:** with `--features postrust-lambda/otel`, telemetry is flushed
    to the collector (the ADOT layer) at the end of every invocation.
  - New API: `postrust_server::otel`; `JwtCache::authenticate_with_outcome`,
    `JwtCache::stats`, `CacheOutcome` and `CacheStats` in `postrust-auth`;
    `postrust_engine::observe`; `subscription::active_subscriptions` in
    `postrust-graphql`; `postrust_lambda::FlushLayer`.

### Upgrading from 2.x

Of the two structs, only code that built `JwtConfig` or `AppState` with a
struct literal is affected. Replace an `AppState { .. }` with `AppState::new(pool, schema_cache,
config)?` and assign any field that should differ -- a test that wants every
token validated sets `state.jwt_cache = None`. Replace a `JwtConfig { .. }` with
`postrust_engine::jwt_config(&config)?`, or with `JwtConfig::default()` and
assignments; `secret` is now `keys`, built with `JwtKeys::from_config`. Both
constructors fail only when the JWT secret does not parse.

A role in `PGRST_ROLE_SETTINGS` whose
`statement_timeout` was written with only REST in mind now limits its GraphQL
operations too. Code that called `build_query(&plan, role)` drops the role;
code embedding the GraphQL handler passes its configuration with
`GraphQLContext::with_config`.

A crate built on `postrust-server` or `postrust-core` needs a Rust toolchain of
1.94 or later and, if it uses sqlx itself -- a route of your own handed
`AppState::pool`, say -- sqlx 0.9 in its own `Cargo.toml`. sqlx 0.9 accepts
only a `&'static str` as SQL, so SQL your own code builds at runtime -- a
`format!` naming a schema, say -- no longer compiles as an argument to
`sqlx::query` and its siblings, or to `Executor::execute`. Wrap it in
`sqlx::AssertSqlSafe(...)` once you have checked that every value in it is a
bound parameter and every identifier is quoted; a string literal needs nothing.
The rest of sqlx's 0.9 changes are in
[its changelog](https://github.com/launchbadge/sqlx/blob/main/CHANGELOG.md).

## 2.4.1

A security release. Every deployment should upgrade. It fixes an SQL
injection and a request that crashes the server, both reachable without a
token, and hardens two related places. Details are in the repository's
security advisories (https://github.com/postrust/postrust/security/advisories).
No API change. `postrust-worker` moves to 1.0.1 for the same fixes.
`postrust-proxy` is not part of this release and stays at 0.6.0.

2.4.0 reached Docker Hub but not crates.io, because its publish step failed.
For crates.io users this release therefore also brings everything listed under
2.4.0.

### Security

- **A cast in `select` is a type name, and nothing more.** The type after
  `::` (`?select=price::text`, `count()::text`, `amount.sum()::text`) was
  written into the statement as the client sent it, and the parser accepted
  spaces there. So words a client appended reached the database as SQL, run
  as the request's role. A cast is now one type name -- letters, digits, `_`
  and `$` -- or one of the SQL standard's multi-word names (`double
  precision`, `character varying`, `bit varying`, `time[stamp] with[out] time
  zone`), written in the standard's spelling. Anything else after `::` is
  refused with 400 PGRST100. Every cast PostgREST accepts is still accepted.
- **A `select` or logic filter nested more than 64 levels deep is refused.**
  Both were parsed recursively with no limit, and one request nested a few
  thousand levels deep, about 15 KB of query string, overflowed the stack. A
  stack overflow aborts the whole process. Such a request is now refused with
  400 PGRST100 before it is parsed. A parenthesis inside a quoted filter value
  is not counted.
- **Hand-quoted string literals no longer depend on
  `standard_conforming_strings`.** JSON path keys (`data->key`) and the keys
  GraphQL writes into `json_build_object` were quoted only by doubling `'`.
  With `standard_conforming_strings = off`, set on a database, role or server,
  a backslash in a key could end the literal early. `postrust_sql::quote_literal`,
  which they now all use, writes a string that contains a backslash as
  `E'...'`. Not exploitable on PostgreSQL's default settings.
- **A role claim of `"none"` or `""` names no role.** The role is switched to
  with `set_config('role', ...)`, and PostgreSQL reads `none` there as "no
  role", so such a token ran its request as the authenticator. An empty role
  was a 500. Both now fall back to the anonymous role, as a role claim that
  is not a string already does, or are refused where there is no anonymous
  role. This needs a validly signed token. A role actually named `None` or
  `NONE` is unaffected.

## 2.4.0

A minor release. GraphQL now follows a schema reload, a write a view cannot
take is a 405, and query errors the schema answers on purpose leave the error
log. It also updates five locked dependencies past RustSec advisories
published since 2.3.2. No breaking change: `cargo semver-checks` against 2.3.2
reports no semver update required for any stable crate. `postrust-worker`
stays at 1.0.0 and `postrust-proxy` at 0.6.0: nothing in them changed, and
their published manifests already accept the new versions.

### Added

- **GraphQL follows `NOTIFY pgrst, 'reload schema'`.** The reload refreshed
  the REST schema cache only; GraphQL kept the schema it built at start-up,
  so a table a migration added was missing from GraphQL until a restart.
  - Each successful reload rebuilds the GraphQL schema beside the old one and
    swaps it in whole. A rebuild that fails leaves the previous schema serving.
  - Enum tables are read again on each rebuild, so a value a migration adds
    reaches the GraphQL enum.
  - The subscription broker carries over to the rebuilt schema and listens on
    the tables it adds. Subscriptions opened before a reload keep receiving
    notifications.
  - `/v1/graphql` is mounted even when start-up could not build a schema. It
    answers `unexpected` until a reload builds one, where it was a 404 until a
    restart.
  - New API: `GraphQLState::continue_subscriptions`, `NotifyBroker::listen`
    and `hasura::unavailable` in `postrust-graphql`.
- **`Allow` on every 405.** RFC 9110 requires it, and it is the answer an
  `OPTIONS` on the resource gives. A function that writes called with GET and
  a PUT with filters it cannot take were 405s without one. `Table::allow` is
  that answer for a relation.

### Fixed

- **A write a view cannot take is a 405, not a 500.** A view PostgreSQL cannot
  write through, such as a join with no `INSTEAD OF` trigger, refuses every
  write with `55000 cannot update view`, which was answered `500 PGRST500
  "Database error"`. It is now `405 PGRST117` with `Allow`, and the message
  names the view. A write to a materialized view (`42809`) was a 400 and is a
  405 too. Recognised from PostgreSQL's English message, as the `xmlagg` and
  missing-role cases are; with another `lc_messages` it stays a 500.
  `DatabaseError::is_unwritable_view` says which.
- **Query errors the schema answers on purpose are logged at debug.** Every
  failed query was logged as `ERROR Query error`, so a schema that answers
  "not found" by raising `PT404` filled the error log. These are now
  `DEBUG Query refused with <status>`:
  - `PT` statuses and `PGRST` raises;
  - integrity constraints (`23`);
  - data exceptions on the client's input (`22`, answered 400, except
    `22012` division by zero);
  - a write in a GET's read-only transaction (`25006`), and a write the view
    cannot take.

  Everything else stays at error, including faults answered with a 4xx: SQL
  the planner got wrong, a table dropped behind the schema cache, a missing
  grant, a bare `RAISE EXCEPTION`.
- **A stopped subscription broker releases its connection at once.** Its
  listener held a database connection until the next notification arrived,
  which on a quiet table was never.

### Security

- rustls-webpki 0.103.15 to 0.103.16: RUSTSEC-2026-0382 (certificate
  verification could accept distrusted intermediates).
- webpki-roots 1.0.5 and webpki-root-certs 1.0.9 to 1.0.11: RUSTSEC-2026-0381
  and RUSTSEC-2026-0383 (bundled roots trusted distrusted emSign
  intermediates).
- ctutils 0.4.3 to 0.4.6: RUSTSEC-2026-0385 (mask generation and `u128`
  comparisons that were not constant-time).
- tokio 1.48.0 to 1.54.1: RUSTSEC-2026-0386, an unsoundness in
  `AsyncFd::new`, which Postrust does not call.
- RUSTSEC-2026-0387 (crypto-bigint 0.5.5) is ignored in `.cargo/audit.toml`
  with its reason. No shipped native binary compiles it. On wasm32, and
  natively under postrust-auth's test-only `jwt-rust-crypto` feature,
  jsonwebtoken's RustCrypto backend brings it in for ES256/ES384, which
  Postrust does not accept, and its fix (0.7.6) is a line p256 0.13 cannot
  take.

tokio is a direct dependency, and the workspace now requires 1.54.1. The
others are transitive, so the fixes reach the published binaries and images
through the lockfile only. A project that depends on the postrust crates from
crates.io takes them with `cargo update -p rustls-webpki -p webpki-roots -p
webpki-root-certs -p ctutils`.

### Upgrading

Nothing to change. Three things behave differently:

- A log alert on `Query error` stops firing for refusals such as `PT404`, a
  duplicate key or a check violation. Faults still log at error.
- A write to a view that cannot take it is a 405 where it was a 500 (a 400
  for a materialized view).
- `/v1/graphql` on a server that could not build its GraphQL schema answers
  `unexpected` rather than 404.

## 2.3.2

**Security: rustls 0.23.46 and quinn-proto 0.11.18.** A patch release that
updates two locked dependencies past seven RustSec advisories published since
2.3.1. Nothing in Postrust's own code changed.

- rustls 0.23.45 to 0.23.46, which is in every crate's tree:
  RUSTSEC-2026-0346 (`Debug` on client session values exposed TLS secrets),
  RUSTSEC-2026-0347, RUSTSEC-2026-0348 and RUSTSEC-2026-0349.
- quinn-proto 0.11.17 to 0.11.18, which is locked only through `reqwest` in
  `postrust-proxy`'s health checker: RUSTSEC-2026-0338, RUSTSEC-2026-0340 and
  RUSTSEC-2026-0343, each a remote peer able to panic the QUIC stack.

The images and the release binaries are built against the new lockfile.
Library users pick the fixes up with `cargo update -p rustls -p quinn-proto`
on any 2.x. `postrust-worker` stays at 1.0.0 and `postrust-proxy` at 0.6.0:
their published manifests already accept the fixed versions.

## 2.3.1

**`postrust-worker` 1.0.0.** The Cloudflare Workers adapter released in 2.3.0
as 0.7.0 keeps semver from here, on a version of its own. Its promise covers
what a deployment sees rather than a Rust API -- the variables it reads and
their defaults, the `HYPERDRIVE` and `POSTRUST_SCHEMA_CACHE` bindings, the
`POSTRUST_RELOAD_SECRET` secret, `/_/health` and `POST /_/reload` -- so a
change that would break a `wrangler.toml` nobody touched needs 2.0.0. The KV
record the schema cache is kept under is not part of it. See
`docs/stability.md`. It is unrelated to the `1.0.0-alpha` prereleases
published before the crates split into lines.

Nothing else changed. The other crates and the images carry 2.3.0's code as
2.3.1, and `postrust-proxy` stays at 0.6.0.

### Measured

- The website's conformance figures are from runs on 2.3.0's code (commit
  `98ec7a1`): PostgREST 1424/1499 and Hasura 454/468, as on 2.2.0.

## 2.3.0

A minor release: additions to the public Rust API -- a new crate,
`postrust-engine`, and the driver-neutral database interface in
`postrust_core::db` -- and no breaking change: `cargo semver-checks` against
2.2.0 passes for every stable crate. Three things in it:

- **The Cloudflare Workers adapter works.** `postrust-worker` 0.7.0 serves
  the REST API in a Worker and reaches PostgreSQL through Hyperdrive. It was
  a stub that answered `{"status": "stub"}`. Getting there took the request
  pipeline out of the server and off sqlx, without changing what the server
  does. Issue #50 planned it in phases, and each phase was measured against
  the one before.
- **A write the response refuses is rolled back,** and a schema reload no
  longer deadlocks a request in flight.
- **Two GraphQL fixes,** found by running two Postrust subgraphs behind
  Apollo Router for the Federation example in `examples/federation`. They
  were cut as 2.2.1 and are released here instead.

`postrust-proxy` stays at 0.6.0: nothing in it changed.

### Added

- **`postrust-worker` 0.7.0.** The server's REST pipeline in a Worker, over
  Hyperdrive.
  - Every request runs in a transaction that switches to the token's role and
    sets its claims, as on the server.
  - The schema cache is kept in Workers KV and refreshed by a Cron Trigger or
    an authenticated `POST /_/reload`. The KV copy expires after
    `PGRST_WORKER_SCHEMA_TTL` (at least 60 seconds), so a migration is seen
    without either.
  - Rows are capped at 1000 unless `PGRST_DB_MAX_ROWS` (or `PGRST_MAX_ROWS`)
    says otherwise.
  - Unquoted `wrangler.toml` variables, which wrangler passes as numbers and
    booleans, are read as the server would read them.
  - REST only.
  - Tested in workerd by `scripts/serverless/worker.sh`, and against
    PostgREST's own suite. See `docs/workers.md`.
- **`postrust-engine`**, a new crate on the stable line. It is the REST
  request pipeline with no web framework: `handle(&state, request)` takes an
  `http::Request` and returns an `http::Response<Bytes>`. The server is now an
  axum wrapper around it.
- **`postrust_core::db`**: the database as the pipeline and the schema-cache
  loader use it.
  - The `Db` and `Tx` traits, with drivers that decode rows straight to JSON
    and are used generically.
  - sqlx's `PgPool` implements them.
  - `PgSocketDb` (feature `pg-socket`) speaks the wire protocol through
    tokio-postgres over any socket.
  - `DbError` says whether PostgreSQL refused a statement (`as_database`),
    whether the connection was what failed (`is_connection`), and whether
    the failure was a queued statement's (`was_queued`).
  - `crates/postrust-server/tests/drivers_agree.rs` holds the two drivers to
    identical answers: every column type, parameter typing, and errors down to
    the SQLSTATE and constraint.
- **Statement pipelining.** `Tx` gains `queue`, `fetch_json_then`,
  `fetch_json_and_commit` and `fetch_json_then_and_commit`. `PgSocketDb`
  uses them to send the statements that set a transaction up, the query, and
  what follows it in one flight: a plain read -- and a `GET` to a function --
  is one flight after connecting, where it was five, and a write two, where
  it was five or six. A read with a count, or a media type the schema
  renders, takes a flight or two more: it commits after its last statement
  has answered. sqlx cannot pipeline, so the server
  runs the same statements one after another, as before. `drivers_agree.rs`
  holds both drivers to the same order and the same first error, and counts
  the socket driver's flights.
- **`postrust_core::VERSION`**, the crate's version. The Worker keys the KV
  copy of the schema cache by it, since the cache's shape is core's.
- `SchemaCache::load_from` and `load_with_search_path_from` load the cache
  through any `Db`. The sqlx-typed `load` and `load_with_search_path` are
  unchanged.
- `AppConfig::try_from_vars`, to build the configuration from name/value
  pairs rather than the process environment.
- `SchemaCache` implements `Serialize` and `Deserialize`.
- `scripts/conformance/conformance.sh` takes `CONFORMANCE_EXTRA_FEATURES`
  (for example `pg-socket`, which routes REST through the socket driver) and
  `CONFORMANCE_WORKER_BUILD`, which makes the Worker in workerd the candidate.
  CI runs the server's database tests with `pg-socket` too.
- `examples/federation`: two databases, a Postrust subgraph over each, Rover
  and Apollo Router in one Compose file, with row-level security deciding what
  each caller sees through the router, and a script that checks it.
- `postrust_graphql::hasura` gains `invalid_jwt` and `missing_role_claim`,
  hidden from the docs, for the server crate's use.

### Fixed

- **A write the response refuses is rolled back.** `Prefer: max-affected`
  with `handling=strict`, and a singular `Accept` on a write that touched
  several rows, were refused after the transaction had been committed, so
  the client got `PGRST124`/`PGRST116` while every row stayed changed.
  PostgREST rolls these back, and now so does Postrust: the transaction
  commits only once the response is known to be one it can send. The same
  holds for every other refusal made after the statement: a `PUT` whose body
  names a different row than its URL (`PGRST115`), an offset past the end of
  a counted call's result (`416`), a function asking for one object and
  answering none or several, and a function setting an invalid
  `response.status` (`PGRST112`) or `response.headers` (`PGRST111`).
  `crates/postrust-server/tests/refused_writes_roll_back.rs` covers them, on
  sqlx and on the socket driver.
- **A schema reload no longer deadlocks a request in flight.** A request held
  the schema cache's read lock from planning to the end of its query and took
  it again along the way. The lock queues readers behind a waiting writer, so
  a reload (`NOTIFY pgrst`) arriving in between waited on the request while
  the request waited on the reload -- and every request after them queued
  behind both, until the server was restarted. The lock is now released
  before the query runs, and never held across a query.
- **A write the database refuses at COMMIT is reported as what it is.** A
  `DEFERRABLE INITIALLY DEFERRED` constraint is only checked at COMMIT, and a
  failure there was answered `500` "Service temporarily unavailable", as if
  the connection pool had failed. It is now reported like any other database
  error, with its SQLSTATE and status: a deferred foreign key violation is a
  `409`, as an immediate one is.
- **A header a function sets that HTTP cannot carry is refused before the
  commit.** `response.headers` was checked only for its JSON shape; a name or
  value HTTP cannot carry -- a newline in a value, say -- failed when the
  response was built, after the function's writes were committed, and the
  client got an empty `200`. It is now refused with `PGRST111` and the writes
  are rolled back. A response that cannot be built for any other reason is
  logged and answered `500` rather than an empty `200`.
- **A header or status a pre-request function sets reaches a read.**
  `db-pre-request` can set `response.headers` and `response.status` on any
  request, and PostgREST honours them on a `GET` -- a `Cache-Control`, say.
  They were read back only after a function call or a write, so on a read of
  a table they were dropped.
- **`__typename` inside a relationship or an aggregate.** Asking for it
  anywhere below the root -- on an object or array relationship, on the rows a
  mutation returned, or inside an aggregate's functions (`max { __typename id
  }`), at the root or through a relationship -- failed the whole query with
  `column "__typename" does not exist`; only the root and an aggregate's
  `nodes` answered it. A function or `aggregate` object asked for nothing but
  `__typename` came back null; it is now the object, named as Hasura names it
  (`books_max_fields`, `books_aggregate_fields`). Apollo Client
  adds `__typename` to every selection set by default, and a Federation router
  asks for it on every entity reference it follows, so a query crossing from
  one subgraph to another through a relationship failed. It is now answered by
  the schema, as it is at the root.
- **A GraphQL token that does not verify is refused.** A token signed with
  the wrong key, expired, or not a bearer token at all was served as the
  anonymous role over GraphQL, where REST refuses it with 401: the client kept
  getting answers, as someone else, with nothing to say why its rows had gone.
  It is now refused as `invalid-jwt`, in the shape Hasura uses -- over HTTP,
  in each part of a batch, and at a WebSocket's `connection_init`. A token
  that verifies and names no role, on a server with no anonymous role, is
  refused as `jwt-missing-role-claims` rather than run as a role called
  `anon`; REST answers both 401. The admin secret still wins over a bad token
  beside it, as in Hasura, and a request with no `Authorization` header is
  unchanged. With no JWT secret configured the header is still ignored, so a
  router forwarding it to every subgraph is unaffected. GraphQL now verifies
  tokens through the same cache as REST (`PGRST_JWT_CACHE_MAX_LIFETIME`).
- **The Worker's schema cache keeps its columns in KV.** `KvStore::put` of a
  `Serialize` value goes through `serde_wasm_bindgen`, which turns maps into
  JavaScript `Map`s that KV stores as `{}`. Any isolate that read the cache
  from KV had tables with no columns, and a filter failed with "No operator
  matches the given name and argument types". Found on Cloudflare; locally
  one isolate never reads back what it wrote. The cache is now written as a
  JSON string, and `worker.sh` restarts workerd to read it back.

### Changed

- **`postrust-core`, `-sql`, `-auth`, `-response` and `-engine` build for
  `wasm32-unknown-unknown`.** CI checks it, and checks that nothing native
  (ring, aws-lc, mio, sqlx) reaches that build.
  - sqlx is an optional, default-on feature of `postrust-core`.
  - The workspace's tokio asks for `sync` and `macros` only.
  - On wasm32, tokens are verified by jsonwebtoken 10 with its pure-Rust
    RustCrypto backend. Native builds stay on jsonwebtoken 9 (see Measured).
- **The schema cache is loaded through the same database interface as
  requests,** on one transaction. Each catalog query returns its row as JSON,
  so a driver only has to decode `json` to load one. A dump of the loaded
  cache was compared with the old loader's on PostgREST's fixtures and ours,
  and it is identical.
- **A read's transaction ends before its response is serialised.** A plain
  read commits with its query; one with a count or a rendered media type
  commits as soon as the last of its statements has answered, so its
  connection goes back to the pool before the response is formatted rather
  than after.
- **Claims nobody measured are gone.** The README, the docs index, the
  website's footer, manifest and features page called Postrust
  "serverless-first", promised "5-20ms cold starts", put the binary at ~5 MB
  and listed CDK as a way to deploy it. No cold start has been measured, the
  binary is about 10 MB, and there is no CDK template; each now says what is
  true. AI search answers were repeating these back.
- `.gitignore` no longer ignores every directory named `db` or `plan`, only
  the ones at the root. The bare patterns had been silently leaving new
  source files out of commits.

### Measured

Each phase of the Workers work was checked against the tree it started from
-- 2.2.0 with the GraphQL fixes, cut as 2.2.1 and called that below -- on
the same host. Conformance was replayed in full for every phase and for the
final tree.

- **Conformance unchanged:** PostgREST 1424/1499 and Hasura 454/468, case for
  case.
- **Database tests:** all pass, on sqlx and with the server's requests
  routed through the socket driver (`--features pg-socket`), including the
  driver differential suite, the schema-cache round trip, and the
  refused-write rollback.
- **Deployment checks:** Lambda 15/15, e2e 50/50.
- **semver:** `cargo semver-checks` against 2.2.0 reports no major change
  for any stable crate.
- **Throughput unchanged.** Six alternating rounds per tree, with database,
  server and load generator on their own cores. Each final-tree range overlaps
  2.2.1's, and the differences go both ways:

  | Scenario | 2.2.1 | final | change |
  |---|---|---|---|
  | point lookup | 3209 req/s | 3345 req/s | +4.3% |
  | 25-row page | 2967 | 2974 | +0.2% |
  | filtered + ordered page | 2793 | 2867 | +2.6% |
  | page with exact count | 283 | 285 | +0.7% |
  | range filter on numeric | 2761 | 2687 | -2.7% |

- **Binary size:** 9,971,968 bytes to 10,067,688 (+1.0%). jsonwebtoken 10
  natively would have cost 2.6 MB (aws-lc-rs) or 0.7 MB (RustCrypto), so
  native builds stay on 9.
- **The Worker:**
  - PostgREST's suite in workerd scores 1422/1499. The difference from the
    server is two explained cases: named prepared statements, and a gzip'd
    empty preflight body.
  - `worker.sh` passes 30/30.
  - The bundle is 0.93 MB gzipped.
  - **On Cloudflare, through a real Hyperdrive** (query caching off; wall
    time inside the Worker, from `wrangler tail`; medians of 64-150
    requests):

    | Where the Worker ran | Read | Write |
    |---|---|---|
    | Singapore, database in Singapore | 41 ms | 45 ms |
    | Singapore, database in Delhi | 255 ms | 318 ms |
    | The same, Worker placed in Delhi | 170 ms | 190 ms |
    | London, database in Delhi | 428 ms | 574 ms |
    | Amsterdam, database in Delhi | 586 ms | 813 ms |

    CPU is 4-11 ms a request: Hyperdrive does not make the Worker do SCRAM,
    which costs about 43 ms in local workerd. Connecting to Hyperdrive takes
    3-6 ms.
  - **Pipelining**, before and after on Cloudflare, measured as an A/B pair
    on one hostname in a run of its own -- so its "after" figures from
    Singapore (240/306 ms, and 48/53 ms in the database's own region) are
    not the table's above (255/318 and 41/45 ms), which came from another
    run. A read from Singapore to a database in Delhi went 380 -> 240 ms, a
    write 424 -> 306 ms; from London 869 -> 428 and 836 -> 574 ms; from
    Amsterdam 1283 -> 586 and 1266 -> 813 ms; in the database's own region
    69 -> 48 and 60 -> 53 ms. That is a read 37-54% faster and a write
    28-36% faster with the database in another region, and 30% and 12% in
    its own. Hyperdrive forwards part of a pipeline one statement at a time,
    so five statements cost about three round trips there. Over a plain
    connection, with 20 ms added to every round trip (`tc netem`), a read
    went from 155 to 72 ms and a write from 156 to 93 ms, against 124 ms for
    the server.

### Upgrading

Nothing to change, unless a GraphQL client relied on an expired or invalid
token being answered anonymously; it now gets an `invalid-jwt` error, and
should drop the token or refresh it. Likewise a token with no role claim on a
server with no anonymous role, which now gets `jwt-missing-role-claims`.

A `db-pre-request` function that sets `response.headers` or
`response.status` now has them applied to reads as well, as PostgREST does.
One that set them meaning only for writes should check `request.method`.

## 2.2.0

A minor release: additions to the public Rust API (`postrust_server::build`,
`App` and `Host`, the `graphql` feature, `schema_cache::enum_values`) and no
breaking change -- `cargo semver-checks` against 2.1.0 passes for every stable
crate. What people running Postrust will notice is that two deployments which
did not work now do: the AWS Lambda adapter, and an authenticator set up the
way PostgREST recommends. `postrust-proxy` and `postrust-worker` stay at
0.6.0: neither changed.

### Fixed

- **The Lambda adapter serves the server's router.** It used to carry a
  request path of its own, which never bound a query's parameters -- so every
  filter and every write failed with "bind message supplies 0 parameters" --
  and never switched to the token's role or set its claims. Connected as a
  least-privilege role, a function found no tables; connected as a superuser,
  it answered every caller with every row, row-level security or not. It now
  hands `postrust_server::build` to the Lambda runtime: REST, GraphQL
  (including Federation), JWT, the role switch and the claims behave exactly as
  they do in the binary. GraphQL subscriptions and the `NOTIFY` schema reloader
  are left out, since a function can hold neither a WebSocket nor a listener.
  The pool defaults to two connections per sandbox.
- **A `NOINHERIT` authenticator sees the schema.** The schema cache read
  `information_schema`, whose views show a connection only what it can use
  without switching role. Connected as the authenticator PostgREST recommends
  -- `LOGIN NOINHERIT`, granted the roles a token may name -- that is nothing,
  so every table answered "Could not find the table" and GraphQL had no fields.
  Tables, columns and the grants that decide what may be written are now read
  from the catalogue, with `information_schema`'s own definitions and one
  change: a role counts when the connection can *become* it (`MEMBER`), not
  only when it already holds its privileges (`USAGE`). Under a superuser or an
  inheriting authenticator the schema cache is unchanged -- compared table by
  table and column by column over PostgREST's fixture schema and this
  project's -- except for the next item.
- The rest of what a NOINHERIT authenticator used to miss: a data
  representation whose cast function is executable only by the API's roles
  (`REVOKE ... FROM PUBLIC`) is kept rather than dropped, and a GraphQL enum
  table is read as a role that may read it rather than failing with
  "permission denied". A deployment that grants `pg_read_all_data` or
  `pg_write_all_data` instead of per-table privileges sees its tables, as it
  did through `information_schema`.
- A column's enum labels come from its own type. They were looked up by type
  name alone, so two schemas with an enum of one name could swap labels, and
  a column of a built-in type took the labels of any enum given the same
  name -- PostgREST's fixtures declare an enum `bit`, and their `bit(5)`
  column was reported as an enum of `one` and `two`.
- A table's description is read from its own comment. It was looked up by OID
  alone, so a function whose OID matched the table's lent it its comment;
  PostgREST's `collision_test_table` fixture exists to catch exactly that.
- The docs and website no longer describe the Cloudflare Workers adapter as
  working. It is a stub, and does not build for WebAssembly yet.

### Added

- `postrust_server::build`, which builds the application a host serves, and a
  `graphql` feature on `postrust-server` that serves GraphQL without the admin
  UI (`admin-ui` implies it).
- `postrust_core::schema_cache::enum_values`, which reads an enum table's
  members as a role the connection can become that may read it.
- Documentation of the role Postrust connects as: a `NOINHERIT`
  authenticator granted the roles a token may name.
- `scripts/serverless/lambda.sh`, run in CI: builds the function with
  cargo-lambda and tests it in AWS's `provided:al2023` image against
  PostgreSQL with row-level security.

### Upgrading

- **Lambda paths move under `/api`**, as on the server: `/api/todos`, not
  `/todos`. Set `PGRST_COMPAT_MODE=true` to serve the REST surface at the root
  as well. The old root paths only ever answered unfiltered reads.
- **Lambda's pool** is two connections per sandbox unless
  `PGRST_DB_POOL_SIZE` says otherwise; it was one, and not configurable.
- **A `NOINHERIT` authenticator works now.** A deployment that switched to
  `INHERIT` to get past "Could not find the table" can switch back.

## 2.1.0

A minor release: one addition to the public Rust API
(`AuthResult::claim_settings`) and no breaking change -- `cargo semver-checks`
against 2.0.0 passes for every stable crate. Everything else is behaviour
people running the server will see. `postrust-proxy` and `postrust-worker` stay
at 0.6.0: neither uses anything that changed.

**GraphQL transactions carry the verified JWT claims.** REST set
`request.jwt.claims` for a row-level policy to read; GraphQL set only the
role and Hasura's session variables, so a policy telling apart users who share
a database role worked over one surface and not the other. Both now set the
claims document, with `role` set to the role the request runs as, and each
top-level claim whose name can be a setting as its own
`request.jwt.claims.<name>` -- the form `docs/authentication.md` had always
shown and nothing had ever set. Thanks to @CBeardSafire,
[#45](https://github.com/postrust/postrust/pull/45).

**GraphQL subscriptions over a WebSocket work.** They had not, on the
server as shipped: the handshake answered `101` and then nothing, not even
`connection_ack`. The adapter that percent-encodes the `>` in
`select=data->>id` before hyper parses a request line follows HTTP/1.1
framing, and did not know about `Upgrade` -- after the handshake it read every
WebSocket frame as the next request line, holding it back until a newline byte
happened to arrive and percent-encoding the high-bit bytes masked frames are
full of. It now stops rewriting after a request that asks to switch protocols,
as it already did for chunked bodies and HTTP/2.

**GraphQL subscriptions are authenticated.** The WebSocket route attached no
identity to the operations it ran: no role, no session variables, no claims.
A connection is now authenticated once, from its `connection_init` payload
(`{"headers": {"Authorization": "Bearer <token>"}}`, as Hasura's clients send
it) and the upgrade request's headers, through the same code an HTTP request
goes through -- the admin secret, `X-Hasura-Role` against the token's allowed
roles, the role's schema and its introspection setting. A connection HTTP would
refuse is refused at `connection_init`, and each live query re-reads as the
connection's identity. A mutation sent over the socket is committed or rolled
back before its answer is sent, as over HTTP.

**A subscription that fails says why.** A live query whose read failed -- a
permission refused, a cast that cannot be made -- ended with a bare
`complete`, the error dropped on the way out, so a refused subscription looked
exactly like one that had finished. The error is now sent, and the subscription
ends after it.

**The documented policies work on pooled connections.** A per-claim setting a
connection has ever held reads as `''`, not `NULL`, once the transaction that
set it ends. `docs/authentication.md` cast them straight to `integer`, so a
token without the claim was answered on some connections and failed with
`invalid input syntax for type integer` on others, depending only on what the
connection had served before. The examples now read the claims document, which
is set on every request; the per-claim settings are documented with
`NULLIF(..., '')`, and with what they cannot reach: nested claims, and names
that cannot be settings.

**Check any policy written the old way.** Before this release the per-claim
settings were never set, so a policy casting one read `NULL` and never failed
-- it also never matched. From this release such a policy matches for a token
carrying the claim, and fails for one without it on a connection that has
served one with it. Read `request.jwt.claims` instead, or wrap the setting in
`NULLIF(..., '')`.

## 2.0.0

**Postrust can be an Apollo Federation v2 subgraph.** Set
`PGRST_GRAPHQL_FEDERATION=true` and the GraphQL API adds `_service { sdl }`,
`_entities(representations:)` and the Federation directives, so an Apollo
router or gateway can compose it with other services. It is opt-in: with the
variable unset, Federation metadata is inert and adds nothing to the SDL, and a
test holds it there. Apollo's own subgraph compatibility suite runs on every
pull request that touches the GraphQL path, and a required-capability failure
fails the build.

- A table's primary key is its `@key`. A table marked
  `"federation": {"shared": true}` in `PGRST_GRAPHQL_METADATA` keeps its
  unprefixed type name, so two subgraphs can both contribute to it, and its
  non-key columns, computed fields and relationships are marked `@shareable`.
- `PGRST_GRAPHQL_TYPE_PREFIX` prefixes generated types and root fields, so
  several Postrust subgraphs over different databases do not collide when
  composed.
- With Federation on, the root types are `Query`, `Mutation` and
  `Subscription`, the names a router expects, rather than Hasura's
  `query_root` family.
- `_entities` resolves each type's representations in one query and lets
  PostgreSQL decide what matches. A key that PostgreSQL considers equal but
  spells differently -- an uppercase UUID, a `citext` value in another case, a
  `timestamptz` with a different offset -- finds its row, and a key that
  matches nothing is a `null` in its position rather than an error for the
  whole list. Select permissions apply to `_entities` exactly as to any other
  read.

**A column can be exposed as GraphQL `ID`.** PostgreSQL has no `ID` type, so a
key that a router or client expects as `ID` could not be one. A
`column_types` entry in `PGRST_GRAPHQL_METADATA` -- `{"id": "ID"}` -- changes
the GraphQL type of a text, integer or UUID column without changing how it is
read, bound or cast.

**A boolean setting that is not a boolean now stops the server.** Every
`PGRST_*` true/false variable used to read anything other than an exact
lowercase `true` as false, so `PGRST_DB_AGGREGATES_ENABLED=TRUE` or `=yes` or a
typo silently disabled the feature. They now accept `true/false`, `1/0`,
`yes/no` and `on/off` in any case, and anything else fails startup with the
variable's name. **A deployment that was relying on a malformed value reading
as false will not start after upgrading**; the error says which variable to
fix. `AppConfig::try_from_env` returns the error; `AppConfig::from_env` panics
with it.

**Why this is 2.0.0.** The features are additive for anyone running the
server. The major bump is for the Rust API: Federation added public fields to
`AppConfig`, `SchemaConfig`, `TableNames`, `GeneratedSchema` and eight other
public structs in `postrust-core` and `postrust-graphql`, and adding a field
breaks any code that builds one of them with an exhaustive struct literal.
[docs/stability.md](docs/stability.md) covers public struct fields, and
`cargo semver-checks` against 1.0.1 agrees.

So that the next feature is not another major release, those structs and the
related configuration, schema and metadata types are now `#[non_exhaustive]`.
From another crate, build one from its default and assign the fields you need:

```rust
let mut config = SchemaConfig::default();
config.exposed_schemas = vec!["public".into()];
config.enable_mutations = true;
```

A struct literal -- including one ending in `..Default::default()` -- no
longer compiles for these types, and an exhaustive `match` on
`ColumnTypeOverride` needs a wildcard arm. The affected types are
`AppConfig` and `RoleSettings` in `postrust-core`; `SchemaConfig`,
`GeneratedSchema`, `FederationEntity`, `FunctionField`, `QueryField`,
`MutationField`, `GraphQLField`, `TableObjectType` and `RelationshipField` in
`postrust_graphql::schema`; and `TableNames`, `FederationConfig`,
`ColumnTypeOverride`, `RolePermissions`, the four `*Permission` structs,
`Comments`, `NameOverrides`, `DeclaredRelationship` and `FunctionNames` in
`postrust_graphql::names`. Nothing else in the public API changed.

`postrust-proxy` and `postrust-worker` go to 0.6.0: they depend on
`postrust-core`, which is now 2.0.0.

Thanks to @CBeardSafire, who asked for Federation in
[#40](https://github.com/postrust/postrust/issues/40) and built it in
[#42](https://github.com/postrust/postrust/pull/42).

**Conformance, measured on this release.**

| suite | |
|---|---|
| PostgREST v16.1, full contract | 1424/1499 (95.0%) |
| Hasura v2.50.1, full body | 454/468 (97.0%) |

Both reproduce the 1.0.0 figures exactly, case for case, so the published data
files are unchanged: Federation is off by default and did not move either
dialect. The Hasura corpus has its own `apollo_federation` group, and all three
of its cases agree.

Apollo's subgraph compatibility suite passes every required check: `_service`,
`@key` (single and multiple) and `@link`, plus introspection. The optional
capabilities listed in
[scripts/apollo-federation/README.md](scripts/apollo-federation/README.md) are
reported as not supported, which is accurate. The one exception is
`@shareable`: the suite looks for the directive on the type, and Postrust puts
it on each non-key field of a shared entity instead. The two compose the same
way. The local run and the CI run on the same commit agree check for check.

**Verified for this release:** `cargo fmt` clean, `cargo clippy --workspace
--all-targets --all-features -D warnings` clean, 782 unit tests and 134
database tests passing with 0 failures, and `cargo semver-checks` against 1.0.1
reporting exactly the breaks described above.

## 1.0.1

**The server could not talk to a hosted database over TLS, and did not say so.**
`sqlx` was built without any TLS feature. Two consequences, neither of them
visible from the logs:

- `sslmode=require` failed outright, with "SQLx was built without TLS support
  enabled". `docs/configuration.md` has carried that exact URL as an example
  since before 1.0.0, so a documented configuration did not work.
- The default `sslmode=prefer` does not fail when TLS is unavailable. It
  connects in cleartext. Every 1.0.0 deployment against a hosted database --
  RDS, Cloud SQL, Neon, Supabase -- was unencrypted on the wire unless it had
  been given `sslmode=require` and had noticed the error.

Fixed by enabling `tls-rustls-ring-native-roots`, which verifies against the
host trust store rather than a bundled root list, so a database behind a
private or organisation-issued CA works without rebuilding. `ca-certificates`
was already installed in both published images. The reason that feature and not
an aws-lc-rs one -- the choice is forced by sqlx, and costs nothing, because
`ring` was already compiled in -- is recorded against the dependency in
`Cargo.toml`.

Thanks to @CBeardSafire, who found this and sent the fix in
[#38](https://github.com/postrust/postrust/pull/38).

`docs/configuration.md` now documents what each `sslmode` actually does,
including that the default downgrades silently, and the production example asks
for `verify-full` rather than `require` -- `require` encrypts but verifies
nothing, which stops a passive listener and not an active one.

A test asserts the feature stays enabled. It reads the parsed workspace
manifest, because Cargo does not expose a dependency's feature selection to a
dependent's `cfg`, and it accepts any of sqlx's TLS features so that revisiting
which one is chosen does not fail the build.

**rustls moves to 0.23.45 for RUSTSEC-2026-0285**, a medium-severity advisory
against TLS 1.3 handshake messages being accepted across encryption level
boundaries. 0.23.35 was already in the lockfile before this release, so the
advisory is not a regression -- it was published on 2026-09-14 and would have
turned the audit red on its own. What this release changes is whether it is
reachable. rustls was previously pulled only by `postrust-proxy`, through ACME
and reqwest, on a 0.x line that carries no stability promise; enabling TLS in
sqlx puts it under `postrust-core` and therefore under every crate on the
stable line. By the test SECURITY.md already sets -- check `cargo tree -i`
before treating an advisory as an exposure -- that moves it from worth fixing
to worth fixing now, and a release whose subject is that TLS works should not
ship a known TLS advisory. The bump is within 0.23 and costs no compatibility:
rustls 0.23.45 declares Rust 1.71, well under the 1.88 floor, checked against
the locked dependency set rather than assumed.

No API changed. `postrust-proxy` and `postrust-worker` go to 0.5.1 for the same
fix, which reaches them through the workspace.

## 1.0.0

The seven crates on the stable line -- `postrust-core`, `postrust-sql`,
`postrust-auth`, `postrust-response`, `postrust-graphql`, `postrust-server`
and `postrust-lambda` -- are 1.0.0 and carry a semver promise from here:
a breaking change to the public Rust API needs a major bump.
`postrust-proxy` and `postrust-worker` stay on their own `0.x` lines and carry
none, for the reasons in [docs/stability.md](docs/stability.md). Nothing in the
stable line depends on either.

No code changed between `1.0.0-beta.1` and this release. What changed is what
can be said about it with evidence.

**Throughput figures are restored.** The beta.1 entry said they remained
withdrawn, and that is no longer true. `scripts/BENCH-FINDINGS.md` set a
precondition for publishing again -- run the comparison on a machine with
native Docker rather than a virtualised one, and confirm that a measurement
repeated at the start and the end of a run agrees with itself. Both are now
met, on a dedicated bare-metal host with each component pinned to its own CCD:

| | first | last | drift |
|---|---|---|---|
| debian | 44186 rps | 44132 rps | 0.1% |
| alpine | 43727 rps | 43179 rps | 1.3% |

That check is no longer something done by hand. `bench-compare.sh` repeats its
first measurement as the last action of the run and exits non-zero if the two
disagree by more than 3%, so a drifted run cannot be mistaken for a publishable
one. Three independent runs of the whole harness agree to 0.7%, against the
1.25x-1.39x spread the previous host produced.

The published numbers move a long way -- point lookup goes from 6,065 rps to
44,186 -- and none of that is a change to this software. The instrument was
losing most of the signal.

Two things measured alongside, and published with the figures rather than
discovered later by someone else: a container costs about 8.6% against the same
binary run natively, almost all of it Docker's network path, charged to every
server equally; and PostgREST's run-to-run spread is 19.6% at worst against
0.8% here, so multiples quoted against it carry roughly +/-10%.

**Conformance, measured on merged main from a clean tree.**

| suite | |
|---|---|
| PostgREST v16.1, full contract | 1424/1499 (95.0%) |
| Hasura v2.50.1, full body | 454/468 (97.0%) |

The PostgREST figure is one above the previously published 1423, and the whole
of that +1 is in writes -- `Query/InsertSpec.hs`, 78/82 to 79/82. Hasura
reproduces its recorded figure exactly on different hardware, a different
kernel and a different Docker, which is the useful part: the number is a
property of the code rather than of the machine that measured it.

The Hasura introspection gap stays where it is, deliberately. Closing it would
mean the server misreporting the schema it actually has, and a conformance
point bought that way is not worth having. See
[docs/hasura-conformance.md](docs/hasura-conformance.md).

**Verified for this release:** `cargo fmt` clean, `cargo clippy --workspace
--all-targets -D warnings` clean, the same with `--features admin-ui` clean,
and 755 tests passing with 0 failures across 25 binaries on a clean checkout.

## 1.0.0-beta.1

Everything on the checklist for a stable release is done except the one thing
that cannot be done from inside the repository: exposure to somebody outside
it. Hence beta rather than 1.0.0.

**What the version numbers now mean.** Seven crates share `1.0.0-beta.1` and a
semver promise once it lands. `postrust-proxy` and `postrust-worker` are on
their own `0.x` lines and carry none -- the proxy because its surface is wide
and nobody outside this repository has depended on it yet, the worker because
it is a stub. Nothing in the stable line depends on either. See
[docs/stability.md](docs/stability.md).

**Throughput figures remain withdrawn**, and are not restored by attributing
them to the release that produced them. `scripts/BENCH-FINDINGS.md` records why:
the same server measured seconds apart differed by about a factor of two, and
whatever ran first was the most understated, so run order determined the
published ratios. Changing only the measurement window moved point lookup
against PostgREST from 3.35x to 1.11x. The figures are a property of the
harness, not of a version. The precondition for publishing again is in that
file.


### Added

**Host-based routing did not work over HTTP/2.** Route selection read only the
`Host` header, and HTTP/2 has none -- RFC 9113 section 8.3.1 replaces it with
`:authority`, which arrives on the URI. So `host` was empty for every h2 and
h2c request and any route with a `match.host` matched nothing. Confirmed
against a running proxy: the same host-matched route answered 200 over
HTTP/1.1 and 404 over HTTP/2, and now answers 200 for both while still
refusing a host that does not match.

**A domain abandoned mid-issuance was never recovered.** Nothing moved a row
out of `ssl_status = 'provisioning'` except the worker finishing with it, and
both of those writes happen in-process after the order returns -- so a restart
during an order, or a database error on the way to recording the result, left
the domain there permanently with nothing retrying and nothing saying so. The
worker now requeues rows stuck longer than 15 minutes, counting the interrupted
attempt so a crash loop cannot spend the CA's rate limit without limit.

**A certificate whose expiry could not be read was never renewed.** The renewal
scan selects on `expires_at IS NOT NULL`, and issuance stored `None` when the
chain would not parse -- so such a certificate was simply left to expire.
Issuance now substitutes a deliberately short assumed lifetime and logs an
error, and the Pebble test asserts the stored expiry is the chain's own rather
than merely present.

**The route regex was compiled on every request**, for every regex route a
request was compared against. Now compiled once and cached by pattern, failures
included, so a typo costs one compilation rather than one per request.

**Route matching honours the whole of `RouteMatch`.** `path_type`, `methods` and
`match.headers` were all declarable and all ignored: the filter chain compared
host and path prefix and nothing else. Each silent no-op *widened* a route past
what its author wrote -- `path_type = "exact"` on `/health` also caught
`/health-internal`, and a route restricted to `methods = ["GET"]` accepted
`DELETE`. A host may now also be a single-label wildcard (`*.example.com`).
A regex that does not compile matches nothing and logs why, rather than being
treated as a prefix or a match-all.

Also removed `BackendManager::find_upstream`, a second route matcher that was
never called and was worse: prefix only, no method, no headers, and it ignored
`priority`.

**Documentation for the proxy**, in [docs/proxy.md](docs/proxy.md) -- there was
none, for a crate that had just gained TLS, HTTP/2, WebSocket and a binary. It
includes a section naming the configuration that is declarable and does nothing
yet (`timeout_secs`, `retry_count`, `watch_config_file`, `https_enabled`,
`acme_directory`), because the alternative is that each one gets discovered.

**Three domain endpoints the documentation promised and the router never had.**
`docs/saas-domains.md` listed `PUT /domains/{id}`,
`POST /domains/{id}/ssl/provision` and `POST /domains/{id}/ssl/upload`; none was
mounted. All three exist now.

`PUT /domains/{id}` is a partial update of the verification method and the SSL
provider. The domain name is deliberately not updatable: it is what the
verification token proves control of, so a rename would carry a proof of
ownership over to a name nobody has proved anything about.

`ssl/upload` **checks the certificate before storing it** -- the key must match
the chain, it must not be expired, and it must cover the domain, wildcards
counting for exactly one label. Skipping any of those gives a listener that
accepts the upload and then fails every handshake, with nothing pointing back at
the cause.

`ssl/provision` returns 202 and queues; the worker issues. It is also how to
retry, so the `ssl/retry` endpoint that briefly existed in this branch is gone --
one endpoint rather than two that overlap.

**Automatic SSL via ACME, over HTTP-01.** `docs/saas-domains.md` advertised
this as a feature and the schema tracked an `ssl_status`, but nothing in the
crate had ever talked to a certificate authority.

A verified domain with `ssl_provider = "acme"` is queued, and a background
worker (`saas::ssl`) places the order, answers the challenge, stores the
certificate, and renews it 30 days before expiry. Failures record `ssl_error`
and retry with exponential backoff, capped at four hours, giving up after ten
attempts; `POST /domains/{id}/ssl/retry` requeues one. There is deliberately no
`provision` endpoint -- issuance is several round trips plus a challenge fetch
the CA has to make back to us, which does not belong in a request.

Three tables came with it, including **`proxy_certificates`, which
`CertificateStore` had queried since it was written and which nothing ever
created** -- so every certificate save had failed with "relation
proxy_certificates does not exist".

Tested end to end against [Pebble](https://github.com/letsencrypt/pebble),
Let's Encrypt's deliberately-misbehaving test CA, through the shipped
`/.well-known/acme-challenge/{token}` handler: `scripts/acme/run.sh`.

**`rustls-acme` was replaced by `instant-acme`** rather than adding a second
ACME client. The old wrapper took a fixed domain list, which cannot serve a
tenant domain that arrives while the proxy is running, and was never
constructed anywhere. Keeping both also resolved two copies of `rcgen`.

**Database-backed proxy configuration actually reads and writes.**
`config::load_from_database` was two `// TODO: Implement database query`
comments returning `Ok(Vec::new())`, so a proxy pointed at a database started,
logged nothing wrong, and answered every request with 503 -- the same failure
that file-configured routing had before `1.0.0-alpha.2`. It now loads real
routes and upstreams from three new tables
(`migrations/20260901000001_proxy_config.sql`), and `postrust-proxy` merges
them into a file-bootstrapped config when `DATABASE_URL` is set.

**The admin API persists.** Its route, upstream and backend mutations replied
`200`/`201` after changing only the in-memory config, behind eight
`// TODO: Persist to database` comments, so every change vanished on restart
while reporting success. They now write through before touching the running
config, and report a failure to persist as a 500 rather than a success. With
`server.database_config = false` -- an explicit "this proxy is configured from
a file" -- edits stay in memory as before.

### Changed

**The Autobahn figures published on the website are now run-stable.** The
OK/NON-STRICT split is tallied over the cases *outside* the intermittent
invalid-frame family, with the family's size reported separately. Members of
that family move between OK and NON-STRICT from run to run with nothing about
the proxy changing -- one run scored 501 OK and 12 NON-STRICT, the next 502 and
11, from a single case. Publishing the raw split would have made the new drift
check fail on the next run for a reason that is not a regression, and an
intermittently red gate teaches everyone to ignore it. Verified stable over
three consecutive runs.

**The conformance suites run on a schedule.** `.github/workflows/conformance.yml`
runs the transport suites nightly (h2spec, Autobahn baseline and proxied, and a
real ACME order against Pebble) and the dialect suites weekly (PostgREST and
Hasura, which need PostGIS and a release build). Each job then **regenerates the
website's data module and fails if the committed one has drifted** -- which is
the point: every figure the site publishes is generated from a run's artifacts,
and nothing was re-running those, so the numbers could go stale silently.

Not on the pull-request path: each suite starts containers and replays hundreds
of cases against two servers. Gating every PR on that would make the common case
slow to catch an uncommon regression.

HTTP Garden is deliberately not scheduled. It is a differential fuzzer that
clones a GPL-3.0 repository and builds images for dozens of other HTTP servers;
its value is in exploring inputs nobody thought of -- which is how it found the
hop-by-hop defect -- not in re-running a fixed set. The reasoning is recorded in
the workflow.

**h2spec and Autobahn now work on Linux.** Both reach the proxy through
`host.docker.internal`, which resolves only on Docker Desktop. They pass
`--add-host host.docker.internal:host-gateway`, so they work on a CI runner as
well as on a developer's Mac.

**The MSRV is declared and checked: Rust 1.88.** It was not declared anywhere,
while the README claimed 1.78, `docs/getting-started.md` claimed 1.75 and
`CONTRIBUTING.md` claimed 1.75 -- three different wrong numbers. The floor is
now 1.88: `hickory-{net,proto,resolver}` 0.26 and `time` 0.3.55 all require it,
and 1.87.0 is refused outright. Both arrived with the security updates above,
so the floor moved from 1.86 to 1.88 as a direct cost of them, which is a trade
worth making. Declared as `rust-version` in `[workspace.package]`, inherited by
every crate, with a CI job that builds on 1.88 `--locked`.

**`postrust-proxy` and `postrust-worker` each moved to their own `0.x` version
line** and no longer share the workspace version. The rest of the workspace is
heading for 1.0.0 and the semver promise that comes with it; neither of these
is ready to make one.

For the proxy: turning `missing_docs` on produces 306 warnings, and automatic
SSL provisioning is still not implemented. (The config and admin-API stubs that
were the other half of the reason are fixed in this release -- see Added.)
For the worker: it is a stub that answers `{"status": "stub"}`.

Nothing in the stable line depends on either, so no crate that makes a semver
promise carries a dependency that cannot keep one. The worker is a `cdylib`,
which cannot be a Rust library dependency at all.

Both lines still release on the same git tags: the publish step reads each
crate's own declared version rather than assuming one across the workspace, and
skips what is already on crates.io.

New: [docs/stability.md](docs/stability.md), stating what a version number
covers and what it does not.

### Removed

**`POST /config/reload` answered "Configuration reload requested" and reloaded
nothing.** It sent on a channel nobody read. That, `ConfigReloader`,
`server.watch_config_file` and the `notify` dependency they existed for are all
gone; configuration changes need a restart, which `docs/proxy.md` now says.

Dropping `notify` also removed `instant` from the lockfile, so the
`RUSTSEC-2024-0384` ignore in `.cargo/audit.toml` is gone -- two documented
ignores left instead of three.

Also removed: `hyper_ext::TokioExecutor`, a duplicate of the `hyper_util` one
the code actually uses; `HealthChecker`'s `PgPool` field, held "for persisting
health status" that was never implemented; and `ApiKeyRow::key_hash`, selected
and never read.

### Fixed

**Every parameterised route in the proxy would panic at construction.** 23
routes across `admin_router` and `saas_router` used axum 0.7's `:param` syntax,
which axum 0.8 -- what the workspace has depended on for some time -- rejects:

```
Path segments must not start with `:`. For capture groups, use `{capture}`.
```

Neither router is mounted anywhere, so nothing had ever called them and nothing
noticed. Found by an ACME test that needed to serve the real router.

**HTTP domain verification proved nothing.** The endpoint serving the
challenge, `/.well-known/postrust-verification/{token}`, computed
`postrust-verify={token}` from whatever token was in the path and returned it.
No database lookup, no host check -- so every token verified, for every
domain. It now answers only for a challenge that is in the database, is not
expired, is not already resolved, and whose domain matches the request's
`Host`, and it returns the value stored when the challenge was issued rather
than one recomputed at serve time.

This was a broken control rather than a demonstrated takeover:
`proxy_domains.domain` is globally unique, so a second tenant cannot register
a domain another tenant already holds.

**A verified domain asking for ACME was marked `provisioning`** when nothing
would ever provision it, leaving it mid-issuance forever with no way to tell
slow from never. It is now left `pending`, with a warning saying a certificate
has to be supplied manually.

**Documentation described endpoints that do not exist.**
`docs/saas-domains.md` documented `PUT /domains/:id`,
`POST /domains/:id/ssl/provision` and `POST /domains/:id/ssl/upload`, none of
which are in the router, and omitted `enable` and `disable`, which are. It
also advertised automatic ACME provisioning as a feature. Corrected against
the router, and the SSL section now says plainly that no ACME client exists.

### Security

**Twelve dependency advisories fixed** by updating the lockfile, including five
in `aws-lc-sys` (X.509 name-constraint bypass via wildcard/Unicode CN, two
PKCS7 signature and chain validation bypasses, a CRL distribution-point scope
error, an AES-CCM timing side channel) and four in `rustls-webpki` (a reachable
panic in CRL parsing, URI name constraints incorrectly accepted, and two more
CRL and wildcard issues). All of these are in the path of a TLS-terminating
proxy. Also `bytes`, `h2` (unbounded empty DATA frames), `time`, `anyhow`,
`event-listener` and both `rand` majors.

**`hickory-resolver` 0.24 to 0.26**, clearing an O(n²) name-compression CPU
exhaustion in `hickory-proto`. The upgrade is a real API migration, and one
part of it would have broken silently: 0.26 replaced the rdata iterator with
`Lookup::answers()`, and `Record`'s `Display` renders the whole record line
(`name ttl class type rdata`), so the previous `record.to_string()` would never
have matched a bare challenge token again. Every DNS verification would have
failed. The comparison now lives in a tested function, `txt_matches`, with a
case asserting that a full record line does *not* match.

**A dependency audit now runs in CI.** `cargo audit --deny warnings`, with
three documented ignores in `.cargo/audit.toml` -- one advisory with no
reachable code path and no upstream fix (`rsa`, not in any build graph), and
two unmaintained crates with the migration each needs written down. The job is
verified green rather than added and hoped for.

**`SECURITY.md`** added: how to report privately, what to expect, what is in
scope, and the known-and-documented weaknesses that are not reports.

## 1.0.0-alpha.2

A release about the front door rather than the dialects. The HTTP proxy was
library-only and, it turned out, had never routed a request from a file
config; fixing that made it testable, and testing it found the rest.

**Still an alpha.** Same caveat as before: the surfaces are measured, the
public Rust API is not settled.

### Fixed

**File-configured routing answered every request with 503.** Route and
upstream registration both guarded on a `Uuid` that only the database path
ever sets, so a proxy started from a TOML file logged its routes and then
failed to match any of them.

**Hop-by-hop headers were forwarded to origins**, against RFC 9110 section
7.6.1 in two ways: `Connection` is itself hop-by-hop, and the headers it names
must be removed. A client could smuggle arbitrary headers past the proxy.

**A request carrying both `Content-Length` and `Transfer-Encoding` is now
rejected** rather than normalised. RFC 9112 section 6.1 gives `Transfer-Encoding`
precedence, but a proxy that quietly resolves the disagreement is how a
smuggling chain starts.

**`TCP_NODELAY` was set on no socket at all** -- not the accepted connection,
not the upgrade connection, not the pooled connector -- so Nagle batched small
writes and added latency to every small forwarded response.

### Added

**TLS with ALPN.** `tls.cert_file` and `tls.key_file` start an HTTPS listener
that offers `h2` and `http/1.1` and dispatches on what was negotiated. Before
this, `https_host` and `https_port` were configuration that nothing listened
on, which left HTTP/2 reachable only as cleartext and WebSocket only as `ws://`.

**HTTP/2.** h2c on the cleartext port alongside HTTP/1.1, h2 over TLS by ALPN,
an optional HTTP/2-only port (`http2_port`), and a per-backend upstream
protocol (`http_version = "h2c"`), since h2c has no ALPN to negotiate with.

**WebSocket**, over HTTP/1.1 and over TLS, and over HTTP/2 by extended CONNECT
(RFC 8441) translated into an HTTP/1.1 upgrade for the origin.

**A runnable binary.** `postrust-proxy <config.toml>`. The crate was a library
with no entry point, so nothing external could be pointed at it.

### Measured

Three conformance suites, wired up and runnable from `scripts/`:

**HTTP Garden** -- a differential fuzzer for proxies; the harness that found
the hop-by-hop defect. 7 of 7 probes correct.

**h2spec** -- 146 tests, 145 passed, 1 skipped, 0 failed.

**Autobahn** -- 517 cases, 501 OK, and no case worse than a baseline run that
bypasses the proxy. One case fails, and fails identically with no proxy in the
path. The baseline matters: postrust splices WebSocket streams rather than
parsing frames, so most of what Autobahn scores belongs to the origin behind
it.

## 1.0.0-alpha.1

The release where both surfaces stop being asserted and start being measured.
Postrust answers PostgREST's REST dialect and Hasura's GraphQL dialect, and
how closely it answers each is a number produced by replaying the other
server's own test suite against both and diffing the live responses.

**This is an alpha.** The surfaces are measured; the public Rust API has not
been lived with by anyone outside this repository, and a prerelease carries no
stability promise. Expect it to move before 1.0.0. The HTTP and GraphQL
surfaces are the part that is meant to be stable.

### Measured against the servers it replaces

Two differential harnesses. Neither interprets a test expectation: the
reference implementation's live response is the oracle, so a mistake in the
extractor shows up as a case both servers answer the same way rather than as a
false failure.

**PostgREST v16.1** — 1499 replayed cases: 98.6% agree on status, 96.7% on
status and body, 94.9% on the full contract including the six headers that are
part of an answer. See [`docs/postgrest-conformance.md`](docs/postgrest-conformance.md).

**Hasura graphql-engine v2.50.1** — 468 cases in 59 groups: 100.0% agree on
status, 97.4% return the same data, 96.6% match the whole body including error
wording. See [`docs/hasura-conformance.md`](docs/hasura-conformance.md).

Both numbers carry their provenance. Each harness builds its own candidate,
because which features it was built with is part of what is measured and
cannot be read off the binary, and writes a `run-meta.json` recording the
reference version, the features, the commit, and whether the reference was
replayed or a recording reused. The generators that put these figures on the
website read that file and refuse a run that cannot account for itself.

### Added — Hasura-dialect GraphQL

A client generated against Hasura points at `/v1/graphql` unchanged.

- **Endpoints**: `/v1/graphql`, `/v1alpha1/graphql`, and `/api/graphql` for
  anything already pointed there. Subscriptions over WebSocket.
- **Schema shape**: `author`, `author_by_pk`, `author_aggregate`,
  `insert_author`, `insert_author_one`, `update_author`, `update_author_by_pk`,
  `update_author_many`, `delete_author`, `delete_author_by_pk`, under
  `query_root` / `mutation_root` / `subscription_root`.
- **Filtering**: generated `<table>_bool_exp` types, so an unknown operator or
  an ill-typed operand is refused by validation rather than by the database.
  Text, `jsonb`, `ltree` and PostGIS comparison groups. Filter by a related
  row, or by an aggregate over a related set.
- **Ordering**: `order_by` as a list, because ordering is ordered. Order by a
  related row's column or by an aggregate of a row's children.
- **Writes**: nested inserts, upserts with `on_conflict`, `update_many`, the
  update operators, and one transaction across a mutation naming several root
  fields.
- **Aggregates**: count, sum and rank a table without fetching it; count a
  row's children without fetching them.
- **Subscriptions**: each field is a live query — the answer now, and again
  whenever it changes.
- **Errors**: Hasura's envelope, including the path that names a place in the
  *request* rather than in the response.
- **Permissions**: a schema per role built from a schema cache already reduced
  to what that role can see; row filters in the same language a `where` is
  written in; reading and writing as two column sets, because a role may write
  a column it cannot read; presets, ceilings, `backend_only`, and `_exists`.
- **Names**: `PGRST_GRAPHQL_METADATA` carries what a schema cannot —
  relationship names, root field names, and what each role may do.
- **Auth**: `PGRST_HASURA_ADMIN_SECRET` and `PGRST_HASURA_UNAUTHORIZED_ROLE`,
  both also read under their `HASURA_GRAPHQL_*` spellings. `x-hasura-*` headers
  become session variables a policy can read.

### Added — REST

- Resource embedding through junction tables, spreads embedded in the parent
  query, and `*` as a select item.
- Filter, order and page an embedded list; order by an embedded column.
- Computed relationships, and functions returning rows of a table.
- `Prefer: missing=default`, `Prefer: max-affected`.
- Custom media types, a search path set per request, and a `Location` on a
  created row.
- Database errors passed through verbatim in compatibility mode.

### Fixed

- **`Range` headers were ignored unless they began `0-`.** Every other range
  silently returned the whole relation. `Range: 5-9` now means rows 5 to 9,
  and an inverted range is refused with 416 rather than widened.
- **`OPTIONS` never reached the handler**, so no response carried `Allow`. The
  CORS layer answers every `OPTIONS` itself and never calls what it wraps.
- **`OPTIONS` on non-API mounts** — `/admin`, `/_`, `/healthz`, `/v1/version`
  and the GraphQL endpoints — was answered as a question about a table, which
  replaced a working CORS preflight with a refusal.
- **JWT `exp` was honoured with 30 seconds of slack.** Expiry is now checked to
  the second; the slack remains only on `nbf` and `iat`, which describe a token
  not yet valid rather than one withdrawn.
- **A non-string role claim named a role.** It now names none.
- **A token could not select among the roles it was issued.** An
  `X-Hasura-Role` header is now honoured against the token's
  `x-hasura-allowed-roles`, which sits inside the signature.
- **PGRST301 and PGRST302 were documented the wrong way round.**
- Computed relationships resolved only where the parent row was already
  present.
- An HTTP/2 connection preface is passed through rather than read as a request
  line.

### Changed — breaking

These are the reason the version is 1.0.0 rather than 0.4.1.

- `postrust_auth::JwtError` lost `MissingHeader`, `Expired`, `NotYetValid`,
  `InvalidToken`, `MissingRole` and `InvalidAudience`, and gained `NoIdentity`,
  `SecretMissing`, `NoSuitableKey` and `Claim`.
- `postrust_core::api_request::Range` gained `offset_explicit`.
- `postrust_response::QueryResult` gained `allow`.
- `postrust_core::schema_cache::Table` gained `unique_constraints`,
  `description`, `row_argument`, `session_argument` and others.

None of these types is `#[non_exhaustive]`, so a downstream struct literal must
be updated. Marking them is planned during the alpha series.

### Known gaps

- **Introspection**, and it is not reachable from here: async-graphql builds
  its own registry and keeps it private, so the directives it installs and the
  order it lists types in cannot be changed from outside the library. Eight of
  the sixteen remaining Hasura divergences are this.
- **`_stream` subscriptions** — the cursor-based half of Hasura's subscription
  surface. Live queries are done.
- **The OpenAPI document at `/`** that PostgREST serves. Postrust serves
  OpenAPI 3.0 for its own surface under `/admin` and does not yet generate
  PostgREST's.
- **Actions and Apollo federation** are subsystems rather than gaps.

Both `FINDINGS.md` files record the rest, including four faults found in the
Hasura harness itself — one of which invalidated eleven runs — and the
divergences kept on purpose.
