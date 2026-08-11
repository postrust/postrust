-- Postrust Benchmark Fixtures
--
-- A single wide-ish table with enough rows that pagination and filtering are
-- doing real work, plus the read-only role the server connects as.
--
-- Loaded by scripts/bench.sh into the postrust_bench database.

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

ANALYZE public.bench_items;

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
