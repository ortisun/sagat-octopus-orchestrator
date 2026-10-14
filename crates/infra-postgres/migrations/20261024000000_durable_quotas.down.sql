-- Refuse rollback when it would silently discard live reservations or configured policy.
DO $rollback$
BEGIN
    IF EXISTS (SELECT 1 FROM blobs WHERE state <> 'COMMITTED' AND reservation_bytes > 0) THEN
        RAISE EXCEPTION 'cannot roll back durable quotas with outstanding blob reservations';
    END IF;
    IF EXISTS (SELECT 1 FROM quota_limits WHERE resource IN
        ('tenant_content_bytes','tenant_log_bytes','api_requests_per_second','events_per_second',
         'event_retention_seconds','script_retention_days')) OR EXISTS
       (SELECT 1 FROM quota_ceilings WHERE resource IN
        ('tenant_content_bytes','tenant_log_bytes','api_requests_per_second','events_per_second',
         'event_retention_seconds','script_retention_days')) THEN
        RAISE EXCEPTION 'cannot roll back durable quotas with configured durable policy';
    END IF;
END
$rollback$;

DROP TRIGGER IF EXISTS tenants_quota_usage ON tenants;
DROP FUNCTION IF EXISTS sagat_quota_provision_tenant_usage();
-- These tables were included in pre-release revisions of this migration. IF EXISTS keeps rollback
-- safe for developer databases without making them part of the final accounting set.
DROP TRIGGER IF EXISTS event_deliveries_quota_flush ON event_deliveries;
DROP TRIGGER IF EXISTS event_deliveries_quota_delta ON event_deliveries;
DROP TRIGGER IF EXISTS run_saga_entries_quota_flush ON run_saga_entries;
DROP TRIGGER IF EXISTS run_saga_entries_quota_delta ON run_saga_entries;
DROP TRIGGER IF EXISTS saga_plan_entries_quota_flush ON saga_plan_entries;
DROP TRIGGER IF EXISTS saga_plan_entries_quota_delta ON saga_plan_entries;
DROP TRIGGER IF EXISTS saga_plans_quota_flush ON saga_plans;
DROP TRIGGER IF EXISTS saga_plans_quota_delta ON saga_plans;
DROP TRIGGER IF EXISTS webhook_attempts_quota_flush ON webhook_attempts;
DROP TRIGGER IF EXISTS webhook_attempts_quota_delta ON webhook_attempts;
DROP TRIGGER IF EXISTS webhook_deliveries_quota_flush ON webhook_deliveries;
DROP TRIGGER IF EXISTS webhook_deliveries_quota_delta ON webhook_deliveries;
DROP TRIGGER IF EXISTS workflow_activations_quota_flush ON workflow_activations;
DROP TRIGGER IF EXISTS workflow_activations_quota_delta ON workflow_activations;
DROP TRIGGER connector_dead_letters_quota_flush ON connector_dead_letters;
DROP TRIGGER connector_dead_letters_quota_delta ON connector_dead_letters;
DROP TRIGGER connector_offsets_quota_flush ON connector_offsets;
DROP TRIGGER connector_offsets_quota_delta ON connector_offsets;
DROP TRIGGER egress_policies_quota_flush ON egress_policies;
DROP TRIGGER egress_policies_quota_delta ON egress_policies;
DROP TRIGGER event_connectors_quota_flush ON event_connectors;
DROP TRIGGER event_connectors_quota_delta ON event_connectors;
DROP TRIGGER event_endpoints_quota_flush ON event_endpoints;
DROP TRIGGER event_endpoints_quota_delta ON event_endpoints;
DROP TRIGGER inbox_events_quota_flush ON inbox_events;
DROP TRIGGER inbox_events_quota_delta ON inbox_events;
DROP TRIGGER memberships_quota_flush ON memberships;
DROP TRIGGER memberships_quota_delta ON memberships;
DROP TRIGGER namespaces_quota_flush ON namespaces;
DROP TRIGGER namespaces_quota_delta ON namespaces;
DROP TRIGGER payloads_quota_flush ON payloads;
DROP TRIGGER payloads_quota_delta ON payloads;
DROP TRIGGER publication_decisions_quota_flush ON publication_decisions;
DROP TRIGGER publication_decisions_quota_delta ON publication_decisions;
DROP TRIGGER publication_requests_quota_flush ON publication_requests;
DROP TRIGGER publication_requests_quota_delta ON publication_requests;
DROP TRIGGER runs_quota_flush ON runs;
DROP TRIGGER runs_quota_delta ON runs;
DROP TRIGGER script_artifacts_quota_flush ON script_artifacts;
DROP TRIGGER script_artifacts_quota_delta ON script_artifacts;
DROP TRIGGER script_executions_quota_flush ON script_executions;
DROP TRIGGER script_executions_quota_delta ON script_executions;
DROP TRIGGER script_test_runs_quota_flush ON script_test_runs;
DROP TRIGGER script_test_runs_quota_delta ON script_test_runs;
DROP TRIGGER script_versions_quota_flush ON script_versions;
DROP TRIGGER script_versions_quota_delta ON script_versions;
DROP TRIGGER scripts_quota_flush ON scripts;
DROP TRIGGER scripts_quota_delta ON scripts;
DROP TRIGGER secret_access_quota_flush ON secret_access;
DROP TRIGGER secret_access_quota_delta ON secret_access;
DROP TRIGGER secret_versions_quota_flush ON secret_versions;
DROP TRIGGER secret_versions_quota_delta ON secret_versions;
DROP TRIGGER secrets_quota_flush ON secrets;
DROP TRIGGER secrets_quota_delta ON secrets;
DROP TRIGGER steps_quota_flush ON steps;
DROP TRIGGER steps_quota_delta ON steps;
DROP TRIGGER webhook_subscriptions_quota_flush ON webhook_subscriptions;
DROP TRIGGER webhook_subscriptions_quota_delta ON webhook_subscriptions;
DROP TRIGGER worker_bindings_quota_flush ON worker_bindings;
DROP TRIGGER worker_bindings_quota_delta ON worker_bindings;
DROP TRIGGER worker_drains_quota_flush ON worker_drains;
DROP TRIGGER worker_drains_quota_delta ON worker_drains;
DROP TRIGGER IF EXISTS worker_sessions_quota_flush ON worker_sessions;
DROP TRIGGER IF EXISTS worker_sessions_quota_delta ON worker_sessions;
DROP TRIGGER workflow_triggers_quota_flush ON workflow_triggers;
DROP TRIGGER workflow_triggers_quota_delta ON workflow_triggers;
DROP TRIGGER workflow_versions_quota_flush ON workflow_versions;
DROP TRIGGER workflow_versions_quota_delta ON workflow_versions;
DROP TRIGGER workflows_quota_flush ON workflows;
DROP TRIGGER workflows_quota_delta ON workflows;
DROP TRIGGER workspaces_quota_flush ON workspaces;
DROP TRIGGER workspaces_quota_delta ON workspaces;
DROP TRIGGER blobs_quota_flush ON blobs;
DROP TRIGGER blobs_quota_delta ON blobs;
DROP TRIGGER blobs_reservation_state ON blobs;
DROP FUNCTION sagat_quota_flush_deltas();
DROP FUNCTION IF EXISTS sagat_quota_lock_log_usage(uuid);
DROP FUNCTION sagat_quota_blob_delta();
DROP FUNCTION sagat_quota_row_delta();
DROP FUNCTION sagat_quota_add_delta(uuid, bigint, bigint, bigint);
DROP FUNCTION IF EXISTS sagat_quota_measure(jsonb, text, text, boolean);
DROP FUNCTION IF EXISTS sagat_quota_measure(jsonb, text, text);
DROP FUNCTION sagat_blob_reservation_state();
ALTER TABLE blobs DROP COLUMN reservation_bytes;
DROP TABLE quota_rate_buckets;
DROP TABLE quota_accounting_control;
DROP TABLE quota_transaction_deltas;
DROP TABLE tenant_retained_usage;

ALTER TABLE quota_limits DROP CONSTRAINT quota_limits_resource_valid;
ALTER TABLE quota_limits ADD CONSTRAINT quota_limits_resource_valid CHECK (resource IN
    ('active_runs', 'tenant_active_runs', 'workflows', 'worker_sessions', 'secrets',
     'event_endpoints', 'webhook_subscriptions', 'broker_subscriptions', 'event_connectors',
     'scripts', 'script_jobs', 'activity_concurrency'));
ALTER TABLE quota_limits DROP CONSTRAINT quota_limits_tenant_scoped;
ALTER TABLE quota_limits ADD CONSTRAINT quota_limits_tenant_scoped CHECK (
    namespace_id IS NULL OR resource IN ('active_runs', 'workflows', 'secrets', 'event_endpoints', 'webhook_subscriptions',
         'broker_subscriptions', 'event_connectors', 'scripts', 'script_jobs',
         'activity_concurrency'));
ALTER TABLE quota_limits DROP CONSTRAINT quota_limits_value_range;
ALTER TABLE quota_limits ADD CONSTRAINT quota_limits_value_range CHECK (max_value BETWEEN 0 AND 1000000000);
ALTER TABLE quota_ceilings DROP CONSTRAINT quota_ceilings_resource_valid;
ALTER TABLE quota_ceilings ADD CONSTRAINT quota_ceilings_resource_valid CHECK (resource IN
    ('active_runs', 'tenant_active_runs', 'workflows', 'worker_sessions', 'secrets',
     'event_endpoints', 'webhook_subscriptions', 'broker_subscriptions', 'event_connectors',
     'scripts', 'script_jobs', 'activity_concurrency'));
ALTER TABLE quota_ceilings DROP CONSTRAINT quota_ceilings_value_range;
ALTER TABLE quota_ceilings ADD CONSTRAINT quota_ceilings_value_range CHECK (max_value BETWEEN 0 AND 1000000000);
