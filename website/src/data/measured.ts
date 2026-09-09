// GENERATED FILE -- do not edit by hand.
//
// Produced by scripts/gen-measured.mjs from the output of
// scripts/bench-compare.sh. Regenerate with:
//
//   node scripts/gen-measured.mjs <results.json> [...]
//
// Measured on:
//   debian variant -- AMD EPYC 9255 24-Core Processor, postgres:16, 30000 requests at concurrency 50
//     governor=performance boost_sysfs=0 smt=off online=0-23
//     self-consistency: pass (0.1% drift over the run)
//   alpine variant -- AMD EPYC 9255 24-Core Processor, postgres:16.11-alpine, 30000 requests at concurrency 50
//     governor=performance boost=0 smt=off online=0-23
//     self-consistency: pass (1.3% drift over the run)

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
  host: "Linux 7.0.0-30-generic x86_64",
  postgres: "postgres:16",
  dataset: "bench_items 100000 rows, bench_reviews 300000 rows",
  requests: 30000,
  concurrency: 50,
  repeats: 3,
  warmup: 500,
  variant: "debian",
  /** The CPU, and the state it was held in. A throughput figure without these is not reproducible. */
  cpu: "AMD EPYC 9255 24-Core Processor",
  cpuState: "governor=performance boost_sysfs=0 smt=off online=0-23",
  /** Which cores each component was confined to, so none of them shared an L3 slice. */
  pinning: { db: "0-5", server: "6-11", load: "12-17" },
  /**
   * The first measurement of the run, repeated as its last action. A run whose
   * two readings disagree did not hold still, and nothing measured in between
   * is comparable across targets.
   */
  selfConsistency: {
    scenario: "point lookup",
    target: "postrust",
    first_rps: 44324,
    last_rps: 44274,
    drift_pct: 0.1,
    threshold_pct: 3,
    status: "pass",
  },
  /**
   * Whether the targets were verified to have answered each scenario with the
   * same number of rows, and the REST targets with the same columns, before
   * any of them was measured. A throughput figure over unequal work is not a
   * comparison.
   */
  equivalence: { mismatches: [], fatal: 0 },
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
 * `onDisk` is the uncompressed size docker reports for the image.
 * `layerBytes` is `docker image inspect .Size`, which is the compressed
 * download size under the containerd snapshotter.
 */
export const variantImages: Record<string, VariantImages> = {
  debian: {
    postgres: "postgres:16",
    images: {
      postrust: {
        image: "postrust:bench-debian",
        onDisk: "149MB",
        layerBytes: 148824698,
        rssIdleKb: 3096,
        rssAfterKb: 17510,
      },
      postgrest: {
        image: "postgrest/postgrest:v16.1",
        onDisk: "27MB",
        layerBytes: 27028137,
        rssIdleKb: 5368,
        rssAfterKb: 52204,
      },
      hasura: {
        image: "hasura/graphql-engine:v2.50.1",
        onDisk: "602MB",
        layerBytes: 601716825,
        rssIdleKb: 170086,
        rssAfterKb: 207360,
      },
      postgraphile: {
        image: "postgraphile:bench-debian",
        onDisk: "2.29GB",
        layerBytes: 2285948897,
        rssIdleKb: 56054,
        rssAfterKb: 220467,
      },
    },
  },
  alpine: {
    postgres: "postgres:16.11-alpine",
    images: {
      postrust: {
        image: "postrust:bench-alpine",
        onDisk: "34.9MB",
        layerBytes: 34905595,
        rssIdleKb: 2816,
        rssAfterKb: 22159,
      },
      postgrest: {
        image: "postgrest/postgrest:v16.1",
        onDisk: "27MB",
        layerBytes: 27028137,
        rssIdleKb: 5484,
        rssAfterKb: 54753,
      },
      hasura: {
        image: "hasura/graphql-engine:v2.50.1",
        onDisk: "602MB",
        layerBytes: 601716825,
        rssIdleKb: 166400,
        rssAfterKb: 208998,
      },
      postgraphile: {
        image: "postgraphile:bench-alpine",
        onDisk: "893MB",
        layerBytes: 893407044,
        rssIdleKb: 52972,
        rssAfterKb: 104038,
      },
    },
  },
};

const measured: Record<
  string,
  { rest: MeasuredRow[]; graphql: MeasuredRow[] }
> = {
  postgrest: {
    rest: [
      {
        scenario: "point lookup",
        postrust: {
          rps: 44324,
          p50_ms: 1.1,
          p95_ms: 1.1,
          p99_ms: 1.2,
        },
        other: {
          rps: 10958,
          p50_ms: 2.8,
          p95_ms: 10.9,
          p99_ms: 23,
        },
      },
      {
        scenario: "25-row page",
        postrust: {
          rps: 34633,
          p50_ms: 1.4,
          p95_ms: 1.5,
          p99_ms: 1.6,
        },
        other: {
          rps: 10488,
          p50_ms: 4,
          p95_ms: 11.2,
          p99_ms: 15.7,
        },
      },
      {
        scenario: "filtered + ordered page",
        postrust: {
          rps: 32701,
          p50_ms: 1.5,
          p95_ms: 1.6,
          p99_ms: 1.6,
        },
        other: {
          rps: 6306,
          p50_ms: 6.6,
          p95_ms: 16.4,
          p99_ms: 27.2,
        },
      },
      {
        scenario: "range filter on numeric",
        postrust: {
          rps: 30837,
          p50_ms: 1.6,
          p95_ms: 1.7,
          p99_ms: 1.7,
        },
        other: {
          rps: 7099,
          p50_ms: 6.1,
          p95_ms: 14.8,
          p99_ms: 19.6,
        },
      },
      {
        scenario: "25-row page + embed",
        postrust: {
          rps: 20093,
          p50_ms: 2.4,
          p95_ms: 2.7,
          p99_ms: 2.8,
        },
        other: {
          rps: 2152,
          p50_ms: 16.8,
          p95_ms: 64.4,
          p99_ms: 97.9,
        },
      },
    ],
    graphql: [],
  },
  hasura: {
    rest: [],
    graphql: [
      {
        scenario: "single row by primary key",
        postrust: {
          rps: 32146,
          p50_ms: 1.5,
          p95_ms: 1.6,
          p99_ms: 1.7,
        },
        other: {
          rps: 9650,
          p50_ms: 5.1,
          p95_ms: 6,
          p99_ms: 8.8,
        },
      },
      {
        scenario: "25-row page",
        postrust: {
          rps: 17411,
          p50_ms: 2.8,
          p95_ms: 3.1,
          p99_ms: 3.2,
        },
        other: {
          rps: 10379,
          p50_ms: 4.7,
          p95_ms: 5.5,
          p99_ms: 7.9,
        },
      },
      {
        scenario: "25-row page + embed",
        postrust: {
          rps: 10092,
          p50_ms: 4.9,
          p95_ms: 5.3,
          p99_ms: 5.5,
        },
        other: {
          rps: 8024,
          p50_ms: 6.1,
          p95_ms: 8.6,
          p99_ms: 10,
        },
      },
    ],
  },
  postgraphile: {
    rest: [],
    graphql: [
      {
        scenario: "single row by primary key",
        postrust: {
          rps: 32146,
          p50_ms: 1.5,
          p95_ms: 1.6,
          p99_ms: 1.7,
        },
        other: {
          rps: 14127,
          p50_ms: 3.3,
          p95_ms: 4.1,
          p99_ms: 4.8,
        },
      },
      {
        scenario: "25-row page",
        postrust: {
          rps: 17411,
          p50_ms: 2.8,
          p95_ms: 3.1,
          p99_ms: 3.2,
        },
        other: {
          rps: 9003,
          p50_ms: 5.6,
          p95_ms: 5.9,
          p99_ms: 7,
        },
      },
      {
        scenario: "25-row page + embed",
        postrust: {
          rps: 10092,
          p50_ms: 4.9,
          p95_ms: 5.3,
          p99_ms: 5.5,
        },
        other: {
          rps: 4498,
          p50_ms: 10.9,
          p95_ms: 11.5,
          p99_ms: 12.8,
        },
      },
    ],
  },
};

export function measuredFor(
  slug: string,
  surface: "rest" | "graphql",
): MeasuredRow[] {
  return measured[slug]?.[surface] ?? [];
}

/** The benchmark target each comparison page is measured against. */
export const targetForSlug: Record<string, string> = {
  postgrest: "postgrest",
  hasura: "hasura",
  postgraphile: "postgraphile",
};

export interface FootprintRow {
  label: string;
  postrust: string;
  other: string | null;
}

const KB = 1024;
const mb = (kb: number) => `${(kb / KB).toFixed(1)} MB`;

/**
 * Image size and memory for Postrust against one other tool, from the variant
 * the published throughput figures come from.
 */
export function footprintFor(slug: string): FootprintRow[] {
  const variant = variantImages[benchMeta.variant];
  const target = targetForSlug[slug];
  if (!variant) return [];

  const ours = variant.images.postrust;
  const theirs = target ? variant.images[target] : undefined;
  if (!ours) return [];

  return [
    {
      label: "Container image, on disk",
      postrust: ours.onDisk,
      other: theirs?.onDisk ?? null,
    },
    {
      label: "Memory, before serving a request",
      postrust: mb(ours.rssIdleKb),
      other: theirs ? mb(theirs.rssIdleKb) : null,
    },
    {
      label: "Memory, after the benchmark",
      postrust: mb(ours.rssAfterKb),
      other: theirs ? mb(theirs.rssAfterKb) : null,
    },
  ];
}

/** Every variant's Postrust image size, for the note under the table. */
export const postrustImages: Record<string, string> = {
  debian: "149MB",
  alpine: "34.9MB",
};
