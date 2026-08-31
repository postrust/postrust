#!/usr/bin/env python3
"""Summarise an Autobahn fuzzingclient report and decide whether to fail.

    summarize.py REPORT [BASELINE_REPORT]

The gate is deliberately narrow, because postrust is a transparent tunnel: it
splices two upgraded byte streams and never parses a WebSocket frame. Most of
what Autobahn scores is therefore the *origin's* behaviour, and failing on it
would mean a red build that says nothing about the proxy.

So:

  with a baseline:    a case worse than the baseline fails. That is the whole
                      rule, and it is the only class of result really about the
                      tunnel. A case the origin also fails is the origin's, not
                      ours -- including FAILED. The trade is deliberate: if the
                      origin and the proxy break the same case, the delta is
                      zero and this stays green. The baseline is the definition
                      of what this origin does; we measure the change.
  without a baseline: any FAILED fails, conservatively, since there is nothing
                      to attribute it against.
  UNIMPLEMENTED       reported as coverage, never a failure on its own. It means
                      the capability was not there to exercise, which for a
                      tunnel is a statement about the endpoint behind it.

Pass a baseline report (from `BASELINE=1 scripts/autobahn/run.sh`) to enable
attribution. Without one the gate is conservative and the summary says so.
"""

import json
import sys
from collections import Counter

ORDER = ["FAILED", "UNIMPLEMENTED", "NON-STRICT", "INFORMATIONAL", "OK"]

# Worst-to-best, for deciding whether a case regressed against the baseline.
RANK = {"OK": 0, "INFORMATIONAL": 1, "NON-STRICT": 2, "UNIMPLEMENTED": 3, "FAILED": 4}

# Cases excluded from the baseline-comparison rule, with the reason. These are
# always printed, never silently applied.
KNOWN_ORIGIN_ARTIFACTS = {
    "3.4": (
        "origin is segmentation-sensitive: AutobahnPython's echoserver fails the "
        "connection without flushing its echo when it reads the valid frame and "
        "the RSV frame in one read. Verified with no proxy in the path -- "
        "octet-wise chops echo 15 bytes, one coalesced write echoes nothing. Any "
        "relay re-chunks, so this is not about the tunnel."
    ),
}


def load(path):
    with open(path) as fh:
        report = json.load(fh)
    # One agent per report; take its cases.
    return next(iter(report.values()))


def main(argv):
    if len(argv) < 2:
        print(__doc__)
        return 2

    cases = load(argv[1])
    baseline = load(argv[2]) if len(argv) > 2 else None

    counts = Counter(result.get("behavior", "?") for result in cases.values())
    total = sum(counts.values())

    print(f"{total} cases")
    for key in ORDER:
        if counts[key]:
            print(f"  {key:<15} {counts[key]}")
    for key in sorted(counts):
        if key not in ORDER:
            print(f"  {key:<15} {counts[key]}")
    print()

    failed = sorted(c for c, r in cases.items() if r.get("behavior") == "FAILED")
    unimplemented = sorted(
        c for c, r in cases.items() if r.get("behavior") == "UNIMPLEMENTED"
    )

    if unimplemented:
        sections = Counter(c.split(".")[0] for c in unimplemented)
        spread = ", ".join(f"{k}.* x{v}" for k, v in sorted(sections.items()))
        print(f"coverage: {len(unimplemented)} cases UNIMPLEMENTED ({spread})")
        print("  not a failure -- the capability was not there to exercise.")
        print()

    problems = 0

    if failed:
        # Printed either way; whether it counts depends on the baseline.
        print(f"FAILED ({len(failed)}):")
        for case in failed:
            note = ""
            if baseline is not None:
                there = baseline.get(case, {}).get("behavior")
                if there == "FAILED":
                    note = "  (origin fails this too -- not counted)"
            print(f"  {case}{note}")
        print()

    if baseline is None:
        print("no baseline given -- every FAILED counts.")
        print("  run BASELINE=1 scripts/autobahn/run.sh to attribute them.")
        problems += len(failed)
    else:
        regressions, excluded = [], []
        for case in sorted(set(cases) & set(baseline)):
            here = cases[case].get("behavior")
            there = baseline[case].get("behavior")
            if RANK.get(here, 9) > RANK.get(there, 9):
                if case in KNOWN_ORIGIN_ARTIFACTS:
                    excluded.append((case, there, here))
                else:
                    regressions.append((case, there, here))

        if excluded:
            print("excluded from the regression check:")
            for case, there, here in excluded:
                print(f"  {case}: baseline={there} proxied={here}")
                print(f"    {KNOWN_ORIGIN_ARTIFACTS[case]}")
            print()

        if regressions:
            print(f"WORSE THAN BASELINE ({len(regressions)}):")
            for case, there, here in regressions:
                print(f"  {case}: baseline={there} -> proxied={here}")
            problems += len(regressions)
            print()
        else:
            print("no regressions against the baseline.")

    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
