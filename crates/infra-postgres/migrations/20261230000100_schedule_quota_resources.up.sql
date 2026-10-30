-- Feature 018: the quota resources of schedules. The ceilings table names every resource it may hold,
-- so the list is widened; the limits themselves default in code (200 schedules and 4 concurrent
-- backfills per namespace). Batch jobs and pending resets get theirs with their tables.
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
-- The per-namespace limits table has its own list, and only namespace-scoped resources may be set
-- for a single namespace. The value ranges of both tables already bound a count resource.
ALTER TABLE quota_limits DROP CONSTRAINT quota_limits_resource_valid;
ALTER TABLE quota_limits ADD CONSTRAINT quota_limits_resource_valid CHECK (resource IN ('active_runs', 'tenant_active_runs', 'workflows', 'worker_sessions', 'secrets', 'event_endpoints', 'webhook_subscriptions', 'broker_subscriptions', 'event_connectors', 'scripts', 'script_jobs', 'activity_concurrency', 'tenant_content_bytes', 'tenant_log_bytes', 'api_requests_per_second', 'events_per_second', 'event_retention_seconds', 'script_retention_days', 'database_statement_bytes', 'database_parameters', 'database_parameter_bytes', 'database_rows', 'database_result_bytes', 'database_timeout_ms', 'database_connect_timeout_ms', 'database_pool_size', 'database_idle_timeout_ms', 'database_connections', 'database_concurrency', 'tenant_database_concurrency', 'secret_provider_operations', 'schedules', 'schedule_backfills'));
ALTER TABLE quota_limits DROP CONSTRAINT quota_limits_tenant_scoped;
ALTER TABLE quota_limits ADD CONSTRAINT quota_limits_tenant_scoped CHECK (namespace_id IS NULL OR resource IN ('active_runs', 'workflows', 'secrets', 'event_endpoints', 'webhook_subscriptions', 'broker_subscriptions', 'event_connectors', 'scripts', 'script_jobs', 'activity_concurrency', 'api_requests_per_second', 'events_per_second', 'event_retention_seconds', 'script_retention_days', 'database_statement_bytes', 'database_parameters', 'database_parameter_bytes', 'database_rows', 'database_result_bytes', 'database_timeout_ms', 'database_connect_timeout_ms', 'database_pool_size', 'database_idle_timeout_ms', 'database_connections', 'database_concurrency', 'secret_provider_operations', 'schedules', 'schedule_backfills'));
