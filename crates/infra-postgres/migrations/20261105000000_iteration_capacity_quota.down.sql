DO $guard$
BEGIN
    IF EXISTS (
        SELECT 1 FROM quota_ceilings WHERE resource = 'iteration_concurrency'
    ) THEN
        RAISE EXCEPTION 'cannot remove iteration capacity governance while a ceiling is configured';
    END IF;
END
$guard$;

DO $grants$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        REVOKE INSERT, UPDATE, DELETE ON tenant_collection_policies FROM sagat_app;
    END IF;
END
$grants$;

ALTER TABLE quota_ceilings DROP CONSTRAINT quota_ceilings_resource_valid;
ALTER TABLE quota_ceilings ADD CONSTRAINT quota_ceilings_resource_valid CHECK (resource IN
    ('active_runs', 'tenant_active_runs', 'workflows', 'worker_sessions', 'secrets',
     'event_endpoints', 'webhook_subscriptions', 'broker_subscriptions', 'event_connectors',
     'scripts', 'script_jobs', 'activity_concurrency',
     'tenant_content_bytes', 'tenant_log_bytes', 'api_requests_per_second',
     'events_per_second', 'event_retention_seconds', 'script_retention_days'));
