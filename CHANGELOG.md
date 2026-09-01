# Changelog

Notable changes, newest first. This file starts at 1.0.0-alpha.1; earlier
releases are described by their tags and the pull requests behind them.

## Unreleased

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
