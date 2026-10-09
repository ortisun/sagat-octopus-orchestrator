-- Feature 009: quotas (FR-009-006). A quota is a hard limit judged against an exact count, taken in
-- the transaction that creates the thing counted (under an advisory lock on the counted scope), so it
-- never depends on a counter that is allowed to be late.

CREATE TABLE quota_limits (
    tenant_id    uuid        NOT NULL REFERENCES tenants (id),
    -- NULL: the setting of the whole tenant (the default of its namespaces).
    namespace_id uuid,
    resource     text        NOT NULL,
    max_value    bigint      NOT NULL,
    updated_by   text        NOT NULL,
    updated_at   timestamptz NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT quota_limits_namespace_fk FOREIGN KEY (tenant_id, namespace_id)
        REFERENCES namespaces (tenant_id, id),
    CONSTRAINT quota_limits_resource_valid CHECK (resource IN
        ('active_runs', 'tenant_active_runs', 'workflows', 'worker_sessions')),
    CONSTRAINT quota_limits_value_range CHECK (max_value BETWEEN 0 AND 1000000000),
    -- A resource counted across the tenant has no namespace level.
    CONSTRAINT quota_limits_tenant_scoped CHECK (
        namespace_id IS NULL OR resource IN ('active_runs', 'workflows'))
);

CREATE UNIQUE INDEX quota_limits_scope_uniq ON quota_limits
    (tenant_id, COALESCE(namespace_id, '00000000-0000-0000-0000-000000000000'::uuid), resource);

-- Counting the runs that have not finished is on the path of every start.
CREATE INDEX runs_active_idx ON runs (tenant_id, namespace_id) WHERE finished_at IS NULL;

DO $grants$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        GRANT SELECT, INSERT, UPDATE, DELETE ON quota_limits TO sagat_app;
    END IF;
END
$grants$;
