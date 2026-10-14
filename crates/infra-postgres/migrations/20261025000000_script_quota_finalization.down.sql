-- Refuse to silently discard diagnostic-loss or terminal storage-failure evidence.
DO $$ BEGIN
    IF EXISTS (SELECT 1 FROM script_versions WHERE build_log_truncated OR failure_code IS NOT NULL)
       OR EXISTS (SELECT 1 FROM inbox_events WHERE last_error_truncated)
       OR EXISTS (SELECT 1 FROM event_connectors WHERE last_error_truncated) THEN
        RAISE EXCEPTION 'script quota finalization evidence prevents rollback';
    END IF;
END $$;
ALTER TABLE script_versions DROP COLUMN failure_code;
ALTER TABLE script_versions DROP COLUMN build_log_truncated;

ALTER TABLE inbox_events DROP COLUMN last_error_truncated;
ALTER TABLE event_connectors DROP COLUMN last_error_truncated;

ALTER TABLE script_executions DROP CONSTRAINT script_executions_trace_id_valid;
