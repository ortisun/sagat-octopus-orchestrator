-- Feature 009 (security review): who may raise a quota. A tenant's administrator sets the limits of
-- the tenant's own namespaces, but only up to a ceiling; without it the quota would be controlled by
-- the party it is meant to constrain. No row means the platform default for that resource
-- (`sagat_domain::quota`). Only an operator writes here (`control-plane quota-ceiling`); the API has
-- no way to.

CREATE TABLE quota_ceilings (
    tenant_id  uuid        NOT NULL REFERENCES tenants (id),
    resource   text        NOT NULL,
    max_value  bigint      NOT NULL,
    granted_by text        NOT NULL,
    granted_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (tenant_id, resource),
    CONSTRAINT quota_ceilings_resource_valid CHECK (resource IN
        ('active_runs', 'tenant_active_runs', 'workflows', 'worker_sessions', 'secrets')),
    CONSTRAINT quota_ceilings_value_range CHECK (max_value BETWEEN 0 AND 1000000000)
);

DO $grants$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        GRANT SELECT, INSERT, UPDATE, DELETE ON quota_ceilings TO sagat_app;
    END IF;
END
$grants$;
