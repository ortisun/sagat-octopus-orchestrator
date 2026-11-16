-- Feature 022, release 0.3.0 slice (T022-013; FR-022-021, FR-022-034, FR-022-047; D022-024, D022-040): the quota
-- resources of the concurrency gate. Counted in the namespace: the runs waiting for a permit or a slot (`queued_runs`,
-- 1,000 by default, 10,000 at most), the live keys (10,000, 100,000) and the run waiters of one key (100, 10,000).
-- Policies of the namespace: the most holders of one concurrency key (10,000), the longest wait (one day) and the
-- node-level declarations of one kind per definition (16, 64). The values are in code (`QuotaResource`); the tables only
-- name what they may hold. The live keys are the semaphores only: rate-limit keys get their own ceiling (T022-077).
--
-- Additive for the schema. Not for an older replica's quota views: they read every stored row and refuse a name their
-- build does not know (`stored_limits`), so a tenant that sets one of these limits gets an error from an older replica's
-- quota listing until it is upgraded (G1192, true of every resource added before this one).
--
-- REBASE HAZARD: the three CHECKs below restate the whole list of resources of the ceilings tables, as every migration
-- that adds a resource name does (written after 20270108000000_pending_resets, the last one that restated them). A
-- migration that adds a resource and is merged before this one is rebased onto it and restates the list with both.
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
     'queued_runs', 'concurrency_live_keys', 'flow_control_key_waiters', 'concurrency_limit', 'flow_control_queue_timeout_ms', 'flow_control_node_declarations'));
ALTER TABLE quota_limits DROP CONSTRAINT quota_limits_resource_valid;
ALTER TABLE quota_limits ADD CONSTRAINT quota_limits_resource_valid CHECK (resource IN ('active_runs', 'tenant_active_runs', 'workflows', 'worker_sessions', 'secrets', 'event_endpoints', 'webhook_subscriptions', 'broker_subscriptions', 'event_connectors', 'scripts', 'script_jobs', 'activity_concurrency', 'tenant_content_bytes', 'tenant_log_bytes', 'api_requests_per_second', 'events_per_second', 'event_retention_seconds', 'script_retention_days', 'database_statement_bytes', 'database_parameters', 'database_parameter_bytes', 'database_rows', 'database_result_bytes', 'database_timeout_ms', 'database_connect_timeout_ms', 'database_pool_size', 'database_idle_timeout_ms', 'database_connections', 'database_concurrency', 'tenant_database_concurrency', 'secret_provider_operations', 'schedules', 'schedule_backfills', 'batch_jobs', 'tenant_batch_jobs', 'pending_resets', 'queued_runs', 'concurrency_live_keys', 'flow_control_key_waiters', 'concurrency_limit', 'flow_control_queue_timeout_ms', 'flow_control_node_declarations'));
ALTER TABLE quota_limits DROP CONSTRAINT quota_limits_tenant_scoped;
ALTER TABLE quota_limits ADD CONSTRAINT quota_limits_tenant_scoped CHECK (namespace_id IS NULL OR resource IN ('active_runs', 'workflows', 'secrets', 'event_endpoints', 'webhook_subscriptions', 'broker_subscriptions', 'event_connectors', 'scripts', 'script_jobs', 'activity_concurrency', 'api_requests_per_second', 'events_per_second', 'event_retention_seconds', 'script_retention_days', 'database_statement_bytes', 'database_parameters', 'database_parameter_bytes', 'database_rows', 'database_result_bytes', 'database_timeout_ms', 'database_connect_timeout_ms', 'database_pool_size', 'database_idle_timeout_ms', 'database_connections', 'database_concurrency', 'secret_provider_operations', 'schedules', 'schedule_backfills', 'batch_jobs', 'pending_resets', 'queued_runs', 'concurrency_live_keys', 'flow_control_key_waiters', 'concurrency_limit', 'flow_control_queue_timeout_ms', 'flow_control_node_declarations'));
