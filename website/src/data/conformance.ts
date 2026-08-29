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
  "commit": "c3026ad955b8ddcbfe5054431dae30a187ae8d1c",
  "measured": "2026-08-29",
  "cases": 1499
} as const;

export const conformance: Record<"all" | "reads" | "writes", Group> = {
  "all": {
    "cases": 1499,
    "status": {
      "passed": 1478,
      "pct": 98.6
    },
    "statusAndBody": {
      "passed": 1450,
      "pct": 96.7
    },
    "exceptContentRange": {
      "passed": 1427,
      "pct": 95.2
    },
    "fullContract": {
      "passed": 1423,
      "pct": 94.9
    }
  },
  "reads": {
    "cases": 1068,
    "status": {
      "passed": 1052,
      "pct": 98.5
    },
    "statusAndBody": {
      "passed": 1028,
      "pct": 96.3
    },
    "exceptContentRange": {
      "passed": 1010,
      "pct": 94.6
    },
    "fullContract": {
      "passed": 1009,
      "pct": 94.5
    }
  },
  "writes": {
    "cases": 431,
    "status": {
      "passed": 426,
      "pct": 98.8
    },
    "statusAndBody": {
      "passed": 422,
      "pct": 97.9
    },
    "exceptContentRange": {
      "passed": 417,
      "pct": 96.8
    },
    "fullContract": {
      "passed": 414,
      "pct": 96.1
    }
  }
};

/** Where the remaining disagreement lives, worst first. */
export const worstSpecs = [
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
  },
  {
    "spec": "Query/UpsertSpec.hs",
    "total": 60,
    "passed": 57,
    "pct": 95
  },
  {
    "spec": "Query/InsertSpec.hs",
    "total": 82,
    "passed": 78,
    "pct": 95.1
  }
];
