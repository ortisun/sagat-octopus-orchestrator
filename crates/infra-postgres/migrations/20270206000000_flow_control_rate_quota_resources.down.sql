-- Undoes 20270206000000_flow_control_rate_quota_resources.up.sql. Rollback is an operator/quiesced-service operation. A
-- limit or a ceiling set for a rate-limit resource is what an administrator or an operator decided: rolling back never
-- destroys it, and refuses while one exists (an older build would not read it).
-- The lists restored below are those in force before this migration on the tree merged with feature 041 (decision
-- D022-089): 20270130000300's names and the step-log names of 20270201000000, stamped between the two.
LOCK TABLE quota_limits, quota_ceilings IN ACCESS EXCLUSIVE MODE;
DO $guard$
BEGIN
    IF EXISTS (SELECT 1 FROM quota_limits WHERE resource IN ('rate_limit_live_keys', 'rate_limit_per_second', 'rate_limit_per_minute', 'rate_limit_per_hour'))
        OR EXISTS (SELECT 1 FROM quota_ceilings WHERE resource IN ('rate_limit_live_keys', 'rate_limit_per_second', 'rate_limit_per_minute', 'rate_limit_per_hour')) THEN
        RAISE EXCEPTION 'limits or ceilings of rate-limit resources are retained; keep a compatible reader'
            USING ERRCODE = 'object_not_in_prerequisite_state';
    END IF;
END $guard$;

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
     'secret_provider_operations', 'schedules', 'schedule_backfills',
     'batch_jobs', 'tenant_batch_jobs', 'pending_resets',
     'queued_runs', 'concurrency_live_keys', 'flow_control_key_waiters', 'concurrency_limit', 'flow_control_queue_timeout_ms', 'flow_control_node_declarations', 'tenant_step_log_bytes', 'step_log_attempt_records', 'step_log_attempt_bytes', 'step_log_retention_days'));
ALTER TABLE quota_limits DROP CONSTRAINT quota_limits_resource_valid;
ALTER TABLE quota_limits ADD CONSTRAINT quota_limits_resource_valid CHECK (resource IN ('active_runs', 'tenant_active_runs', 'workflows', 'worker_sessions', 'secrets', 'event_endpoints', 'webhook_subscriptions', 'broker_subscriptions', 'event_connectors', 'scripts', 'script_jobs', 'activity_concurrency', 'tenant_content_bytes', 'tenant_log_bytes', 'api_requests_per_second', 'events_per_second', 'event_retention_seconds', 'script_retention_days', 'database_statement_bytes', 'database_parameters', 'database_parameter_bytes', 'database_rows', 'database_result_bytes', 'database_timeout_ms', 'database_connect_timeout_ms', 'database_pool_size', 'database_idle_timeout_ms', 'database_connections', 'database_concurrency', 'tenant_database_concurrency', 'secret_provider_operations', 'schedules', 'schedule_backfills', 'batch_jobs', 'tenant_batch_jobs', 'pending_resets', 'queued_runs', 'concurrency_live_keys', 'flow_control_key_waiters', 'concurrency_limit', 'flow_control_queue_timeout_ms', 'flow_control_node_declarations', 'tenant_step_log_bytes', 'step_log_attempt_records', 'step_log_attempt_bytes', 'step_log_retention_days'));
ALTER TABLE quota_limits DROP CONSTRAINT quota_limits_tenant_scoped;
ALTER TABLE quota_limits ADD CONSTRAINT quota_limits_tenant_scoped CHECK (namespace_id IS NULL OR resource IN ('active_runs', 'workflows', 'secrets', 'event_endpoints', 'webhook_subscriptions', 'broker_subscriptions', 'event_connectors', 'scripts', 'script_jobs', 'activity_concurrency', 'api_requests_per_second', 'events_per_second', 'event_retention_seconds', 'script_retention_days', 'database_statement_bytes', 'database_parameters', 'database_parameter_bytes', 'database_rows', 'database_result_bytes', 'database_timeout_ms', 'database_connect_timeout_ms', 'database_pool_size', 'database_idle_timeout_ms', 'database_connections', 'database_concurrency', 'secret_provider_operations', 'schedules', 'schedule_backfills', 'batch_jobs', 'pending_resets', 'queued_runs', 'concurrency_live_keys', 'flow_control_key_waiters', 'concurrency_limit', 'flow_control_queue_timeout_ms', 'flow_control_node_declarations', 'step_log_attempt_records', 'step_log_attempt_bytes', 'step_log_retention_days'));
