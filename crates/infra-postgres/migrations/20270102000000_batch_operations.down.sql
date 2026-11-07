-- Rollback is an operator/quiesced-service operation. A batch job and its items are the evidence of
-- what was done to which runs, by whom and why: rolling back never destroys them, and it never drops
-- the quota resources while a limit or a ceiling for them is set.
LOCK TABLE batch_jobs, batch_items IN ACCESS EXCLUSIVE MODE;
DO $guard$
BEGIN
    IF EXISTS (SELECT 1 FROM batch_jobs) OR EXISTS (SELECT 1 FROM batch_items) THEN
        RAISE EXCEPTION 'batch jobs and their items are retained; keep a compatible reader'
            USING ERRCODE = 'object_not_in_prerequisite_state';
    END IF;
    IF EXISTS (SELECT 1 FROM quota_limits WHERE resource IN ('batch_jobs', 'tenant_batch_jobs'))
        OR EXISTS (SELECT 1 FROM quota_ceilings WHERE resource IN ('batch_jobs', 'tenant_batch_jobs')) THEN
        RAISE EXCEPTION 'a limit or a ceiling for batch jobs is set; clear it before the rollback'
            USING ERRCODE = 'object_not_in_prerequisite_state';
    END IF;
END $guard$;

DROP TABLE batch_items;
DROP FUNCTION batch_items_guard();
DROP TABLE batch_jobs;
DROP FUNCTION batch_jobs_guard();

ALTER TABLE quota_ceilings DROP CONSTRAINT quota_ceilings_resource_valid;
ALTER TABLE quota_ceilings ADD CONSTRAINT quota_ceilings_resource_valid CHECK (resource IN
    ('active_runs', 'tenant_active_runs', 'workflows', 'worker_sessions', 'secrets',
     'event_endpoints', 'webhook_subscriptions', 'broker_subscriptions', 'event_connectors',
     'scripts', 'script_jobs', 'activity_concurrency', 'iteration_concurrency',
     'tenant_content_bytes', 'tenant_log_bytes', 'api_requests_per_second',
     'events_per_second', 'event_retention_seconds', 'script_retention_days',
     'database_statement_bytes', 'database_parameters', 'database_parameter_bytes',
     'database_rows', 'database_result_bytes', 'database_timeout_ms',
     'database_connect_timeout_ms', 'database_pool_size', 'database_idle_timeout_ms',
     'database_connections', 'database_concurrency', 'tenant_database_concurrency',
     'secret_provider_operations', 'schedules', 'schedule_backfills'));
ALTER TABLE quota_limits DROP CONSTRAINT quota_limits_resource_valid;
ALTER TABLE quota_limits ADD CONSTRAINT quota_limits_resource_valid CHECK (resource IN ('active_runs', 'tenant_active_runs', 'workflows', 'worker_sessions', 'secrets', 'event_endpoints', 'webhook_subscriptions', 'broker_subscriptions', 'event_connectors', 'scripts', 'script_jobs', 'activity_concurrency', 'tenant_content_bytes', 'tenant_log_bytes', 'api_requests_per_second', 'events_per_second', 'event_retention_seconds', 'script_retention_days', 'database_statement_bytes', 'database_parameters', 'database_parameter_bytes', 'database_rows', 'database_result_bytes', 'database_timeout_ms', 'database_connect_timeout_ms', 'database_pool_size', 'database_idle_timeout_ms', 'database_connections', 'database_concurrency', 'tenant_database_concurrency', 'secret_provider_operations', 'schedules', 'schedule_backfills'));
ALTER TABLE quota_limits DROP CONSTRAINT quota_limits_tenant_scoped;
ALTER TABLE quota_limits ADD CONSTRAINT quota_limits_tenant_scoped CHECK (namespace_id IS NULL OR resource IN ('active_runs', 'workflows', 'secrets', 'event_endpoints', 'webhook_subscriptions', 'broker_subscriptions', 'event_connectors', 'scripts', 'script_jobs', 'activity_concurrency', 'api_requests_per_second', 'events_per_second', 'event_retention_seconds', 'script_retention_days', 'database_statement_bytes', 'database_parameters', 'database_parameter_bytes', 'database_rows', 'database_result_bytes', 'database_timeout_ms', 'database_connect_timeout_ms', 'database_pool_size', 'database_idle_timeout_ms', 'database_connections', 'database_concurrency', 'secret_provider_operations', 'schedules', 'schedule_backfills'));
