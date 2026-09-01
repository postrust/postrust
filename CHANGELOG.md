# Changelog

Notable changes, newest first. This file starts at 1.0.0-alpha.1; earlier
releases are described by their tags and the pull requests behind them.

## Unreleased

### Fixed

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

### Changed

**`postrust-worker` moved to its own `0.x` version line** as well, for a
blunter reason: it is a stub. Its fetch handler answers `{"status": "stub"}`
and does not parse requests, reach a database, or return data. It is built as
a `cdylib`, so it cannot be a Rust library dependency at all.

**`postrust-proxy` moved to its own `0.x` version line** and no longer shares
the workspace version. The rest of the workspace is heading for 1.0.0 and a
semver promise; this crate is not ready to make one. Turning `missing_docs` on
for it produces 306 warnings, and parts of its public surface answer
successfully without doing anything -- `config::database` returns an empty
route set instead of querying, and the admin API's route and upstream mutations
change only the in-memory config while replying `200`/`201`. Nothing depends on
`postrust-proxy`, so no crate that makes a semver promise carries a dependency
that cannot keep one.

Both lines still release on the same git tags: the publish step now reads each
crate's own declared version rather than assuming one across the workspace, and
skips what is already on crates.io.

New: [docs/stability.md](docs/stability.md), stating what a version number
covers and what it does not.

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
