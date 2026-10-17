DO $rollback$
BEGIN
    IF EXISTS (SELECT 1 FROM loop_scopes)
    THEN
        RAISE EXCEPTION 'loop progress is retained; keep a compatible reader until affected runs drain'
            USING ERRCODE = 'object_not_in_prerequisite_state';
    END IF;
END
$rollback$;

ALTER TABLE steps DROP CONSTRAINT steps_waiting_kind_valid;
ALTER TABLE steps ADD CONSTRAINT steps_waiting_kind_valid CHECK (
    waiting_kind IS NULL OR waiting_kind IN
    ('TIMER', 'EVENT', 'BRANCHES', 'CHILD', 'SAGA', 'RECONCILIATION', 'OPERATOR', 'COLLECTION')) NOT VALID;

DROP TRIGGER loop_checkpoints_append_only ON loop_checkpoints;
DROP INDEX loop_timers_due_idx;
DROP TABLE loop_timers;
DROP TABLE loop_checkpoints;
DROP INDEX loop_iterations_active_idx;
DROP TABLE loop_iterations;
DROP TABLE loop_scopes;
