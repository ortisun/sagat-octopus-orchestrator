-- Fails closed while database-specific settings or grants remain; clear them before rollback.

ALTER TABLE quota_limits DROP CONSTRAINT quota_limits_resource_valid;
ALTER TABLE quota_limits ADD CONSTRAINT quota_limits_resource_valid CHECK (resource IN
    ('active_runs', 'tenant_active_runs', 'workflows', 'worker_sessions', 'secrets',
     'event_endpoints', 'webhook_subscriptions', 'broker_subscriptions', 'event_connectors',
     'scripts', 'script_jobs', 'activity_concurrency',
     'tenant_content_bytes', 'tenant_log_bytes', 'api_requests_per_second',
     'events_per_second', 'event_retention_seconds', 'script_retention_days'));
ALTER TABLE quota_limits DROP CONSTRAINT quota_limits_tenant_scoped;
ALTER TABLE quota_limits ADD CONSTRAINT quota_limits_tenant_scoped CHECK (
    namespace_id IS NULL OR resource IN
        ('active_runs', 'workflows', 'secrets', 'event_endpoints', 'webhook_subscriptions',
         'broker_subscriptions', 'event_connectors', 'scripts', 'script_jobs',
         'activity_concurrency', 'api_requests_per_second', 'events_per_second',
         'event_retention_seconds', 'script_retention_days'));
ALTER TABLE quota_limits DROP CONSTRAINT quota_limits_value_range;
ALTER TABLE quota_limits ADD CONSTRAINT quota_limits_value_range CHECK (
    CASE resource
      WHEN 'tenant_content_bytes' THEN max_value BETWEEN 0 AND 1125899906842624
      WHEN 'tenant_log_bytes' THEN max_value BETWEEN 0 AND 1125899906842624
      WHEN 'api_requests_per_second' THEN max_value BETWEEN 0 AND 1000000
      WHEN 'events_per_second' THEN max_value BETWEEN 0 AND 1000000
      WHEN 'event_retention_seconds' THEN max_value = 0 OR max_value BETWEEN 60 AND 2592000
      WHEN 'script_retention_days' THEN max_value BETWEEN 0 AND 365
      ELSE max_value BETWEEN 0 AND 1000000000
    END);

ALTER TABLE quota_ceilings DROP CONSTRAINT quota_ceilings_resource_valid;
ALTER TABLE quota_ceilings ADD CONSTRAINT quota_ceilings_resource_valid CHECK (resource IN
    ('active_runs', 'tenant_active_runs', 'workflows', 'worker_sessions', 'secrets',
     'event_endpoints', 'webhook_subscriptions', 'broker_subscriptions', 'event_connectors',
     'scripts', 'script_jobs', 'activity_concurrency', 'iteration_concurrency',
     'tenant_content_bytes', 'tenant_log_bytes', 'api_requests_per_second',
     'events_per_second', 'event_retention_seconds', 'script_retention_days'));
ALTER TABLE quota_ceilings DROP CONSTRAINT quota_ceilings_value_range;
ALTER TABLE quota_ceilings ADD CONSTRAINT quota_ceilings_value_range CHECK (
    CASE resource
      WHEN 'tenant_content_bytes' THEN max_value BETWEEN 0 AND 1125899906842624
      WHEN 'tenant_log_bytes' THEN max_value BETWEEN 0 AND 1125899906842624
      WHEN 'api_requests_per_second' THEN max_value BETWEEN 0 AND 1000000
      WHEN 'events_per_second' THEN max_value BETWEEN 0 AND 1000000
      WHEN 'event_retention_seconds' THEN max_value = 0 OR max_value BETWEEN 60 AND 2592000
      WHEN 'script_retention_days' THEN max_value BETWEEN 0 AND 365
      ELSE max_value BETWEEN 0 AND 1000000000
    END);

