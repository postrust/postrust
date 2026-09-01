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

## Current result: 145 passed, 1 skipped, 0 failed

`run.sh` targets a **dedicated HTTP/2-only listener** (`http2_port`, default
19081), which is why §3.5 case 2 now passes.

That case, "sends invalid connection preface", expects
`GOAWAY(PROTOCOL_ERROR)`. The shared port cannot give it one: it sniffs the
opening bytes to choose a protocol, and a corrupted h2 preface is
indistinguishable from a malformed HTTP/1 request. Confirmed directly against
the shared port:

| bytes sent | response |
| --- | --- |
| `PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n` | HTTP/2 `SETTINGS` frame |
| `PRI * HTTP/2.0\r\n\r\nXX\r\n\r\n` | `HTTP/1.1 400 Bad Request`, then close |

A listener that only ever speaks HTTP/2 has no such ambiguity, so it answers the
way the RFC asks. Crucially this is **additive**: the shared port still serves
HTTP/1.1 and h2c together, so nothing was traded away for the last case. Set
`http2_port` in the config to enable it.

To see the old behaviour, aim the suite at the shared port:

```bash
H2_PORT= scripts/h2spec/run.sh http2/3.5
```

## Running part of the suite

```bash
scripts/h2spec/run.sh http2/6.9
scripts/h2spec/run.sh --strict
```

Arguments pass straight through to h2spec. Logs land in `.work/` (gitignored):
`h2spec.log` for the run, `proxy.log` for the proxy side.
