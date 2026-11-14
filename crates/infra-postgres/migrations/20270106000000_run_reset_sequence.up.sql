-- Reset to a point (feature 018, FR-018-037): the run a reset creates re-emits the events 1..N of its source, and
-- `reset_sequence = N` says so, so a reader of the journal (telemetry, analytics, the run-permit holder count)
-- can tell the restored history from what the run did itself without reading another table. It is set when the
-- run is created and never changes. Null for every run that exists and for every run that no reset created; the
-- source is the run `rerun_of` already names, so the sequence has no meaning without it.
ALTER TABLE runs ADD COLUMN reset_sequence bigint;

-- NOT VALID first, then validated: every row that exists already satisfies it (the column is null).
ALTER TABLE runs ADD CONSTRAINT runs_reset_sequence_valid CHECK (
    reset_sequence IS NULL OR (rerun_of IS NOT NULL AND reset_sequence >= 1)) NOT VALID;
ALTER TABLE runs VALIDATE CONSTRAINT runs_reset_sequence_valid;

-- What a run says about the reset that created it is fixed when it is created. The trigger fires only for an
-- UPDATE that names the column, so ordinary updates of a run pay nothing.
CREATE FUNCTION runs_reset_sequence_guard() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.reset_sequence IS DISTINCT FROM OLD.reset_sequence THEN
        RAISE EXCEPTION 'the reset sequence of a run never changes'
            USING ERRCODE = 'restrict_violation';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER runs_reset_sequence BEFORE UPDATE OF reset_sequence ON runs
    FOR EACH ROW EXECUTE FUNCTION runs_reset_sequence_guard();
ALTER TABLE runs ENABLE ALWAYS TRIGGER runs_reset_sequence;
