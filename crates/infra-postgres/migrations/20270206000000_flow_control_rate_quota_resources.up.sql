-- Feature 022, release 0.3.0 slice (T022-077; FR-022-034, FR-022-047; D022-064): the quota resources of the rate gate.
-- Counted in the namespace: the live rate-limit keys (window rows of either scope; 10,000 by default, 100,000 at most).
-- Policies of the namespace: the most grants one key counts in a second (20, the deployment's ceiling until the benchmark
-- of SC-022-029 meets its floor), a minute (6,000) and an hour (10,000). The run waiters of a key, the queued runs and the
-- longest wait are the concurrency gate's resources, shared (20270130000300). The values are in code (`QuotaResource`);
-- the tables only name what they may hold.
--
-- Additive for the schema; an older replica's quota views refuse a stored name they do not know (G1192, as for every
-- resource added before these).
--
-- REBASE HAZARD: the three CHECKs below restate the whole list of resources of the ceilings tables (written after
-- 20270130000300_flow_control_quota_resources, the last one that restated them). Added NOT VALID; validated by
-- 20270206000100. Feature 041's 20270201000000_step_log_quotas (another branch) restates the same lists: whichever merges
-- second restates them with both sets of names.
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
     'queued_runs', 'concurrency_live_keys', 'flow_control_key_waiters', 'concurrency_limit', 'flow_control_queue_timeout_ms', 'flow_control_node_declarations', 'rate_limit_live_keys', 'rate_limit_per_second', 'rate_limit_per_minute', 'rate_limit_per_hour')) NOT VALID;
ALTER TABLE quota_limits DROP CONSTRAINT quota_limits_resource_valid;
ALTER TABLE quota_limits ADD CONSTRAINT quota_limits_resource_valid CHECK (resource IN ('active_runs', 'tenant_active_runs', 'workflows', 'worker_sessions', 'secrets', 'event_endpoints', 'webhook_subscriptions', 'broker_subscriptions', 'event_connectors', 'scripts', 'script_jobs', 'activity_concurrency', 'tenant_content_bytes', 'tenant_log_bytes', 'api_requests_per_second', 'events_per_second', 'event_retention_seconds', 'script_retention_days', 'database_statement_bytes', 'database_parameters', 'database_parameter_bytes', 'database_rows', 'database_result_bytes', 'database_timeout_ms', 'database_connect_timeout_ms', 'database_pool_size', 'database_idle_timeout_ms', 'database_connections', 'database_concurrency', 'tenant_database_concurrency', 'secret_provider_operations', 'schedules', 'schedule_backfills', 'batch_jobs', 'tenant_batch_jobs', 'pending_resets', 'queued_runs', 'concurrency_live_keys', 'flow_control_key_waiters', 'concurrency_limit', 'flow_control_queue_timeout_ms', 'flow_control_node_declarations', 'rate_limit_live_keys', 'rate_limit_per_second', 'rate_limit_per_minute', 'rate_limit_per_hour')) NOT VALID;
ALTER TABLE quota_limits DROP CONSTRAINT quota_limits_tenant_scoped;
ALTER TABLE quota_limits ADD CONSTRAINT quota_limits_tenant_scoped CHECK (namespace_id IS NULL OR resource IN ('active_runs', 'workflows', 'secrets', 'event_endpoints', 'webhook_subscriptions', 'broker_subscriptions', 'event_connectors', 'scripts', 'script_jobs', 'activity_concurrency', 'api_requests_per_second', 'events_per_second', 'event_retention_seconds', 'script_retention_days', 'database_statement_bytes', 'database_parameters', 'database_parameter_bytes', 'database_rows', 'database_result_bytes', 'database_timeout_ms', 'database_connect_timeout_ms', 'database_pool_size', 'database_idle_timeout_ms', 'database_connections', 'database_concurrency', 'secret_provider_operations', 'schedules', 'schedule_backfills', 'batch_jobs', 'pending_resets', 'queued_runs', 'concurrency_live_keys', 'flow_control_key_waiters', 'concurrency_limit', 'flow_control_queue_timeout_ms', 'flow_control_node_declarations', 'rate_limit_live_keys', 'rate_limit_per_second', 'rate_limit_per_minute', 'rate_limit_per_hour')) NOT VALID;
