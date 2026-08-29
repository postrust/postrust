#!/usr/bin/env node
// Generates website/src/data/hasura-conformance.ts from a run's diff.json.
//
// As with gen-measured.mjs and gen-conformance.mjs: the website never has
// numbers typed into it by hand. What is published is what was measured, and
// it is recomputed here from the per-case detail rather than scraped from
// report.py's printed summary, so the page and the report cannot drift apart.
//
// Usage:
//   node scripts/gen-hasura-conformance.mjs scripts/hasura-conformance/.work/diff.json
//
// What was measured is read from run-meta.json beside the diff, written by
// conformance.sh -- not from arguments, because a run measured with the wrong
// build features produces a number that looks exactly like a good one.

import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const OUT = join(here, "..", "website", "src", "data", "hasura-conformance.ts");

const argv = process.argv.slice(2);
const positional = argv.filter((a) => !a.startsWith("--"));

if (positional.length === 0) {
  console.error(
    "usage: node scripts/gen-hasura-conformance.mjs <diff.json>",
  );
  process.exit(1);
}

const rows = JSON.parse(readFileSync(positional[0], "utf8"));
if (!Array.isArray(rows) || rows.length === 0) {
  console.error(`${positional[0]} holds no cases`);
  process.exit(1);
}

// The same four levels report.py prints, computed the same way.
//
// The third is the one that matters and the one that needs explaining: two
// servers agree about data when they return the same rows, and they also agree
// when both refuse. Counting only the first would score a case where Hasura
// itself raises an error as a failure of this server to match it, which is
// backwards. The strict level below it compares the whole body, errors
// included -- wording and all.
const LEVELS = [
  ["status", (r) => r.status_ok],
  ["sameOutcome", (r) => r.status_ok && r.outcome_ok],
  [
    "sameData",
    (r) =>
      r.status_ok &&
      r.outcome_ok &&
      (r.data_ok || r.ref_outcome === "errors"),
  ],
  ["fullBody", (r) => r.status_ok && r.body_ok],
];

const score = (subset) => {
  const out = { cases: subset.length };
  for (const [name, ok] of LEVELS) {
    const passed = subset.filter(ok).length;
    out[name] = {
      passed,
      pct: subset.length
        ? Number(((100 * passed) / subset.length).toFixed(1))
        : 0,
    };
  }
  return out;
};

const groups = {
  all: score(rows),
  reads: score(rows.filter((r) => !r.mutating)),
  writes: score(rows.filter((r) => r.mutating)),
};

// How the third level divides: cases where both servers returned the same
// rows, and cases where both refused. Published because "97.4% agree" means
// something different depending on the split, and a reader is entitled to it.
const agreeing = rows.filter(
  (r) =>
    r.status_ok &&
    r.outcome_ok &&
    (r.data_ok || r.ref_outcome === "errors"),
);
const agreement = {
  sameData: agreeing.filter((r) => r.ref_outcome === "data").length,
  bothRefuse: agreeing.filter((r) => r.ref_outcome === "errors").length,
};

// Which groups are furthest from agreement, for anyone wanting to know where
// the remaining failures actually live rather than just how many there are.
const byGroup = new Map();
for (const r of rows) {
  const e = byGroup.get(r.group) ?? { total: 0, passed: 0 };
  e.total += 1;
  if (
    r.status_ok &&
    r.outcome_ok &&
    (r.data_ok || r.ref_outcome === "errors")
  ) {
    e.passed += 1;
  }
  byGroup.set(r.group, e);
}
const worstGroups = [...byGroup.entries()]
  .filter(([, e]) => e.total >= 5 && e.passed < e.total)
  .map(([group, e]) => ({
    group,
    total: e.total,
    passed: e.passed,
    pct: Number(((100 * e.passed) / e.total).toFixed(1)),
  }))
  .sort((a, b) => a.pct - b.pct || b.total - a.total)
  .slice(0, 8);

// Provenance comes from the run itself, not from what was typed on this
// command line. conformance.sh writes run-meta.json beside diff.json with the
// features it built, the Hasura version it measured against, whether it
// replayed the reference or reused a recording, and the commit it was
// measuring -- the things that decide whether a number means anything.
const metaPath = join(dirname(positional[0]), "run-meta.json");
let runMeta;
try {
  runMeta = JSON.parse(readFileSync(metaPath, "utf8"));
} catch {
  console.error(
    `error: ${metaPath} not found.\n` +
      "       It records what was actually built and measured, and without it\n" +
      "       there is no way to tell a good run from one measured with the\n" +
      "       wrong binary. Re-run scripts/hasura-conformance/conformance.sh.",
  );
  process.exit(1);
}

// Without `admin-ui` there is no GraphQL surface to measure at all: every case
// answers 404, which is a configuration fault wearing the costume of a result.
if (!String(runMeta.features ?? "").includes("admin-ui")) {
  console.error(
    "error: this run was measured without admin-ui, so the GraphQL routes\n" +
      "       were never mounted. Not publishable.",
  );
  process.exit(1);
}

if (!runMeta.hasura) {
  console.error(
    "error: run-meta.json does not say which Hasura it measured against.\n" +
      "       A conformance number without a reference version is not one.",
  );
  process.exit(1);
}

const meta = {
  hasura: runMeta.hasura,
  features: runMeta.features,
  referenceReused: runMeta.referenceReused ?? null,
  commit: runMeta.commit,
  measured: runMeta.measured,
  cases: rows.length,
  groups: byGroup.size,
};

const body = `// GENERATED FILE -- do not edit by hand.
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

export const hasuraConformanceMeta = ${JSON.stringify(meta, null, 2)} as const;

export const hasuraConformance: Record<"all" | "reads" | "writes", Group> = ${JSON.stringify(
  groups,
  null,
  2,
)};

/** How the "same data" level divides: agreeing rows, and mutual refusals. */
export const hasuraAgreement = ${JSON.stringify(agreement, null, 2)};

/** Where the remaining disagreement lives, worst first. */
export const worstGroups = ${JSON.stringify(worstGroups, null, 2)};
`;

writeFileSync(OUT, body);
console.log(`wrote ${OUT}`);
console.log(
  `  ${meta.cases} cases in ${meta.groups} groups: ` +
    `${groups.all.sameData.pct}% same data, ${groups.all.fullBody.pct}% full body`,
);
