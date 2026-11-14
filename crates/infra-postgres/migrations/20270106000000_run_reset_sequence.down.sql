-- Rollback is an operator/quiesced-service operation. A run that a reset created keeps saying so, and its journal
-- says which of its events are restored: dropping the column would lose that evidence, so the rollback refuses
-- while any run carries it.
-- `runs` first: the test of the leaf certification rollback waits for the statement that takes this lock
LOCK TABLE runs IN SHARE ROW EXCLUSIVE MODE;
DO $guard$
BEGIN
    IF EXISTS (SELECT 1 FROM runs WHERE reset_sequence IS NOT NULL) THEN
        RAISE EXCEPTION 'runs created by a reset are retained; keep a compatible reader'
            USING ERRCODE = 'object_not_in_prerequisite_state';
    END IF;
END $guard$;

DROP TRIGGER runs_reset_sequence ON runs;
DROP FUNCTION runs_reset_sequence_guard();
ALTER TABLE runs
    DROP CONSTRAINT runs_reset_sequence_valid,
    DROP COLUMN reset_sequence;
