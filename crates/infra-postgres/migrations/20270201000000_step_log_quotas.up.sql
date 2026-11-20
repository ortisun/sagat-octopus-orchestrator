-- Feature 041 (step logs), T041-4: the four quota resources of specs/041-step-logs/data-model.md §6 and the usage column
-- of the step-log class of the ADR-0027 Amendment of 2026-11-15 (OD-2026-11-15-5, -16), so that the quota view of
-- `tenant_step_log_bytes` has a column to read. The rest of the Amendment's accounting (the delta, the routing of the
-- class, the finalizer's branch, the control flag and the accessor) comes with the tables in T041-5.
--
-- Every CHECK is added `NOT VALID` and validated by 20270201000100_step_log_quotas_validate, so that no table is
-- scanned under the `ACCESS EXCLUSIVE` lock of the `ADD` (design 043, D043-145, D043-153).
--
-- Additive: an older reader never reads the new column. An older replica cannot parse a step-log resource name, so
-- once a step-log row exists in quota_limits or quota_ceilings its quota listing fails on it: finish the upgrade
-- before setting one (design 041 section 10; the runbook docs/runbooks/step-logs.md of T041-10 and T041-25 says so).
--
-- REBASE HAZARD: the three CHECKs below restate the whole list of resources of the ceilings tables, as every
-- migration that adds a resource name does. This file follows 20270108000000_pending_resets; feature 022
-- (20270130000300_flow_control_quota_resources, not merged when this was written) adds names too. Migrations run in
-- version order, not merge order: whichever of the two merges second is re-stamped after the other's last migration
-- and restates both lists, in its up and in its down. The test
-- the_last_restatement_of_each_quota_resource_list_names_every_resource (integration_platform_foundation) fails
-- while the list in force misses a resource the code knows.
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
     'batch_jobs', 'tenant_batch_jobs', 'pending_resets', 'tenant_step_log_bytes', 'step_log_attempt_records', 'step_log_attempt_bytes', 'step_log_retention_days')) NOT VALID;
ALTER TABLE quota_limits DROP CONSTRAINT quota_limits_resource_valid;
ALTER TABLE quota_limits ADD CONSTRAINT quota_limits_resource_valid CHECK (resource IN ('active_runs', 'tenant_active_runs', 'workflows', 'worker_sessions', 'secrets', 'event_endpoints', 'webhook_subscriptions', 'broker_subscriptions', 'event_connectors', 'scripts', 'script_jobs', 'activity_concurrency', 'tenant_content_bytes', 'tenant_log_bytes', 'api_requests_per_second', 'events_per_second', 'event_retention_seconds', 'script_retention_days', 'database_statement_bytes', 'database_parameters', 'database_parameter_bytes', 'database_rows', 'database_result_bytes', 'database_timeout_ms', 'database_connect_timeout_ms', 'database_pool_size', 'database_idle_timeout_ms', 'database_connections', 'database_concurrency', 'tenant_database_concurrency', 'secret_provider_operations', 'schedules', 'schedule_backfills', 'batch_jobs', 'tenant_batch_jobs', 'pending_resets', 'tenant_step_log_bytes', 'step_log_attempt_records', 'step_log_attempt_bytes', 'step_log_retention_days')) NOT VALID;
ALTER TABLE quota_limits DROP CONSTRAINT quota_limits_tenant_scoped;
ALTER TABLE quota_limits ADD CONSTRAINT quota_limits_tenant_scoped CHECK (namespace_id IS NULL OR resource IN ('active_runs', 'workflows', 'secrets', 'event_endpoints', 'webhook_subscriptions', 'broker_subscriptions', 'event_connectors', 'scripts', 'script_jobs', 'activity_concurrency', 'api_requests_per_second', 'events_per_second', 'event_retention_seconds', 'script_retention_days', 'database_statement_bytes', 'database_parameters', 'database_parameter_bytes', 'database_rows', 'database_result_bytes', 'database_timeout_ms', 'database_connect_timeout_ms', 'database_pool_size', 'database_idle_timeout_ms', 'database_connections', 'database_concurrency', 'secret_provider_operations', 'schedules', 'schedule_backfills', 'batch_jobs', 'pending_resets', 'step_log_attempt_records', 'step_log_attempt_bytes', 'step_log_retention_days')) NOT VALID;

-- The value ranges of data-model.md §6, restated with the step-log resources (the last restatement is
-- 20261127000000_database_policies): `tenant_step_log_bytes` is a byte quota up to 2^50 like the other two,
-- `step_log_attempt_records` takes 0 (the capture switch), the other two do not.
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
      WHEN 'tenant_step_log_bytes' THEN max_value BETWEEN 0 AND 1125899906842624
      WHEN 'step_log_attempt_records' THEN max_value BETWEEN 0 AND 100000
      WHEN 'step_log_attempt_bytes' THEN max_value BETWEEN 1024 AND 16777216
      WHEN 'step_log_retention_days' THEN max_value BETWEEN 1 AND 365
      ELSE max_value BETWEEN 0 AND 1000000000
    END) NOT VALID;
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
      WHEN 'tenant_step_log_bytes' THEN max_value BETWEEN 0 AND 1125899906842624
      WHEN 'step_log_attempt_records' THEN max_value BETWEEN 0 AND 100000
      WHEN 'step_log_attempt_bytes' THEN max_value BETWEEN 1024 AND 16777216
      WHEN 'step_log_retention_days' THEN max_value BETWEEN 1 AND 365
      ELSE max_value BETWEEN 0 AND 1000000000
    END) NOT VALID;

-- The usage of the step-log class (`tenant_step_log_bytes`), written by the quota finalizer from T041-5 on.
ALTER TABLE tenant_retained_usage ADD COLUMN step_log_bytes bigint NOT NULL DEFAULT 0;
-- No CHECK (step_log_bytes >= 0): under ADR-0097 (Proposed) S may go below zero while released bytes were charged in
-- quota leases; the usage S + Σ used is kept non-negative by the release (design 041, D041-112).
