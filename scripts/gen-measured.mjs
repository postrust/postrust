#!/usr/bin/env node
// Generates website/src/data/measured.ts from bench-compare.sh output.
//
// The website never has numbers typed into it by hand: they are copied from
// results.json by this script, so what is published is what was measured.
//
// Usage:
//   node scripts/gen-measured.mjs <results.json> [more-results.json ...]
//
// Multiple files may be passed (for example the debian and alpine runs). The
// first file supplies the request figures; later files add any image/memory
// rows for variants the earlier ones did not cover.

import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join, resolve } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const OUT = join(here, "..", "website", "src", "data", "measured.ts");

const files = process.argv.slice(2);
if (files.length === 0) {
  console.error("usage: node scripts/gen-measured.mjs <results.json> [...]");
  process.exit(1);
}

const runs = files.map((f) => ({ path: resolve(f), data: JSON.parse(readFileSync(f, "utf8")) }));
const primary = runs[0].data;

// Which target each comparison page is measured against, and on which surface.
// Supabase is deliberately absent: its REST layer *is* PostgREST, so presenting
// PostgREST's numbers under a Supabase heading would be misleading.
const PAGES = {
  postgrest: { rest: "postgrest" },
  hasura: { graphql: "hasura" },
  postgraphile: { graphql: "postgraphile" },
};

function point(m) {
  if (!m || !m.supported) return null;
  return { rps: m.rps, p50_ms: m.p50_ms, p95_ms: m.p95_ms, p99_ms: m.p99_ms };
}

function rowsFor(surface, otherTarget) {
  const ms = primary.measurements.filter((m) => m.surface === surface);
  const scenarios = [...new Set(ms.map((m) => m.scenario))];

  return scenarios
    .map((scenario) => {
      const find = (t) => ms.find((m) => m.scenario === scenario && m.target === t);
      const postrust = point(find("postrust"));
      const other = point(find(otherTarget));
      // A row where neither side produced a number says nothing.
      if (!postrust && !other) return null;
      return { scenario, postrust, other };
    })
    .filter(Boolean);
}

const data = {};
for (const [slug, surfaces] of Object.entries(PAGES)) {
  data[slug] = { rest: [], graphql: [] };
  for (const [surface, target] of Object.entries(surfaces)) {
    data[slug][surface] = rowsFor(surface, target);
  }
}

// Image and memory figures, per variant, for every run supplied.
const variants = {};
for (const { data: d } of runs) {
  variants[d.variant ?? "unknown"] = {
    postgres: d.postgres,
    images: Object.fromEntries(
      Object.entries(d.images).map(([target, v]) => [
        target,
        {
          image: v.image,
          onDisk: v.size_on_disk,
          layerBytes: v.size_layers_bytes,
          rssIdleKb: v.rss_idle_kb,
          rssAfterKb: v.rss_after_kb,
        },
      ]),
    ),
  };
}

const banner = `// GENERATED FILE -- do not edit by hand.
//
// Produced by scripts/gen-measured.mjs from the output of
// scripts/bench-compare.sh. Regenerate with:
//
//   node scripts/gen-measured.mjs <results.json> [...]
//
// Measured on:
${runs
  .map(
    (r) =>
      `//   ${r.data.variant ?? "unknown"} variant -- ${r.data.host}, ${r.data.postgres}, ` +
      `${r.data.requests} requests at concurrency ${r.data.concurrency}`,
  )
  .join("\n")}
`;

const body = `${banner}
export interface Point {
  rps: number;
  p50_ms: number;
  p95_ms: number;
  p99_ms: number;
}

export interface MeasuredRow {
  scenario: string;
  /** null where the tool does not support the scenario. */
  postrust: Point | null;
  other: Point | null;
}

export const benchMeta = {
  host: ${JSON.stringify(primary.host)},
  postgres: ${JSON.stringify(primary.postgres)},
  dataset: ${JSON.stringify(primary.dataset)},
  requests: ${primary.requests},
  concurrency: ${primary.concurrency},
  variant: ${JSON.stringify(primary.variant ?? "unknown")},
} as const;

export interface VariantImages {
  postgres: string;
  images: Record<
    string,
    {
      image: string;
      onDisk: string;
      layerBytes: number;
      rssIdleKb: number;
      rssAfterKb: number;
    }
  >;
}

/**
 * Image size and memory, per base-image variant.
 *
 * \`onDisk\` is the uncompressed size docker reports for the image.
 * \`layerBytes\` is \`docker image inspect .Size\`, which is the compressed
 * download size under the containerd snapshotter.
 */
export const variantImages: Record<string, VariantImages> = ${JSON.stringify(variants, null, 2)};

const measured: Record<string, { rest: MeasuredRow[]; graphql: MeasuredRow[] }> = ${JSON.stringify(
  data,
  null,
  2,
)};

export function measuredFor(slug: string, surface: "rest" | "graphql"): MeasuredRow[] {
  return measured[slug]?.[surface] ?? [];
}
`;

writeFileSync(OUT, body);
console.log(`wrote ${OUT}`);
for (const [slug, v] of Object.entries(data)) {
  console.log(`  ${slug}: ${v.rest.length} rest, ${v.graphql.length} graphql`);
}
