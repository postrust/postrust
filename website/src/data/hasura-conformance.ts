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
  hasura: "v2.50.1",
  features: "admin-ui,compat-key-order",
  referenceReused: true,
  commit: "1bb4dce2eb7fdd2e1dcc1938bfa585f014a35d9d",
  measured: "2026-11-18",
  cases: 468,
  groups: 59,
} as const;

export const hasuraConformance: Record<"all" | "reads" | "writes", Group> = {
  all: {
    cases: 468,
    status: {
      passed: 468,
      pct: 100,
    },
    sameOutcome: {
      passed: 467,
      pct: 99.8,
    },
    sameData: {
      passed: 457,
      pct: 97.6,
    },
    fullBody: {
      passed: 455,
      pct: 97.2,
    },
  },
  reads: {
    cases: 271,
    status: {
      passed: 271,
      pct: 100,
    },
    sameOutcome: {
      passed: 270,
      pct: 99.6,
    },
    sameData: {
      passed: 260,
      pct: 95.9,
    },
    fullBody: {
      passed: 259,
      pct: 95.6,
    },
  },
  writes: {
    cases: 197,
    status: {
      passed: 197,
      pct: 100,
    },
    sameOutcome: {
      passed: 197,
      pct: 100,
    },
    sameData: {
      passed: 197,
      pct: 100,
    },
    fullBody: {
      passed: 196,
      pct: 99.5,
    },
  },
};

/** How the "same data" level divides: agreeing rows, and mutual refusals. */
export const hasuraAgreement = {
  sameData: 326,
  bothRefuse: 131,
};

/** Where the remaining disagreement lives, worst first. */
export const worstGroups = [
  {
    group: "graphql_query/computed_fields",
    total: 11,
    passed: 10,
    pct: 90.9,
  },
];
