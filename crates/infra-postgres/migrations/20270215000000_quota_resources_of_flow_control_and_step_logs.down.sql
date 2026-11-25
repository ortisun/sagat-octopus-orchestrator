-- Undoes 20270215000000_quota_resources_of_flow_control_and_step_logs.up.sql: restores the lists of 20270206000000,
-- which name feature 022's resources and not feature 041's (the state of a tree stamped up to 20270214000000). Rollback
-- is an operator/quiesced-service operation. A limit or a ceiling of a step-log resource is what a tenant or an operator
-- decided: rolling back never destroys it, and refuses while one exists (the restored lists would not accept it).
LOCK TABLE quota_limits, quota_ceilings IN ACCESS EXCLUSIVE MODE;
DO $guard$
BEGIN
    IF EXISTS (SELECT 1 FROM quota_limits WHERE resource IN ('tenant_step_log_bytes', 'step_log_attempt_records', 'step_log_attempt_bytes', 'step_log_retention_days'))
        OR EXISTS (SELECT 1 FROM quota_ceilings WHERE resource IN ('tenant_step_log_bytes', 'step_log_attempt_records', 'step_log_attempt_bytes', 'step_log_retention_days')) THEN
        RAISE EXCEPTION 'limits or ceilings of step-log resources are retained; keep a compatible reader'
            USING ERRCODE = 'object_not_in_prerequisite_state';
    END IF;
END $guard$;

ALTER TABLE quota_ceilings DROP CONSTRAINT quota_ceilings_resource_valid;
ALTER TABLE quota_ceilings ADD CONSTRAINT quota_ceilings_resource_valid CHECK (resource IN
    ('active_runs', 'tenant_active_runs', 'workflows', 'worker_sessions', 'secrets', 'event_endpoints',
     'webhook_subscriptions', 'broker_subscriptions', 'event_connectors', 'scripts', 'script_jobs',
     'activity_concurrency', 'iteration_concurrency', 'tenant_content_bytes', 'tenant_log_bytes',
     'api_requests_per_second', 'events_per_second', 'event_retention_seconds', 'script_retention_days',
     'database_statement_bytes', 'database_parameters', 'database_parameter_bytes', 'database_rows',
     'database_result_bytes', 'database_timeout_ms', 'database_connect_timeout_ms', 'database_pool_size',
     'database_idle_timeout_ms', 'database_connections', 'database_concurrency', 'tenant_database_concurrency',
     'secret_provider_operations', 'schedules', 'schedule_backfills', 'batch_jobs', 'tenant_batch_jobs',
     'pending_resets', 'queued_runs', 'concurrency_live_keys', 'flow_control_key_waiters', 'concurrency_limit',
     'flow_control_queue_timeout_ms', 'flow_control_node_declarations', 'rate_limit_live_keys',
     'rate_limit_per_second', 'rate_limit_per_minute', 'rate_limit_per_hour'));
ALTER TABLE quota_limits DROP CONSTRAINT quota_limits_resource_valid;
ALTER TABLE quota_limits ADD CONSTRAINT quota_limits_resource_valid CHECK (resource IN
    ('active_runs', 'tenant_active_runs', 'workflows', 'worker_sessions', 'secrets', 'event_endpoints',
     'webhook_subscriptions', 'broker_subscriptions', 'event_connectors', 'scripts', 'script_jobs',
     'activity_concurrency', 'tenant_content_bytes', 'tenant_log_bytes', 'api_requests_per_second',
     'events_per_second', 'event_retention_seconds', 'script_retention_days', 'database_statement_bytes',
     'database_parameters', 'database_parameter_bytes', 'database_rows', 'database_result_bytes',
     'database_timeout_ms', 'database_connect_timeout_ms', 'database_pool_size', 'database_idle_timeout_ms',
     'database_connections', 'database_concurrency', 'tenant_database_concurrency', 'secret_provider_operations',
     'schedules', 'schedule_backfills', 'batch_jobs', 'tenant_batch_jobs', 'pending_resets', 'queued_runs',
     'concurrency_live_keys', 'flow_control_key_waiters', 'concurrency_limit', 'flow_control_queue_timeout_ms',
     'flow_control_node_declarations', 'rate_limit_live_keys', 'rate_limit_per_second', 'rate_limit_per_minute',
     'rate_limit_per_hour'));
ALTER TABLE quota_limits DROP CONSTRAINT quota_limits_tenant_scoped;
ALTER TABLE quota_limits ADD CONSTRAINT quota_limits_tenant_scoped CHECK (namespace_id IS NULL OR resource IN
    ('active_runs', 'workflows', 'secrets', 'event_endpoints', 'webhook_subscriptions', 'broker_subscriptions',
     'event_connectors', 'scripts', 'script_jobs', 'activity_concurrency', 'api_requests_per_second',
     'events_per_second', 'event_retention_seconds', 'script_retention_days', 'database_statement_bytes',
     'database_parameters', 'database_parameter_bytes', 'database_rows', 'database_result_bytes',
     'database_timeout_ms', 'database_connect_timeout_ms', 'database_pool_size', 'database_idle_timeout_ms',
     'database_connections', 'database_concurrency', 'secret_provider_operations', 'schedules', 'schedule_backfills',
     'batch_jobs', 'pending_resets', 'queued_runs', 'concurrency_live_keys', 'flow_control_key_waiters',
     'concurrency_limit', 'flow_control_queue_timeout_ms', 'flow_control_node_declarations', 'rate_limit_live_keys',
     'rate_limit_per_second', 'rate_limit_per_minute', 'rate_limit_per_hour'));
