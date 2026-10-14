-- Feature 004 G77: NATS, Kafka and SQS are destinations in the existing durable delivery pipeline.
-- The webhook_* names are historical physical names retained to avoid a risky table rewrite.

ALTER TABLE webhook_subscriptions ADD COLUMN destination_kind text NOT NULL DEFAULT 'WEBHOOK';
ALTER TABLE webhook_subscriptions ADD COLUMN destination_settings jsonb NOT NULL DEFAULT '{}'::jsonb;
ALTER TABLE webhook_subscriptions ALTER COLUMN url DROP NOT NULL;
ALTER TABLE webhook_subscriptions ALTER COLUMN secret_name DROP NOT NULL;
ALTER TABLE webhook_subscriptions ADD CONSTRAINT webhook_subscriptions_destination_kind_valid
    CHECK (destination_kind IN ('WEBHOOK', 'NATS', 'KAFKA', 'SQS'));
ALTER TABLE webhook_subscriptions ADD CONSTRAINT webhook_subscriptions_destination_shape
    CHECK (
        (destination_kind = 'WEBHOOK' AND url IS NOT NULL AND secret_name IS NOT NULL
         AND destination_settings = '{}'::jsonb)
        OR
        (destination_kind <> 'WEBHOOK' AND url IS NULL AND jsonb_typeof(destination_settings) = 'object')
    );

ALTER TABLE webhook_attempts DROP CONSTRAINT webhook_attempts_outcome_valid;
ALTER TABLE webhook_attempts ADD CONSTRAINT webhook_attempts_outcome_valid CHECK (outcome IN
    ('DELIVERED', 'REJECTED', 'SERVER_ERROR', 'THROTTLED', 'TIMEOUT', 'NETWORK', 'DENIED',
     'UNSIGNED', 'CREDENTIAL_UNAVAILABLE'));

ALTER TABLE quota_limits DROP CONSTRAINT quota_limits_resource_valid;
ALTER TABLE quota_limits ADD CONSTRAINT quota_limits_resource_valid CHECK (resource IN
    ('active_runs', 'tenant_active_runs', 'workflows', 'worker_sessions', 'secrets',
     'event_endpoints', 'webhook_subscriptions', 'broker_subscriptions', 'event_connectors',
     'scripts', 'script_jobs', 'activity_concurrency'));
ALTER TABLE quota_limits DROP CONSTRAINT quota_limits_tenant_scoped;
ALTER TABLE quota_limits ADD CONSTRAINT quota_limits_tenant_scoped CHECK (
    namespace_id IS NULL OR resource IN
        ('active_runs', 'workflows', 'secrets', 'event_endpoints', 'webhook_subscriptions',
         'broker_subscriptions', 'event_connectors', 'scripts', 'script_jobs',
         'activity_concurrency'));
ALTER TABLE quota_ceilings DROP CONSTRAINT quota_ceilings_resource_valid;
ALTER TABLE quota_ceilings ADD CONSTRAINT quota_ceilings_resource_valid CHECK (resource IN
    ('active_runs', 'tenant_active_runs', 'workflows', 'worker_sessions', 'secrets',
     'event_endpoints', 'webhook_subscriptions', 'broker_subscriptions', 'event_connectors',
     'scripts', 'script_jobs', 'activity_concurrency'));
