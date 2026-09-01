#!/usr/bin/env node
//
// Generate website/src/data/transport-conformance.ts from the transport
// conformance runs, the same way gen-conformance.mjs does for the dialects.
//
// The releases page states that nothing on it is typed by hand. That has to
// keep being true for these figures too, so they are read out of the suites'
// own artifacts rather than transcribed:
//
//   scripts/h2spec/.work/h2spec.log         (h2spec's summary line)
//   scripts/autobahn/reports/index.json     (proxied run)
//   scripts/autobahn/reports-baseline/index.json  (no proxy in the path)
//
// Regenerate with:
//
//   scripts/h2spec/run.sh
//   BASELINE=1 scripts/autobahn/run.sh
//   scripts/autobahn/run.sh
//   node scripts/gen-transport-conformance.mjs
//
// Exits non-zero rather than emitting a partial file: a figure that cannot
// account for itself should stop the build, not reach the page.

import { readFileSync, writeFileSync, existsSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const H2SPEC_LOG = resolve(ROOT, "scripts/h2spec/.work/h2spec.log");
const AUTOBAHN = resolve(ROOT, "scripts/autobahn/reports/index.json");
const AUTOBAHN_BASELINE = resolve(ROOT, "scripts/autobahn/reports-baseline/index.json");
const OUT = resolve(ROOT, "website/src/data/transport-conformance.ts");

function die(message) {
  console.error(`error: ${message}`);
  process.exit(1);
}

function requireFile(path, how) {
  if (!existsSync(path)) die(`missing ${path}\n  produce it with: ${how}`);
  return path;
}

// --- h2spec -----------------------------------------------------------------

function readH2spec() {
  const text = readFileSync(
    requireFile(H2SPEC_LOG, "scripts/h2spec/run.sh"),
    "utf8",
  );
  // "146 tests, 145 passed, 1 skipped, 0 failed"
  const m = text.match(
    /(\d+) tests?, (\d+) passed, (\d+) skipped, (\d+) failed/,
  );
  if (!m) die("no h2spec summary line in the log; did the run finish?");
  return {
    tests: Number(m[1]),
    passed: Number(m[2]),
    skipped: Number(m[3]),
    failed: Number(m[4]),
  };
}

// --- Autobahn ---------------------------------------------------------------

// The invalid-frame family: send a valid message, then an invalid frame, expect
// the echo of the first. The origin fails the connection without flushing that
// echo when both arrive in one read, and a relay coalesces what the client
// chopped -- so these move between OK and NON-STRICT from run to run without
// anything about the proxy changing. Kept separate from real regressions rather
// than dropped, so the page can say how many there were.
const SEGMENTATION_SENSITIVE = new Set([
  "3.4",
  "4.1.3",
  "4.1.4",
  "4.1.5",
  "4.2.3",
  "4.2.4",
  "4.2.5",
  "5.15",
]);

const RANK = {
  OK: 0,
  INFORMATIONAL: 1,
  "NON-STRICT": 2,
  UNIMPLEMENTED: 3,
  FAILED: 4,
};

function readAutobahnCases(path, how) {
  const report = JSON.parse(readFileSync(requireFile(path, how), "utf8"));
  const agents = Object.keys(report);
  if (agents.length !== 1) die(`expected one agent in ${path}, found ${agents.length}`);
  return report[agents[0]];
}

function tally(cases) {
  const counts = {};
  for (const result of Object.values(cases)) {
    const behavior = result.behavior ?? "?";
    counts[behavior] = (counts[behavior] ?? 0) + 1;
  }
  return counts;
}

function readAutobahn() {
  const proxied = readAutobahnCases(AUTOBAHN, "scripts/autobahn/run.sh");
  const baseline = readAutobahnCases(
    AUTOBAHN_BASELINE,
    "BASELINE=1 scripts/autobahn/run.sh",
  );

  // The only figure that is really about the proxy: cases the tunnel made
  // worse than they are with no proxy in the path.
  const regressions = [];
  const intermittent = [];
  for (const name of Object.keys(proxied)) {
    if (!(name in baseline)) continue;
    const here = RANK[proxied[name].behavior] ?? 9;
    const there = RANK[baseline[name].behavior] ?? 9;
    if (here > there) {
      (SEGMENTATION_SENSITIVE.has(name) ? intermittent : regressions).push(name);
    }
  }

  const counts = tally(proxied);
  const total = Object.values(counts).reduce((a, b) => a + b, 0);
  if (total === 0) die("the Autobahn report has no cases");

  // The two runs must cover the same cases. A run that stops early still writes
  // a valid-looking report -- one truncated mid-section-13 published 428 cases
  // against a 517-case baseline and read as a clean pass. Comparing the sets is
  // what makes a short run loud instead of silent.
  const missing = Object.keys(baseline).filter((c) => !(c in proxied));
  const extra = Object.keys(proxied).filter((c) => !(c in baseline));
  if (missing.length || extra.length) {
    die(
      `the two Autobahn runs do not cover the same cases ` +
        `(proxied ${total}, baseline ${Object.keys(baseline).length}).\n` +
        (missing.length
          ? `  ${missing.length} in the baseline but not proxied, e.g. ${missing.slice(0, 5).join(", ")}\n`
          : "") +
        (extra.length
          ? `  ${extra.length} proxied but not in the baseline, e.g. ${extra.slice(0, 5).join(", ")}\n`
          : "") +
        "  one of the runs stopped early; re-run both before generating.",
    );
  }

  return {
    cases: total,
    ok: counts.OK ?? 0,
    nonStrict: counts["NON-STRICT"] ?? 0,
    informational: counts.INFORMATIONAL ?? 0,
    unimplemented: counts.UNIMPLEMENTED ?? 0,
    failed: counts.FAILED ?? 0,
    baselineCases: Object.keys(baseline).length,
    baselineFailed: tally(baseline).FAILED ?? 0,
    regressions: regressions.sort(),
    intermittent: intermittent.sort(),
  };
}

// --- emit -------------------------------------------------------------------

const h2spec = readH2spec();
const autobahn = readAutobahn();

const banner = `// GENERATED FILE -- do not edit by hand.
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
// is \`regressions\`: cases the tunnel made worse than the baseline.
`;

const body = `
export const h2spec = ${JSON.stringify(h2spec, null, 2)} as const;

export const autobahn = ${JSON.stringify(autobahn, null, 2)} as const;
`;

writeFileSync(OUT, banner + body);
console.log(`wrote ${OUT}`);
console.log(
  `  h2spec:   ${h2spec.passed}/${h2spec.tests} passed, ${h2spec.failed} failed`,
);
console.log(
  `  autobahn: ${autobahn.ok}/${autobahn.cases} OK, ${autobahn.failed} failed, ` +
    `${autobahn.regressions.length} worse than baseline, ` +
    `${autobahn.intermittent.length} in the intermittent invalid-frame family`,
);
