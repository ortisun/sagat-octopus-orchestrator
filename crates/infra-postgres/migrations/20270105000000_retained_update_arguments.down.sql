-- Rollback is an operator/quiesced-service operation. A receipt that points at retained arguments says what the
-- engine of this version kept for a reset: rolling back never destroys that, so it refuses while any exists.
-- `runs` first: the test of the leaf certification rollback waits for the statement that takes this lock
LOCK TABLE runs, operation_update_receipts IN SHARE ROW EXCLUSIVE MODE;
DO $guard$
BEGIN
    IF EXISTS (SELECT 1 FROM operation_update_receipts WHERE arguments_digest IS NOT NULL) THEN
        RAISE EXCEPTION 'retained update arguments are kept; keep a compatible reader'
            USING ERRCODE = 'object_not_in_prerequisite_state';
    END IF;
END $guard$;

ALTER TABLE operation_update_receipts
    DROP CONSTRAINT operation_update_receipts_arguments,
    DROP CONSTRAINT operation_update_receipts_arguments_fk,
    DROP COLUMN arguments_digest;
