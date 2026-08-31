#!/usr/bin/env python3
"""Summarise an Autobahn fuzzingclient report.

Autobahn writes one entry per case with a `behavior` (and a separate
`behaviorClose` for the close handshake). Anything that is not OK or
INFORMATIONAL is worth a look; FAILED is worth fixing.
"""

import json
import sys
from collections import Counter

# Autobahn's own vocabulary, ordered worst-first so the report reads top-down.
ORDER = ["FAILED", "UNIMPLEMENTED", "NON-STRICT", "INFORMATIONAL", "OK"]


def main(path):
    with open(path) as fh:
        report = json.load(fh)

    exit_code = 0
    for agent, cases in report.items():
        counts = Counter()
        close_counts = Counter()
        bad = []
        for case, result in sorted(cases.items()):
            behavior = result.get("behavior", "?")
            close = result.get("behaviorClose", "?")
            counts[behavior] += 1
            close_counts[close] += 1
            if behavior in ("FAILED", "UNIMPLEMENTED"):
                bad.append((case, behavior, close))

        total = sum(counts.values())
        print(f"agent: {agent}")
        print(f"  {total} cases")
        for key in ORDER:
            if counts[key]:
                print(f"    {key:<15} {counts[key]}")
        for key in sorted(counts):
            if key not in ORDER:
                print(f"    {key:<15} {counts[key]}")

        # The close handshake is scored separately and is easy to miss.
        close_bad = {k: v for k, v in close_counts.items() if k not in ("OK", "INFORMATIONAL")}
        if close_bad:
            print("  close handshake, non-OK:")
            for key, value in sorted(close_bad.items()):
                print(f"    {key:<15} {value}")

        if bad:
            print(f"  cases needing attention ({len(bad)}):")
            for case, behavior, close in bad:
                print(f"    {case:<10} behavior={behavior} close={close}")
            exit_code = 1
        else:
            print("  no FAILED or UNIMPLEMENTED cases")
        print()

    return exit_code


if __name__ == "__main__":
    sys.exit(main(sys.argv[1]))
