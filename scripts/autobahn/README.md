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
baseline too — all origin properties, confirmed. Exactly **one** case is worse
through the proxy than direct.

## Known flake: case 3.4

Case 3.4 scores OK direct but NON-STRICT through the proxy — and it is
intermittent. Three consecutive runs of section 3:

```
run 1: 3.4=NON-STRICT
run 2: 3.4=OK
run 3: 3.4=NON-STRICT
```

3.2 and 3.3 were NON-STRICT in all three, matching the baseline; only 3.4 moves.

The case sends a valid text message, then a frame with a reserved bit set, and
expects the endpoint to fail the connection. The intermittency points at a
flush-versus-teardown race in the tunnel: whether the echoed text reaches the
client before the origin's abrupt close propagates through
`copy_bidirectional`. It is a race on an invalid-frame path, scored NON-STRICT
rather than FAILED, so it is a candidate for follow-up rather than a defect in
normal operation — but it is ours, not the origin's, and it should not be
written off as noise without someone looking at the shutdown ordering.

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
