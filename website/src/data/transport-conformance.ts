// GENERATED FILE -- do not edit by hand.
//
// Produced by scripts/gen-transport-conformance.mjs from the transport
// conformance runs. Regenerate with:
//
//   scripts/h2spec/run.sh
//   BASELINE=1 scripts/autobahn/run.sh
//   scripts/autobahn/run.sh
//   node scripts/gen-transport-conformance.mjs
//
// h2spec speaks HTTP/2 to the proxy's listener. Autobahn runs twice: once
// through the proxy and once straight at the origin, because postrust splices
// WebSocket streams rather than parsing frames, so most of what the suite
// scores belongs to the endpoint behind it. The number that is about the proxy
// is `regressions`: cases the tunnel made worse than the baseline.

export const h2spec = {
  "tests": 146,
  "passed": 145,
  "skipped": 1,
  "failed": 0
} as const;

export const autobahn = {
  "cases": 517,
  "ok": 501,
  "nonStrict": 12,
  "informational": 3,
  "unimplemented": 0,
  "failed": 1,
  "baselineCases": 517,
  "baselineFailed": 1,
  "regressions": [],
  "intermittent": [
    "4.2.5"
  ]
} as const;
