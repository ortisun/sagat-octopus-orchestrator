DO $revoke$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        REVOKE ALL ON event_endpoints, workflow_triggers, inbox_events, event_deliveries,
            webhook_subscriptions, webhook_deliveries, webhook_attempts,
            event_connectors, connector_offsets, connector_leases, connector_dead_letters FROM sagat_app;
    END IF;
END
$revoke$;

DELETE FROM quota_limits WHERE resource IN ('event_endpoints', 'webhook_subscriptions', 'event_connectors');
DELETE FROM quota_ceilings WHERE resource IN ('event_endpoints', 'webhook_subscriptions', 'event_connectors');
ALTER TABLE quota_ceilings DROP CONSTRAINT quota_ceilings_resource_valid;
ALTER TABLE quota_ceilings ADD CONSTRAINT quota_ceilings_resource_valid CHECK (resource IN
    ('active_runs', 'tenant_active_runs', 'workflows', 'worker_sessions', 'secrets'));
ALTER TABLE quota_limits DROP CONSTRAINT quota_limits_tenant_scoped;
ALTER TABLE quota_limits ADD CONSTRAINT quota_limits_tenant_scoped CHECK (
    namespace_id IS NULL OR resource IN ('active_runs', 'workflows', 'secrets'));
ALTER TABLE quota_limits DROP CONSTRAINT quota_limits_resource_valid;
ALTER TABLE quota_limits ADD CONSTRAINT quota_limits_resource_valid CHECK (resource IN
    ('active_runs', 'tenant_active_runs', 'workflows', 'worker_sessions', 'secrets'));

DROP INDEX steps_event_unscanned_idx;
ALTER TABLE steps DROP COLUMN event_scanned_at;
ALTER TABLE steps DROP COLUMN event_since;

DROP TABLE connector_dead_letters;
DROP TABLE connector_leases;
DROP TABLE connector_offsets;
DROP TABLE event_connectors;
DROP TABLE webhook_attempts;
DROP TABLE webhook_deliveries;
DROP TABLE webhook_subscriptions;
DROP TABLE workflow_triggers;
DROP TABLE event_deliveries;
DROP TABLE inbox_events;
DROP TABLE event_endpoints;
