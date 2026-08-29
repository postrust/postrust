#!/usr/bin/env node
// Generates website/src/data/conformance.ts from a conformance run's diff.json.
//
// As with gen-measured.mjs: the website never has numbers typed into it by
// hand. What is published is what was measured, and it is recomputed here from
// the per-case detail rather than scraped from report.py's printed summary, so
// the page and the report cannot drift apart.
//
// Usage:
//   node scripts/gen-conformance.mjs scripts/conformance/.work/diff.json
//
// What was measured is read from run-meta.json beside the diff, written by
// conformance.sh -- not from arguments, because a run measured with the wrong
// build features produces a number that looks exactly like a good one.

import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const OUT = join(here, "..", "website", "src", "data", "conformance.ts");

const argv = process.argv.slice(2);
const positional = argv.filter((a) => !a.startsWith("--"));

if (positional.length === 0) {
  console.error("usage: node scripts/gen-conformance.mjs <diff.json>");
  process.exit(1);
}

const rows = JSON.parse(readFileSync(positional[0], "utf8"));
if (!Array.isArray(rows) || rows.length === 0) {
  console.error(`${positional[0]} holds no cases`);
  process.exit(1);
}

// The same four levels report.py prints, computed the same way. A single
// systemic gap -- one header never emitted -- would otherwise sink every case
// and hide the hundreds that differ in nothing else.
const LEVELS = [
  ["status", (r) => r.status_ok],
  ["statusAndBody", (r) => r.status_ok && r.body_ok],
  ["exceptContentRange", (r) => r.status_ok && r.body_ok && r.hdr_ok_excl_range],
  ["fullContract", (r) => r.status_ok && r.body_ok && r.hdr_ok],
];

const score = (subset) => {
  const out = { cases: subset.length };
  for (const [name, ok] of LEVELS) {
    const passed = subset.filter(ok).length;
    out[name] = {
      passed,
      pct: subset.length ? Number(((100 * passed) / subset.length).toFixed(1)) : 0,
    };
  }
  return out;
};

const groups = {
  all: score(rows),
  reads: score(rows.filter((r) => !r.mutating)),
  writes: score(rows.filter((r) => r.mutating)),
};

// Which specs are furthest from agreement, for anyone wanting to know where
// the remaining failures actually live rather than just how many there are.
const bySpec = new Map();
for (const r of rows) {
  const e = bySpec.get(r.spec) ?? { total: 0, passed: 0 };
  e.total += 1;
  if (r.status_ok && r.body_ok) e.passed += 1;
  bySpec.set(r.spec, e);
}
const worstSpecs = [...bySpec.entries()]
  .filter(([, e]) => e.total >= 5)
  .map(([spec, e]) => ({
    spec,
    total: e.total,
    passed: e.passed,
    pct: Number(((100 * e.passed) / e.total).toFixed(1)),
  }))
  .sort((a, b) => a.pct - b.pct || b.total - a.total)
  .slice(0, 8);

// Provenance comes from the run itself, not from what was typed on this
// command line. conformance.sh writes run-meta.json beside diff.json with the
// features it built, the reference version it measured against, and the commit
// it was measuring -- the things that decide whether a number means anything.
//
// Run 4's diff.json is the reason this is not a flag. It was measured with a
// binary that `cargo test --workspace` had quietly rebuilt without
// `compat-key-order`, and nothing in the file says so; a `--features` flag
// would have stamped it as correct.
const metaPath = join(dirname(positional[0]), "run-meta.json");
let runMeta;
try {
  runMeta = JSON.parse(readFileSync(metaPath, "utf8"));
} catch {
  console.error(
    `error: ${metaPath} not found.\n` +
      "       It records what was actually built and measured, and without it\n" +
      "       there is no way to tell a good run from one measured with the\n" +
      "       wrong binary. Re-run scripts/conformance/conformance.sh.",
  );
  process.exit(1);
}

if (!String(runMeta.features ?? "").includes("compat-key-order")) {
  console.error(
    "error: this run was measured without compat-key-order, so its object\n" +
      "       key order -- and every CSV column order case -- diverged for a\n" +
      "       reason that is not a bug. Not publishable.",
  );
  process.exit(1);
}

const meta = {
  postgrest: runMeta.postgrest,
  features: runMeta.features,
  compatMode: runMeta.compatMode ?? true,
  commit: runMeta.commit,
  measured: runMeta.measured,
  cases: rows.length,
};

const body = `// GENERATED FILE -- do not edit by hand.
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

export const conformanceMeta = ${JSON.stringify(meta, null, 2)} as const;

export const conformance: Record<"all" | "reads" | "writes", Group> = ${JSON.stringify(
  groups,
  null,
  2,
)};

/** Where the remaining disagreement lives, worst first. */
export const worstSpecs = ${JSON.stringify(worstSpecs, null, 2)};
`;

writeFileSync(OUT, body);
console.log(`wrote ${OUT}`);
console.log(
  `  ${meta.cases} cases: ${groups.all.statusAndBody.pct}% status+body, ` +
    `${groups.all.fullContract.pct}% full contract`,
);
