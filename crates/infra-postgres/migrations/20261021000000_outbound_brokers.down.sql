DO $guard$
BEGIN
    IF EXISTS (SELECT 1 FROM webhook_subscriptions WHERE destination_kind <> 'WEBHOOK') THEN
        RAISE EXCEPTION 'cannot roll back outbound brokers while broker subscriptions or deliveries exist';
    END IF;
END
$guard$;

DELETE FROM quota_limits WHERE resource = 'broker_subscriptions';
DELETE FROM quota_ceilings WHERE resource = 'broker_subscriptions';
ALTER TABLE quota_limits DROP CONSTRAINT quota_limits_tenant_scoped;
ALTER TABLE quota_limits ADD CONSTRAINT quota_limits_tenant_scoped CHECK (
    namespace_id IS NULL OR resource IN
        ('active_runs', 'workflows', 'secrets', 'event_endpoints', 'webhook_subscriptions',
         'event_connectors', 'scripts', 'script_jobs', 'activity_concurrency'));
ALTER TABLE quota_limits DROP CONSTRAINT quota_limits_resource_valid;
ALTER TABLE quota_limits ADD CONSTRAINT quota_limits_resource_valid CHECK (resource IN
    ('active_runs', 'tenant_active_runs', 'workflows', 'worker_sessions', 'secrets',
     'event_endpoints', 'webhook_subscriptions', 'event_connectors', 'scripts', 'script_jobs',
     'activity_concurrency'));
ALTER TABLE quota_ceilings DROP CONSTRAINT quota_ceilings_resource_valid;
ALTER TABLE quota_ceilings ADD CONSTRAINT quota_ceilings_resource_valid CHECK (resource IN
    ('active_runs', 'tenant_active_runs', 'workflows', 'worker_sessions', 'secrets',
     'event_endpoints', 'webhook_subscriptions', 'event_connectors', 'scripts', 'script_jobs',
     'activity_concurrency'));

ALTER TABLE webhook_attempts DROP CONSTRAINT webhook_attempts_outcome_valid;
ALTER TABLE webhook_attempts ADD CONSTRAINT webhook_attempts_outcome_valid CHECK (outcome IN
    ('DELIVERED', 'REJECTED', 'SERVER_ERROR', 'THROTTLED', 'TIMEOUT', 'NETWORK', 'DENIED', 'UNSIGNED'));
ALTER TABLE webhook_subscriptions DROP CONSTRAINT webhook_subscriptions_destination_shape;
ALTER TABLE webhook_subscriptions DROP CONSTRAINT webhook_subscriptions_destination_kind_valid;
ALTER TABLE webhook_subscriptions ALTER COLUMN url SET NOT NULL;
ALTER TABLE webhook_subscriptions ALTER COLUMN secret_name SET NOT NULL;
ALTER TABLE webhook_subscriptions DROP COLUMN destination_settings;
ALTER TABLE webhook_subscriptions DROP COLUMN destination_kind;
