-- Control-plane fixes for the SaaS tables.
--
-- Safe to run more than once, like the migrations before it: the tests apply
-- every migration on every run.

-- ---------------------------------------------------------------------------
-- 1. The config-change trigger, made to work on proxy_domain_backends.
--
-- The original function read NEW.tenant_id, and proxy_domain_backends has no
-- tenant_id column, so every insert, update or delete of a backend failed with
-- `record "new" has no field "tenant_id"` -- creating an upstream with backends
-- answered 500 after the upstream row had already been written. PL/pgSQL
-- resolves NEW.<field> against the row type at run time, so the only way one
-- function can serve tables of different shapes is to read the row as JSON.
--
-- A backend's tenant is its upstream's. When the backend is going because its
-- upstream was deleted, the upstream row is already gone and the tenant is
-- reported as NULL; the upstream's own notification carries it.
-- ---------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION proxy_notify_config_change()
RETURNS TRIGGER AS $$
DECLARE
    row_data JSONB;
    tenant UUID;
BEGIN
    IF TG_OP = 'DELETE' THEN
        row_data := to_jsonb(OLD);
    ELSE
        row_data := to_jsonb(NEW);
    END IF;

    IF row_data ? 'tenant_id' THEN
        tenant := (row_data ->> 'tenant_id')::uuid;
    ELSIF row_data ? 'upstream_id' THEN
        SELECT u.tenant_id INTO tenant
        FROM proxy_domain_upstreams u
        WHERE u.id = (row_data ->> 'upstream_id')::uuid;
    END IF;

    PERFORM pg_notify('proxy_config_change', json_build_object(
        'table', TG_TABLE_NAME,
        'operation', TG_OP,
        'tenant_id', tenant
    )::text);

    -- AFTER ... FOR EACH ROW: the return value is ignored.
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

-- ---------------------------------------------------------------------------
-- 2. Domain names are unique regardless of case.
--
-- `domain` was UNIQUE as typed, so `VICTIM.test` could be registered by one
-- tenant while another owned `victim.test`. They are the same name to DNS, to
-- a certificate authority and to the Host-header match, so the second row let
-- its tenant answer verification challenges -- and later hold certificates --
-- for a name it had never proved anything about. New names are lowercased on
-- the way in; this index makes the database refuse a duplicate whatever writes
-- it.
--
-- Rows that already collide have to be resolved before the index can exist.
-- One row per name is kept, chosen in this order:
--
--   1. a verified row over an unverified one -- it is the one that proved
--      control;
--   2. among those, the earliest verified, then the earliest registered.
--
-- The others are copied whole, as JSON, into proxy_domain_case_conflicts and
-- then deleted, which also removes their routes, challenges and any
-- certificate linked to them. Copied rather than merely deleted so an operator
-- can see what was removed and from whom; on an installation that never hit
-- the bug the table stays empty.
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS proxy_domain_case_conflicts (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    removed_domain_id UUID NOT NULL,
    removed_tenant_id UUID,
    domain VARCHAR(253) NOT NULL,
    kept_domain_id UUID NOT NULL,
    removed_row JSONB NOT NULL,
    removed_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

WITH ranked AS (
    SELECT d.*,
           first_value(d.id) OVER w AS kept_id,
           row_number() OVER w AS rank_in_name
    FROM proxy_domains d
    WINDOW w AS (
        PARTITION BY lower(d.domain)
        ORDER BY (d.verification_status = 'verified') DESC,
                 d.verified_at ASC NULLS LAST,
                 d.created_at ASC NULLS LAST,
                 d.id ASC
    )
),
removed AS (
    INSERT INTO proxy_domain_case_conflicts
        (removed_domain_id, removed_tenant_id, domain, kept_domain_id, removed_row)
    SELECT r.id, r.tenant_id, r.domain, r.kept_id,
           to_jsonb(r) - 'kept_id' - 'rank_in_name'
    FROM ranked r
    WHERE r.rank_in_name > 1
    RETURNING removed_domain_id
)
DELETE FROM proxy_domains
WHERE id IN (SELECT removed_domain_id FROM removed);

CREATE UNIQUE INDEX IF NOT EXISTS idx_proxy_domains_domain_lower
    ON proxy_domains (lower(domain));

-- ---------------------------------------------------------------------------
-- 3. Per-tenant limits on what was unlimited.
--
-- max_domains and max_routes_per_domain were the only quotas, and the second
-- was never enforced. Upstreams, backends and API keys had no limit at all.
-- The defaults are generous; they exist to bound a runaway client, not to
-- shape a plan.
--
-- Checked first rather than left to ADD COLUMN IF NOT EXISTS, which takes an
-- ACCESS EXCLUSIVE lock on the table even when there is nothing to add. On a
-- re-run against a live database that lock, held to the end of the
-- transaction, deadlocks with any request that has locked a tenant row and
-- then touches another table.
-- ---------------------------------------------------------------------------
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_schema = current_schema()
          AND table_name = 'proxy_tenants'
          AND column_name = 'max_api_keys'
    ) THEN
        ALTER TABLE proxy_tenants
            ADD COLUMN IF NOT EXISTS max_upstreams INTEGER NOT NULL DEFAULT 20,
            ADD COLUMN IF NOT EXISTS max_backends_per_upstream INTEGER NOT NULL DEFAULT 20,
            ADD COLUMN IF NOT EXISTS max_api_keys INTEGER NOT NULL DEFAULT 20;
    END IF;
END $$;

-- ---------------------------------------------------------------------------
-- 4. ACME certificates belong to their domain.
--
-- The issuance worker stored certificates with domain_id NULL, so deleting a
-- domain left its certificate -- and its private key -- in proxy_certificates,
-- where the TLS listener went on serving it. The worker now links the row when
-- it records success; this links the ones issued before that, for domains
-- whose issuance finished. Only those: an unlinked row for a name whose
-- issuance never completed may be the operator's own certificate, and linking
-- it would let the tenant delete it.
-- ---------------------------------------------------------------------------
UPDATE proxy_certificates c
SET domain_id = d.id,
    tenant_id = d.tenant_id,
    updated_at = NOW()
FROM proxy_domains d
WHERE c.domain_id IS NULL
  AND c.domain = d.domain
  AND d.verification_status = 'verified'
  AND d.ssl_provider = 'acme'
  AND d.ssl_status = 'active';
