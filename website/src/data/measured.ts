// GENERATED FILE -- do not edit by hand.
//
// Produced by scripts/gen-measured.mjs from the output of
// scripts/bench-compare.sh. Regenerate with:
//
//   node scripts/gen-measured.mjs <results.json> [...]
//
// Measured on:
//   debian variant -- Darwin 25.2.0 arm64, postgres:16, 3000 requests at concurrency 50
//   alpine variant -- Darwin 25.2.0 arm64, postgres:16.11-alpine, 3000 requests at concurrency 50

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
  host: "Darwin 25.2.0 arm64",
  postgres: "postgres:16",
  dataset: "bench_items 100000 rows, bench_reviews 300000 rows",
  requests: 3000,
  concurrency: 50,
  repeats: 5,
  warmup: 500,
  variant: "debian",
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
        onDisk: "168MB",
        layerBytes: 36898663,
        rssIdleKb: 1968,
        rssAfterKb: 15155,
      },
      postgrest: {
        image: "postgrest/postgrest:v16.1",
        onDisk: "26.8MB",
        layerBytes: 6433834,
        rssIdleKb: 18729,
        rssAfterKb: 59443,
      },
      hasura: {
        image: "hasura/graphql-engine:v2.44.0",
        onDisk: "711MB",
        layerBytes: 159655847,
        rssIdleKb: 171930,
        rssAfterKb: 203674,
      },
      postgraphile: {
        image: "postgraphile:bench-debian",
        onDisk: "2.27GB",
        layerBytes: 540044768,
        rssIdleKb: 112742,
        rssAfterKb: 247910,
      },
    },
  },
  alpine: {
    postgres: "postgres:16.11-alpine",
    images: {
      postrust: {
        image: "postrust:bench-alpine",
        onDisk: "31.7MB",
        layerBytes: 9791771,
        rssIdleKb: 3844,
        rssAfterKb: 28447,
      },
      postgrest: {
        image: "postgrest/postgrest:v16.1",
        onDisk: "26.8MB",
        layerBytes: 6433834,
        rssIdleKb: 18739,
        rssAfterKb: 53709,
      },
      hasura: {
        image: "hasura/graphql-engine:v2.44.0",
        onDisk: "711MB",
        layerBytes: 159655847,
        rssIdleKb: 170803,
        rssAfterKb: 200602,
      },
      postgraphile: {
        image: "postgraphile:bench-alpine",
        onDisk: "886MB",
        layerBytes: 198538661,
        rssIdleKb: 116326,
        rssAfterKb: 212582,
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
          rps: 14479,
          p50_ms: 3.2,
          p95_ms: 4.7,
          p99_ms: 7.6,
        },
        other: {
          rps: 4323,
          p50_ms: 8.8,
          p95_ms: 31.1,
          p99_ms: 46,
        },
      },
      {
        scenario: "25-row page",
        postrust: {
          rps: 13800,
          p50_ms: 3.5,
          p95_ms: 4.4,
          p99_ms: 5,
        },
        other: {
          rps: 4152,
          p50_ms: 8.7,
          p95_ms: 34.2,
          p99_ms: 57.3,
        },
      },
      {
        scenario: "filtered + ordered page",
        postrust: {
          rps: 11292,
          p50_ms: 4.3,
          p95_ms: 5.2,
          p99_ms: 6,
        },
        other: {
          rps: 7392,
          p50_ms: 6,
          p95_ms: 10,
          p99_ms: 14.7,
        },
      },
      {
        scenario: "range filter on numeric",
        postrust: {
          rps: 10892,
          p50_ms: 4.4,
          p95_ms: 5.5,
          p99_ms: 6.9,
        },
        other: {
          rps: 7851,
          p50_ms: 5.3,
          p95_ms: 10.9,
          p99_ms: 28.7,
        },
      },
      {
        scenario: "25-row page + embed",
        postrust: {
          rps: 9488,
          p50_ms: 5.1,
          p95_ms: 5.9,
          p99_ms: 6.8,
        },
        other: {
          rps: 6005,
          p50_ms: 7.5,
          p95_ms: 12.2,
          p99_ms: 18,
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
          rps: 12440,
          p50_ms: 3.8,
          p95_ms: 5.1,
          p99_ms: 6.1,
        },
        other: {
          rps: 4535,
          p50_ms: 9.2,
          p95_ms: 44,
          p99_ms: 59.9,
        },
      },
      {
        scenario: "25-row page",
        postrust: {
          rps: 8477,
          p50_ms: 5.7,
          p95_ms: 7,
          p99_ms: 7.9,
        },
        other: {
          rps: 4907,
          p50_ms: 10.7,
          p95_ms: 17.4,
          p99_ms: 26.7,
        },
      },
      {
        scenario: "25-row page + embed",
        postrust: {
          rps: 5193,
          p50_ms: 9.5,
          p95_ms: 11.4,
          p99_ms: 12.1,
        },
        other: {
          rps: 3588,
          p50_ms: 14.8,
          p95_ms: 20,
          p99_ms: 32.5,
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
          rps: 12440,
          p50_ms: 3.8,
          p95_ms: 5.1,
          p99_ms: 6.1,
        },
        other: {
          rps: 10231,
          p50_ms: 4.6,
          p95_ms: 6.7,
          p99_ms: 10.4,
        },
      },
      {
        scenario: "25-row page",
        postrust: {
          rps: 8477,
          p50_ms: 5.7,
          p95_ms: 7,
          p99_ms: 7.9,
        },
        other: {
          rps: 6593,
          p50_ms: 7.2,
          p95_ms: 10.2,
          p99_ms: 19.7,
        },
      },
      {
        scenario: "25-row page + embed",
        postrust: {
          rps: 5193,
          p50_ms: 9.5,
          p95_ms: 11.4,
          p99_ms: 12.1,
        },
        other: {
          rps: 3495,
          p50_ms: 14,
          p95_ms: 16.5,
          p99_ms: 31,
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
  debian: "168MB",
  alpine: "31.7MB",
};
