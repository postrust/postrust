// GENERATED FILE -- do not edit by hand.
//
// Produced by scripts/gen-conformance.mjs from a conformance run's diff.json.
// Regenerate with:
//
//   scripts/conformance/conformance.sh
//   node scripts/gen-conformance.mjs scripts/conformance/.work/diff.json
//
// Every figure here is a replay of PostgREST's own test cases against both
// servers on identically loaded fixture databases, diffed against PostgREST's
// live response. See docs/postgrest-conformance.md.

export interface Level {
  passed: number;
  pct: number;
}

export interface Group {
  cases: number;
  /** Status code only. */
  status: Level;
  /** Status and body, compared as parsed JSON. */
  statusAndBody: Level;
  /** The above plus every compared header except Content-Range. */
  exceptContentRange: Level;
  /** Status, body, and all six compared headers. */
  fullContract: Level;
}

/**
 * The headers that count. Date, Server and Connection differ between any two
 * servers and say nothing about conformance, so they are not compared.
 */
export const comparedHeaders = [
  "Content-Type",
  "Content-Range",
  "Location",
  "Preference-Applied",
  "Allow",
  "WWW-Authenticate",
] as const;

export const conformanceMeta = {
  "postgrest": "v16.1",
  "features": "admin-ui,compat-key-order",
  "compatMode": true,
  "commit": "e38770f33f3ab6dab464177234e1dff77648996b",
  "measured": "2026-08-29",
  "cases": 1499
} as const;

export const conformance: Record<"all" | "reads" | "writes", Group> = {
  "all": {
    "cases": 1499,
    "status": {
      "passed": 1472,
      "pct": 98.2
    },
    "statusAndBody": {
      "passed": 1440,
      "pct": 96.1
    },
    "exceptContentRange": {
      "passed": 1417,
      "pct": 94.5
    },
    "fullContract": {
      "passed": 1413,
      "pct": 94.3
    }
  },
  "reads": {
    "cases": 1068,
    "status": {
      "passed": 1045,
      "pct": 97.8
    },
    "statusAndBody": {
      "passed": 1021,
      "pct": 95.6
    },
    "exceptContentRange": {
      "passed": 1003,
      "pct": 93.9
    },
    "fullContract": {
      "passed": 1002,
      "pct": 93.8
    }
  },
  "writes": {
    "cases": 431,
    "status": {
      "passed": 427,
      "pct": 99.1
    },
    "statusAndBody": {
      "passed": 419,
      "pct": 97.2
    },
    "exceptContentRange": {
      "passed": 414,
      "pct": 96.1
    },
    "fullContract": {
      "passed": 411,
      "pct": 95.4
    }
  }
};

/** Where the remaining disagreement lives, worst first. */
export const worstSpecs = [
  {
    "spec": "Query/ComputedRelsSpec.hs",
    "total": 30,
    "passed": 22,
    "pct": 73.3
  },
  {
    "spec": "Query/RelatedQueriesSpec.hs",
    "total": 36,
    "passed": 33,
    "pct": 91.7
  },
  {
    "spec": "Query/Preferences/MaxAffectedSpec.hs",
    "total": 13,
    "passed": 12,
    "pct": 92.3
  },
  {
    "spec": "Query/SpreadQueriesSpec.hs",
    "total": 56,
    "passed": 52,
    "pct": 92.9
  },
  {
    "spec": "Query/EmbedDisambiguationSpec.hs",
    "total": 58,
    "passed": 54,
    "pct": 93.1
  },
  {
    "spec": "Query/CustomMediaSpec.hs",
    "total": 50,
    "passed": 47,
    "pct": 94
  },
  {
    "spec": "Query/EmbedInnerJoinSpec.hs",
    "total": 57,
    "passed": 54,
    "pct": 94.7
  },
  {
    "spec": "Query/QuerySpec.hs",
    "total": 301,
    "passed": 286,
    "pct": 95
  }
];
