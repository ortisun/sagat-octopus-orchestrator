-- Rollback is an operator/quiesced-service operation. A reset that waits for its source, a reset that
-- failed for one of the reasons of this migration and a limit or ceiling of the pending resets are
-- evidence of what was asked: rolling back never destroys them.
LOCK TABLE run_resets, quota_limits, quota_ceilings IN ACCESS EXCLUSIVE MODE;
DO $guard$
BEGIN
    IF EXISTS (SELECT 1 FROM run_resets WHERE state = 'PENDING_SOURCE' OR source_policy = 'CANCEL_SOURCE'
                  OR failure_reason IN ('AUTHORITY_REVOKED', 'REEXECUTION_NOT_ACKNOWLEDGED', 'REAPPLY_TOO_MANY')
                  OR acknowledged_reexecution)
        OR EXISTS (SELECT 1 FROM quota_limits WHERE resource = 'pending_resets')
        OR EXISTS (SELECT 1 FROM quota_ceilings WHERE resource = 'pending_resets') THEN
        RAISE EXCEPTION 'resets of an open source, or their limits, are retained; keep a compatible reader'
            USING ERRCODE = 'object_not_in_prerequisite_state';
    END IF;
END $guard$;

CREATE OR REPLACE FUNCTION run_resets_guard() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        IF OLD.state = 'PENDING_SOURCE' THEN
            RAISE EXCEPTION 'a reset that waits for its source is never purged'
                USING ERRCODE = 'restrict_violation';
        END IF;
        RETURN OLD;
    END IF;
    IF NEW.tenant_id IS DISTINCT FROM OLD.tenant_id
        OR NEW.namespace_id IS DISTINCT FROM OLD.namespace_id
        OR NEW.id IS DISTINCT FROM OLD.id
        OR NEW.source_run_id IS DISTINCT FROM OLD.source_run_id
        OR NEW.source_sequence IS DISTINCT FROM OLD.source_sequence
        OR NEW.prefix_digest IS DISTINCT FROM OLD.prefix_digest
        OR NEW.reapply IS DISTINCT FROM OLD.reapply
        OR NEW.business_id_policy IS DISTINCT FROM OLD.business_id_policy
        OR NEW.source_policy IS DISTINCT FROM OLD.source_policy
        OR NEW.actor IS DISTINCT FROM OLD.actor
        OR NEW.reason IS DISTINCT FROM OLD.reason
        OR NEW.created_at IS DISTINCT FROM OLD.created_at THEN
        RAISE EXCEPTION 'what a reset is never changes; only its outcome does'
            USING ERRCODE = 'restrict_violation';
    END IF;
    IF OLD.state <> 'PENDING_SOURCE' AND (
        NEW.state IS DISTINCT FROM OLD.state
        OR NEW.failure_reason IS DISTINCT FROM OLD.failure_reason
        OR NEW.new_run_id IS DISTINCT FROM OLD.new_run_id
        OR NEW.completed_at IS DISTINCT FROM OLD.completed_at) THEN
        RAISE EXCEPTION 'a reset that has completed stays as it completed'
            USING ERRCODE = 'restrict_violation';
    END IF;
    RETURN NEW;
END;
$$;

ALTER TABLE run_resets DROP CONSTRAINT run_resets_failure_reason_valid;
ALTER TABLE run_resets ADD CONSTRAINT run_resets_failure_reason_valid CHECK (failure_reason IS NULL OR failure_reason IN
    ('BOUNDARY_NO_LONGER_ELIGIBLE', 'ADMISSION_REFUSED', 'BUSINESS_ID_IN_USE',
     'CONCURRENCY_PERMIT_UNAVAILABLE', 'RUN_PURGED'));
DROP INDEX run_resets_pending_namespace_idx;
ALTER TABLE run_resets DROP COLUMN acknowledged_reexecution;

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
     'batch_jobs', 'tenant_batch_jobs'));
ALTER TABLE quota_limits DROP CONSTRAINT quota_limits_resource_valid;
ALTER TABLE quota_limits ADD CONSTRAINT quota_limits_resource_valid CHECK (resource IN ('active_runs', 'tenant_active_runs', 'workflows', 'worker_sessions', 'secrets', 'event_endpoints', 'webhook_subscriptions', 'broker_subscriptions', 'event_connectors', 'scripts', 'script_jobs', 'activity_concurrency', 'tenant_content_bytes', 'tenant_log_bytes', 'api_requests_per_second', 'events_per_second', 'event_retention_seconds', 'script_retention_days', 'database_statement_bytes', 'database_parameters', 'database_parameter_bytes', 'database_rows', 'database_result_bytes', 'database_timeout_ms', 'database_connect_timeout_ms', 'database_pool_size', 'database_idle_timeout_ms', 'database_connections', 'database_concurrency', 'tenant_database_concurrency', 'secret_provider_operations', 'schedules', 'schedule_backfills', 'batch_jobs', 'tenant_batch_jobs'));
ALTER TABLE quota_limits DROP CONSTRAINT quota_limits_tenant_scoped;
ALTER TABLE quota_limits ADD CONSTRAINT quota_limits_tenant_scoped CHECK (namespace_id IS NULL OR resource IN ('active_runs', 'workflows', 'secrets', 'event_endpoints', 'webhook_subscriptions', 'broker_subscriptions', 'event_connectors', 'scripts', 'script_jobs', 'activity_concurrency', 'api_requests_per_second', 'events_per_second', 'event_retention_seconds', 'script_retention_days', 'database_statement_bytes', 'database_parameters', 'database_parameter_bytes', 'database_rows', 'database_result_bytes', 'database_timeout_ms', 'database_connect_timeout_ms', 'database_pool_size', 'database_idle_timeout_ms', 'database_connections', 'database_concurrency', 'secret_provider_operations', 'schedules', 'schedule_backfills', 'batch_jobs'));
