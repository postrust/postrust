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
        layerBytes: 36888694,
        rssIdleKb: 6900,
        rssAfterKb: 20388,
      },
      postgrest: {
        image: "postgrest/postgrest:v16.1",
        onDisk: "26.8MB",
        layerBytes: 6433834,
        rssIdleKb: 13711,
        rssAfterKb: 49900,
      },
      hasura: {
        image: "hasura/graphql-engine:v2.44.0",
        onDisk: "711MB",
        layerBytes: 159655847,
        rssIdleKb: 167219,
        rssAfterKb: 195072,
      },
      postgraphile: {
        image: "postgraphile:bench-debian",
        onDisk: "2.27GB",
        layerBytes: 540044768,
        rssIdleKb: 44186,
        rssAfterKb: 141107,
      },
    },
  },
  alpine: {
    postgres: "postgres:16.11-alpine",
    images: {
      postrust: {
        image: "postrust:bench-alpine",
        onDisk: "31.5MB",
        layerBytes: 9721853,
        rssIdleKb: 5968,
        rssAfterKb: 14848,
      },
      postgrest: {
        image: "postgrest/postgrest:v16.1",
        onDisk: "26.8MB",
        layerBytes: 6433834,
        rssIdleKb: 4212,
        rssAfterKb: 37243,
      },
      hasura: {
        image: "hasura/graphql-engine:v2.44.0",
        onDisk: "711MB",
        layerBytes: 159655847,
        rssIdleKb: 46561,
        rssAfterKb: 74650,
      },
      postgraphile: {
        image: "postgraphile:bench-alpine",
        onDisk: "886MB",
        layerBytes: 198538661,
        rssIdleKb: 115610,
        rssAfterKb: 173261,
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
          rps: 13475,
          p50_ms: 3.3,
          p95_ms: 4.9,
          p99_ms: 15.1,
        },
        other: {
          rps: 7496,
          p50_ms: 4.7,
          p95_ms: 17.9,
          p99_ms: 45.8,
        },
      },
      {
        scenario: "25-row page",
        postrust: {
          rps: 13439,
          p50_ms: 3.6,
          p95_ms: 4.7,
          p99_ms: 5.5,
        },
        other: {
          rps: 5727,
          p50_ms: 5.4,
          p95_ms: 25.2,
          p99_ms: 44.6,
        },
      },
      {
        scenario: "filtered + ordered page",
        postrust: {
          rps: 11292,
          p50_ms: 4.2,
          p95_ms: 5.4,
          p99_ms: 6.2,
        },
        other: {
          rps: 6565,
          p50_ms: 5.7,
          p95_ms: 18.6,
          p99_ms: 40.9,
        },
      },
      {
        scenario: "range filter on numeric",
        postrust: {
          rps: 11822,
          p50_ms: 4.1,
          p95_ms: 4.9,
          p99_ms: 5.7,
        },
        other: {
          rps: 4309,
          p50_ms: 7.9,
          p95_ms: 33.7,
          p99_ms: 51.3,
        },
      },
      {
        scenario: "25-row page + embed",
        postrust: {
          rps: 7629,
          p50_ms: 6.4,
          p95_ms: 7.8,
          p99_ms: 9,
        },
        other: {
          rps: 4649,
          p50_ms: 7.9,
          p95_ms: 26.5,
          p99_ms: 53.3,
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
          rps: 12608,
          p50_ms: 3.7,
          p95_ms: 5.2,
          p99_ms: 6.2,
        },
        other: {
          rps: 3603,
          p50_ms: 11.6,
          p95_ms: 19.5,
          p99_ms: 121.2,
        },
      },
      {
        scenario: "25-row page",
        postrust: {
          rps: 8062,
          p50_ms: 6.1,
          p95_ms: 7.2,
          p99_ms: 8.1,
        },
        other: {
          rps: 4807,
          p50_ms: 10.7,
          p95_ms: 14,
          p99_ms: 17.4,
        },
      },
      {
        scenario: "25-row page + embed",
        postrust: {
          rps: 4379,
          p50_ms: 11.1,
          p95_ms: 14.6,
          p99_ms: 19.2,
        },
        other: {
          rps: 5945,
          p50_ms: 3.5,
          p95_ms: 33.6,
          p99_ms: 38.5,
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
          rps: 12608,
          p50_ms: 3.7,
          p95_ms: 5.2,
          p99_ms: 6.2,
        },
        other: {
          rps: 6344,
          p50_ms: 6.3,
          p95_ms: 11.4,
          p99_ms: 39.6,
        },
      },
      {
        scenario: "25-row page",
        postrust: {
          rps: 8062,
          p50_ms: 6.1,
          p95_ms: 7.2,
          p99_ms: 8.1,
        },
        other: {
          rps: 6340,
          p50_ms: 7.1,
          p95_ms: 10.6,
          p99_ms: 32.3,
        },
      },
      {
        scenario: "25-row page + embed",
        postrust: {
          rps: 4379,
          p50_ms: 11.1,
          p95_ms: 14.6,
          p99_ms: 19.2,
        },
        other: {
          rps: 3536,
          p50_ms: 13.6,
          p95_ms: 15.6,
          p99_ms: 43.2,
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
