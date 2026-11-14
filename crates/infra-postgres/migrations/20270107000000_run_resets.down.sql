-- Rollback is an operator/quiesced-service operation. The register of resets is evidence of which run was
-- made from which point of which, by whom and why: rolling back never destroys it.
-- `runs` first: the test of the leaf certification rollback waits for the statement that takes this lock
LOCK TABLE runs, run_resets IN ACCESS EXCLUSIVE MODE;
DO $guard$
BEGIN
    IF EXISTS (SELECT 1 FROM run_resets) THEN
        RAISE EXCEPTION 'the register of resets is retained; keep a compatible reader'
            USING ERRCODE = 'object_not_in_prerequisite_state';
    END IF;
END $guard$;

DROP TABLE run_resets;
DROP FUNCTION run_resets_guard();
