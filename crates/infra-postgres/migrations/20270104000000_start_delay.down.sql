-- Rollback is an operator/quiesced-service operation. A delayed run, and the journal of one, say what
-- the engine of this version understood (`run.start_delayed`, `run.start_released`): rolling back never
-- destroys either, so it refuses while any exists.
LOCK TABLE runs, run_events IN SHARE ROW EXCLUSIVE MODE;
DO $guard$
BEGIN
    IF EXISTS (SELECT 1 FROM runs WHERE start_delayed_until IS NOT NULL)
       OR EXISTS (SELECT 1 FROM run_events WHERE event_type IN ('run.start_delayed', 'run.start_released')) THEN
        RAISE EXCEPTION 'delayed starts are retained; keep a compatible reader'
            USING ERRCODE = 'object_not_in_prerequisite_state';
    END IF;
END $guard$;

DROP TRIGGER runs_start_delay ON runs;
DROP FUNCTION runs_start_delay_guard();
DROP INDEX runs_start_delayed_idx;
ALTER TABLE runs
    DROP CONSTRAINT runs_start_delay_bounded,
    DROP COLUMN start_delayed_until;
