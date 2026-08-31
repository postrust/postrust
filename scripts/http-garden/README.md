# Conformance harnesses for postrust-proxy

Two external suites, targeting the two places the proxy is least tested.

## HTTP Garden — HTTP/1.1 parsing and forwarding

[HTTP Garden](https://github.com/narfindustries/http-garden) is a differential
fuzzer for HTTP servers and proxies. It sends a payload through the proxy and
shows how several origin servers parsed what came out the other side. Where they
disagree, there is a parsing discrepancy — the request-smuggling family of bugs.

This is the suite that matters most right now: nothing in
`crates/postrust-proxy/src/vendored/forwarder.rs` strips hop-by-hop headers, so
`Connection`, `Upgrade`, `Transfer-Encoding`, `TE`, and `Keep-Alive` are all
forwarded verbatim to the upstream. RFC 9110 §7.6.1 says a proxy must not do
that.

```bash
scripts/http-garden/setup.sh
```

That clones the Garden into `scripts/http-garden/garden` (gitignored), installs
`images/postrust` into it, snapshots the working tree so uncommitted changes are
what gets tested, registers the target as a `transducer` in the Garden's
`docker-compose.yml`, and builds the image. Then:

```bash
cd scripts/http-garden/garden
./garden.sh start postrust hyper nginx gunicorn
./garden.sh repl
```

In the repl:

```
garden> payload 'GET / HTTP/1.1\r\nHost: a\r\nConnection: keep-alive\r\n\r\n' | transduce postrust | fanout | grid
```

`transduce postrust` pushes the payload through our proxy; `fanout` hands the
result to each origin; `grid` shows which origins agreed. Re-run `setup.sh`
after changing proxy source to refresh the snapshot.

Note the Garden is GPL-3.0. It is a harness we run, not code we link, so it
lives outside this repo and is never vendored.

## Pebble — ACME

[Pebble](https://github.com/letsencrypt/pebble) is Let's Encrypt's test CA. It
misbehaves on purpose: it rejects a share of nonces, varies challenge ordering,
and returns states a happy-path client will not expect. That is what makes it
useful — no code in this crate has ever talked to a certificate authority.

```bash
docker compose --profile acme up -d pebble challtestsrv
curl -sk https://localhost:14000/dir              # ACME directory
curl -sk https://localhost:15000/roots/0 > pebble-root.pem   # trust this root
```

`challtestsrv` answers every DNS query with one address, so point it at whatever
should serve http-01 challenges before ordering:

```bash
curl -X POST http://localhost:8055/set-default-ipv4 -d '{"ip":"10.30.50.4"}'
```

`10.30.50.4` is deliberately left free in the `acmenet` subnet for the proxy
under test to occupy.

Tear down with `docker compose --profile acme down`.

## Not wired up

- **h2spec** — waiting on HTTP/2 support. `ProxyService::serve_http` uses
  `hyper::server::conn::http1` only, despite the crate docs claiming H2.
- **Autobahn|Testsuite** — waiting on `Upgrade` handling, which does not exist.
