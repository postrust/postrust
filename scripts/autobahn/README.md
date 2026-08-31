# Autobahn — WebSocket conformance

[Autobahn](https://github.com/crossbario/autobahn-testsuite) is the reference
RFC 6455 suite. It runs as a client through the proxy to a WebSocket echo
origin:

```
fuzzingclient (docker) -> postrust-proxy (native) -> echoserver (docker)
```

```bash
scripts/autobahn/run.sh              # whole suite, through the proxy
BASELINE=1 scripts/autobahn/run.sh   # same suite, straight at the origin
```

Only Autobahn runs in Docker; the proxy runs natively and is reached through
`host.docker.internal`, so nothing needs building as an image.

## Read it against the baseline

This matters more here than for the other suites. postrust does **not** parse
WebSocket frames — it splices two upgraded byte streams — so most of what
Autobahn scores is the *echoserver's* frame handling, not ours. A non-OK case
proves nothing on its own.

**Run both, and compare.** A non-OK case that appears in the baseline as well is
a property of the origin. Only a case that is worse *through* the proxy than
direct is about the tunnel. `BASELINE=1` writes to `reports-baseline/` so the
two survive side by side.

What Autobahn genuinely tests about our code is whether the splice is faithful:
byte-exact, ordered, and correct under fragmentation, large payloads, and the
close handshake.

## Current result: 517 cases, no regressions

Against the default deflate origin, proxied and baseline are identical:

| behavior | proxied | direct |
| --- | --- | --- |
| OK | 502 | 502 |
| NON-STRICT | 11 | 11 |
| INFORMATIONAL | 3 | 3 |
| FAILED | 1 | 1 |

The single FAILED case, 7.1.5, fails with **no proxy in the path at all**, so it
is the origin's and the gate does not count it. Everything else matches. The
tunnel changes nothing that Autobahn can see.

For contrast, the old `ORIGIN=autobahn` echoserver gave 290 OK and 216
UNIMPLEMENTED, because it does not negotiate `permessage-deflate`. Those 216
compression cases were a coverage hole, not a result; they are now exercised.

The proxy forwards extension negotiation faithfully — verified by reading what
the origin receives:

```
sec-websocket-extensions: permessage-deflate; client_max_window_bits
sec-websocket-protocol: chat, superchat
```

Neither header is hop-by-hop, so negotiation stays end-to-end, which is why
pointing the suite at a deflate-capable origin was all it took.

## Case 3.4 — an origin artifact

Worth keeping on record, because it cost real time and looked like a proxy bug.

Against `ORIGIN=autobahn`, case 3.4 scored OK direct but NON-STRICT proxied,
intermittently. The client received nothing where the echo was expected, and the
proxy's own accounting said `0 bytes down` — the echo never left the origin.
Sending the exact 3.4 sequence straight at that echoserver, **no proxy in the
path**:

| how the bytes arrive | origin's response |
| --- | --- |
| octet-wise chops | 15 bytes — echo received |
| one coalesced write | 0 bytes — no echo |

AutobahnPython's echoserver fails the connection without flushing its echo when
it reads the valid frame and the RSV frame in a single `read()`. A relay buffers,
so it coalesces what the client chopped, and TCP does not preserve message
boundaries anyway — any proxy re-chunks.

The default origin confirms it from the other side: against `websockets`, 3.4
behaves identically proxied and direct, and the exclusion in `summarize.py`
never fires. That exclusion is therefore specific to `ORIGIN=autobahn`.

Two things were changed while chasing this. Neither fixed 3.4; both are worth
keeping anyway:

- The tunnel no longer uses `copy_bidirectional`, which returns on the first
  error in either direction and abandons the other mid-flight — losing data when
  the upstream closes while the client is still writing. Each direction now
  drains independently and half-closes its peer.
- `TCP_NODELAY` is set on both legs and the pooled connector. It was set
  nowhere, so Nagle batched small writes and added latency to every small
  forwarded response, WebSocket or not.

## Options

```bash
CASES='["1.*","2.*","7.*"]' scripts/autobahn/run.sh
EXCLUDE_CASES='["9.*"]' scripts/autobahn/run.sh
PORT=9080 ORIGIN_PORT=9002 scripts/autobahn/run.sh
```

A narrowed run prints a `PARTIAL run` notice, so a subset can never be mistaken
for a clean full pass. The full suite is the default for that reason.

Reports land in `reports/` (HTML plus `index.json`) and logs in `.work/`; both
are gitignored. `summarize.py` prints the table above and exits non-zero if any
case is FAILED or UNIMPLEMENTED, which makes it usable as a CI gate — though
with this origin the UNIMPLEMENTED compression cases mean it exits non-zero
today.
