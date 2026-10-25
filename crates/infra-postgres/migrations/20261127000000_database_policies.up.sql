-- FR-017-021: operator grants and namespace narrowing use existing quota governance.

ALTER TABLE quota_limits DROP CONSTRAINT quota_limits_resource_valid;
ALTER TABLE quota_limits ADD CONSTRAINT quota_limits_resource_valid CHECK (resource IN
    ('active_runs', 'tenant_active_runs', 'workflows', 'worker_sessions', 'secrets',
     'event_endpoints', 'webhook_subscriptions', 'broker_subscriptions', 'event_connectors',
     'scripts', 'script_jobs', 'activity_concurrency',
     'tenant_content_bytes', 'tenant_log_bytes', 'api_requests_per_second',
     'events_per_second', 'event_retention_seconds', 'script_retention_days', 'database_statement_bytes', 'database_parameters', 'database_parameter_bytes', 'database_rows', 'database_result_bytes', 'database_timeout_ms', 'database_connect_timeout_ms', 'database_pool_size', 'database_idle_timeout_ms', 'database_connections', 'database_concurrency', 'tenant_database_concurrency', 'secret_provider_operations'));
ALTER TABLE quota_limits DROP CONSTRAINT quota_limits_tenant_scoped;
ALTER TABLE quota_limits ADD CONSTRAINT quota_limits_tenant_scoped CHECK (
    namespace_id IS NULL OR resource IN
        ('active_runs', 'workflows', 'secrets', 'event_endpoints', 'webhook_subscriptions',
         'broker_subscriptions', 'event_connectors', 'scripts', 'script_jobs',
         'activity_concurrency', 'api_requests_per_second', 'events_per_second',
         'event_retention_seconds', 'script_retention_days', 'database_statement_bytes', 'database_parameters', 'database_parameter_bytes', 'database_rows', 'database_result_bytes', 'database_timeout_ms', 'database_connect_timeout_ms', 'database_pool_size', 'database_idle_timeout_ms', 'database_connections', 'database_concurrency', 'secret_provider_operations'));
ALTER TABLE quota_limits DROP CONSTRAINT quota_limits_value_range;
ALTER TABLE quota_limits ADD CONSTRAINT quota_limits_value_range CHECK (
    CASE resource
      WHEN 'tenant_content_bytes' THEN max_value BETWEEN 0 AND 1125899906842624
      WHEN 'tenant_log_bytes' THEN max_value BETWEEN 0 AND 1125899906842624
      WHEN 'api_requests_per_second' THEN max_value BETWEEN 0 AND 1000000
      WHEN 'events_per_second' THEN max_value BETWEEN 0 AND 1000000
      WHEN 'event_retention_seconds' THEN max_value = 0 OR max_value BETWEEN 60 AND 2592000
      WHEN 'script_retention_days' THEN max_value BETWEEN 0 AND 365
      WHEN 'database_statement_bytes' THEN max_value BETWEEN 1 AND 65536
      WHEN 'database_parameters' THEN max_value BETWEEN 1 AND 256
      WHEN 'database_parameter_bytes' THEN max_value BETWEEN 1 AND 262144
      WHEN 'database_rows' THEN max_value BETWEEN 1 AND 10000
      WHEN 'database_result_bytes' THEN max_value BETWEEN 1 AND 4194304
      WHEN 'database_timeout_ms' THEN max_value BETWEEN 1 AND 30000
      WHEN 'database_connect_timeout_ms' THEN max_value BETWEEN 1 AND 10000
      WHEN 'database_pool_size' THEN max_value BETWEEN 1 AND 16
      WHEN 'database_idle_timeout_ms' THEN max_value BETWEEN 1 AND 60000
      WHEN 'database_connections' THEN max_value BETWEEN 0 AND 1000
      WHEN 'database_concurrency' THEN max_value BETWEEN 0 AND 64
      WHEN 'tenant_database_concurrency' THEN max_value BETWEEN 0 AND 256
      WHEN 'secret_provider_operations' THEN max_value BETWEEN 0 AND 128
      ELSE max_value BETWEEN 0 AND 1000000000
    END);

ALTER TABLE quota_ceilings DROP CONSTRAINT quota_ceilings_resource_valid;
ALTER TABLE quota_ceilings ADD CONSTRAINT quota_ceilings_resource_valid CHECK (resource IN
    ('active_runs', 'tenant_active_runs', 'workflows', 'worker_sessions', 'secrets',
     'event_endpoints', 'webhook_subscriptions', 'broker_subscriptions', 'event_connectors',
     'scripts', 'script_jobs', 'activity_concurrency', 'iteration_concurrency',
     'tenant_content_bytes', 'tenant_log_bytes', 'api_requests_per_second',
     'events_per_second', 'event_retention_seconds', 'script_retention_days', 'database_statement_bytes', 'database_parameters', 'database_parameter_bytes', 'database_rows', 'database_result_bytes', 'database_timeout_ms', 'database_connect_timeout_ms', 'database_pool_size', 'database_idle_timeout_ms', 'database_connections', 'database_concurrency', 'tenant_database_concurrency', 'secret_provider_operations'));
ALTER TABLE quota_ceilings DROP CONSTRAINT quota_ceilings_value_range;
ALTER TABLE quota_ceilings ADD CONSTRAINT quota_ceilings_value_range CHECK (
    CASE resource
      WHEN 'tenant_content_bytes' THEN max_value BETWEEN 0 AND 1125899906842624
      WHEN 'tenant_log_bytes' THEN max_value BETWEEN 0 AND 1125899906842624
      WHEN 'api_requests_per_second' THEN max_value BETWEEN 0 AND 1000000
      WHEN 'events_per_second' THEN max_value BETWEEN 0 AND 1000000
      WHEN 'event_retention_seconds' THEN max_value = 0 OR max_value BETWEEN 60 AND 2592000
      WHEN 'script_retention_days' THEN max_value BETWEEN 0 AND 365
      WHEN 'database_statement_bytes' THEN max_value BETWEEN 1 AND 65536
      WHEN 'database_parameters' THEN max_value BETWEEN 1 AND 256
      WHEN 'database_parameter_bytes' THEN max_value BETWEEN 1 AND 262144
      WHEN 'database_rows' THEN max_value BETWEEN 1 AND 10000
      WHEN 'database_result_bytes' THEN max_value BETWEEN 1 AND 4194304
      WHEN 'database_timeout_ms' THEN max_value BETWEEN 1 AND 30000
      WHEN 'database_connect_timeout_ms' THEN max_value BETWEEN 1 AND 10000
      WHEN 'database_pool_size' THEN max_value BETWEEN 1 AND 16
      WHEN 'database_idle_timeout_ms' THEN max_value BETWEEN 1 AND 60000
      WHEN 'database_connections' THEN max_value BETWEEN 0 AND 1000
      WHEN 'database_concurrency' THEN max_value BETWEEN 0 AND 64
      WHEN 'tenant_database_concurrency' THEN max_value BETWEEN 0 AND 256
      WHEN 'secret_provider_operations' THEN max_value BETWEEN 0 AND 128
      ELSE max_value BETWEEN 0 AND 1000000000
    END);

