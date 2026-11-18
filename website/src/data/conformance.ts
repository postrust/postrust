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
  commit: "730b801502ea74a66049037c645cfd125652cd1b",
  measured: "2026-11-18",
  cases: 1499,
} as const;

export const conformance: Record<"all" | "reads" | "writes", Group> = {
  all: {
    cases: 1499,
    status: {
      passed: 1499,
      pct: 100,
    },
    statusAndBody: {
      passed: 1488,
      pct: 99.3,
    },
    exceptContentRange: {
      passed: 1486,
      pct: 99.1,
    },
    fullContract: {
      passed: 1486,
      pct: 99.1,
    },
  },
  reads: {
    cases: 1068,
    status: {
      passed: 1068,
      pct: 100,
    },
    statusAndBody: {
      passed: 1058,
      pct: 99.1,
    },
    exceptContentRange: {
      passed: 1056,
      pct: 98.9,
    },
    fullContract: {
      passed: 1056,
      pct: 98.9,
    },
  },
  writes: {
    cases: 431,
    status: {
      passed: 431,
      pct: 100,
    },
    statusAndBody: {
      passed: 430,
      pct: 99.8,
    },
    exceptContentRange: {
      passed: 430,
      pct: 99.8,
    },
    fullContract: {
      passed: 430,
      pct: 99.8,
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
    spec: "Query/MultipleSchemaSpec.hs",
    total: 25,
    passed: 24,
    pct: 96,
  },
  {
    spec: "Query/InsertSpec.hs",
    total: 82,
    passed: 81,
    pct: 98.8,
  },
  {
    spec: "Query/QuerySpec.hs",
    total: 302,
    passed: 300,
    pct: 99.3,
  },
  {
    spec: "Query/RpcSpec.hs",
    total: 201,
    passed: 201,
    pct: 100,
  },
  {
    spec: "Query/AndOrParamsSpec.hs",
    total: 73,
    passed: 73,
    pct: 100,
  },
  {
    spec: "Query/JsonOperatorSpec.hs",
    total: 64,
    passed: 64,
    pct: 100,
  },
  {
    spec: "Query/UpdateSpec.hs",
    total: 63,
    passed: 63,
    pct: 100,
  },
];
