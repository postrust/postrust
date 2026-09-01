# ACME — certificate issuance against a real CA

```bash
scripts/acme/run.sh
```

Runs `crates/postrust-proxy/tests/acme_issuance.rs` against
[Pebble](https://github.com/letsencrypt/pebble), Let's Encrypt's test CA.
Everything it starts is removed on exit.

## Why Pebble and not a mock

Pebble deliberately misbehaves. It rejects a share of nonces, varies challenge
ordering, and returns states a happy-path client does not expect. A mock CA
would confirm that our code does what we wrote; Pebble confirms it survives a
CA that does not cooperate. It is also the same server Let's Encrypt uses to
test its own clients.

## What has to be true, and how the harness arranges it

An ACME test is mostly plumbing, because HTTP-01 requires the CA to reach *us*:

| Requirement | How |
| --- | --- |
| A CA to talk to | `pebble`, on the `acmenet` subnet from `docker-compose.yml` |
| We trust the CA's directory TLS | its minica root, copied out of the image |
| The test domain resolves to something the CA can reach | `challtestsrv`, answering every query with one address |
| The CA can reach our challenge endpoint at that address | a `socat` container at `10.30.50.4`, forwarding to the test's listener on the host |

Two details that cost real time to find, recorded so they do not have to be
found again:

- **The root is not `/roots/0`.** The management API's `/roots/0` returns the
  root pebble *issues* from. Pebble's own directory is served under
  `test/certs/localhost/cert.pem`, signed by `test/certs/pebble.minica.pem`
  baked into the image — that is the one a client has to trust. Using
  `/roots/0` fails verification, and the client reports it as
  `client error (Connect)`, which says nothing about certificates.
- **Pebble validates HTTP-01 on port 5002**, not 80, so the forwarder listens
  there. A real CA uses 80.

`ACME_DIRECTORY` uses `127.0.0.1` rather than `localhost`, because `localhost`
resolves to `::1` first on macOS and pebble binds IPv4 only.

## What the test covers

End to end, with no mocking below the CA:

- account registration, and its persistence in `proxy_acme_accounts` — so a
  second issuance does not register again and spend a rate limit
- order placement and the `http-01` challenge
- the **shipped** `/.well-known/acme-challenge/{token}` handler, served from
  `saas_router`, reading the row the issuer wrote
- pebble fetching that challenge over the network and validating it
- finalization, the CSR, and the chain coming back
- the issued certificate covering the domain that was asked for
- storage reaching `proxy_certificates`, not only the file cache
- challenge rows being deleted afterwards — a token left answerable is a value
  the proxy hands to anyone who asks

And the common failure: a domain that does not resolve to the proxy must fail
with a message saying so, and must not leave its challenge row behind.

## Options

| Variable | Default | Purpose |
| --- | --- | --- |
| `CHALLENGE_PORT` | `15080` | Host port the test's challenge listener binds |
| `PG_PORT` | `55436` | Host port for the throwaway PostgreSQL |
| `TEST_DOMAIN` | `acme-test.example` | The name to get a certificate for |

Logs land in `.work/` (gitignored): `test.log` for the run, and
`pebble-root.pem` for the root that was used.

## Not covered

Renewal, and the worker in `saas::ssl`. The worker's scheduling and backoff are
unit-tested, but nothing here advances a clock far enough to watch a
certificate come up for renewal. Pebble issues short-lived certificates, so
that is possible to add.
