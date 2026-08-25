#!/usr/bin/env python3
"""
Diff two replay runs and classify every divergence.

GraphQL answers almost everything with 200, so status agreement says very
little on its own and a strict body comparison says too much at first --
error text is the last thing to match and would hide every case that differs
in nothing else. Four levels sit between those two, and the one that matters
for a client is the third: did the same query come back with the same data?
"""
import collections
import json
import sys

with open(sys.argv[1]) as fh:
    reference = json.load(fh)
with open(sys.argv[2]) as fh:
    candidate = json.load(fh)
OUT = sys.argv[3]

by_id = {r["id"]: r for r in candidate["results"]}


def parse(body):
    try:
        return True, json.loads(body)
    except Exception:
        return False, body


def outcome(body):
    """What a client would branch on: data, errors, or something unreadable."""
    ok, value = parse(body)
    if not ok or not isinstance(value, dict):
        return "unparseable"
    if value.get("errors"):
        return "errors"
    if "data" in value:
        return "data"
    return "neither"


def data_of(body):
    ok, value = parse(body)
    return value.get("data") if ok and isinstance(value, dict) else None


def codes_of(body):
    ok, value = parse(body)
    if not ok or not isinstance(value, dict):
        return []
    return sorted(
        e.get("extensions", {}).get("code")
        for e in (value.get("errors") or [])
        if isinstance(e, dict) and isinstance(e.get("extensions"), dict)
    )


rows = []
for record in reference["results"]:
    other = by_id.get(record["id"])
    if other is None:
        continue
    ref_outcome, cand_outcome = outcome(record["body"]), outcome(other["body"])
    ok, ref_body = parse(record["body"])
    ok2, cand_body = parse(other["body"])
    rows.append({
        "id": record["id"],
        "group": record["group"],
        "mutating": record["mutating"],
        "ref_status": record["status"], "cand_status": other["status"],
        "status_ok": record["status"] == other["status"],
        "outcome_ok": ref_outcome == cand_outcome,
        "ref_outcome": ref_outcome, "cand_outcome": cand_outcome,
        "data_ok": ref_outcome == cand_outcome == "data"
                   and data_of(record["body"]) == data_of(other["body"]),
        "body_ok": (ref_body == cand_body) if (ok and ok2)
                   else record["body"].strip() == other["body"].strip(),
        "ref_codes": codes_of(record["body"]), "cand_codes": codes_of(other["body"]),
        "ref_body": record["body"][:600], "cand_body": other["body"][:600],
    })

# A case both servers answer with errors is not a data match, but it is not a
# divergence either -- the client branches the same way. Counted at the
# outcome level and again at the strict level, never in between.
LEVELS = [
    ("HTTP status", lambda r: r["status_ok"]),
    ("+ same outcome (data vs errors)", lambda r: r["status_ok"] and r["outcome_ok"]),
    ("+ same data payload",
     lambda r: r["status_ok"] and r["outcome_ok"]
               and (r["data_ok"] or r["ref_outcome"] == "errors")),
    ("full body, error text included",
     lambda r: r["status_ok"] and r["body_ok"]),
]


def table(subset, title):
    total = len(subset)
    if not total:
        return
    print(f"\n{title}  ({total} cases)")
    for label, predicate in LEVELS:
        hit = sum(1 for r in subset if predicate(r))
        bar = "#" * round(30 * hit / total)
        print(f"  {100*hit/total:5.1f}%  {hit:>4}/{total:<4} {bar:<30} {label}")


table(rows, "ALL")
table([r for r in rows if not r["mutating"]], "READS (query)")
table([r for r in rows if r["mutating"]], "WRITES (mutation)")

# What the third level is made of. Half of it is not agreement about data at
# all: a case where both servers answer with errors counts as agreement --
# correctly, a client branches the same way -- but it is agreement by mutual
# refusal, and it evaporates the moment this server can answer a query it
# previously could not. Every feature added moves cases from the left column to
# the right one, so a headline that mixes them falls while the server improves.
passing = [r for r in rows if r["status_ok"] and r["outcome_ok"]
           and (r["data_ok"] or r["ref_outcome"] == "errors")]
mutual = [r for r in passing if r["ref_outcome"] == "errors"]
same_data = [r for r in passing if r["ref_outcome"] == "data"]
diverged = [r for r in rows if r["ref_outcome"] == "errors" and r["cand_outcome"] == "data"]

print("\nwhat the third level is made of")
print(f"  {len(same_data):>4}  the same data came back")
print(f"  {len(mutual):>4}  both answered with errors -- agreement by mutual refusal")
print(f"  {len(diverged):>4}  Hasura refused and this answered, which is where the")
print("        mutual-refusal cases go as this server gains fields")

print("\nwhere the divergences are")
print(f"  status differs        {sum(1 for r in rows if not r['status_ok']):>4}")
print(f"  outcome differs       {sum(1 for r in rows if not r['outcome_ok']):>4}")
print(f"  data differs          "
      f"{sum(1 for r in rows if r['outcome_ok'] and r['ref_outcome'] == 'data' and not r['data_ok']):>4}")

print("\noutcome pairs, worst first")
pairs = collections.Counter((r["ref_outcome"], r["cand_outcome"]) for r in rows if not r["outcome_ok"])
for (a, b), count in pairs.most_common(10):
    print(f"  Hasura {a:<12} -> Postrust {b:<12} {count}")

print("\nerror codes Postrust does not produce")
missing = collections.Counter()
for r in rows:
    for code in r["ref_codes"]:
        if code not in r["cand_codes"]:
            missing[code] += 1
for code, count in missing.most_common(12):
    print(f"  {str(code):<28} {count}")

print("\nagreement by group (same data payload), worst first")
by_group = collections.defaultdict(lambda: [0, 0])
for r in rows:
    by_group[r["group"]][0] += 1
    if r["status_ok"] and r["outcome_ok"] and (r["data_ok"] or r["ref_outcome"] == "errors"):
        by_group[r["group"]][1] += 1
for group, (total, passed) in sorted(by_group.items(), key=lambda kv: (kv[1][1] / kv[1][0], -kv[1][0])):
    bar = "#" * round(16 * passed / total)
    print(f"  {100*passed/total:5.1f}%  {passed:>3}/{total:<3} {bar:<16} {group}")

unusable = {**reference.get("group_failures", {}), **candidate.get("group_failures", {})}
if unusable:
    print(f"\ngroups that never loaded ({len(unusable)}) -- measuring nothing, not failing")
    for group, why in sorted(unusable.items()):
        print(f"  {group}: {why[:110]}")

with open(OUT, "w") as fh:
    json.dump(rows, fh, indent=1)
print(f"\nper-case detail: {OUT}")
