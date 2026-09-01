-- ACME certificate issuance.
--
-- Three new tables and four new columns, all of them needed before the proxy
-- can obtain a certificate from a certificate authority.

-- Certificates, keyed by domain.
--
-- `CertificateStore` has queried this table since it was written, and nothing
-- ever created it -- so every save failed with "relation proxy_certificates
-- does not exist". The SaaS migration has an `IF EXISTS` block that adds
-- tenant_id, domain_id, provider and auto_renew to this table *if* it is
-- already there; it never was, so that block has always been a no-op. Those
-- four columns are declared here instead, which leaves the same schema as if
-- the two migrations had run in the other order.
CREATE TABLE IF NOT EXISTS proxy_certificates (
    domain VARCHAR(253) PRIMARY KEY,
    cert_pem BYTEA NOT NULL,
    key_pem BYTEA NOT NULL,
    expires_at TIMESTAMPTZ,
    tenant_id UUID REFERENCES proxy_tenants(id) ON DELETE CASCADE,
    domain_id UUID REFERENCES proxy_domains(id) ON DELETE CASCADE,
    provider VARCHAR(20) NOT NULL DEFAULT 'acme'
        CHECK (provider IN ('acme', 'manual', 'none')),
    auto_renew BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_proxy_certificates_tenant ON proxy_certificates(tenant_id);
CREATE INDEX IF NOT EXISTS idx_proxy_certificates_domain_id ON proxy_certificates(domain_id);
-- Renewal scans read this: everything expiring inside the threshold.
CREATE INDEX IF NOT EXISTS idx_proxy_certificates_expires_at
    ON proxy_certificates(expires_at) WHERE auto_renew;

-- The ACME account, one per directory.
--
-- Persisted because registering an account is rate-limited -- Let's Encrypt
-- allows 10 per IP per 3 hours -- so a proxy that registered a fresh account on
-- every restart would lock itself out, and would lose the authorizations
-- already granted to the old one.
CREATE TABLE IF NOT EXISTS proxy_acme_accounts (
    directory_url TEXT PRIMARY KEY,
    -- instant_acme::AccountCredentials, which contains the account's private
    -- key. Readable only by the proxy's database role.
    credentials JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Pending HTTP-01 challenge responses.
--
-- In a table rather than in memory because the CA fetches
-- `http://{domain}/.well-known/acme-challenge/{token}` and DNS decides which
-- instance answers. With more than one proxy behind a domain, the instance that
-- placed the order is usually not the one the CA reaches, and an in-memory map
-- would fail the challenge whenever they differed.
CREATE TABLE IF NOT EXISTS proxy_acme_challenges (
    token VARCHAR(255) PRIMARY KEY,
    domain VARCHAR(253) NOT NULL,
    -- `{token}.{account_key_thumbprint}`, served verbatim as the response body.
    key_authorization TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    -- An order that is never finalised must not leave a token answerable for
    -- ever. An hour is far longer than validation takes.
    expires_at TIMESTAMPTZ NOT NULL DEFAULT NOW() + INTERVAL '1 hour'
);

CREATE INDEX IF NOT EXISTS idx_proxy_acme_challenges_domain ON proxy_acme_challenges(domain);
CREATE INDEX IF NOT EXISTS idx_proxy_acme_challenges_expires ON proxy_acme_challenges(expires_at);

-- Why the last issuance attempt failed, and how many there have been.
--
-- Without these the worker cannot back off: a domain whose DNS was never
-- pointed at the proxy fails for ever, and retrying it every cycle burns the
-- CA's rate limit for the domains that would succeed. `ssl_error` is also the
-- only way an operator learns *why* a domain is stuck.
ALTER TABLE proxy_domains ADD COLUMN IF NOT EXISTS ssl_error TEXT;
ALTER TABLE proxy_domains ADD COLUMN IF NOT EXISTS ssl_attempts INTEGER NOT NULL DEFAULT 0;
ALTER TABLE proxy_domains ADD COLUMN IF NOT EXISTS ssl_last_attempt_at TIMESTAMPTZ;

-- The worker's claim query: verified, wants ACME, and not yet active.
CREATE INDEX IF NOT EXISTS idx_proxy_domains_ssl_pending
    ON proxy_domains(ssl_status, ssl_provider)
    WHERE verification_status = 'verified';
