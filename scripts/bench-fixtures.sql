-- Postrust Benchmark Fixtures
--
-- A wide-ish parent table with enough rows that pagination and filtering are
-- doing real work, a child table so relationship embedding can be measured,
-- plus the read-only role the server connects as.
--
-- Loaded by scripts/bench.sh and scripts/bench-compare.sh into the
-- postrust_bench database.

DROP TABLE IF EXISTS public.bench_reviews CASCADE;
DROP TABLE IF EXISTS public.bench_items CASCADE;

CREATE TABLE public.bench_items (
    id SERIAL PRIMARY KEY,
    name TEXT NOT NULL,
    category TEXT NOT NULL,
    price NUMERIC(10, 2) NOT NULL,
    stock INTEGER NOT NULL,
    is_active BOOLEAN NOT NULL DEFAULT true,
    metadata JSONB NOT NULL DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Row count is fixed so results are comparable between runs.
INSERT INTO public.bench_items (name, category, price, stock, is_active, metadata)
SELECT
    'item-' || g,
    'cat-' || (g % 20),
    ((g % 1000)::numeric / 10),
    g % 500,
    (g % 7) <> 0,
    jsonb_build_object('tier', g % 3, 'sku', 'sku-' || g)
FROM generate_series(1, 100000) g;

CREATE INDEX bench_items_category_idx ON public.bench_items (category);
CREATE INDEX bench_items_price_idx ON public.bench_items (price);

-- Child rows for the embedding scenarios. Three per item, which is small enough
-- that an embed of a 25-row page stays a realistic request and large enough
-- that a per-row query strategy costs visibly more than a batched one.
CREATE TABLE public.bench_reviews (
    id SERIAL PRIMARY KEY,
    item_id INTEGER NOT NULL REFERENCES public.bench_items (id),
    rating INTEGER NOT NULL,
    body TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

INSERT INTO public.bench_reviews (item_id, rating, body)
SELECT
    i.id,
    1 + ((i.id + r) % 5),
    'review ' || r || ' for item ' || i.id
FROM public.bench_items i
CROSS JOIN generate_series(1, 3) r;

-- The foreign key alone does not create this. Without it every tool falls back
-- to a sequential scan of 300k rows per embed, which measures the missing index
-- rather than the embedding strategy.
CREATE INDEX bench_reviews_item_id_idx ON public.bench_reviews (item_id);

ANALYZE public.bench_items;
ANALYZE public.bench_reviews;

-- Anonymous role used by the benchmark server process.
DO $$
BEGIN
    IF NOT EXISTS (SELECT FROM pg_catalog.pg_roles WHERE rolname = 'bench_anon') THEN
        CREATE ROLE bench_anon NOLOGIN;
    END IF;
END
$$;

GRANT USAGE ON SCHEMA public TO bench_anon;
GRANT SELECT ON public.bench_items TO bench_anon;
GRANT SELECT ON public.bench_reviews TO bench_anon;
