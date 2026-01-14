-- SaaS Domain Management Schema
-- Multi-tenant domain ownership validation and reverse proxy routing

-- Tenants (SaaS customers)
CREATE TABLE IF NOT EXISTS proxy_tenants (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name VARCHAR(255) NOT NULL,
    slug VARCHAR(100) NOT NULL UNIQUE,
    email VARCHAR(255) NOT NULL,
    status VARCHAR(20) DEFAULT 'active' CHECK (status IN ('active', 'suspended', 'pending')),
    plan VARCHAR(50) DEFAULT 'free',
    max_domains INTEGER DEFAULT 5,
    max_routes_per_domain INTEGER DEFAULT 10,
    settings JSONB DEFAULT '{}',
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_proxy_tenants_slug ON proxy_tenants(slug);
CREATE INDEX IF NOT EXISTS idx_proxy_tenants_status ON proxy_tenants(status);

-- API Keys for tenant authentication
CREATE TABLE IF NOT EXISTS proxy_api_keys (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL REFERENCES proxy_tenants(id) ON DELETE CASCADE,
    name VARCHAR(255) NOT NULL,
    key_hash VARCHAR(64) NOT NULL UNIQUE,
    key_prefix VARCHAR(8) NOT NULL,
    scopes TEXT[] DEFAULT ARRAY['domains:read', 'domains:write'],
    last_used_at TIMESTAMPTZ,
    expires_at TIMESTAMPTZ,
    enabled BOOLEAN DEFAULT true,
    created_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_proxy_api_keys_tenant ON proxy_api_keys(tenant_id);
CREATE INDEX IF NOT EXISTS idx_proxy_api_keys_hash ON proxy_api_keys(key_hash);

-- Tenant upstreams (backend server groups) - created before routes due to FK
CREATE TABLE IF NOT EXISTS proxy_domain_upstreams (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL REFERENCES proxy_tenants(id) ON DELETE CASCADE,
    name VARCHAR(255) NOT NULL,
    lb_strategy VARCHAR(20) DEFAULT 'round_robin'
        CHECK (lb_strategy IN ('round_robin', 'least_connections', 'weighted', 'random', 'sticky')),
    health_check_enabled BOOLEAN DEFAULT true,
    health_check_path VARCHAR(500) DEFAULT '/health',
    health_check_interval_secs INTEGER DEFAULT 30,
    health_check_timeout_secs INTEGER DEFAULT 5,
    healthy_threshold INTEGER DEFAULT 2,
    unhealthy_threshold INTEGER DEFAULT 3,
    enabled BOOLEAN DEFAULT true,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW(),
    UNIQUE(tenant_id, name)
);

CREATE INDEX IF NOT EXISTS idx_proxy_domain_upstreams_tenant ON proxy_domain_upstreams(tenant_id);

-- Upstream backends
CREATE TABLE IF NOT EXISTS proxy_domain_backends (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    upstream_id UUID NOT NULL REFERENCES proxy_domain_upstreams(id) ON DELETE CASCADE,
    address VARCHAR(255) NOT NULL,
    scheme VARCHAR(10) DEFAULT 'http' CHECK (scheme IN ('http', 'https')),
    weight INTEGER DEFAULT 100,
    enabled BOOLEAN DEFAULT true,
    created_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_proxy_domain_backends_upstream ON proxy_domain_backends(upstream_id);

-- Custom domains
CREATE TABLE IF NOT EXISTS proxy_domains (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL REFERENCES proxy_tenants(id) ON DELETE CASCADE,
    domain VARCHAR(253) NOT NULL UNIQUE,
    verification_status VARCHAR(20) DEFAULT 'pending'
        CHECK (verification_status IN ('pending', 'verified', 'failed', 'expired')),
    verification_method VARCHAR(10) DEFAULT 'dns'
        CHECK (verification_method IN ('dns', 'http')),
    verification_token VARCHAR(64) NOT NULL,
    verification_attempts INTEGER DEFAULT 0,
    verified_at TIMESTAMPTZ,
    last_verification_attempt TIMESTAMPTZ,
    ssl_status VARCHAR(20) DEFAULT 'pending'
        CHECK (ssl_status IN ('pending', 'provisioning', 'active', 'failed', 'expired')),
    ssl_provider VARCHAR(20) DEFAULT 'acme'
        CHECK (ssl_provider IN ('acme', 'manual', 'none')),
    ssl_expires_at TIMESTAMPTZ,
    enabled BOOLEAN DEFAULT false,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_proxy_domains_tenant ON proxy_domains(tenant_id);
CREATE INDEX IF NOT EXISTS idx_proxy_domains_verification ON proxy_domains(verification_status);
CREATE INDEX IF NOT EXISTS idx_proxy_domains_ssl_status ON proxy_domains(ssl_status);
CREATE INDEX IF NOT EXISTS idx_proxy_domains_domain ON proxy_domains(domain);

-- Domain routes (tenant-specific routing)
CREATE TABLE IF NOT EXISTS proxy_domain_routes (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    domain_id UUID NOT NULL REFERENCES proxy_domains(id) ON DELETE CASCADE,
    tenant_id UUID NOT NULL REFERENCES proxy_tenants(id) ON DELETE CASCADE,
    name VARCHAR(255) NOT NULL,
    path_pattern VARCHAR(500) DEFAULT '/',
    path_type VARCHAR(10) DEFAULT 'prefix'
        CHECK (path_type IN ('prefix', 'exact', 'regex')),
    methods TEXT[],
    priority INTEGER DEFAULT 100,
    upstream_id UUID REFERENCES proxy_domain_upstreams(id) ON DELETE SET NULL,
    strip_path BOOLEAN DEFAULT false,
    add_headers JSONB DEFAULT '{}',
    remove_headers TEXT[] DEFAULT '{}',
    rate_limit_requests INTEGER,
    rate_limit_window_secs INTEGER,
    timeout_secs INTEGER DEFAULT 30,
    enabled BOOLEAN DEFAULT true,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW(),
    UNIQUE(domain_id, path_pattern, path_type)
);

CREATE INDEX IF NOT EXISTS idx_proxy_domain_routes_domain ON proxy_domain_routes(domain_id);
CREATE INDEX IF NOT EXISTS idx_proxy_domain_routes_tenant ON proxy_domain_routes(tenant_id);

-- Verification challenges tracking
CREATE TABLE IF NOT EXISTS proxy_verification_challenges (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    domain_id UUID NOT NULL REFERENCES proxy_domains(id) ON DELETE CASCADE,
    challenge_type VARCHAR(10) NOT NULL CHECK (challenge_type IN ('dns', 'http')),
    token VARCHAR(64) NOT NULL,
    expected_value TEXT NOT NULL,
    status VARCHAR(20) DEFAULT 'pending'
        CHECK (status IN ('pending', 'checking', 'verified', 'failed')),
    error_message TEXT,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    expires_at TIMESTAMPTZ DEFAULT NOW() + INTERVAL '7 days',
    verified_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_proxy_verification_challenges_domain ON proxy_verification_challenges(domain_id);
CREATE INDEX IF NOT EXISTS idx_proxy_verification_challenges_status ON proxy_verification_challenges(status);

-- Audit log for domain operations
CREATE TABLE IF NOT EXISTS proxy_domain_audit_log (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID REFERENCES proxy_tenants(id) ON DELETE SET NULL,
    domain_id UUID REFERENCES proxy_domains(id) ON DELETE SET NULL,
    action VARCHAR(50) NOT NULL,
    actor_type VARCHAR(20) NOT NULL CHECK (actor_type IN ('api_key', 'jwt', 'system')),
    actor_id VARCHAR(255),
    details JSONB DEFAULT '{}',
    ip_address INET,
    created_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_proxy_domain_audit_log_tenant ON proxy_domain_audit_log(tenant_id);
CREATE INDEX IF NOT EXISTS idx_proxy_domain_audit_log_domain ON proxy_domain_audit_log(domain_id);
CREATE INDEX IF NOT EXISTS idx_proxy_domain_audit_log_created ON proxy_domain_audit_log(created_at);

-- Add tenant/domain columns to existing certificates table if it exists
DO $$
BEGIN
    IF EXISTS (SELECT FROM pg_tables WHERE tablename = 'proxy_certificates') THEN
        ALTER TABLE proxy_certificates ADD COLUMN IF NOT EXISTS tenant_id UUID REFERENCES proxy_tenants(id) ON DELETE CASCADE;
        ALTER TABLE proxy_certificates ADD COLUMN IF NOT EXISTS domain_id UUID REFERENCES proxy_domains(id) ON DELETE CASCADE;
        ALTER TABLE proxy_certificates ADD COLUMN IF NOT EXISTS provider VARCHAR(20) DEFAULT 'acme';
        ALTER TABLE proxy_certificates ADD COLUMN IF NOT EXISTS auto_renew BOOLEAN DEFAULT true;
        CREATE INDEX IF NOT EXISTS idx_proxy_certificates_tenant ON proxy_certificates(tenant_id);
        CREATE INDEX IF NOT EXISTS idx_proxy_certificates_domain_id ON proxy_certificates(domain_id);
    END IF;
END $$;

-- Function to get all enabled routes for proxy runtime
CREATE OR REPLACE FUNCTION proxy_get_active_routes()
RETURNS TABLE (
    route_id UUID,
    domain VARCHAR,
    path_pattern VARCHAR,
    path_type VARCHAR,
    methods TEXT[],
    priority INTEGER,
    upstream_id UUID,
    upstream_name VARCHAR,
    strip_path BOOLEAN,
    add_headers JSONB,
    remove_headers TEXT[],
    timeout_secs INTEGER,
    rate_limit_requests INTEGER,
    rate_limit_window_secs INTEGER,
    tenant_id UUID
) AS $$
BEGIN
    RETURN QUERY
    SELECT
        r.id as route_id,
        d.domain,
        r.path_pattern,
        r.path_type,
        r.methods,
        r.priority,
        r.upstream_id,
        u.name as upstream_name,
        r.strip_path,
        r.add_headers,
        r.remove_headers,
        r.timeout_secs,
        r.rate_limit_requests,
        r.rate_limit_window_secs,
        r.tenant_id
    FROM proxy_domain_routes r
    JOIN proxy_domains d ON r.domain_id = d.id
    LEFT JOIN proxy_domain_upstreams u ON r.upstream_id = u.id
    WHERE r.enabled = true
      AND d.enabled = true
      AND d.verification_status = 'verified'
    ORDER BY r.priority DESC, d.domain;
END;
$$ LANGUAGE plpgsql STABLE;

-- Function to get upstreams with backends
CREATE OR REPLACE FUNCTION proxy_get_active_upstreams(p_tenant_id UUID DEFAULT NULL)
RETURNS TABLE (
    upstream_id UUID,
    upstream_name VARCHAR,
    lb_strategy VARCHAR,
    health_check_enabled BOOLEAN,
    health_check_path VARCHAR,
    health_check_interval_secs INTEGER,
    health_check_timeout_secs INTEGER,
    healthy_threshold INTEGER,
    unhealthy_threshold INTEGER,
    backend_id UUID,
    backend_address VARCHAR,
    backend_scheme VARCHAR,
    backend_weight INTEGER,
    tenant_id UUID
) AS $$
BEGIN
    RETURN QUERY
    SELECT
        u.id as upstream_id,
        u.name as upstream_name,
        u.lb_strategy,
        u.health_check_enabled,
        u.health_check_path,
        u.health_check_interval_secs,
        u.health_check_timeout_secs,
        u.healthy_threshold,
        u.unhealthy_threshold,
        b.id as backend_id,
        b.address as backend_address,
        b.scheme as backend_scheme,
        b.weight as backend_weight,
        u.tenant_id
    FROM proxy_domain_upstreams u
    JOIN proxy_domain_backends b ON b.upstream_id = u.id
    WHERE u.enabled = true AND b.enabled = true
      AND (p_tenant_id IS NULL OR u.tenant_id = p_tenant_id);
END;
$$ LANGUAGE plpgsql STABLE;

-- Trigger function for config change notifications
CREATE OR REPLACE FUNCTION proxy_notify_config_change()
RETURNS TRIGGER AS $$
BEGIN
    PERFORM pg_notify('proxy_config_change', json_build_object(
        'table', TG_TABLE_NAME,
        'operation', TG_OP,
        'tenant_id', COALESCE(NEW.tenant_id, OLD.tenant_id)
    )::text);
    RETURN COALESCE(NEW, OLD);
END;
$$ LANGUAGE plpgsql;

-- Apply triggers for config change notifications
DROP TRIGGER IF EXISTS proxy_domains_change ON proxy_domains;
CREATE TRIGGER proxy_domains_change
    AFTER INSERT OR UPDATE OR DELETE ON proxy_domains
    FOR EACH ROW EXECUTE FUNCTION proxy_notify_config_change();

DROP TRIGGER IF EXISTS proxy_domain_routes_change ON proxy_domain_routes;
CREATE TRIGGER proxy_domain_routes_change
    AFTER INSERT OR UPDATE OR DELETE ON proxy_domain_routes
    FOR EACH ROW EXECUTE FUNCTION proxy_notify_config_change();

DROP TRIGGER IF EXISTS proxy_domain_upstreams_change ON proxy_domain_upstreams;
CREATE TRIGGER proxy_domain_upstreams_change
    AFTER INSERT OR UPDATE OR DELETE ON proxy_domain_upstreams
    FOR EACH ROW EXECUTE FUNCTION proxy_notify_config_change();

DROP TRIGGER IF EXISTS proxy_domain_backends_change ON proxy_domain_backends;
CREATE TRIGGER proxy_domain_backends_change
    AFTER INSERT OR UPDATE OR DELETE ON proxy_domain_backends
    FOR EACH ROW EXECUTE FUNCTION proxy_notify_config_change();
