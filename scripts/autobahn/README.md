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

## Current result: 517 cases, 0 failed

| behavior | count |
| --- | --- |
| OK | 289 |
| UNIMPLEMENTED | 216 |
| NON-STRICT | 9 |
| INFORMATIONAL | 3 |
| **FAILED** | **0** |

**The 216 UNIMPLEMENTED are all sections 12 and 13** — `permessage-deflate`
compression — and they are the origin's limitation, not the proxy's.
AutobahnPython 0.10.9 in `-m echoserver` mode does not negotiate the extension,
so its 101 comes back without a `Sec-WebSocket-Extensions` header and the client
scores every compression case UNIMPLEMENTED.

The proxy forwards the negotiation faithfully. Verified by sending an offer
through and reading what the origin received:

```
sec-websocket-extensions: permessage-deflate; client_max_window_bits
sec-websocket-protocol: chat, superchat
```

Both arrive intact, which is what you would expect: neither header is
hop-by-hop, so extension and subprotocol negotiation stays end-to-end. Point the
suite at an origin that offers deflate and these cases should exercise properly.

**The 9 NON-STRICT** (3.2–3.4, 4.1.3–4.1.5, 4.2.3–4.2.4, 5.15) are reserved-bit,
reserved-opcode, and fragmentation cases — judgements about how the endpoint
handles invalid frames, which the proxy never inspects.

The baseline run settles which of these are ours. Comparing the two:

| behavior | proxied | direct |
| --- | --- | --- |
| OK | 289 | 290 |
| NON-STRICT | 9 | 8 |
| INFORMATIONAL | 3 | 3 |
| UNIMPLEMENTED | 216 | 216 |
| FAILED | 0 | 0 |

UNIMPLEMENTED is identical in both, and 8 of the 9 NON-STRICT appear in the
baseline too — all origin properties, confirmed. Exactly one case is worse
through the proxy than direct, and it turns out to be an origin property as
well; see below.

## Case 3.4 — an origin artifact, not a tunnel bug

Case 3.4 scores OK direct but NON-STRICT through the proxy, intermittently. It
looks like a proxy defect and is not one.

The case sends a valid text frame, then the same frame with a reserved bit set,
then a Ping — **in octet-wise chops** — and expects the echo of the first
message before the connection is failed. Through the proxy the client receives
nothing (`received: []`), and the proxy's own tunnel accounting says
`0 bytes down`: the echo never arrives from the origin at all.

The origin's behaviour depends on TCP segmentation. Sending the exact 3.4
sequence straight at the echoserver, **with no proxy in the path**:

| how the bytes arrive | origin's response |
| --- | --- |
| octet-wise chops | 15 bytes — echo received |
| one coalesced write | 0 bytes — no echo |

AutobahnPython's echoserver fails the connection without flushing its echo when
it reads the valid frame and the RSV frame in a single `read()`. A proxy relays
through a buffer, so it naturally coalesces what the client chopped — and TCP
does not preserve message boundaries, so any relay is entitled to. The test is
measuring segmentation, which the proxy legitimately changes.

This is why `summarize.py` excludes 3.4 from the regression check by name, with
that reasoning attached, and prints the exclusion on every run rather than
applying it silently.

Two things were changed while chasing this, both worth keeping on their own
merits, neither of which fixed 3.4:

- The tunnel no longer uses `copy_bidirectional`, which returns on the first
  error in either direction and abandons the other mid-flight. Each direction
  now drains independently and half-closes its peer.
- `TCP_NODELAY` is now set on both legs and on the pooled client. It was set
  nowhere, so Nagle was batching small writes and adding latency to every
  small forwarded response, WebSocket or not.

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
