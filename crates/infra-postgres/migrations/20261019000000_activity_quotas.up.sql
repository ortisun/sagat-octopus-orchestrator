-- Feature 010 FR-010-002: exact per-activity concurrency quotas.
-- A missing override preserves the generic namespace/tenant activity_concurrency limit, whose
-- platform default is effectively unlimited. Claim transactions serialize only on one
-- tenant/namespace/activity and count unexpired held leases before changing READY to LEASED.

ALTER TABLE quota_limits DROP CONSTRAINT quota_limits_resource_valid;
ALTER TABLE quota_limits ADD CONSTRAINT quota_limits_resource_valid CHECK (resource IN
    ('active_runs', 'tenant_active_runs', 'workflows', 'worker_sessions', 'secrets',
     'event_endpoints', 'webhook_subscriptions', 'event_connectors', 'scripts', 'script_jobs',
     'activity_concurrency'));
ALTER TABLE quota_limits DROP CONSTRAINT quota_limits_tenant_scoped;
ALTER TABLE quota_limits ADD CONSTRAINT quota_limits_tenant_scoped CHECK (
    namespace_id IS NULL OR resource IN
        ('active_runs', 'workflows', 'secrets', 'event_endpoints', 'webhook_subscriptions',
         'event_connectors', 'scripts', 'script_jobs', 'activity_concurrency'));

ALTER TABLE quota_ceilings DROP CONSTRAINT quota_ceilings_resource_valid;
ALTER TABLE quota_ceilings ADD CONSTRAINT quota_ceilings_resource_valid CHECK (resource IN
    ('active_runs', 'tenant_active_runs', 'workflows', 'worker_sessions', 'secrets',
     'event_endpoints', 'webhook_subscriptions', 'event_connectors', 'scripts', 'script_jobs',
     'activity_concurrency'));

CREATE TABLE activity_quota_limits (
    tenant_id    uuid        NOT NULL,
    namespace_id uuid        NOT NULL,
    activity     text        NOT NULL,
    max_inflight bigint      NOT NULL,
    updated_by   text        NOT NULL,
    updated_at   timestamptz NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (tenant_id, namespace_id, activity),
    CONSTRAINT activity_quota_limits_namespace_fk FOREIGN KEY (tenant_id, namespace_id)
        REFERENCES namespaces (tenant_id, id) ON DELETE CASCADE,
    CONSTRAINT activity_quota_limits_name CHECK
        (length(activity) BETWEEN 1 AND 128 AND activity ~ '^[A-Za-z0-9][A-Za-z0-9_.:-]*$'),
    CONSTRAINT activity_quota_limits_value CHECK (max_inflight BETWEEN 0 AND 1000000000)
);

DO $grants$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        GRANT SELECT, INSERT, UPDATE, DELETE ON activity_quota_limits TO sagat_app;
    END IF;
END
$grants$;
