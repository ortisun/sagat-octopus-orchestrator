DO $revoke$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        REVOKE ALL ON scripts, script_versions, script_artifacts, script_build_jobs,
            script_test_runs, script_executions FROM sagat_app;
    END IF;
END
$revoke$;

DELETE FROM quota_limits WHERE resource IN ('scripts', 'script_jobs');
DELETE FROM quota_ceilings WHERE resource IN ('scripts', 'script_jobs');
ALTER TABLE quota_ceilings DROP CONSTRAINT quota_ceilings_resource_valid;
ALTER TABLE quota_ceilings ADD CONSTRAINT quota_ceilings_resource_valid CHECK (resource IN
    ('active_runs', 'tenant_active_runs', 'workflows', 'worker_sessions', 'secrets',
     'event_endpoints', 'webhook_subscriptions', 'event_connectors'));
ALTER TABLE quota_limits DROP CONSTRAINT quota_limits_tenant_scoped;
ALTER TABLE quota_limits ADD CONSTRAINT quota_limits_tenant_scoped CHECK (
    namespace_id IS NULL OR resource IN ('active_runs', 'workflows', 'secrets', 'event_endpoints',
                                         'webhook_subscriptions', 'event_connectors'));
ALTER TABLE quota_limits DROP CONSTRAINT quota_limits_resource_valid;
ALTER TABLE quota_limits ADD CONSTRAINT quota_limits_resource_valid CHECK (resource IN
    ('active_runs', 'tenant_active_runs', 'workflows', 'worker_sessions', 'secrets',
     'event_endpoints', 'webhook_subscriptions', 'event_connectors'));

DROP TABLE script_executions;
DROP TABLE script_test_runs;
DROP TABLE script_build_jobs;
-- The guards refuse the delete of a published version: the rollback removes them first.
DROP TRIGGER script_versions_immutable ON script_versions;
DROP TRIGGER script_versions_no_truncate ON script_versions;
DROP TABLE script_versions;
DROP FUNCTION script_versions_guard();
DROP TRIGGER script_artifacts_immutable ON script_artifacts;
DROP TRIGGER script_artifacts_no_truncate ON script_artifacts;
DROP TABLE script_artifacts;
DROP FUNCTION script_artifacts_reject_mutation();
DROP TABLE scripts;

DELETE FROM memberships WHERE role = 'runner';
ALTER TABLE memberships DROP CONSTRAINT memberships_role_valid;
ALTER TABLE memberships
    ADD CONSTRAINT memberships_role_valid
    CHECK (role IN ('admin', 'developer', 'operator', 'viewer', 'worker'));
