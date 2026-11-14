-- Feature 018, US06 (T018-057): `CANCEL_SOURCE`, the reset of a run that is still open. The request cancels the
-- source in the ordinary way and leaves an entry of the register in state `PENDING_SOURCE`; a fenced pass of the
-- engine creates the run once the source has ended, if the point is still eligible (FR-018-038).
--
-- Additive: an older reader never reads the new column, the new quota resource or the new reasons.

-- The ceilings tables name every resource they may hold: the pending resets of a namespace are counted in
-- the namespace (16 by default in code, hard 64).
--
-- REBASE HAZARD: the three CHECKs below restate the whole list of resources of the ceilings tables, as every
-- migration that adds a resource name does. A migration that adds a resource and is written after the one it
-- follows, but merged before this one, would be undone by it: whoever merges the second rebases it onto the first and
-- restates the list with both names (this file was written after 20270102000000_batch_operations).
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
     'batch_jobs', 'tenant_batch_jobs', 'pending_resets'));
ALTER TABLE quota_limits DROP CONSTRAINT quota_limits_resource_valid;
ALTER TABLE quota_limits ADD CONSTRAINT quota_limits_resource_valid CHECK (resource IN ('active_runs', 'tenant_active_runs', 'workflows', 'worker_sessions', 'secrets', 'event_endpoints', 'webhook_subscriptions', 'broker_subscriptions', 'event_connectors', 'scripts', 'script_jobs', 'activity_concurrency', 'tenant_content_bytes', 'tenant_log_bytes', 'api_requests_per_second', 'events_per_second', 'event_retention_seconds', 'script_retention_days', 'database_statement_bytes', 'database_parameters', 'database_parameter_bytes', 'database_rows', 'database_result_bytes', 'database_timeout_ms', 'database_connect_timeout_ms', 'database_pool_size', 'database_idle_timeout_ms', 'database_connections', 'database_concurrency', 'tenant_database_concurrency', 'secret_provider_operations', 'schedules', 'schedule_backfills', 'batch_jobs', 'tenant_batch_jobs', 'pending_resets'));
ALTER TABLE quota_limits DROP CONSTRAINT quota_limits_tenant_scoped;
ALTER TABLE quota_limits ADD CONSTRAINT quota_limits_tenant_scoped CHECK (namespace_id IS NULL OR resource IN ('active_runs', 'workflows', 'secrets', 'event_endpoints', 'webhook_subscriptions', 'broker_subscriptions', 'event_connectors', 'scripts', 'script_jobs', 'activity_concurrency', 'api_requests_per_second', 'events_per_second', 'event_retention_seconds', 'script_retention_days', 'database_statement_bytes', 'database_parameters', 'database_parameter_bytes', 'database_rows', 'database_result_bytes', 'database_timeout_ms', 'database_connect_timeout_ms', 'database_pool_size', 'database_idle_timeout_ms', 'database_connections', 'database_concurrency', 'secret_provider_operations', 'schedules', 'schedule_backfills', 'batch_jobs', 'pending_resets'));

-- The count that judges the limit of a namespace reads the entries that wait: a partial index keeps it a short read
-- however many entries have ended (the register keeps them).
CREATE INDEX run_resets_pending_namespace_idx ON run_resets (tenant_id, namespace_id)
    WHERE state = 'PENDING_SOURCE';

-- What the caller acknowledged when the request was made. The pass that completes the reset reads it again with
-- the journal the source ended with: steps that completed after the point and run again need it.
ALTER TABLE run_resets ADD COLUMN acknowledged_reexecution boolean NOT NULL DEFAULT false;

-- The reasons a reset can fail with. The first five are those of the contract; the last three are named by this
-- task (D1996..D2005 of the research register): the requester lost the authority to reset, the source ended with
-- steps to run again that the request had not acknowledged, and the source received more to deliver again than
-- a reset may.
ALTER TABLE run_resets DROP CONSTRAINT run_resets_failure_reason_valid;
ALTER TABLE run_resets ADD CONSTRAINT run_resets_failure_reason_valid CHECK (failure_reason IS NULL OR failure_reason IN
    ('BOUNDARY_NO_LONGER_ELIGIBLE', 'ADMISSION_REFUSED', 'BUSINESS_ID_IN_USE',
     'CONCURRENCY_PERMIT_UNAVAILABLE', 'RUN_PURGED',
     'AUTHORITY_REVOKED', 'REEXECUTION_NOT_ACKNOWLEDGED', 'REAPPLY_TOO_MANY'));

-- What a reset is, and why it exists, never changes; only a pending one is completed, once. The acknowledgement
-- is part of what was asked.
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
        OR NEW.acknowledged_reexecution IS DISTINCT FROM OLD.acknowledged_reexecution
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
