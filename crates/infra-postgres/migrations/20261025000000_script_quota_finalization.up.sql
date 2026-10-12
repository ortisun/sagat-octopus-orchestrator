-- ADR-0027: terminal codes survive a completely exhausted diagnostic-log allowance.
ALTER TABLE script_versions ADD COLUMN build_log_truncated boolean NOT NULL DEFAULT false;
ALTER TABLE script_versions ADD COLUMN failure_code text;
ALTER TABLE script_versions ADD CONSTRAINT script_versions_failure_code_valid CHECK (
    failure_code IS NULL OR failure_code IN
    ('SCRIPT_BUILD_REJECTED', 'SCRIPT_SANDBOX_UNAVAILABLE', 'STORAGE_QUOTA_EXCEEDED'));

ALTER TABLE inbox_events ADD COLUMN last_error_truncated boolean NOT NULL DEFAULT false;
ALTER TABLE event_connectors ADD COLUMN last_error_truncated boolean NOT NULL DEFAULT false;

-- Validate existing rows as well. Invalid historical identifiers require explicit operator repair;
-- the migration must not silently erase or exempt arbitrary data from content accounting.
ALTER TABLE script_executions ADD CONSTRAINT script_executions_trace_id_valid CHECK (
    trace_id IS NULL OR (trace_id ~ '^[0-9a-f]{32}$' AND trace_id <> repeat('0',32)));
