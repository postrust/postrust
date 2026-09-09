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
  postgrest: "v16.1",
  features: "admin-ui,compat-key-order",
  compatMode: true,
  commit: "c0dbe0045aa9ba4203ac400dae36cd432b522670",
  measured: "2026-09-09",
  cases: 1499,
} as const;

export const conformance: Record<"all" | "reads" | "writes", Group> = {
  all: {
    cases: 1499,
    status: {
      passed: 1479,
      pct: 98.7,
    },
    statusAndBody: {
      passed: 1451,
      pct: 96.8,
    },
    exceptContentRange: {
      passed: 1428,
      pct: 95.3,
    },
    fullContract: {
      passed: 1424,
      pct: 95,
    },
  },
  reads: {
    cases: 1068,
    status: {
      passed: 1052,
      pct: 98.5,
    },
    statusAndBody: {
      passed: 1028,
      pct: 96.3,
    },
    exceptContentRange: {
      passed: 1010,
      pct: 94.6,
    },
    fullContract: {
      passed: 1009,
      pct: 94.5,
    },
  },
  writes: {
    cases: 431,
    status: {
      passed: 427,
      pct: 99.1,
    },
    statusAndBody: {
      passed: 423,
      pct: 98.1,
    },
    exceptContentRange: {
      passed: 418,
      pct: 97,
    },
    fullContract: {
      passed: 415,
      pct: 96.3,
    },
  },
};

/** Where the remaining disagreement lives, worst first. */
export const worstSpecs = [
  {
    spec: "Query/RelatedQueriesSpec.hs",
    total: 36,
    passed: 33,
    pct: 91.7,
  },
  {
    spec: "Query/Preferences/MaxAffectedSpec.hs",
    total: 13,
    passed: 12,
    pct: 92.3,
  },
  {
    spec: "Query/SpreadQueriesSpec.hs",
    total: 56,
    passed: 52,
    pct: 92.9,
  },
  {
    spec: "Query/EmbedDisambiguationSpec.hs",
    total: 58,
    passed: 54,
    pct: 93.1,
  },
  {
    spec: "Query/EmbedInnerJoinSpec.hs",
    total: 57,
    passed: 54,
    pct: 94.7,
  },
  {
    spec: "Query/QuerySpec.hs",
    total: 302,
    passed: 287,
    pct: 95,
  },
  {
    spec: "Query/UpsertSpec.hs",
    total: 60,
    passed: 57,
    pct: 95,
  },
  {
    spec: "Query/JsonOperatorSpec.hs",
    total: 64,
    passed: 61,
    pct: 95.3,
  },
];
