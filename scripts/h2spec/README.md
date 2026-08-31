# h2spec — HTTP/2 conformance

[h2spec](https://github.com/summerwind/h2spec) speaks HTTP/2 directly to
postrust-proxy's h2c listener.

```bash
scripts/h2spec/run.sh
```

The proxy runs natively; only h2spec runs in Docker, reaching the host through
`host.docker.internal`. Nothing needs to be built as an image.

## What this actually covers

h2spec exercises the **listener's** HTTP/2 handling — framing, flow control,
HPACK, stream state. It does not exercise the upstream leg, which postrust
always speaks as HTTP/1.1 regardless of how the client arrived.

So most of what passes here is hyper's HTTP/2 implementation, not ours, and a
broad pass is the expected result rather than an achievement. The value is in
catching places where our own wiring breaks h2 semantics. That is not
hypothetical: when h2c was first turned on, the forwarded request kept its
HTTP/2 version onto the HTTP/1.1 upstream connection and hyper's client rejected
every one as `UserUnsupportedVersion` — a 502 on all h2c traffic.

## Current result: 144 passed, 1 skipped, 1 failed

The one failure is **§3.5 case 2, "Sends invalid connection preface"**. h2spec
expects `GOAWAY(PROTOCOL_ERROR)`; it gets the connection closed without one.

This is inherent to serving HTTP/1.1 and h2c on the same port, not a bug in the
proxy. The listener sniffs the first bytes to decide which protocol it is
speaking, and a corrupted h2 preface is indistinguishable from a malformed
HTTP/1 request. Confirmed directly:

| bytes sent | response |
| --- | --- |
| `PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n` | HTTP/2 `SETTINGS` frame |
| `PRI * HTTP/2.0\r\n\r\nXX\r\n\r\n` | `HTTP/1.1 400 Bad Request`, then close |

The RFC's requirement is that the endpoint terminate the connection, which it
does; what is missing is the `GOAWAY` frame, because the server never concluded
the connection was HTTP/2. A real h2c client sends a valid preface and is
unaffected — this only shows up for a broken or hostile one, which still gets
rejected and disconnected.

**145/146 is therefore the ceiling for a sniffing listener.** Getting the last
case would mean serving HTTP/2 on a dedicated port with `http2::Builder`, giving
up h1-and-h2c-on-one-port. That trade has not been made.

## Running part of the suite

```bash
scripts/h2spec/run.sh http2/6.9
scripts/h2spec/run.sh --strict
```

Arguments pass straight through to h2spec. Logs land in `.work/` (gitignored):
`h2spec.log` for the run, `proxy.log` for the proxy side.
