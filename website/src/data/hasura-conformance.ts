// GENERATED FILE -- do not edit by hand.
//
// Produced by scripts/gen-hasura-conformance.mjs from a run's diff.json.
// Regenerate with:
//
//   scripts/hasura-conformance/conformance.sh
//   node scripts/gen-hasura-conformance.mjs scripts/hasura-conformance/.work/diff.json
//
// Every figure here is a replay of Hasura's own test corpus against both
// servers on identically loaded fixture databases, diffed against Hasura's
// live response. See docs/hasura-conformance.md.

export interface Level {
  passed: number;
  pct: number;
}

export interface Group {
  cases: number;
  /** HTTP status only. */
  status: Level;
  /** Status, and whether both answered with data or both with errors. */
  sameOutcome: Level;
  /** The above, and the same rows -- or both refusing. */
  sameData: Level;
  /** The whole body, error wording included. */
  fullBody: Level;
}

export const hasuraConformanceMeta = {
  "hasura": "v2.50.1",
  "features": "admin-ui,compat-key-order",
  "referenceReused": false,
  "commit": "ddc5291893ff847711a50f991d1e8517b6d4f52e",
  "measured": "2026-08-29",
  "cases": 468,
  "groups": 59
} as const;

export const hasuraConformance: Record<"all" | "reads" | "writes", Group> = {
  "all": {
    "cases": 468,
    "status": {
      "passed": 468,
      "pct": 100
    },
    "sameOutcome": {
      "passed": 466,
      "pct": 99.6
    },
    "sameData": {
      "passed": 456,
      "pct": 97.4
    },
    "fullBody": {
      "passed": 452,
      "pct": 96.6
    }
  },
  "reads": {
    "cases": 271,
    "status": {
      "passed": 271,
      "pct": 100
    },
    "sameOutcome": {
      "passed": 270,
      "pct": 99.6
    },
    "sameData": {
      "passed": 260,
      "pct": 95.9
    },
    "fullBody": {
      "passed": 259,
      "pct": 95.6
    }
  },
  "writes": {
    "cases": 197,
    "status": {
      "passed": 197,
      "pct": 100
    },
    "sameOutcome": {
      "passed": 196,
      "pct": 99.5
    },
    "sameData": {
      "passed": 196,
      "pct": 99.5
    },
    "fullBody": {
      "passed": 193,
      "pct": 98
    }
  }
};

/** How the "same data" level divides: agreeing rows, and mutual refusals. */
export const hasuraAgreement = {
  "sameData": 325,
  "bothRefuse": 131
};

/** Where the remaining disagreement lives, worst first. */
export const worstGroups = [
  {
    "group": "graphql_query/computed_fields",
    "total": 11,
    "passed": 10,
    "pct": 90.9
  }
];
