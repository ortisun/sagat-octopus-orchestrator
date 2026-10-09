DO $revoke$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        REVOKE ALL ON secret_access, secret_versions, secrets FROM sagat_app;
    END IF;
END
$revoke$;

DROP TABLE secret_access;
DROP TABLE secret_versions;
DROP TABLE secrets;

DELETE FROM quota_limits WHERE resource = 'secrets';
ALTER TABLE quota_limits DROP CONSTRAINT quota_limits_tenant_scoped;
ALTER TABLE quota_limits ADD CONSTRAINT quota_limits_tenant_scoped CHECK (
    namespace_id IS NULL OR resource IN ('active_runs', 'workflows'));
ALTER TABLE quota_limits DROP CONSTRAINT quota_limits_resource_valid;
ALTER TABLE quota_limits ADD CONSTRAINT quota_limits_resource_valid CHECK (resource IN
    ('active_runs', 'tenant_active_runs', 'workflows', 'worker_sessions'));
