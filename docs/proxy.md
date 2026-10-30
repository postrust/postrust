# Reverse Proxy

`postrust-proxy` is a reverse proxy: it terminates HTTP, matches a request
against a route table, and forwards it to a backend. It is used to put Postrust
behind one hostname alongside other services, and to give SaaS customers custom
domains of their own.

> **On its own `0.x` line, with no stability promise.** The rest of the
> workspace is heading for 1.0.0; this crate is not, and its version says so.
> See [Stability and Versioning](./stability.md) for exactly why. It is useful
> and it is measured — but a minor release may break it.

## Running it

```bash
postrust-proxy path/to/config.toml
# or
POSTRUST_PROXY_CONFIG=path/to/config.toml postrust-proxy
```

The smallest config that does something:

```toml
[server]
http_host = "0.0.0.0"
http_port = 8080

[[upstreams]]
name = "api"

[[upstreams.backends]]
address = "127.0.0.1:3000"
scheme = "http"

[[routes]]
name = "everything"
upstream = "api"

[routes.match]
path = "/"
```

`DATABASE_URL` is optional. It is needed for database-backed routing, the admin
API's persistence, the SaaS module, and ACME; without it the proxy runs from the
file alone.

## What it speaks

| | Client side | Upstream side |
| --- | --- | --- |
| HTTP/1.1 | yes | yes (default) |
| HTTP/2 cleartext (h2c) | yes, on the same port, by sniffing | opt-in per backend |
| HTTP/2 over TLS | yes, by ALPN | no |
| WebSocket | `ws://` and `wss://` | `ws://` |
| WebSocket over HTTP/2 (RFC 8441) | yes, translated to an HTTP/1.1 upgrade | — |

**HTTP/2 is per-hop.** A request that arrives over h2 is forwarded as HTTP/1.1
unless the backend declares otherwise, because that is what HTTP/2 means — it is
a connection protocol, not an end-to-end one. Getting this wrong is not
hypothetical: when h2c was first turned on, the forwarded request kept its
HTTP/2 version and hyper's client rejected every one as
`UserUnsupportedVersion`, making every h2c request a 502.

For a backend that does speak cleartext HTTP/2:

```toml
[[upstreams.backends]]
address = "10.0.0.1:8080"
scheme = "http"
http_version = "h2c"     # aliases: "h2", "http2"
```

There is no negotiation. h2c has no ALPN to fall back on, so it has to be
declared.

## Listeners

### The main port

Serves HTTP/1.1 and h2c together, choosing by looking at the opening bytes.

That sniffing has one visible consequence: a corrupted HTTP/2 connection preface
gets an HTTP/1 `400 Bad Request` rather than the `GOAWAY(PROTOCOL_ERROR)` the
RFC asks for, because a corrupted preface is indistinguishable from a malformed
HTTP/1 request. Measured directly:

| bytes sent | response |
| --- | --- |
| `PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n` | HTTP/2 `SETTINGS` |
| `PRI * HTTP/2.0\r\n\r\nXX\r\n\r\n` | `HTTP/1.1 400`, then close |

### Connection limits and timeouts

Every listener -- the main port, the HTTP/2-only port and TLS -- shares these
`[server]` settings:

```toml
[server]
max_connections = 10000            # open connections; past it, new ones wait in the backlog
header_read_timeout_secs = 30      # to read a request head, and to wait for the next one
idle_timeout_secs = 60             # with no request in flight: before the first bytes,
                                   # during a TLS handshake, or an HTTP/2 connection with no streams
http2_keepalive_interval_secs = 20 # PING an idle HTTP/2 connection; 0 turns it off
http2_keepalive_timeout_secs = 20  # close it if the PING is not answered
tunnel_idle_timeout_secs = 300     # a WebSocket with no bytes either way
```

Without them a client that opens connections and sends nothing, or trickles
a header line at a time, holds them open for as long as it likes. A timeout
of 0 is read as 1 second. An upgraded tunnel keeps its connection slot while
it is open.

### An HTTP/2-only port

```toml
[server]
http2_port = 8081
```

Optional and **additive** — the main port keeps serving both. A listener that
only ever speaks HTTP/2 has no ambiguity to resolve, so it answers a corrupted
preface the way the RFC asks. This is what makes h2spec's §3.5 case pass.

### TLS

```toml
[tls]
cert_file = "/etc/postrust/fullchain.pem"
key_file  = "/etc/postrust/privkey.pem"

[server]
https_host = "0.0.0.0"
https_port = 8443
```

`cert_file` and `key_file` must be set together, or neither. The listener offers
ALPN `h2` and `http/1.1` and dispatches on what was negotiated — which is what
makes HTTP/2 reachable as browsers actually use it, and WebSocket reachable as
`wss://`.

**The certificate is chosen per handshake, by SNI.** Certificates stored by ACME
issuance or by `ssl/upload` are served for the domains they were issued for; the
pair above, if set, is the fallback for a name that matches none of them and for
a client that sends no SNI. A wildcard covers exactly one label, as RFC 6125
requires.

**HTTPS starts when the configuration asks for it**, which is any of:

| | |
| --- | --- |
| `tls.cert_file` **and** `tls.key_file` | serve that pair, plus anything in the store |
| `tls.acme_enabled` | serve issued certificates, no static pair needed |
| `server.https_enabled` | serve stored certificates, for uploads without ACME |

With none of them, `https_host` and `https_port` alone do nothing — and neither
does a `DATABASE_URL`. Keeping routes in PostgreSQL is not a request to
terminate TLS, and treating it as one opened `0.0.0.0:8443` on proxies that had
no certificate to answer a handshake with.

Domain names are compared without regard to case or a trailing dot, so a
certificate stored as `App.Example.com.` is served for `app.example.com`.

**Private keys on disk are owner-only.** The certificate cache writes each
key as `0600`, atomically, in a directory it creates `0700`; a key left
readable by an earlier version is tightened at startup. A file name is built
only from a lowercase DNS name, so no stored domain can name a path outside
the cache.

New certificates are picked up within a minute without a restart; the resolver
re-reads the store on a timer, from the database rather than from a cache, so a
renewal by another instance is seen too.

## Routing

A route matches on host, path, headers and method, and names an upstream:

```toml
[[routes]]
name = "api"
upstream = "api_servers"
priority = 200          # higher wins; ties broken by longer path. Default 100
strip_path = true       # remove the matched prefix before forwarding
enabled = true          # false takes the route out of matching entirely
add_headers = { "x-forwarded-by" = "postrust" }
remove_headers = ["x-internal-token"]

[routes.match]
host = "api.example.com"   # exact, or `*.example.com`, or `*` for any
path = "/v1"
path_type = "prefix"       # prefix | exact | regex
methods = ["GET", "POST"]  # case-insensitive; omit or leave empty for any
headers = { "x-tenant" = "acme" }
```

**Every criterion that is set has to match**, and one left out matches anything.
Adding a criterion can only narrow a route, never widen it.

- `path_type = "exact"` means exactly that: a route on `/health` does **not**
  catch `/health-internal`.
- `path_type = "prefix"` matches whole segments: `/v1` matches `/v1` and
  `/v1/users`, not `/v1beta`. A prefix that ends in `/` keeps its literal
  meaning. `strip_path` removes the matched segments, so `/v1/users` is
  forwarded as `/users`.
- **A path that could step outside its route is refused with 400** before any
  route is matched: a `.` or `..` segment, literal or percent-encoded in any
  case (`%2e%2e`, `..;x`), an encoded `/` or `\` (`%2f`, `%5c`), or a literal
  `\`. Matching the raw text would forward `/public/../admin` under the
  `/public` route to a backend that may well resolve it to `/admin`.
- `path_type = "regex"` compiles the path as a regex. One that does not compile
  matches *nothing*, and logs why — treating it as a prefix, or as a match-all,
  would turn a typo into a route that catches traffic it was never meant to.
- A `*.example.com` host covers exactly one label: `api.example.com`, but not
  `example.com` and not `a.b.example.com`.

A route on a disabled upstream gets **503**, and a disabled backend gets no
traffic. When every enabled backend of an upstream is failing its health
check, requests are still spread over the enabled ones rather than refused: a
wrong health check should not become an outage, and a backend that really is
down answers 502 anyway. A disabled backend is never the fallback.

`timeout_secs` is honoured: an upstream that does not answer in time gets a
**504**, distinct from the 502 an upstream that refuses gets, because an
operator reads those differently. `retry_count` is still declarable and unread.

Upstreams are matched by name:

```toml
[[upstreams]]
name = "api_servers"
lb_strategy = "round_robin"   # round_robin | least_connections | weighted | random | sticky
enabled = true

[upstreams.health_check]
enabled = true
path = "/health"
interval_secs = 30
timeout_secs = 5
healthy_threshold = 2
unhealthy_threshold = 3

[[upstreams.backends]]
address = "10.0.0.1:8080"
scheme = "http"               # http | https
weight = 1                    # share under `weighted`; 0 drains it
enabled = true
```

Under `weighted`, shares are computed over the backends that are eligible at
that moment. A backend with weight 0 next to others gets nothing; if every
weight is 0 they are treated as equal.

### Rate limiting

Defaults apply to everything; a route can override them.

```toml
[rate_limit]
requests = 1000
window_secs = 60
burst = 50
max_tracked_keys = 100000     # clients tracked at once, across routes

[routes.rate_limit]
requests = 100
window_secs = 60
key = "client_ip"             # client_ip | route | { header = "x-api-key" }
```

- The rate is `requests / window_secs` as a fraction, so `requests = 3,
  window_secs = 60` refills one request every 20 seconds.
- Each route counts on its own: a client's requests to one route do not use
  up, or set the size of, its allowance on another.
- `key` says what a bucket belongs to: `client_ip` (the connecting address,
  with an IPv6 client counted by its /64), `route` (one bucket for the whole
  route), or a header's value (stored as a digest; a request without the
  header is counted by its address).
- Buckets that have refilled completely are dropped in the background. If
  `max_tracked_keys` are in use and none can be dropped, a new client gets 429
  rather than going unlimited.

## What it does to requests

**Hop-by-hop headers are stripped** before forwarding, per RFC 9110 §7.6.1:
`Connection`, `Keep-Alive`, `Proxy-Authenticate`, `Proxy-Authorization`, `TE`,
`Trailer`, `Transfer-Encoding`, `Upgrade` — *and* every token the incoming
`Connection` header names. Both halves matter. Forwarding `Connection` itself
was letting a client smuggle arbitrary headers past the proxy.

**A request's framing is the proxy's, not the client's.** The HTTP/1 parser
refuses a duplicate or disagreeing `Content-Length` and a `Transfer-Encoding`
that does not end in `chunked`; given both `Content-Length` and
`Transfer-Encoding: chunked`, it reads the chunked body and drops the length,
as RFC 9112 §6.3 requires. The body is then re-framed for the upstream, so
the backend never sees the client's framing headers at all.

**Forwarding headers are the proxy's own.** By default `x-forwarded-for` and
`x-real-ip` are set to the connecting address, `x-forwarded-proto` and
`x-forwarded-host` to what the proxy saw, and a client's `forwarded` header is
removed: anything a client sends in these is replaced, since a backend that
trusts them would otherwise believe whatever the client claimed. Under HTTP/2
there is no `Host` header, so the host falls back to the `:authority`
pseudo-header.

Behind another proxy you control, set `server.trust_forwarded_headers = true`:
the incoming `x-forwarded-for` and `forwarded` chains are kept and this hop is
appended, and `x-forwarded-proto`/`-host` are passed on. `x-real-ip` is still
the connecting address. Rate limiting by `client_ip` always uses the
connecting address.

**Only WebSocket upgrades are tunnelled.** `server.allowed_upgrade_protocols`
(default `["websocket"]`) lists the protocols an HTTP/1 `Upgrade` or an
HTTP/2 extended `CONNECT` may ask for. Any other HTTP/1 upgrade -- `h2c`
above all -- is forwarded as an ordinary request with the upgrade removed, and
`HTTP2-Settings` is stripped; another HTTP/2 `:protocol` gets 501. A tunnel to
a backend that accepted `h2c` would carry requests the proxy never saw, past
every route's path and method rules.

**`TCP_NODELAY` is set** on the accepted socket, the upgraded socket, and the
pooled upstream connector. Without it Nagle batches small writes and adds
latency to every small forwarded response.

## Database-backed configuration

Routes and upstreams can live in the database instead of, or as well as, the
file:

```bash
DATABASE_URL=postgres://... postrust-proxy config.toml
```

The tables are in `crates/postrust-proxy/migrations/`. Database entries are
added to whatever the file declared, and **a name declared in both refuses to
start** — upstream identity is derived from the name, so a duplicate would
collide in the routing tables and one upstream would silently take the other's
traffic.

The trigger is `DATABASE_URL` being set, not `server.database_config` (which
defaults to `true`): keying off the flag alone would make every file-configured
proxy try to reach a database that is not there.

## Admin API

`admin::admin_router()` serves CRUD over routes, upstreams and backends, and
reads health. Mount it on a port you control — it has **no authentication of its
own**.

Mutations write through to the database before touching the running config, and
return `500` if they cannot. With `server.database_config = false` — an explicit
"this proxy is configured from a file" — they change only the running config and
last as long as the process.

## Multi-tenant custom domains

A separate, larger surface: tenants, API keys, domain-ownership verification,
per-domain routes, and ACME certificates. See
[SaaS Domain Management](./saas-domains.md).

## What is measured

Three suites, all runnable from `scripts/`, and re-run nightly by
`.github/workflows/conformance.yml`:

| Suite | What it covers | Result |
| --- | --- | --- |
| [h2spec](https://github.com/summerwind/h2spec) | HTTP/2 at the listener — framing, flow control, HPACK, stream state | 146 tests, 145 passed, 1 skipped, 0 failed |
| [Autobahn](https://github.com/crossbario/autobahn-testsuite) | WebSocket, RFC 6455 | 517 cases, no case worse than a run that bypasses the proxy |
| [HTTP Garden](https://github.com/narfindustries/http-garden) | Differential fuzzing for smuggling and header handling | 7 of 7 probes correct |

Two things worth reading honestly. Most of what h2spec passes is hyper's HTTP/2
implementation rather than ours, so a broad pass is the expected result and the
value is in catching places where our own wiring breaks h2 semantics. And
Autobahn scores the *endpoint* as much as the tunnel — postrust splices upgraded
streams rather than parsing WebSocket frames — so the figure that is about the
proxy is the comparison against a baseline run with no proxy in the path, not
the raw pass count.

Throughput is deliberately absent. See [Benchmarking](./benchmarking.md).

## Configuration that does nothing yet

Listed rather than left to be discovered:

- **`retry_count` on a route.** Declarable, and nothing reads it. Retrying needs
  the request body to be replayable, which a streamed body is not, so this is a
  design question rather than a missing line.
- **`tls.acme_directory`** (the old field) is not read either; use
  `tls.acme_directory_url`, or `acme_staging` for Let's Encrypt staging.

**There is no configuration reload.** Changing the config needs a restart.
`ConfigReloader`, `server.watch_config_file` and `POST /config/reload` used to
suggest otherwise -- the endpoint answered "Configuration reload requested" and
sent on a channel nobody read -- and all three are gone rather than left to be
believed.

## Origins

The forwarding core is vendored from
[rust-rpxy](https://github.com/junkurihara/rust-rpxy), with database-backed
configuration, health checking, rate limiting and the transport work added. The
vendored parts live under `src/vendored/`.
