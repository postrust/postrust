#!/usr/bin/env python3
"""
Diff two replay runs and classify every divergence.

Agreement is reported at four strictness levels. A single systemic gap -- one
header never emitted, say -- would otherwise sink every case and hide the
hundreds that differ in nothing else.
"""
import collections
import json
import sys

ref = json.load(open(sys.argv[1]))
cand = {r["id"] + "|" + r["method"] + r["path"]: r for r in json.load(open(sys.argv[2]))}
OUT = sys.argv[3]

CONTRACT = ["content-type", "content-range", "location",
            "preference-applied", "allow", "www-authenticate"]


def parse(b):
    try:
        return True, json.loads(b)
    except Exception:
        return False, b


def same_body(a, b):
    oka, va = parse(a)
    okb, vb = parse(b)
    if oka and okb:
        return va == vb
    if oka != okb:
        return False
    return a.strip() == b.strip()


def ctype(h):
    # charset is a harmless spelling difference; compare the essence.
    return (h or "").split(";")[0].strip().lower()


def code_of(body):
    ok, v = parse(body)
    return v.get("code") if ok and isinstance(v, dict) else None


rows = []
for r in ref:
    c = cand.get(r["id"] + "|" + r["method"] + r["path"])
    if c is None:
        continue
    hdr = {}
    for h in CONTRACT:
        rv, cv = r["headers"].get(h), c["headers"].get(h)
        if h == "content-type":
            rv, cv = ctype(rv), ctype(cv)
        if rv != cv:
            hdr[h] = [rv, cv]
    rows.append({
        "id": r["id"], "spec": r["spec"], "method": r["method"], "path": r["path"],
        "mutating": r.get("mutating", False),
        "ref_status": r["status"], "cand_status": c["status"],
        "status_ok": r["status"] == c["status"],
        "body_ok": same_body(r["body"], c["body"]),
        "hdr_diffs": hdr,
        "hdr_ok": not hdr,
        "hdr_ok_excl_range": not {k: v for k, v in hdr.items() if k != "content-range"},
        "ref_body": r["body"][:500], "cand_body": c["body"][:500],
        "ref_code": code_of(r["body"]), "cand_code": code_of(c["body"]),
    })

LEVELS = [
    ("status code only", lambda r: r["status_ok"]),
    ("status + body", lambda r: r["status_ok"] and r["body_ok"]),
    ("+ headers, ignoring Content-Range",
     lambda r: r["status_ok"] and r["body_ok"] and r["hdr_ok_excl_range"]),
    ("full contract (strict)",
     lambda r: r["status_ok"] and r["body_ok"] and r["hdr_ok"]),
]


def table(subset, title):
    n = len(subset)
    if not n:
        return
    print(f"\n{title}  ({n} cases)")
    for label, fn in LEVELS:
        k = sum(1 for r in subset if fn(r))
        print(f"  {100*k/n:5.1f}%  {k:>4}/{n:<4} {'#' * round(30 * k / n):<30} {label}")


table(rows, "ALL")
table([r for r in rows if not r["mutating"]], "READS (GET / HEAD / OPTIONS)")
table([r for r in rows if r["mutating"]], "WRITES (POST / PATCH / PUT / DELETE)")

print("\nwhere the divergences are")
print(f"  status differs        {sum(1 for r in rows if not r['status_ok']):>4}")
print(f"  body differs          {sum(1 for r in rows if not r['body_ok']):>4}")
for h, k in collections.Counter(h for r in rows for h in r["hdr_diffs"]).most_common():
    print(f"  header {h:<15}{k:>4}")

print("\nstatus-code pairs")
sp = collections.Counter((r["ref_status"], r["cand_status"]) for r in rows if not r["status_ok"])
for (a, b), k in sp.most_common(10):
    print(f"  PostgREST {a} -> Postrust {b}   {k}")

print("\nagreement by spec file (status + body), worst first")
by = collections.defaultdict(lambda: [0, 0])
for r in rows:
    by[r["spec"]][0] += 1
    if r["status_ok"] and r["body_ok"]:
        by[r["spec"]][1] += 1
for spec, (t, p) in sorted(by.items(), key=lambda kv: (kv[1][1] / kv[1][0], -kv[1][0])):
    if t >= 5:
        print(f"  {100*p/t:5.1f}%  {p:>4}/{t:<4} {'#' * round(16 * p / t):<16} {spec}")

json.dump(rows, open(OUT, "w"), indent=1)
print(f"\nper-case detail: {OUT}")
