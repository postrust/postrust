# Security Policy

## Reporting a vulnerability

Please report security issues privately, through
[GitHub's security advisory form](https://github.com/postrust/postrust/security/advisories/new).
That opens a private thread with the maintainers and lets us prepare a fix and
an advisory together.

**Please do not open a public issue for a vulnerability.** A public report
starts the clock for everyone running the software, including people who cannot
upgrade the same day.

If the advisory form does not work for you, email
[technology@bimaplan.co](mailto:technology@bimaplan.co) with `SECURITY` in the
subject.

What helps most, in rough order:

- what an attacker can do, and what they need to start
- the smallest reproduction you have — a request, a config, a schema
- the version or commit you saw it on
- whether you think it is already public

You do not need a CVE, a severity score, or a patch. A clear description of the
behaviour is enough.

## What to expect

- **Acknowledgement within 3 working days.** If you have not heard back in a
  week, please chase — a missed notification is far more likely than a decision
  not to reply.
- An assessment of whether we can reproduce it, and what we think the impact
  is. If we disagree with your severity, we will say why rather than quietly
  downgrading it.
- A fix, or an explanation of why the behaviour is intended, along with the
  configuration that avoids it.
- Credit in the advisory and the changelog, unless you would rather not be
  named.

We do not run a bug bounty.

## Supported versions

Fixes land on `main` and go out in the next release. There is no separate
maintenance branch: this project is young enough that "upgrade to the latest
release" is the whole backport policy.

Two crates are on their own `0.x` lines and carry no stability promise —
`postrust-proxy` and `postrust-worker`. A security fix for either may arrive in
a release that also breaks their API. See
[docs/stability.md](docs/stability.md).

| Version | Supported |
| --- | --- |
| `1.0.0-beta.1` and later | yes |
| everything earlier | no — upgrade |

## Scope

In scope, and treated as vulnerabilities:

- request smuggling, header smuggling, or response splitting through the proxy
- authentication or authorization bypass — a JWT accepted that should not be, a
  role or RLS policy escaped, a tenant reading another tenant's data
- SQL injection, including through a filter, an embedded resource, or an RPC
  argument
- a domain-ownership check that can be passed without controlling the domain
- secrets in logs, error bodies, or responses
- remote crashes reachable without authentication

Known and documented, so not a report — but a *better* attack on any of these
is very much in scope:

- **HTTP domain verification is weaker than DNS.** The proxy serves the
  challenge for a domain that already points at it, so passing shows the domain
  resolves here, not that the claimant controls it. DNS verification is the
  default and the one to use. See
  [docs/saas-domains.md](docs/saas-domains.md).
- **Automatic ACME provisioning is not implemented.** A domain configured for
  `acme` is left `pending`; nothing here has talked to a certificate authority.
- **`postrust-worker` is a stub.** It answers a fixed JSON body.

Out of scope:

- findings from a scanner with no demonstrated impact, including a version
  number matched against a CVE database without a reachable code path — see
  below
- denial of service by ordinary load, or by a configuration the documentation
  says not to use
- anything requiring a database superuser, filesystem access, or an already
  compromised host
- missing hardening headers on the admin UI, absent a concrete attack

## A note on dependency advisories

We check reachability before treating a locked version as an exposure, and we
ask that reports do the same.

A recent example: `Cargo.lock` carried a `quinn-proto` version flagged for a
QUIC parsing panic. `quinn` is an optional dependency of `reqwest`, and this
workspace uses `reqwest` with `default-features = false`, so `cargo tree -i
quinn-proto` finds it in **no** build graph — nothing QUIC is ever compiled,
let alone listening. The lockfile entry was still worth updating, and it was;
but it was never an exposure, and reporting it as a remotely exploitable
denial of service would have been wrong.

`cargo audit` runs in CI so lockfile advisories surface on their own. What we
need from a report is the part a tool cannot supply: the path from an attacker
to the vulnerable code.
