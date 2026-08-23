#!/usr/bin/env python3
"""
Replay extracted PostgREST cases against one server and record raw responses.

Every case must start from the same database state, or one divergence
cascades into phantom failures in everything after it. Reads cannot disturb
that state, so they run first as a block. Each mutating case then gets the
fixture data restored immediately before it.

The restore reloads `data.sql` only. That file is data-only -- it truncates
and re-inserts, with no DDL -- so object OIDs never change and neither server
needs restarting to drop a stale schema cache.
"""
import json
import subprocess
import sys
import time
import urllib.error
import urllib.request

CASES, BASE, OUT = sys.argv[1], sys.argv[2].rstrip("/"), sys.argv[3]
# Optional: a shell command that restores the fixture data in place.
RESET_CMD = sys.argv[4] if len(sys.argv) > 4 else None

cases = json.load(open(CASES))
reads = [c for c in cases if not c["mutating"]]
writes = [c for c in cases if c["mutating"]]


def encode_path(p):
    """hspec-wai accepts raw spaces in a URL; urllib does not. Encode only the
    characters that are illegal on the wire, leaving PostgREST's query syntax
    (parens, commas, stars, dots) untouched."""
    return "".join("%%%02X" % ord(ch) if ch == " " or ord(ch) < 0x21 else ch for ch in p)


def reset():
    if RESET_CMD:
        subprocess.run(RESET_CMD, shell=True, check=True,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def send(c):
    url = BASE + encode_path(c["path"])
    body = c["body"].encode() if c["body"] else None
    req = urllib.request.Request(url, data=body, method=c["method"])
    # hspec-wai sends JSON bodies without an explicit content type; both
    # servers need one to parse them, and PostgREST's own default is JSON.
    if body and not any(h[0].lower() == "content-type" for h in c["headers"]):
        req.add_header("Content-Type", "application/json")
    # `add_header` overwrites, so a case sending the same header twice --
    # `Prefer: return=representation` and `Prefer: resolution=merge-duplicates`
    # are separate headers in the spec suite -- would reach the server as only
    # the last of them. Combining them into one comma-separated value is what
    # RFC 7230 says a recipient may do, and it is the only spelling `urllib`
    # can send.
    combined = {}
    for name, value in c["headers"]:
        key = name.lower()
        combined[key] = f"{combined[key]}, {value}" if key in combined else value
    for name, value in combined.items():
        req.add_header(name, value)

    rec = {k: c[k] for k in ("id", "spec", "method", "path", "mutating")}
    try:
        with urllib.request.urlopen(req, timeout=20) as r:
            rec.update(status=r.status,
                       headers={k.lower(): v for k, v in r.headers.items()},
                       body=r.read().decode("utf-8", "replace"))
    except urllib.error.HTTPError as e:
        rec.update(status=e.code,
                   headers={k.lower(): v for k, v in e.headers.items()},
                   body=e.read().decode("utf-8", "replace"))
    except Exception as e:                       # connection reset, timeout, ...
        rec.update(status=None, headers={}, body="",
                   error=f"{type(e).__name__}: {e}")
    return rec


results = []
t0 = time.time()

# Reads first, from one pristine load: none of them can disturb the others.
reset()
for n, c in enumerate(reads, 1):
    results.append(send(c))
    if n % 250 == 0:
        print(f"  reads  {n}/{len(reads)}  ({time.time() - t0:.0f}s)", flush=True)

# Then writes, each from a freshly restored copy of the same data.
for n, c in enumerate(writes, 1):
    reset()
    results.append(send(c))
    if n % 50 == 0:
        print(f"  writes {n}/{len(writes)}  ({time.time() - t0:.0f}s)", flush=True)

json.dump(results, open(OUT, "w"))
errs = sum(1 for r in results if r["status"] is None)
print(f"done: {len(results)} requests "
      f"({len(reads)} read, {len(writes)} write), "
      f"{errs} transport failures, {time.time() - t0:.0f}s")
