-- Global proxy configuration: one proxy's own route table.
--
-- Distinct from the proxy_domain_* tables in 20240115000001_saas_domains.sql,
-- which are the multi-tenant SaaS module's per-domain configuration scoped to a
-- tenant. These three hold the configuration that `config::load_from_database`
-- reads at startup and the admin API edits at runtime -- the database-backed
-- equivalent of the `[[routes]]` and `[[upstreams]]` tables in a TOML config.
--
-- The columns mirror `config::types::{Route, Upstream, Backend}` field for
-- field, so that a config loaded from here and one parsed from TOML are the
-- same value. Where a Rust type has no direct SQL equivalent the mapping is
-- called out in a comment on the column.

-- Upstreams: a named group of backend servers.
CREATE TABLE IF NOT EXISTS proxy_upstreams (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    -- Routes reference an upstream by name, and `Upstream::resolved_id`
    -- derives a stable UUID from it, so the name has to be unique.
    name VARCHAR(255) NOT NULL UNIQUE,
    description TEXT,
    lb_strategy VARCHAR(20) NOT NULL DEFAULT 'round_robin'
        CHECK (lb_strategy IN ('round_robin', 'least_connections', 'weighted', 'random', 'sticky')),
    -- HealthCheckConfig, flattened. It is a fixed set of scalars with no
    -- identity of its own, so a child table would buy nothing.
    health_check_enabled BOOLEAN NOT NULL DEFAULT true,
    health_check_path VARCHAR(500) NOT NULL DEFAULT '/health',
    health_check_interval_secs INTEGER NOT NULL DEFAULT 30 CHECK (health_check_interval_secs > 0),
    health_check_timeout_secs INTEGER NOT NULL DEFAULT 5 CHECK (health_check_timeout_secs > 0),
    healthy_threshold INTEGER NOT NULL DEFAULT 2 CHECK (healthy_threshold > 0),
    unhealthy_threshold INTEGER NOT NULL DEFAULT 3 CHECK (unhealthy_threshold > 0),
    enabled BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Backends belonging to an upstream.
CREATE TABLE IF NOT EXISTS proxy_upstream_backends (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    upstream_id UUID NOT NULL REFERENCES proxy_upstreams(id) ON DELETE CASCADE,
    address VARCHAR(255) NOT NULL,
    scheme VARCHAR(10) NOT NULL DEFAULT 'http' CHECK (scheme IN ('http', 'https')),
    weight INTEGER NOT NULL DEFAULT 1 CHECK (weight > 0),
    -- UpstreamHttpVersion. Stored as the serde-canonical form of each variant,
    -- not the aliases, so there is one spelling in the database.
    http_version VARCHAR(10) NOT NULL DEFAULT 'http11'
        CHECK (http_version IN ('http11', 'h2c')),
    enabled BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    -- One entry per address within an upstream: two rows for the same backend
    -- would silently double its share of round-robin.
    UNIQUE (upstream_id, address)
);

CREATE INDEX IF NOT EXISTS idx_proxy_upstream_backends_upstream
    ON proxy_upstream_backends(upstream_id);

-- Routes: which requests go to which upstream.
CREATE TABLE IF NOT EXISTS proxy_routes (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name VARCHAR(255) NOT NULL UNIQUE,
    description TEXT,

    -- RouteMatch, flattened with a match_ prefix.
    match_host VARCHAR(253),
    match_path VARCHAR(500),
    match_path_type VARCHAR(10) NOT NULL DEFAULT 'prefix'
        CHECK (match_path_type IN ('prefix', 'exact', 'regex')),
    -- HashMap<String, String>. JSONB rather than hstore: no extension needed,
    -- and it round-trips through serde_json without a custom type.
    match_headers JSONB NOT NULL DEFAULT '{}'::jsonb,
    -- Option<Vec<String>>: NULL means "any method", which is not the same as
    -- an empty list, so this is nullable rather than defaulting to '{}'.
    match_methods TEXT[],

    priority INTEGER NOT NULL DEFAULT 100,
    -- Upstream *name*, matching `Route::upstream`. Not a foreign key: a TOML
    -- config can name an upstream that is not in the database, and refusing to
    -- load a route because its upstream lives elsewhere would be wrong.
    upstream VARCHAR(255) NOT NULL,
    strip_path BOOLEAN NOT NULL DEFAULT false,
    add_headers JSONB NOT NULL DEFAULT '{}'::jsonb,
    remove_headers TEXT[] NOT NULL DEFAULT ARRAY[]::TEXT[],

    -- Option<RouteRateLimit>. RateLimitKey is an enum with a payload
    -- (`Header(String)`), so the discriminant and the header name get a column
    -- each rather than being packed into one string that has to be parsed.
    rate_limit_requests INTEGER CHECK (rate_limit_requests > 0),
    rate_limit_window_secs INTEGER CHECK (rate_limit_window_secs > 0),
    rate_limit_key VARCHAR(20) CHECK (rate_limit_key IN ('client_ip', 'header', 'route')),
    rate_limit_header VARCHAR(255),

    timeout_secs INTEGER NOT NULL DEFAULT 30 CHECK (timeout_secs > 0),
    retry_count INTEGER NOT NULL DEFAULT 0 CHECK (retry_count >= 0),
    enabled BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- `Option<RouteRateLimit>` is all-or-nothing. Without this a half-written
    -- rate limit would load as no rate limit at all, which is the failure mode
    -- where a limit silently stops applying.
    CONSTRAINT proxy_routes_rate_limit_whole CHECK (
        num_nonnulls(rate_limit_requests, rate_limit_window_secs, rate_limit_key) IN (0, 3)
    ),

    -- `RateLimitKey::Header` carries a name; the other two variants do not.
    CONSTRAINT proxy_routes_rate_limit_header CHECK (
        (rate_limit_key = 'header') = (rate_limit_header IS NOT NULL)
    )
);

-- Routes are matched highest priority first, which is also the load order.
CREATE INDEX IF NOT EXISTS idx_proxy_routes_priority ON proxy_routes(priority DESC);
CREATE INDEX IF NOT EXISTS idx_proxy_routes_upstream ON proxy_routes(upstream);
