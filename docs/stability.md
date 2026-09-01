# Stability and versioning

What a Postrust version number promises, and what it does not.

## Two version lines

The workspace does not ship at a single version. There are two lines, because
the crates in them are at genuinely different stages and one number cannot
honestly describe both.

| Line | Crates | Promise |
| --- | --- | --- |
| **Stable** | `postrust-core`, `postrust-sql`, `postrust-auth`, `postrust-response`, `postrust-graphql`, `postrust-server`, `postrust-lambda` | Semver. A breaking change to the public Rust API needs a major bump. |
| **Unstable** | `postrust-proxy`, `postrust-worker` | None. Each on its own `0.x`, where a minor bump may break anything. |

Nothing in the stable line depends on either unstable crate, so no crate that
makes a semver promise carries a dependency that cannot keep one.
`postrust-proxy` depends on `postrust-core`, not the other way round, and
`postrust-worker` is built as a `cdylib`, which cannot be a Rust library
dependency at all.

Both lines release on the same git tags. A tag publishes whatever version each
crate currently declares and skips what is already on crates.io, so the proxy
is not dragged along to a version it has not earned.

One wrinkle from the transition: both unstable crates were published at
`1.0.0-alpha.1` and `1.0.0-alpha.2` before the split, and that `1.0.0` line
will never get a stable successor for either. Both are prereleases, so Cargo
will not select them by default — `cargo add postrust-proxy` resolves to the
newest `0.x`. Only an explicit request reaches code that predates the split.

## Why the worker is held back

`postrust-worker` is a stub, and says so in its own response body: the fetch
handler answers `{"status": "stub"}` and nothing else. It does not parse
requests, reach a database, or return data. Finishing it needs Cloudflare
pieces that are not written yet -- Hyperdrive for database connections, KV or
Durable Objects for the schema cache.

A 1.0.0 on that would be a promise to keep an interface that does not do
anything yet.

## Why the proxy is held back

Not caution for its own sake. Three specific things:

**The public surface is largely undocumented.** Turning `missing_docs` on for
`postrust-proxy` produces hundreds of warnings, most of them struct fields. A
1.0.0 would freeze all of it as it stands. `cargo clippy` with the crate-level
`allow`s removed is the current count.

**Manual certificate upload is not implemented.** ACME issuance is, and is
tested against a real CA, but a tenant that wants to supply its own certificate
has no endpoint to do it through. `ssl_provider = "manual"` is a state the
schema can hold and nothing acts on.

**It has only just grown its transport layer.** TLS, HTTP/2, WebSocket and
RFC 8441 extended CONNECT all landed in `1.0.0-alpha.2`. They are measured, by
h2spec and Autobahn and the HTTP Garden differential fuzzer — see
`scripts/h2spec/`, `scripts/autobahn/` and `scripts/http-garden/` — but
measured is not the same as lived with.

The crate is useful and it is tested. It is not finished, and its version
number says so.

## What semver covers in the stable line

Covered — a breaking change here requires a major bump:

- The public Rust API of each stable crate: exported types, traits, functions,
  their signatures, and public enum variants and struct fields.
- The HTTP surface of `postrust-server`: routes, request grammar, status codes,
  and the response shapes the conformance suites replay.
- Configuration keys and their meanings.
- The minimum supported Rust version.

Not covered:

- `postrust-proxy` and `postrust-worker`, entirely.
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
