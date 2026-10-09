DO $revoke$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        REVOKE ALL ON event_outbox, worker_sessions, steps, run_events, runs, payloads FROM sagat_app;
    END IF;
END
$revoke$;

DROP TABLE event_outbox;
DROP FUNCTION outbox_reject_undelivered_delete();
DROP TABLE worker_sessions;
DROP TABLE steps;
DROP TRIGGER run_events_no_truncate ON run_events;
DROP TRIGGER run_events_append_only ON run_events;
DROP TABLE run_events;
DROP TABLE runs;
DROP TRIGGER payloads_no_truncate ON payloads;
DROP TRIGGER payloads_append_only ON payloads;
DROP TABLE payloads;
DROP FUNCTION execution_reject_mutation();
