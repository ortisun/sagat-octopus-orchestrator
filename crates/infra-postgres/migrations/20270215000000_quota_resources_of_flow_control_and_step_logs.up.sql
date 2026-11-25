-- The merge of feature 022 (flow control) and feature 041 (step logs) into one tree (decision D022-089 of
-- specs/022-flow-control-step-policies/research.md): the three CHECKs of the ceilings tables restated with the names of
-- both features. Migrations run in version order, not merge order. Feature 022's 20270130000300 added its six concurrency
-- names; feature 041's 20270201000000_step_log_quotas, written without them, restated the lists after it and so dropped
-- them; feature 022's 20270206000000_flow_control_rate_quota_resources, written without feature 041's names, restated the
-- lists after that and so dropped the four step-log names. This migration, stamped after every migration of both, is the
-- restatement in force on a fresh database and on an upgraded one: it names every value of `QuotaResource::ALL`
-- (`migrations::the_last_restatement_of_each_quota_resource_list_names_every_resource` reads its text and
-- `migrations::every_quota_resource_is_accepted_by_the_checks_in_force_on_a_fresh_database` inserts every resource,
-- both in integration_platform_foundation).
--
-- The value ranges are not restated: 20270201000000 restated them with the step-log ranges, and feature 022's resources
-- take the general range (their bounds are in code, `QuotaResource::maximum`).
--
-- Additive: it only widens what the tables accept. Added `NOT VALID`, so no table is scanned under the `ACCESS
-- EXCLUSIVE` lock of the `ADD`; 20270215000100 validates them (design 043, D043-145, D043-153).
--
-- REBASE HAZARD: the three CHECKs below restate the whole list of resources of the ceilings tables. A branch that adds
-- a resource name restates them again, stamped after this file, with every name of this one.
ALTER TABLE quota_ceilings DROP CONSTRAINT quota_ceilings_resource_valid;
ALTER TABLE quota_ceilings ADD CONSTRAINT quota_ceilings_resource_valid CHECK (resource IN
    ('active_runs', 'tenant_active_runs', 'workflows', 'worker_sessions', 'secrets', 'event_endpoints',
     'webhook_subscriptions', 'broker_subscriptions', 'event_connectors', 'scripts', 'script_jobs',
     'activity_concurrency', 'iteration_concurrency', 'tenant_content_bytes', 'tenant_log_bytes',
     'api_requests_per_second', 'events_per_second', 'event_retention_seconds', 'script_retention_days',
     'database_statement_bytes', 'database_parameters', 'database_parameter_bytes', 'database_rows',
     'database_result_bytes', 'database_timeout_ms', 'database_connect_timeout_ms', 'database_pool_size',
     'database_idle_timeout_ms', 'database_connections', 'database_concurrency', 'tenant_database_concurrency',
     'secret_provider_operations', 'schedules', 'schedule_backfills', 'batch_jobs', 'tenant_batch_jobs',
     'pending_resets', 'queued_runs', 'concurrency_live_keys', 'flow_control_key_waiters', 'concurrency_limit',
     'flow_control_queue_timeout_ms', 'flow_control_node_declarations', 'rate_limit_live_keys',
     'rate_limit_per_second', 'rate_limit_per_minute', 'rate_limit_per_hour', 'tenant_step_log_bytes',
     'step_log_attempt_records', 'step_log_attempt_bytes', 'step_log_retention_days')) NOT VALID;
ALTER TABLE quota_limits DROP CONSTRAINT quota_limits_resource_valid;
ALTER TABLE quota_limits ADD CONSTRAINT quota_limits_resource_valid CHECK (resource IN
    ('active_runs', 'tenant_active_runs', 'workflows', 'worker_sessions', 'secrets', 'event_endpoints',
     'webhook_subscriptions', 'broker_subscriptions', 'event_connectors', 'scripts', 'script_jobs',
     'activity_concurrency', 'tenant_content_bytes', 'tenant_log_bytes', 'api_requests_per_second',
     'events_per_second', 'event_retention_seconds', 'script_retention_days', 'database_statement_bytes',
     'database_parameters', 'database_parameter_bytes', 'database_rows', 'database_result_bytes',
     'database_timeout_ms', 'database_connect_timeout_ms', 'database_pool_size', 'database_idle_timeout_ms',
     'database_connections', 'database_concurrency', 'tenant_database_concurrency', 'secret_provider_operations',
     'schedules', 'schedule_backfills', 'batch_jobs', 'tenant_batch_jobs', 'pending_resets', 'queued_runs',
     'concurrency_live_keys', 'flow_control_key_waiters', 'concurrency_limit', 'flow_control_queue_timeout_ms',
     'flow_control_node_declarations', 'rate_limit_live_keys', 'rate_limit_per_second', 'rate_limit_per_minute',
     'rate_limit_per_hour', 'tenant_step_log_bytes', 'step_log_attempt_records', 'step_log_attempt_bytes',
     'step_log_retention_days')) NOT VALID;
ALTER TABLE quota_limits DROP CONSTRAINT quota_limits_tenant_scoped;
ALTER TABLE quota_limits ADD CONSTRAINT quota_limits_tenant_scoped CHECK (namespace_id IS NULL OR resource IN
    ('active_runs', 'workflows', 'secrets', 'event_endpoints', 'webhook_subscriptions', 'broker_subscriptions',
     'event_connectors', 'scripts', 'script_jobs', 'activity_concurrency', 'api_requests_per_second',
     'events_per_second', 'event_retention_seconds', 'script_retention_days', 'database_statement_bytes',
     'database_parameters', 'database_parameter_bytes', 'database_rows', 'database_result_bytes',
     'database_timeout_ms', 'database_connect_timeout_ms', 'database_pool_size', 'database_idle_timeout_ms',
     'database_connections', 'database_concurrency', 'secret_provider_operations', 'schedules', 'schedule_backfills',
     'batch_jobs', 'pending_resets', 'queued_runs', 'concurrency_live_keys', 'flow_control_key_waiters',
     'concurrency_limit', 'flow_control_queue_timeout_ms', 'flow_control_node_declarations', 'rate_limit_live_keys',
     'rate_limit_per_second', 'rate_limit_per_minute', 'rate_limit_per_hour', 'step_log_attempt_records',
     'step_log_attempt_bytes', 'step_log_retention_days')) NOT VALID;
